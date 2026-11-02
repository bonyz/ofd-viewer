// OFD 查看器入口
mod app;
mod i18n;
mod ofd;
mod render;

use eframe::egui;

/// macOS 下接管 Finder 双击文件的 odoc AppleEvent。
/// winit 不处理 kAEOpenDocuments，打包成 .app 后双击文件只会启动应用不会打开，
/// 这里用 Carbon AEInstallEventHandler 装处理器（与 GTK macOS 后端同款做法），
/// 把路径推入队列，UI 每帧 drain 后加载。
#[cfg(target_os = "macos")]
mod mac_open {
    use std::sync::{Mutex, OnceLock};

    use objc2::{define_class, msg_send, AnyThread};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, Sel};
    use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager};

    use crate::egui;

    static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static UI_CTX: OnceLock<egui::Context> = OnceLock::new();
    static HANDLER: OnceLock<Retained<OpenDocHandler>> = OnceLock::new();
    /// 我们 handler 对象的指针（guard 用来放行自己的注册）
    static HANDLER_PTR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// 安装 odoc 处理器。
    /// 冷启动（双击文件）时 AppKit 会在 finishLaunching 中注册它自己的 odoc
    /// 处理器并覆盖先装的（后注册替换前者），事件被吞。
    /// 对策：swizzle 掉 AppKit 用来注册的 `setEventHandler:andSelector:
    /// forEventClass:andEventID:`——凡注册 ('aevt','odoc') 且目标不是我们的，
    /// 直接丢弃；其余调用转发原实现。这样无论 AppKit 何时注册，我们的都保留。
    pub fn install_once() {
        let handler = match HANDLER.get() {
            Some(h) => h.clone(),
            None => {
                let h: Retained<OpenDocHandler> =
                    unsafe { msg_send![OpenDocHandler::alloc(), init] };
                let _ = HANDLER.set(h.clone());
                h
            }
        };
        HANDLER_PTR.store(
            &*handler as *const OpenDocHandler as *const AnyObject as usize,
            std::sync::atomic::Ordering::SeqCst,
        );
        let aem = NSAppleEventManager::sharedAppleEventManager();
        let sel = Sel::register(c"handleOpenDoc:withReplyEvent:");
        unsafe {
            let receiver = &*aem as *const NSAppleEventManager as *const AnyObject;
            let target = &*handler as *const OpenDocHandler as *const AnyObject;
            let _: () = msg_send![receiver,
                setEventHandler: target,
                andSelector: sel,
                forEventClass: 0x6165_7674_u32,
                andEventID: 0x6F64_6F63_u32,
            ];
        }
    }

    /// odoc 直接参数三种形态统一转 POSIX 路径：
    /// file://URL（含 %XX）、HFS 冒号路径（"Macintosh HD:Users:..."）、POSIX 路径
    fn normalize_path(raw: &str) -> String {
        let s = raw.trim();
        if let Some(rest) = s.strip_prefix("file://") {
            // 去掉可选的 host（file://localhost/...）
            let rest = rest.strip_prefix("localhost").unwrap_or(rest);
            return percent_decode(rest);
        }
        if s.contains(':') && !s.starts_with('/') {
            // HFS 路径：去掉卷名，':' 换 '/'
            let trimmed = s.splitn(2, ':').nth(1).unwrap_or(s);
            return trimmed.replace(':', "/");
        }
        s.to_string()
    }

    fn percent_decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                if let Ok(v) = u8::from_str_radix(
                    std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"),
                    16,
                ) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn push_opened(tag: &str, raw: String) {
        let path = normalize_path(&raw);
        eprintln!("[ofd-viewer] open via {tag}: {}", path);
        if let Ok(mut q) = OPENED.lock() {
            q.push(path);
        }
        if let Some(ctx) = UI_CTX.get() {
            ctx.request_repaint();
        }
    }

    pub fn set_ui_ctx(ctx: egui::Context) {
        let _ = UI_CTX.set(ctx);
    }

    /// UI 线程每帧调用：取走所有待打开路径
    pub fn drain_opened() -> Vec<String> {
        match OPENED.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => Vec::new(),
        }
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "OfdOpenDocHandler"]
        struct OpenDocHandler;

        impl OpenDocHandler {
            #[unsafe(method(handleOpenDoc:withReplyEvent:))]
            unsafe fn handle_open_doc(&self, event: &AnyObject, _reply: &AnyObject) {
                // keyDirectObject '----'：odoc 事件的直接参数是文件描述符
                // （binding 未导出 paramDescriptorForKeyword，走 msg_send）
                const KEY_DIRECT_OBJECT: u32 = 0x2D2D_2D2D;
                let param: Option<Retained<NSAppleEventDescriptor>> =
                    unsafe { msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT] };
                if let Some(param) = param {
                    if let Some(s) = param.stringValue() {
                        // stringValue 对文件描述符返回 POSIX 路径
                        push_opened("AE-handler", s.to_string());
                    }
                }
            }
        }
    );

    /// Swizzle NSAppleEventManager 的 setEventHandler:andSelector:
    /// forEventClass:andEventID:：AppKit 注册 ('aevt','odoc') 时直接忽略，
    /// 其余照常转发。必须在 main 最开头（任何 AE 注册发生前）执行一次。
    pub fn guard_aem_odoc_registration() {
        use objc2::ffi::{class_getInstanceMethod, method_getImplementation, method_setImplementation};
        use objc2::runtime::{AnyClass, Imp};

        thread_local! {
            static ORIGINAL_AEM_SET_HANDLER: std::cell::Cell<Option<Imp>> =
                const { std::cell::Cell::new(None) };
        }

        extern "C-unwind" fn guarded_set_handler(
            this: &AnyObject,
            _cmd: Sel,
            handler: *const AnyObject,
            selector: Sel,
            cls: u32,
            id: u32,
        ) {
            // ('aevt'=0x61657674, 'odoc'=0x6F646F63)：odoc 注册——放行我们自己的，
            // 拦截 AppKit 内置的（它会在 finishLaunching 时覆盖我们先装的注册）
            let ours = HANDLER_PTR.load(std::sync::atomic::Ordering::SeqCst);
            if cls == 0x6165_7674 && id == 0x6F64_6F63 && handler as usize != ours {
                return;
            }
            let orig = ORIGINAL_AEM_SET_HANDLER.with(|c| c.get());
            if let Some(orig) = orig {
                unsafe {
                    let f: unsafe extern "C-unwind" fn(&AnyObject, Sel, *const AnyObject, Sel, u32, u32) =
                        std::mem::transmute(orig);
                    f(this, _cmd, handler, selector, cls, id);
                }
            }
        }

        unsafe {
            let Some(aem_cls) = AnyClass::get(c"NSAppleEventManager") else { return };
            let cls_mut = aem_cls as *const AnyClass as *mut AnyClass;
            let sel = Sel::register(c"setEventHandler:andSelector:forEventClass:andEventID:");
            let method = class_getInstanceMethod(cls_mut, sel);
            if method.is_null() {
                eprintln!("[ofd-viewer] guard: AEM method not found");
                return;
            }
            let orig = method_getImplementation(method);
            ORIGINAL_AEM_SET_HANDLER.with(|c| c.set(orig));
            let new_imp: Imp = std::mem::transmute::<
                unsafe extern "C-unwind" fn(&AnyObject, Sel, *const AnyObject, Sel, u32, u32),
                unsafe extern "C-unwind" fn(),
            >(guarded_set_handler);
            method_setImplementation(method, new_imp);
            eprintln!("[ofd-viewer] guard: swizzle installed");
        }
    }

    /// 给指定类补上文件打开的代理方法（幂等）：
    /// - application:openFile:（POSIX 路径，单个）
    /// - application:openFiles:（路径数组）
    /// - application:open:（NSURL 数组，现代回调）
    /// AppKit 在启动时把 LS 的 open-document 请求转成这些代理回调。
    fn add_open_methods(cls: *mut objc2::runtime::AnyClass) {
        use objc2::ffi::{class_addMethod, class_getInstanceMethod};
        use objc2::runtime::{AnyClass, Bool, Imp};
        use objc2_foundation::{NSArray, NSURL, NSString};

        extern "C-unwind" fn app_open_file(
            _this: &AnyObject,
            _cmd: Sel,
            _sender: &AnyObject,
            filename: &NSString,
        ) -> Bool {
            push_opened("delegate-openFile", filename.to_string());
            Bool::YES
        }

        extern "C-unwind" fn app_open_files(
            _this: &AnyObject,
            _cmd: Sel,
            _sender: &AnyObject,
            filenames: &NSArray<NSString>,
        ) -> Bool {
            for i in 0..filenames.count() {
                push_opened("delegate-openFiles", filenames.objectAtIndex(i).to_string());
            }
            Bool::YES
        }

        extern "C-unwind" fn app_open_urls(
            _this: &AnyObject,
            _cmd: Sel,
            _sender: &AnyObject,
            urls: &NSArray<NSURL>,
        ) {
            for i in 0..urls.count() {
                if let Some(s) = urls.objectAtIndex(i).absoluteString() {
                    push_opened("delegate-openURLs", s.to_string());
                }
            }
        }

        unsafe {
            let mut patched = 0;
            let sels: [(Sel, Imp, *const std::ffi::c_char); 3] = [
                (
                    Sel::register(c"application:openFile:"),
                    std::mem::transmute::<
                        unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, &NSString) -> Bool,
                        unsafe extern "C-unwind" fn(),
                    >(app_open_file),
                    c"c@:@@".as_ptr(),
                ),
                (
                    Sel::register(c"application:openFiles:"),
                    std::mem::transmute::<
                        unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, &NSArray<NSString>) -> Bool,
                        unsafe extern "C-unwind" fn(),
                    >(app_open_files),
                    c"c@:@@".as_ptr(),
                ),
                (
                    Sel::register(c"application:open:"),
                    std::mem::transmute::<
                        unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, &NSArray<NSURL>),
                        unsafe extern "C-unwind" fn(),
                    >(app_open_urls),
                    c"v@:@@".as_ptr(),
                ),
            ];
            for (sel, imp, types) in sels {
                if class_getInstanceMethod(cls, sel).is_null() {
                    if class_addMethod(cls, sel, imp, types).into() {
                        patched += 1;
                    }
                }
            }
            if patched > 0 {
                eprintln!("[ofd-viewer] delegate patch: {patched} methods added");
            }
        }
    }

    /// Swizzle NSApplication 的 setDelegate:：winit 设置应用代理的那一刻，
    /// 立刻给代理类补上文件打开方法。此时点早于 finishLaunching 与任何
    /// open-document 派发，是唯一确定能赶上冷启动事件的时机。
    pub fn swizzle_nsapp_set_delegate() {
        use objc2::ffi::{class_getInstanceMethod, method_getImplementation, method_setImplementation};
        use objc2::runtime::{AnyClass, AnyObject, Imp};

        thread_local! {
            static ORIGINAL_SET_DELEGATE: std::cell::Cell<Option<Imp>> =
                const { std::cell::Cell::new(None) };
        }

        extern "C-unwind" fn hooked_set_delegate(
            this: &AnyObject,
            _cmd: Sel,
            delegate: *const AnyObject,
        ) {
            unsafe {
                if !delegate.is_null() {
                    let cls = objc2::ffi::object_getClass(
                        delegate as *const AnyObject as *const objc2::runtime::AnyObject,
                    );
                    if !cls.is_null() {
                        add_open_methods(cls as *mut objc2::runtime::AnyClass);
                    }
                }
            }
            if let Some(orig) = ORIGINAL_SET_DELEGATE.with(|c| c.get()) {
                unsafe {
                    let f: unsafe extern "C-unwind" fn(&AnyObject, Sel, *const AnyObject) =
                        std::mem::transmute(orig);
                    f(this, _cmd, delegate);
                }
            }
        }

        unsafe {
            let nsapp_cls = AnyClass::get(c"NSApplication");
            let Some(nsapp_cls) = nsapp_cls else { return };
            let cls_mut = nsapp_cls as *const AnyClass as *mut AnyClass;
            let sel = Sel::register(c"setDelegate:");
            let method = class_getInstanceMethod(cls_mut, sel);
            if method.is_null() {
                return;
            }
            let orig = method_getImplementation(method);
            ORIGINAL_SET_DELEGATE.with(|c| c.set(orig));
            let new_imp: Imp = std::mem::transmute::<
                unsafe extern "C-unwind" fn(&AnyObject, Sel, *const AnyObject),
                unsafe extern "C-unwind" fn(),
            >(hooked_set_delegate);
            method_setImplementation(method, new_imp);
            eprintln!("[ofd-viewer] setDelegate swizzle installed");
        }
    }
}

/// 无头渲染：解析 + 渲染为 PNG（150DPI），out 为空时输出到源文件同名 .png
fn headless_render(path: &str, out: Option<&str>) -> anyhow::Result<usize> {
    let doc = ofd::open(path)?;
    let pages = render::render_doc(&doc, 150.0)?;
    let stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("ofd")
        .to_string();
    let n = pages.len();
    for (i, pm) in pages.iter().enumerate() {
        let file = match out {
            Some(o) if n == 1 => o.to_string(),
            Some(o) => format!("{}_p{}.png", o.trim_end_matches(".png"), i + 1),
            None if n == 1 => format!("{}.png", stem),
            None => format!("{}_p{}.png", stem, i + 1),
        };
        let png = pm.encode_png()?;
        std::fs::write(&file, png)?;
        println!("{}", file);
    }
    Ok(n)
}

fn main() -> eframe::Result<()> {
    // macOS：先 swizzle 掉 AppKit 对 odoc 处理器的覆盖注册（冷启动双击文件时
    // AppKit 会在 finishLaunching 中注册内置处理器吞掉事件），再注册我们自己的
    #[cfg(target_os = "macos")]
    {
        mac_open::guard_aem_odoc_registration();
        mac_open::swizzle_nsapp_set_delegate();
        mac_open::install_once();
    }

    // 命令行可传一个 .ofd 路径直接打开
    let args: Vec<String> = std::env::args().collect();

    // 无头渲染调试：ofd-viewer --render <file.ofd> [out.png]
    // 解析+渲染整个文档并输出 PNG，不走 GUI，便于脚本化排查
    if args.get(1).map(|s| s.as_str()) == Some("--render") {
        let path = args.get(2).expect("用法: ofd-viewer --render <file.ofd> [out.png]");
        return match headless_render(path, args.get(3).map(|s| s.as_str())) {
            Ok(n) => {
                println!("已渲染 {} 页", n);
                Ok(())
            }
            Err(e) => {
                eprintln!("渲染失败: {:#}", e);
                std::process::exit(1);
            }
        };
    }

    let mut viewer = app::ViewerApp::new();
    if let Some(path) = args.get(1) {
        viewer.open_path(path);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([950.0, 1150.0])
            .with_min_inner_size([420.0, 420.0]),
        ..Default::default()
    };

    eframe::run_native(
        "OFD 查看器",
        options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx); // 注入中文字体，否则界面按钮乱码
            #[cfg(target_os = "macos")]
            mac_open::set_ui_ctx(cc.egui_ctx.clone());
            Ok(Box::new(viewer))
        }),
    )
}

/// 注入界面字体：macOS 优先 SF Pro / Helvetica（拉丁）+ Menlo（等宽），
/// 中文统一走 CJK 候选列表；并把默认字号调小一档
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // CJK 兜底（先插入，随后拉丁字体插到最前）
    for path in render::text::default_font_candidates() {
        let Ok(data) = std::fs::read(path) else { continue };
        // egui 底层用 ab_glyph 光栅化，先验证可解析再注入，
        // 避免 PingFang.ttc（CFF2 轮廓）这类读得了但解析不了的字体混进来
        if ab_glyph::FontVec::try_from_vec(data.clone()).is_err() {
            continue;
        }
        fonts.font_data.insert("cjk".into(), egui::FontData::from_owned(data).into());
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "cjk".into());
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, "cjk".into());
        break;
    }

    if cfg!(target_os = "macos") {
        // 界面拉丁字体：SF Pro 是系统字体但为 CFF2 变体轮廓，ab_glyph 可能解析
        // 不了，解析失败自动退 Helvetica；插到最前使拉丁字符走 Mac 风格字体
        for path in ["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Helvetica.ttc"] {
            let Ok(data) = std::fs::read(path) else { continue };
            if ab_glyph::FontVec::try_from_vec(data.clone()).is_err() {
                continue;
            }
            fonts.font_data.insert("ui-latin".into(), egui::FontData::from_owned(data).into());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "ui-latin".into());
            break;
        }
        // 等宽用 Menlo
        if let Ok(data) = std::fs::read("/System/Library/Fonts/Menlo.ttc") {
            if ab_glyph::FontVec::try_from_vec(data.clone()).is_ok() {
                fonts.font_data.insert("ui-mono".into(), egui::FontData::from_owned(data).into());
                fonts
                    .families
                    .entry(egui::FontFamily::Monospace)
                    .or_default()
                    .insert(0, "ui-mono".into());
            }
        }
    }

    ctx.set_fonts(fonts);

    // 字号整体调小一档（egui 默认 Body/Button 14.5、Heading 18，观感偏大）
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (egui::TextStyle::Heading, egui::FontId::proportional(16.0)),
        (egui::TextStyle::Body, egui::FontId::proportional(12.5)),
        (egui::TextStyle::Button, egui::FontId::proportional(12.5)),
        (egui::TextStyle::Small, egui::FontId::proportional(9.0)),
        (egui::TextStyle::Monospace, egui::FontId::monospace(12.0)),
    ]
    .into();
    ctx.set_style(style);
}

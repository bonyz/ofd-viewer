//! 展示层：egui 桌面 GUI（打开 / 翻页 / 缩放）
use eframe::egui;
use eframe::egui::{ColorImage, TextureHandle, TextureOptions};
use tiny_skia::Pixmap;

use crate::ofd;
use crate::render;

#[derive(Clone, Copy, Debug)]
struct SearchMatch {
    page: usize,
    start_char: usize, // 页内字符序（含）
    end_char: usize,   // 页内字符序（含）
}

pub struct ViewerApp {
    file_path: Option<String>,
    pages: Vec<TextureHandle>,
    page_index: usize,
    zoom: f32,
    dpi: f32,
    status: String,
    pending_path: Option<String>,
    scroll_accum: f32,
    show_about: bool,
    // 逐字文字索引（选中/搜索用）
    doc_text: crate::render::text_index::DocTextIndex,
    page_rects: Vec<egui::Rect>,   // 当前帧各页屏幕 rect（命中用）
    sel_anchor: Option<(usize, f32, f32)>, // 框选起 (页,mm_x,mm_y)
    sel_end: Option<(usize, f32, f32)>,    // 框选止
    // 目录数据（解析保留，UI 暂未显示）
    #[allow(dead_code)]
    outlines: Vec<crate::ofd::OutlineNode>,
    #[allow(dead_code)]
    page_ids: Vec<String>,
    scroll_target: Option<usize>,  // 待滚动到的页索引
    page_input: String,
    show_outline: bool,
    // 搜索（T12）
    search_query: String,
    search_matches: Vec<SearchMatch>,
    cur_match: Option<usize>,
    show_search: bool,
    // 语言
    lang_pref: crate::i18n::LangPref,
    lang: crate::i18n::Lang,
}

impl Default for ViewerApp {
    fn default() -> Self {
        Self {
            file_path: None,
            pages: vec![],
            page_index: 0,
            zoom: 1.0,
            dpi: 150.0,
            status: crate::i18n::texts(crate::i18n::LangPref::Auto.resolve())
                .status_no_file
                .into(),
            pending_path: None,
            scroll_accum: 0.0,
            show_about: false,
            doc_text: vec![],
            page_rects: vec![],
            sel_anchor: None,
            sel_end: None,
            outlines: vec![],
            page_ids: vec![],
            scroll_target: None,
            page_input: String::new(),
            show_outline: true,
            search_query: String::new(),
            search_matches: vec![],
            cur_match: None,
            show_search: false,
            lang_pref: crate::i18n::LangPref::Auto,
            lang: crate::i18n::LangPref::Auto.resolve(),
        }
    }
}

impl ViewerApp {
    pub fn new() -> Self {
        Self::default()
    }

    /// 预置待打开路径（命令行参数）
    pub fn open_path(&mut self, path: impl Into<String>) {
        self.pending_path = Some(path.into());
    }

    fn load(&mut self, ctx: &egui::Context, path: &str) {
        self.status = format!("正在加载 {}…", short_name(path));
        self.pages.clear();

        let doc = match ofd::open(path) {
            Ok(d) => d,
            Err(e) => {
                self.status = format!("打开失败（非 OFD 或已损坏）: {}", e);
                return;
            }
        };
        let pms = match render::render_doc(&doc, self.dpi) {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("渲染失败: {}", e);
                return;
            }
        };
        let n = pms.len();
        let mut pages = Vec::with_capacity(n);
        for (i, pm) in pms.iter().enumerate() {
            pages.push(ctx.load_texture(
                format!("ofd_page_{}", i),
                pixmap_to_image(pm),
                TextureOptions::LINEAR,
            ));
        }
        // 构建逐字文字索引（选中/搜索用）
        let doc_text = match render::text_index::build_index(&doc, self.dpi) {
            Ok(i) => i,
            Err(_) => Vec::new(),
        };
        self.pages = pages;
        self.doc_text = doc_text;
        self.sel_anchor = None;
        self.sel_end = None;
        self.outlines = doc.outlines.clone();
        self.page_ids = doc.page_ids.clone();
        self.scroll_target = None;
        self.page_input.clear();
        self.page_index = 0;
        self.zoom = 1.0;
        self.file_path = Some(path.to_string());
        self.status = if n > 0 {
            format!(
                "{}：{}（{} {}）",
                crate::i18n::pick(self.lang, "已加载", "Loaded"),
                short_name(path),
                n,
                crate::i18n::pick(self.lang, "页", "pages")
            )
        } else {
            crate::i18n::pick(self.lang, "文档为空（0 页）", "Empty document (0 pages)").into()
        };
    }

    // ---------- 框选辅助 ----------

    /// 屏幕坐标 -> (页索引, mm_x, mm_y)
    fn hit_point(&self, pos: egui::Pos2) -> Option<(usize, f32, f32)> {
        for (pi, rect) in self.page_rects.iter().enumerate() {
            if !rect.contains(pos) {
                continue;
            }
            let idx = self.doc_text.get(pi)?;
            if idx.page_h_mm <= 0.0 {
                continue;
            }
            let s = rect.height() / idx.page_h_mm;
            let mx = (pos.x - rect.left()) / s;
            let my = (pos.y - rect.top()) / s;
            return Some((pi, mx, my));
        }
        None
    }

    /// 框选区域内相交的文字（按视觉 y,x 排序，跨行插换行）
    fn selected_text(&self) -> Option<String> {
        let a = self.sel_anchor?;
        let e = self.sel_end?;
        let pi = a.0;
        let (x1, x2) = (a.1.min(e.1), a.1.max(e.1));
        let (y1, y2) = (a.2.min(e.2), a.2.max(e.2));
        let idx = self.doc_text.get(pi)?;
        let mut hits: Vec<_> = idx
            .chars
            .iter()
            .filter(|c| {
                c.x_mm < x2 && (c.x_mm + c.w_mm) > x1 && c.y_mm < y2 && (c.y_mm + c.h_mm) > y1
            })
            .collect();
        hits.sort_by(|a, b| {
            a.y_mm
                .partial_cmp(&b.y_mm)
                .unwrap()
                .then(a.x_mm.partial_cmp(&b.x_mm).unwrap())
        });
        let mut out = String::new();
        let mut last_y: Option<f32> = None;
        for c in hits {
            if let Some(ly) = last_y {
                if (c.y_mm - ly).abs() > c.h_mm * 0.5 {
                    out.push('\n');
                }
            }
            out.push(c.ch);
            last_y = Some(c.y_mm);
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    // ---------- 目录 / 页码跳转 ----------

    fn show_outline_panel(&mut self, ui: &mut egui::Ui) {
        let t = crate::i18n::texts(self.lang);
        ui.heading(t.panel_title);
        ui.separator();
        let n = self.pages.len();
        ui.label(format!(
            "{}{}{}",
            crate::i18n::pick(self.lang, "共 ", ""),
            n.max(1),
            crate::i18n::pick(self.lang, " 页", " pages")
        ));
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.page_input)
                    .hint_text(t.page_hint)
                    .desired_width(60.0),
            );
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (n > 0 && ui.button(t.go).clicked()) || enter {
                if let Ok(p) = self.page_input.trim().parse::<usize>() {
                    if p >= 1 && p <= n {
                        self.scroll_target = Some(p - 1);
                        self.page_input.clear();
                    }
                }
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.add_enabled(n > 0, egui::Button::new(t.first)).clicked() {
                self.scroll_target = Some(0);
            }
            if ui.add_enabled(n > 0, egui::Button::new(t.last)).clicked() {
                self.scroll_target = Some(n.saturating_sub(1));
            }
        });
    }

    // ---------- Ctrl+F 搜索 ----------

    fn run_search(&mut self) {
        let raw = self.search_query.clone();
        let hits = crate::render::text_index::search(&self.doc_text, &raw);
        self.search_matches = hits
            .into_iter()
            .map(|(page, start_char, end_char)| SearchMatch {
                page,
                start_char,
                end_char,
            })
            .collect();
        if self.search_matches.is_empty() {
            self.cur_match = None;
        } else {
            self.cur_match = Some(0);
            self.scroll_target = Some(self.search_matches[0].page);
        }
    }

    fn next_match(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let n = self.search_matches.len();
        let i = self.cur_match.map(|i| (i + 1) % n).unwrap_or(0);
        self.cur_match = Some(i);
        self.scroll_target = Some(self.search_matches[i].page);
    }

    fn prev_match(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let n = self.search_matches.len();
        let i = self
            .cur_match
            .map(|i| if i == 0 { n - 1 } else { i - 1 })
            .unwrap_or(n - 1);
        self.cur_match = Some(i);
        self.scroll_target = Some(self.search_matches[i].page);
    }
}

fn short_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// 复制到系统剪贴板（Windows API 直写 CF_UNICODETEXT，绕过 arboard/egui 后端）
#[cfg(windows)]
fn copy_to_clipboard(_ctx: &egui::Context, txt: &str) -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let mut wide: Vec<u16> = txt.encode_utf16().collect();
        wide.push(0u16);
        let size = wide.len() * 2;
        let mut ok = false;
        if let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, size) {
            let ptr = GlobalLock(hmem);
            if !ptr.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr as *mut u8, size);
                let _ = GlobalUnlock(hmem);
                // HGLOBAL 与 HANDLE 同布局，所有权转交剪贴板
                let handle = HANDLE(hmem.0);
                ok = SetClipboardData(13, Some(handle)).is_ok();
            }
        }
        let _ = CloseClipboard();
        ok
    }
}

/// 非 Windows：走 egui 的 copied_text 输出，eframe(winit) 在帧末写入系统剪贴板
#[cfg(not(windows))]
fn copy_to_clipboard(ctx: &egui::Context, txt: &str) -> bool {
    ctx.copy_text(txt.to_owned());
    true
}

/// tiny-skia Pixmap(premultiplied) -> egui ColorImage(unmultiplied)
fn pixmap_to_image(pm: &Pixmap) -> ColorImage {
    let pixels = pm
        .pixels()
        .iter()
        .map(|p| {
            let a = p.alpha();
            if a == 0 {
                egui::Color32::TRANSPARENT
            } else {
                let f = 255.0 / a as f32;
                egui::Color32::from_rgba_unmultiplied(
                    (p.red() as f32 * f).min(255.0) as u8,
                    (p.green() as f32 * f).min(255.0) as u8,
                    (p.blue() as f32 * f).min(255.0) as u8,
                    a,
                )
            }
        })
        .collect();
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    ColorImage {
        size: [w, h],
        pixels,
        source_size: egui::Vec2::new(w as f32, h as f32),
    }
}

impl eframe::App for ViewerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // macOS：第一帧安装 odoc AppleEvent 处理器 + 取走 Finder 双击的待打开文件
        #[cfg(target_os = "macos")]
        {
            crate::mac_open::install_once();
            for p in crate::mac_open::drain_opened() {
                self.open_path(p);
            }
        }
        // Ctrl+F 查找
        if ui.input(|i| i.key_pressed(egui::Key::F) && i.modifiers.ctrl) {
            self.show_search = true;
        }
        // Ctrl+C 复制框选文本
        // 根因：egui-winit 后端把 Ctrl+C 转成 Event::Copy 后直接 return，
        // 不再产生 Event::Key{Key::C}，所以 key_pressed(Key::C) 永远为 false。
        // 正确做法是直接在 input events 里检测 Event::Copy。
        // Event::Copy 不会被 TextEdit 从 events 中移除（TextEdit 用 filtered_events 只读），
        // 因此即使页码输入框有焦点也能在本帧检测到。
        let copy_requested = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
        if copy_requested {
            match self.selected_text() {
                Some(txt) => {
                    if copy_to_clipboard(&ctx, &txt) {
                        self.status = format!(
                            "{}：{}…",
                            crate::i18n::pick(self.lang, "已复制", "Copied"),
                            txt.chars().take(16).collect::<String>()
                        );
                    } else {
                        self.status =
                            crate::i18n::pick(self.lang, "复制失败（剪贴板）", "Copy failed").into();
                    }
                }
                None => self.status =
                    crate::i18n::pick(self.lang, "未选中文字，先框选区域", "No selection").into(),
            }
        }
        if let Some(p) = self.pending_path.take() {
            self.load(&ctx, &p);
        }
        // 拖拽打开 .ofd
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(f) = dropped.into_iter().find_map(|d| d.path) {
            if f.extension().map_or(false, |e| e == "ofd") {
                self.pending_path = Some(f.to_string_lossy().into_owned());
            }
        }
        // 关于对话框
        if self.show_about {
            let t = crate::i18n::texts(self.lang);
            egui::Window::new(t.about)
                .open(&mut self.show_about)
                .resizable(false)
                .default_pos(egui::pos2(120.0, 80.0))
                .show(&ctx, |ui| {
                    ui.heading(format!("{}  v0.3.0", t.title));
                    ui.label(t.subtitle);
                    ui.separator();
                    ui.label(t.author);
                    ui.label(t.copyright);
                    ui.label(format!("{}：{}", t.homepage_label, t.homepage_url));
                    ui.separator();
                    ui.label(t.about_shortcuts);
                });
        }

        // 查找对话框（默认位置放工具栏下方，避免盖住工具栏按钮）
        if self.show_search {
            let mut open = self.show_search;
            let t = crate::i18n::texts(self.lang);
            egui::Window::new(t.find)
                .open(&mut open)
                .resizable(false)
                // 锚定在工具栏下方（每帧生效）：default_pos 会被 eframe 持久化的
                // 旧窗口位置顶掉，导致窗口弹回左上角盖住工具栏
                .anchor(egui::Align2::LEFT_TOP, [16.0, 48.0])
                .show(&ctx, |ui| {
                    ui.horizontal(|ui| {
                        let resp = ui.text_edit_singleline(&mut self.search_query);
                        if resp.changed() {
                            self.run_search();
                        }
                        if resp.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        {
                            self.next_match();
                        }
                        if ui.button(t.prev).clicked() {
                            self.prev_match();
                        }
                        if ui.button(t.next).clicked() {
                            self.next_match();
                        }
                    });
                    let total = self.search_matches.len();
                    let cur = self.cur_match.map(|i| i + 1).unwrap_or(0);
                    ui.label(format!(
                        "{} {}/{}",
                        crate::i18n::pick(self.lang, "匹配", "Match"),
                        cur,
                        total
                    ));
                });
            self.show_search = open;
        }

        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            let t = crate::i18n::texts(self.lang);
            ui.horizontal(|ui| {
                if ui.button(t.open).clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("OFD", &["ofd"])
                        .pick_file()
                    {
                        self.pending_path = Some(path.to_string_lossy().into_owned());
                    }
                }
                if ui.button(t.about).clicked() {
                    self.show_about = true;
                }
                if ui
                    .button(if self.show_outline {
                        t.hide_panel
                    } else {
                        t.show_panel
                    })
                    .clicked()
                {
                    self.show_outline = !self.show_outline;
                }
                // 语言切换
                ui.menu_button(t.lang, |ui| {
                    use crate::i18n::LangPref;
                    for p in [LangPref::Auto, LangPref::Zh, LangPref::En] {
                        if ui
                            .radio_value(&mut self.lang_pref, p, p.label(self.lang))
                            .changed()
                        {
                            self.lang = self.lang_pref.resolve();
                        }
                    }
                });
                ui.separator();

                let n = self.pages.len();
                ui.add_enabled_ui(n > 0, |ui| {
                    ui.label(format!(
                        "{} {}{}",
                        n.max(1),
                        crate::i18n::pick(self.lang, "页", "pages"),
                        crate::i18n::pick(
                            self.lang,
                            "（滚轮滚动 · PageUp/PageDown 翻页 · Ctrl+滚轮缩放）",
                            " · scroll / PageUp-Down / Ctrl+wheel zoom"
                        )
                    ));
                    ui.separator();
                    ui.label(t.zoom);
                    if ui.button("−").clicked() {
                        self.zoom = (self.zoom * 0.8).max(0.1);
                    }
                    ui.label(format!("{:.0}%", self.zoom * 100.0));
                    if ui.button("＋").clicked() {
                        self.zoom = (self.zoom * 1.25).min(8.0);
                    }
                    if ui.button(t.fit).clicked() {
                        self.zoom = 1.0;
                    }
                    ui.separator();
                    if ui.button(t.export_pdf).clicked() {
                        if let Some(p) = self.file_path.clone() {
                            if let Some(save) =
                                rfd::FileDialog::new().add_filter("PDF", &["pdf"]).save_file()
                            {
                                let dpi = self.dpi;
                                let s = save.to_string_lossy().to_string();
                                self.status = format!(
                                    "{} {}…",
                                    crate::i18n::pick(self.lang, "正在导出", "Exporting"),
                                    save.display()
                                );
                                match render::pdf_export::export_pdf(&p, dpi, &s) {
                                    Ok(_) => self.status = format!(
                                        "{}{}",
                                        crate::i18n::pick(self.lang, "已导出 PDF：", "Exported PDF: "),
                                        save.display()
                                    ),
                                    Err(e) => self.status = format!(
                                        "{}{}",
                                        crate::i18n::pick(self.lang, "导出失败：", "Export failed: "),
                                        e
                                    ),
                                }
                            }
                        }
                    }
                    if ui.button(t.copy).clicked() {
                        match self.selected_text() {
                            Some(txt) => {
                                if copy_to_clipboard(&ctx, &txt) {
                                    self.status = format!(
                                        "{}：{}…",
                                        crate::i18n::pick(self.lang, "已复制", "Copied"),
                                        txt.chars().take(16).collect::<String>()
                                    );
                                } else {
                                    self.status = crate::i18n::pick(
                                        self.lang,
                                        "复制失败（剪贴板）",
                                        "Copy failed (clipboard)",
                                    )
                                    .into();
                                }
                            }
                            None => self.status = crate::i18n::pick(
                                self.lang,
                                "未选中文字（先框选区域）",
                                "No selection (drag to select first)",
                            )
                            .into(),
                        }
                    }
                    #[cfg(windows)]
                    if ui.button(t.print).clicked() {
                        if let Some(p) = self.file_path.clone() {
                            self.status = crate::i18n::pick(
                                self.lang,
                                "正在准备打印（300DPI 渲染）…",
                                "Preparing print (300DPI)...",
                            )
                            .into();
                            let pms = ofd::open(&p)
                                .ok()
                                .and_then(|d| render::render_doc(&d, 300.0).ok());
                            match pms {
                                Some(pages) => match render::print::print_pixmaps(&pages) {
                                    Ok(_) => self.status =
                                        crate::i18n::pick(self.lang, "打印流程结束", "Print finished")
                                            .into(),
                                    Err(e) => self.status = format!(
                                        "{}{}",
                                        crate::i18n::pick(self.lang, "打印失败：", "Print failed: "),
                                        e
                                    ),
                                },
                                None => self.status = crate::i18n::pick(
                                    self.lang,
                                    "打印准备失败（解析/渲染失败）",
                                    "Print prepare failed",
                                )
                                .into(),
                            }
                        }
                    }
                });
            });
        });

        egui::Panel::bottom("statusbar").show_inside(ui, |ui| {
            ui.label(&self.status);
        });

        // 左侧目录 / 书签：与工具栏/状态栏一样用 show_inside 注册到根 ui。
        // egui 0.34 的面板 show_inside 会顺序收缩可用区域（cursor.min.x = 面板右缘），
        // 后面的 CentralPanel::show_inside 取 available_rect_before_wrap 自动占剩余部分。
        // 不能用 show(&ctx)：CentralPanel::show_inside 不感知 ctx 级面板，会被白底盖住。
        if self.show_outline && !self.pages.is_empty() {
            egui::SidePanel::left("outline")
                .resizable(true)
                .default_width(200.0)
                .width_range(120.0..=400.0)
                .show_inside(ui, |ui| {
                    self.show_outline_panel(ui);
                });
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            if self.pages.is_empty() {
                let t = crate::i18n::texts(self.lang);
                ui.vertical_centered(|ui| {
                    ui.add_space(120.0);
                    ui.heading(t.empty_title);
                    ui.add_space(16.0);
                    ui.label(t.empty_hint);
                });
                return;
            }

            let avail_h = ui.available_height().max(1.0);
            let avail_w = ui.available_width().max(1.0);
            // Ctrl+滚轮缩放
            let zf = ui.input(|i| i.zoom_delta());
            if zf != 1.0 {
                self.zoom = (self.zoom * zf).clamp(0.1, 8.0);
            }
            // PageUp/PageDown 滚动一屏
            if ui.input(|i| i.key_pressed(egui::Key::PageUp)) {
                ui.input_mut(|i| i.smooth_scroll_delta.y -= avail_h);
            }
            if ui.input(|i| i.key_pressed(egui::Key::PageDown)) {
                ui.input_mut(|i| i.smooth_scroll_delta.y += avail_h);
            }

            // 命中（用上一帧 page_rects），更新选区
            let pointer = ui.input(|i| i.pointer.hover_pos());
            let pressed = ui.input(|i| i.pointer.primary_pressed());
            let down = ui.input(|i| i.pointer.primary_down());
            // 右键复制：改用 secondary_pressed（纯按下事件）。
            // 根因：secondary_clicked() 依赖 click 判定，而 has_moved_too_much_for_a_click 是
            // 全局标志——左键框选拖拽距离超阈值后会置为 true，导致后续右键 release 不被识别为 click，
            // secondary_clicked() 返回 false。secondary_pressed 只看按下，不受拖拽距离影响。
            let secondary = ui.input(|i| i.pointer.secondary_pressed());
            // 右键直接复制当前选中
            if secondary {
                if let Some(txt) = self.selected_text() {
                    if copy_to_clipboard(&ctx, &txt) {
                        self.status = format!(
                            "{}：{}…",
                            crate::i18n::pick(self.lang, "已复制", "Copied"),
                            txt.chars().take(16).collect::<String>()
                        );
                    }
                }
            }
            if pressed {
                // 只在点击页面区域时开始框选；点工具栏/侧栏不清除
                if let Some(p) = pointer {
                    if self.page_rects.iter().any(|r| r.contains(p)) {
                        let h = self.hit_point(p);
                        self.sel_anchor = h;
                        self.sel_end = h;
                        self.status = match h {
                            Some(_) => crate::i18n::pick(
                                self.lang,
                                "框选中（拖动选区域，Ctrl+C/右键复制）",
                                "Selecting (drag, Ctrl+C / right-click)",
                            )
                            .into(),
                            None => crate::i18n::pick(self.lang, "未命中页面", "Missed page").into(),
                        };
                    }
                }
            } else if down {
                if let Some(p) = pointer {
                    if self.page_rects.iter().any(|r| r.contains(p)) {
                        if let Some(h) = self.hit_point(p) {
                            self.sel_end = Some(h);
                        }
                    }
                }
            }

            let sel_a = self.sel_anchor;
            let sel_e = self.sel_end;
            let want_scroll = self.scroll_target;
            let mut new_rects: Vec<egui::Rect> = Vec::with_capacity(self.pages.len());
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .drag_to_scroll(false)
                .show(ui, |ui| {
                    for (pi, tex) in self.pages.iter().enumerate() {
                        let ts = tex.size_vec2();
                        // zoom=1 时整页适配窗口：高度适配与宽度适配取小者，
                        // 横版页（如行程单 230x133.5mm）默认不再超宽
                        let fit_h = (avail_h.min(avail_w * ts.y / ts.x)).max(1.0);
                        let target_h = fit_h * self.zoom;
                        let tw = target_h * ts.x / ts.y;
                        let inner = ui.vertical_centered(|ui| {
                            ui.add(
                                egui::Image::from_texture(tex)
                                    .fit_to_exact_size(egui::Vec2::new(tw, target_h)),
                            )
                        });
                        let img_resp = inner.inner;
                        let rect = img_resp.rect; // image 自身 rect（居中后精确左边）
                        new_rects.push(rect);
                        if want_scroll == Some(pi) {
                            ui.scroll_to_rect(rect, Some(egui::Align::TOP));
                        }
                        // 框选覆盖层：选择矩形 + 相交字高亮
                        if let (Some(a), Some(e)) = (sel_a, sel_e) {
                            if a.0 == pi {
                                if let Some(idx) = self.doc_text.get(pi) {
                                    if idx.page_h_mm > 0.0 {
                                        let s = rect.height() / idx.page_h_mm;
                                        let painter = ui.painter();
                                        let (x1, x2) = (a.1.min(e.1), a.1.max(e.1));
                                        let (y1, y2) = (a.2.min(e.2), a.2.max(e.2));
                                        // 选择矩形（半透明蓝底）
                                        let rr = egui::Rect::from_min_max(
                                            egui::pos2(rect.left() + x1 * s, rect.top() + y1 * s),
                                            egui::pos2(rect.left() + x2 * s, rect.top() + y2 * s),
                                        );
                                        painter.rect_filled(
                                            rr,
                                            0.0,
                                            egui::Color32::from_rgba_unmultiplied(80, 120, 255, 45),
                                        );
                                        // 相交字加深高亮
                                        for c in idx.chars.iter() {
                                            if c.x_mm < x2
                                                && (c.x_mm + c.w_mm) > x1
                                                && c.y_mm < y2
                                                && (c.y_mm + c.h_mm) > y1
                                            {
                                                let r = egui::Rect::from_min_size(
                                                    egui::pos2(
                                                        rect.left() + c.x_mm * s,
                                                        rect.top() + c.y_mm * s,
                                                    ),
                                                    egui::vec2(c.w_mm * s, c.h_mm * s),
                                                );
                                                painter.rect_filled(
                                                    r,
                                                    0.0,
                                                    egui::Color32::from_rgba_unmultiplied(
                                                        80, 120, 255, 90,
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // 搜索匹配覆盖层
                        if let Some(idx) = self.doc_text.get(pi) {
                            if idx.page_h_mm > 0.0 {
                                let s = rect.height() / idx.page_h_mm;
                                let painter = ui.painter();
                                for (mi, m) in self.search_matches.iter().enumerate() {
                                    if m.page != pi {
                                        continue;
                                    }
                                    let color = if Some(mi) == self.cur_match {
                                        egui::Color32::from_rgba_unmultiplied(255, 150, 0, 120)
                                    } else {
                                        egui::Color32::from_rgba_unmultiplied(255, 225, 0, 80)
                                    };
                                    for ci in m.start_char..=m.end_char {
                                        if let Some(c) = idx.chars.get(ci) {
                                            let r = egui::Rect::from_min_size(
                                                egui::pos2(
                                                    rect.left() + c.x_mm * s,
                                                    rect.top() + c.y_mm * s,
                                                ),
                                                egui::vec2(c.w_mm * s, c.h_mm * s),
                                            );
                                            painter.rect_filled(r, 0.0, color);
                                        }
                                    }
                                }
                            }
                        }
                    }
                });
            self.scroll_target = None; // 跳转已应用
            self.page_rects = new_rects;
        });
    }
}

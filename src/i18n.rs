//! 国际化：中英文，跟随系统语言（可手动切换）
use sys_locale::get_locale;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Zh,
    En,
}

/// 用户语言偏好：Auto=跟随系统，Zh/En=强制
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LangPref {
    Auto,
    Zh,
    En,
}

/// 识别系统语言（中文系统→Zh，其余→En）
pub fn detect() -> Lang {
    match get_locale() {
        Some(l) if l.to_lowercase().starts_with('z') => Lang::Zh,
        _ => Lang::En,
    }
}

impl LangPref {
    /// 解析为实际语言
    pub fn resolve(self) -> Lang {
        match self {
            LangPref::Auto => detect(),
            LangPref::Zh => Lang::Zh,
            LangPref::En => Lang::En,
        }
    }
    /// 在语言菜单里显示的选项名
    pub fn label(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (LangPref::Auto, Lang::Zh) => "跟随系统",
            (LangPref::Auto, Lang::En) => "Auto",
            (LangPref::Zh, _) => "中文",
            (LangPref::En, _) => "English",
        }
    }
}

/// 固定 UI 文字集合
pub struct Texts {
    pub title: &'static str,
    pub open: &'static str,
    pub about: &'static str,
    pub hide_panel: &'static str,
    pub show_panel: &'static str,
    pub copy: &'static str,
    pub print: &'static str,
    pub export_pdf: &'static str,
    pub zoom: &'static str,
    pub fit: &'static str,
    pub toolbar_hint: &'static str,
    pub panel_title: &'static str,
    pub first: &'static str,
    pub last: &'static str,
    pub go: &'static str,
    pub page_hint: &'static str,
    pub find: &'static str,
    pub prev: &'static str,
    pub next: &'static str,
    pub subtitle: &'static str,
    pub author: &'static str,
    pub copyright: &'static str,
    pub homepage_label: &'static str,
    pub homepage_url: &'static str,
    pub about_shortcuts: &'static str,
    pub empty_title: &'static str,
    pub empty_hint: &'static str,
    pub lang: &'static str,
    pub status_no_file: &'static str,
}

pub fn texts(lang: Lang) -> Texts {
    match lang {
        Lang::Zh => Texts {
            title: "OFD 查看器",
            open: "打开 OFD",
            about: "关于",
            hide_panel: "隐藏侧栏",
            show_panel: "显示侧栏",
            copy: "复制选中",
            print: "打印",
            export_pdf: "导出 PDF",
            zoom: "缩放",
            fit: "适应",
            toolbar_hint: "滚轮滚动 · PageUp/PageDown 翻页 · Ctrl+滚轮缩放",
            panel_title: "页码跳转",
            first: "首页",
            last: "末页",
            go: "跳转",
            page_hint: "页码",
            find: "查找",
            prev: "上一个",
            next: "下一个",
            subtitle: "纯 Rust 桌面 OFD（GB/T 33190）查看器",
            author: "作者：shaoyi1998",
            copyright: "版权 © 2026 shaoyi1998",
            homepage_label: "主页",
            homepage_url: "https://github.com/shaoyi1998/ofd-viewer",
            about_shortcuts: "滚轮滚动 · PageUp/PageDown 翻页 · Ctrl+滚轮缩放 · 拖拽 .ofd 打开",
            empty_title: "OFD 查看器",
            empty_hint: "点击左上角「打开 OFD」选择文件",
            lang: "语言",
            status_no_file: "未打开文件。点击「打开 OFD」选择文件",
        },
        Lang::En => Texts {
            title: "OFD Viewer",
            open: "Open OFD",
            about: "About",
            hide_panel: "Hide Panel",
            show_panel: "Show Panel",
            copy: "Copy",
            print: "Print",
            export_pdf: "Export PDF",
            zoom: "Zoom",
            fit: "Fit",
            toolbar_hint: "Scroll · PageUp/Down · Ctrl+Wheel to zoom",
            panel_title: "Go to Page",
            first: "First",
            last: "Last",
            go: "Go",
            page_hint: "Page",
            find: "Find",
            prev: "Previous",
            next: "Next",
            subtitle: "Pure Rust desktop OFD (GB/T 33190) viewer",
            author: "Author: shaoyi1998",
            copyright: "Copyright © 2026 shaoyi1998",
            homepage_label: "Homepage",
            homepage_url: "https://github.com/shaoyi1998/ofd-viewer",
            about_shortcuts: "Scroll · PageUp/Down · Ctrl+Wheel zoom · Drag .ofd to open",
            empty_title: "OFD Viewer",
            empty_hint: "Click \"Open OFD\" to select a file",
            lang: "Language",
            status_no_file: "No file open. Click \"Open OFD\" to select",
        },
    }
}

impl Lang {
    pub fn zh(self) -> bool {
        matches!(self, Lang::Zh)
    }
}

/// 按语言选字符串（zh/en 传字面量）
pub fn pick<'a>(lang: Lang, zh: &'a str, en: &'a str) -> &'a str {
    if lang.zh() {
        zh
    } else {
        en
    }
}

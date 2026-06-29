// OFD 查看器入口
mod app;
mod i18n;
mod ofd;
mod render;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // 命令行可传一个 .ofd 路径直接打开
    let args: Vec<String> = std::env::args().collect();
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
            Ok(Box::new(viewer))
        }),
    )
}

/// 注入中文字体（系统黑体）到 egui，解决默认字体无 CJK 导致的界面乱码
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Ok(data) = std::fs::read(r"C:\Windows\Fonts\msyh.ttc")
        .or_else(|_| std::fs::read(r"C:\Windows\Fonts\simhei.ttf"))
    {
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
    }
    ctx.set_fonts(fonts);
}

//! 渲染层：文档模型 -> RGBA 位图（Pixmap）
pub mod imgobj;
pub mod path;
pub mod pdf_export;
pub mod text;
pub mod text_index;
#[cfg(windows)]
pub mod print;

use std::collections::HashMap;

use ab_glyph::FontVec;
use anyhow::Result;
use tiny_skia::Pixmap;

use crate::ofd::{ImageData, Obj, OfdDoc, Page};

const MM_PER_INCH: f32 = 25.4;
const A4_W_MM: f32 = 210.0;
const A4_H_MM: f32 = 297.0;

/// 加载字体表：嵌入子集字体 + 系统黑体 fallback（render 与 text_index 共用）
pub fn load_font_table(doc: &OfdDoc) -> Result<(HashMap<String, FontVec>, FontVec)> {
    let fallback = text::load_default_font()?;
    let mut fonts: HashMap<String, FontVec> = HashMap::new();
    for (id, bytes) in &doc.fonts {
        if let Ok(f) = FontVec::try_from_vec(bytes.clone()) {
            fonts.insert(id.clone(), f);
        }
    }
    Ok((fonts, fallback))
}

/// 渲染整个文档（多页）
pub fn render_doc(doc: &OfdDoc, dpi: f32) -> Result<Vec<Pixmap>> {
    let (fonts, fallback) = load_font_table(doc)?;
    doc.pages
        .iter()
        .map(|p| render_page(p, dpi, &doc.images, &fonts, &fallback))
        .collect()
}

/// 渲染单页
pub fn render_page(
    page: &Page,
    dpi: f32,
    images: &HashMap<String, ImageData>,
    fonts: &HashMap<String, FontVec>,
    fallback: &FontVec,
) -> Result<Pixmap> {
    let scale = dpi / MM_PER_INCH;
    let (w_mm, h_mm) = page_size_mm(page);
    let w = ((w_mm * scale).round() as u32).max(1);
    let h = ((h_mm * scale).round() as u32).max(1);
    let mut pm = Pixmap::new(w, h).ok_or_else(|| anyhow::anyhow!("创建画布失败"))?;
    pm.fill(tiny_skia::Color::WHITE);

    // 印章图先画（底）；文字 objects 后画（印章内时间文字覆盖印章图清晰显示）
    for stamp in &page.stamps {
        if let Some(data) = images.get(&stamp.resource_id) {
            imgobj::draw_image_at(&mut pm, &stamp.boundary, data, scale, h_mm);
        }
    }
    for obj in &page.objects {
        match obj {
            Obj::Text(t) => {
                let primary: &FontVec = fonts.get(&t.font_id).unwrap_or(fallback);
                let _ = text::draw_text(&mut pm, t, scale, h_mm, primary, fallback);
            }
            Obj::Path(p) => path::draw_path(&mut pm, p, scale, h_mm),
            Obj::Image(im) => {
                if let Some(data) = images.get(&im.resource_id) {
                    imgobj::draw_image(&mut pm, im, data, scale, h_mm);
                }
            }
        }
    }
    Ok(pm)
}

fn page_size_mm(page: &Page) -> (f32, f32) {
    match page.physical_box {
        Some((_, _, w, h)) => (w, h),
        None => (A4_W_MM, A4_H_MM),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试样本路径由环境变量 OFD_SAMPLE 提供，避免在源码中硬编码本地路径。
    fn sample_path() -> Option<String> {
        std::env::var("OFD_SAMPLE").ok().filter(|s| !s.is_empty())
    }

    #[test]
    fn render_all_pages_png() {
        let Some(sample) = sample_path() else {
            eprintln!("跳过：未设置 OFD_SAMPLE 环境变量");
            return;
        };
        let doc = crate::ofd::open(&sample).expect("解析失败");
        let pms = render_doc(&doc, 150.0).expect("渲染失败");
        for (i, pm) in pms.iter().enumerate() {
            let path = format!("docs/page{}.png", i);
            pm.save_png(&path).expect("保存失败");
            let ns = doc.pages[i]
                .objects
                .iter()
                .filter(|o| matches!(o, Obj::Text(t) if t.stroke))
                .count();
            println!(
                "页{}: {}x{} box={:?} stamps={} stroke文字={}",
                i,
                pm.width(),
                pm.height(),
                doc.pages[i].physical_box,
                doc.pages[i].stamps.len(),
                ns
            );
        }
        println!("嵌入字体 {} 个", doc.fonts.len());
    }

    #[test]
    fn export_pdf_test() {
        let Some(sample) = sample_path() else {
            eprintln!("跳过：未设置 OFD_SAMPLE 环境变量");
            return;
        };
        let save = "docs/test_export.pdf";
        pdf_export::export_pdf(&sample, 150.0, save).expect("导出失败");
        println!("已导出 {}", save);
    }
}

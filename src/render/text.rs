//! 文字渲染：按 OFD 坐标栅格化贴到画布，应用 CTM 变换与 Stroke 描边（加粗）。
use ab_glyph::{Font, FontVec, OutlinedGlyph, PxScale, ScaleFont};
use anyhow::{Context, Result};
use tiny_skia::{Paint, Pixmap, Stroke, Transform};

use crate::ofd::TextObj;

const FONT_PATH: &str = r"C:\Windows\Fonts\simhei.ttf";

pub fn load_default_font() -> Result<FontVec> {
    let data = std::fs::read(FONT_PATH)
        .with_context(|| format!("读取系统字体失败: {}", FONT_PATH))?;
    FontVec::try_from_vec(data).context("字体格式无效")
}

/// 把 glyph 像素（覆盖率 cov）填充到画布（白底黑字）
fn paint_outline(pm: &mut Pixmap, outlined: &OutlinedGlyph, pw: i32, ph: i32) {
    let bb = outlined.px_bounds();
    let (min_x, min_y) = (bb.min.x, bb.min.y);
    outlined.draw(|gx, gy, cov| {
        if cov <= 0.0 {
            return;
        }
        let p2x = (min_x + gx as f32) as i32;
        let p2y = (min_y + gy as f32) as i32;
        if p2x < 0 || p2y < 0 || p2x >= pw || p2y >= ph {
            return;
        }
        let idx = (p2y as usize * pm.width() as usize + p2x as usize) * 4;
        let v = ((1.0 - cov) * 255.0) as u8;
        let dd = pm.data_mut();
        dd[idx] = v;
        dd[idx + 1] = v;
        dd[idx + 2] = v;
    });
}

/// 绘制一个文字对象。CTM 应用缩放/平移；Stroke=true 时取字体轮廓曲线做真描边（加粗，不发糊）。
pub fn draw_text(
    pm: &mut Pixmap,
    t: &TextObj,
    scale: f32,
    _page_h_mm: f32,
    primary: &FontVec,
    fallback: &FontVec,
) -> Result<()> {
    if t.size <= 0.0 || t.text.is_empty() {
        return Ok(());
    }
    let (a, _b, _c, d, e, f) = t.ctm.unwrap_or((1.0, 0.0, 0.0, 1.0, 0.0, 0.0));
    let mut x_local = t.x;
    let chars: Vec<char> = t.text.chars().collect();
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let ax = a.abs().max(0.0001);

    for (i, ch) in chars.iter().enumerate() {
        let mx = a * x_local + e;
        let my = d * t.y + f;
        let px = (t.boundary.0 + mx) * scale;
        let py = (t.boundary.1 + my) * scale;

        let used: &FontVec = if primary.glyph_id(*ch).0 != 0 { primary } else { fallback };
        let sx = (t.size * scale * a).abs().max(0.5);
        let sy = (t.size * scale * d).abs().max(0.5);
        let scaled = used.as_scaled(PxScale { x: sx, y: sy });

        // 填充
        let mut g = scaled.scaled_glyph(*ch);
        g.position = ab_glyph::point(px, py);
        if let Some(outlined) = used.outline_glyph(g) {
            paint_outline(pm, &outlined, pw, ph);
        }
        // 真描边（Stroke=true 加粗）：取 unscaled 轮廓曲线 → 缩放定位 → tiny-skia stroke
        if t.stroke && t.line_width > 0.0 {
            let upem = used.units_per_em().unwrap_or(1000.0).max(1.0);
            let hf = sx / upem;
            let vf = sy / upem;
            let to_screen = |p: ab_glyph::Point| (p.x * hf + px, -p.y * vf + py);
            if let Some(outline) = used.outline(used.glyph_id(*ch)) {
                let mut pb = tiny_skia::PathBuilder::new();
                for c in &outline.curves {
                    match c {
                        ab_glyph::OutlineCurve::Line(p0, p1) => {
                            let a = to_screen(*p0);
                            let b = to_screen(*p1);
                            pb.move_to(a.0, a.1);
                            pb.line_to(b.0, b.1);
                        }
                        ab_glyph::OutlineCurve::Quad(p0, c1, p1) => {
                            let a = to_screen(*p0);
                            let c = to_screen(*c1);
                            let b = to_screen(*p1);
                            pb.move_to(a.0, a.1);
                            pb.quad_to(c.0, c.1, b.0, b.1);
                        }
                        ab_glyph::OutlineCurve::Cubic(p0, c1, c2, p1) => {
                            let a = to_screen(*p0);
                            let c1 = to_screen(*c1);
                            let c2 = to_screen(*c2);
                            let b = to_screen(*p1);
                            pb.move_to(a.0, a.1);
                            pb.cubic_to(c1.0, c1.1, c2.0, c2.1, b.0, b.1);
                        }
                    }
                }
                if let Some(path) = pb.finish() {
                    let mut paint = Paint::default();
                    paint.set_color_rgba8(0, 0, 0, 255);
                    paint.anti_alias = true;
                    let mut s = Stroke::default();
                    s.width = (t.line_width * scale).max(1.0);
                    let _ = pm.stroke_path(&path, &paint, &s, Transform::identity(), None);
                }
            }
        }

        if i < t.delta_x.len() {
            x_local += t.delta_x[i];
        } else {
            x_local += scaled.h_advance(scaled.glyph_id(*ch)) / scale / ax;
        }
    }
    Ok(())
}

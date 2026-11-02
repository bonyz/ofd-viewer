//! 文字渲染：按 OFD 坐标栅格化贴到画布，应用 CTM 变换与 Stroke 描边（加粗）。
use ab_glyph::{Font, FontVec, OutlinedGlyph, PxScale, ScaleFont};
use anyhow::Result;
use tiny_skia::{Paint, Pixmap, Stroke, Transform};

use crate::ofd::TextObj;

/// 系统兜底字体（需含 CJK），按平台依次尝试直到成功。
/// 注意 PingFang.ttc 是 CFF2 轮廓，ab_glyph 解析不了，不要放进列表。
pub fn default_font_candidates() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/STHeiti Medium.ttc",
            "/System/Library/Fonts/Supplemental/Songti.ttc",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        ]
    } else if cfg!(target_os = "linux") {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        ]
    } else {
        &[r"C:\Windows\Fonts\simhei.ttf", r"C:\Windows\Fonts\msyh.ttc"]
    }
}

/// 按文档声明的字体名匹配系统字体（OFD 未内嵌字体时用）。
/// 命中不了返回 None，由调用方落到兜底字体。
pub fn load_system_font_by_name(name: &str) -> Option<FontVec> {
    let mut candidates: Vec<&'static str> = Vec::new();
    if cfg!(target_os = "macos") {
        let n = name.to_lowercase();
        if n.contains("song") || name.contains('宋') {
            candidates.push("/System/Library/Fonts/Supplemental/Songti.ttc");
        } else if n.contains("hei") || name.contains('黑') {
            candidates.push("/System/Library/Fonts/STHeiti Medium.ttc");
            candidates.push("/System/Library/Fonts/STHeiti Light.ttc");
        } else if n.contains("kai") || name.contains('楷') {
            // Kaiti.ttc 部分系统没有，落到 Songti（细衬线，笔画粗细最接近楷体）
            candidates.push("/System/Library/Fonts/Supplemental/Kaiti.ttc");
            candidates.push("/System/Library/Fonts/Supplemental/Songti.ttc");
        } else if n.contains("fang") || name.contains('仿') {
            // macOS 无独立仿宋，落 Songti
            candidates.push("/System/Library/Fonts/Supplemental/Songti.ttc");
        } else if n.contains("courier") {
            candidates.push("/System/Library/Fonts/Supplemental/Courier New.ttf");
        } else if n.contains("times") {
            candidates.push("/System/Library/Fonts/Supplemental/Times New Roman.ttf");
        } else if n.contains("arial") {
            candidates.push("/System/Library/Fonts/Supplemental/Arial.ttf");
        }
    } else if cfg!(target_os = "windows") {
        let n = name.to_lowercase();
        if n.contains("song") || name.contains('宋') {
            candidates.push(r"C:\Windows\Fonts\simsun.ttc");
        } else if n.contains("hei") || name.contains('黑') {
            candidates.push(r"C:\Windows\Fonts\simhei.ttf");
        } else if n.contains("kai") || name.contains('楷') {
            candidates.push(r"C:\Windows\Fonts\simkai.ttf");
        } else if n.contains("fang") || name.contains('仿') {
            candidates.push(r"C:\Windows\Fonts\simfang.ttf");
        } else if n.contains("courier") {
            candidates.push(r"C:\Windows\Fonts\cour.ttf");
        } else if n.contains("times") {
            candidates.push(r"C:\Windows\Fonts\times.ttf");
        }
    } else {
        // Linux：统一落 Noto CJK
        candidates.push("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc");
    }
    for path in candidates {
        if let Ok(data) = std::fs::read(path) {
            if let Ok(f) = FontVec::try_from_vec(data) {
                return Some(f);
            }
        }
    }
    None
}

pub fn load_default_font() -> Result<FontVec> {
    let candidates = default_font_candidates();
    let mut last: Option<anyhow::Error> = None;
    for path in candidates {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                last = Some(anyhow::anyhow!("读取 {} 失败: {}", path, e));
                continue;
            }
        };
        // ttc 字体集合格式默认取 index 0，ab_glyph 自动处理
        match FontVec::try_from_vec(data) {
            Ok(f) => return Ok(f),
            Err(e) => last = Some(anyhow::anyhow!("字体 {} 解析失败: {}", path, e)),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("未找到可用的系统中文字体")))
}

/// 把 glyph 像素（覆盖率 cov）填充到画布：白底向指定颜色过渡
fn paint_outline(pm: &mut Pixmap, outlined: &OutlinedGlyph, pw: i32, ph: i32, color: [u8; 3]) {
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
        let dd = pm.data_mut();
        dd[idx] = 255 - ((255 - color[0]) as f32 * cov) as u8;
        dd[idx + 1] = 255 - ((255 - color[1]) as f32 * cov) as u8;
        dd[idx + 2] = 255 - ((255 - color[2]) as f32 * cov) as u8;
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
    let color = t.fill_color.unwrap_or([0, 0, 0]);

    for (i, ch) in chars.iter().enumerate() {
        let mx = a * x_local + e;
        let my = d * t.y + f;
        let px = (t.boundary.0 + mx) * scale;
        let py = (t.boundary.1 + my) * scale;

        let used: &FontVec = if primary.glyph_id(*ch).0 != 0 { primary } else { fallback };
        let sx = (t.size * scale * a).abs().max(0.5);
        let sy = (t.size * scale * d).abs().max(0.5);
        let gid = used.glyph_id(*ch);
        let scaled = used.as_scaled(PxScale { x: sx, y: sy });

        // 字符步进：优先文档 DeltaX（每字符到前一字符的位移），无则用自然宽度。
        // 税务类 OFD 按"半角拉丁 0.5em"网格给 DeltaX 且不内嵌字体；实际字体更宽时
        // 把字形横向压缩到步进内（h_scale = 步进/自然宽度，上限 1 不放大），
        // 既不叠字也不溢出版式格子，观感等同官方半角宋体。
        let natural_mm = scaled.h_advance(gid) / scale / ax;
        let (step, condense) = if i < t.delta_x.len() {
            let s = t.delta_x[i];
            let c = if natural_mm > 0.0 && s > 0.0 { (s / natural_mm).min(1.0) } else { 1.0 };
            (s, c)
        } else {
            (natural_mm, 1.0)
        };
        let sx_c = sx * condense;
        let scaled_c = used.as_scaled(PxScale { x: sx_c, y: sy });

        // 填充（横向按压缩比例）
        let mut g = scaled_c.scaled_glyph(*ch);
        g.position = ab_glyph::point(px, py);
        if let Some(outlined) = used.outline_glyph(g) {
            paint_outline(pm, &outlined, pw, ph, color);
        }
        // 真描边（Stroke=true 加粗）：取 unscaled 轮廓曲线 → 缩放定位 → tiny-skia stroke
        if t.stroke && t.line_width > 0.0 {
            let upem = used.units_per_em().unwrap_or(1000.0).max(1.0);
            let hf = sx_c / upem;
            let vf = sy / upem;
            let to_screen = |p: ab_glyph::Point| (p.x * hf + px, -p.y * vf + py);
            if let Some(outline) = used.outline(gid) {
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
                    paint.set_color_rgba8(color[0], color[1], color[2], 255);
                    paint.anti_alias = true;
                    let mut s = Stroke::default();
                    s.width = (t.line_width * scale).max(1.0);
                    let _ = pm.stroke_path(&path, &paint, &s, Transform::identity(), None);
                }
            }
        }

        x_local += step;
    }
    Ok(())
}

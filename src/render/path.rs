//! 路径渲染：解析 AbbreviatedData（M/L/Q/B/Z），用 tiny-skia 描边/填充。
use tiny_skia::{FillRule, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

use crate::ofd::PathObj;

pub fn draw_path(pm: &mut Pixmap, p: &PathObj, scale: f32, page_h_mm: f32) {
    let path = match build_path(&p.abbreviated, p.boundary, scale, page_h_mm) {
        Some(x) => x,
        None => return,
    };
    let stroke_c = p.stroke_color.unwrap_or([0, 0, 0]);
    let fill_c = p.fill_color.unwrap_or([0, 0, 0]);

    if p.fill.is_some() {
        let mut fpaint = Paint::default();
        fpaint.set_color_rgba8(fill_c[0], fill_c[1], fill_c[2], 255);
        fpaint.anti_alias = true;
        let _ = pm.fill_path(&path, &fpaint, FillRule::Winding, Transform::identity(), None);
    }
    let mut paint = Paint::default();
    paint.set_color_rgba8(stroke_c[0], stroke_c[1], stroke_c[2], 255);
    paint.anti_alias = true;
    let mut s = Stroke::default();
    s.width = if p.line_width > 0.0 {
        p.line_width * scale
    } else {
        0.35 * scale
    };
    let _ = pm.stroke_path(&path, &paint, &s, Transform::identity(), None);
}

/// OFD 路径坐标原点在 Boundary 左下角、y 向上 → 屏幕左上原点 y 向下
fn conv(x: f32, y: f32, bd: (f32, f32, f32, f32), scale: f32, _page_h: f32) -> (f32, f32) {
    ((bd.0 + x) * scale, (bd.1 + y) * scale)
}

fn exec(cmd: u8, nums: &[f32], pb: &mut PathBuilder, bd: (f32, f32, f32, f32), scale: f32, ph: f32) {
    match cmd {
        b'M' => {
            let mut it = nums.chunks(2);
            if let Some(p) = it.next() {
                if p.len() == 2 {
                    let (x, y) = conv(p[0], p[1], bd, scale, ph);
                    pb.move_to(x, y);
                }
            }
            for p in it {
                if p.len() == 2 {
                    let (x, y) = conv(p[0], p[1], bd, scale, ph);
                    pb.line_to(x, y);
                }
            }
        }
        b'L' => {
            for p in nums.chunks(2) {
                if p.len() == 2 {
                    let (x, y) = conv(p[0], p[1], bd, scale, ph);
                    pb.line_to(x, y);
                }
            }
        }
        b'Q' => {
            for p in nums.chunks(4) {
                if p.len() == 4 {
                    let (x1, y1) = conv(p[0], p[1], bd, scale, ph);
                    let (x2, y2) = conv(p[2], p[3], bd, scale, ph);
                    pb.quad_to(x1, y1, x2, y2);
                }
            }
        }
        b'B' => {
            for p in nums.chunks(6) {
                if p.len() == 6 {
                    let (x1, y1) = conv(p[0], p[1], bd, scale, ph);
                    let (x2, y2) = conv(p[2], p[3], bd, scale, ph);
                    let (x3, y3) = conv(p[4], p[5], bd, scale, ph);
                    pb.cubic_to(x1, y1, x2, y2, x3, y3);
                }
            }
        }
        b'Z' => pb.close(),
        _ => {} // A（弧）等 v1 暂不支持
    }
}

fn build_path(data: &str, boundary: (f32, f32, f32, f32), scale: f32, page_h_mm: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let mut nums: Vec<f32> = Vec::new();
    let mut cmd: u8 = 0;
    let mut tok = String::new();

    for ch in data.chars() {
        if ch.is_ascii_alphabetic() {
            if !tok.is_empty() {
                if let Ok(v) = tok.parse() {
                    nums.push(v);
                }
                tok.clear();
            }
            if cmd != 0 && !nums.is_empty() {
                exec(cmd, &nums, &mut pb, boundary, scale, page_h_mm);
                nums.clear();
            }
            cmd = (ch as u8).to_ascii_uppercase();
        } else if ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == '+' || ch == 'e' || ch == 'E' {
            tok.push(ch);
        } else if !tok.is_empty() {
            if let Ok(v) = tok.parse() {
                nums.push(v);
            }
            tok.clear();
        }
    }
    if !tok.is_empty() {
        if let Ok(v) = tok.parse() {
            nums.push(v);
        }
    }
    if cmd != 0 && !nums.is_empty() {
        exec(cmd, &nums, &mut pb, boundary, scale, page_h_mm);
    }
    pb.finish()
}

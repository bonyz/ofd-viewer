//! 图片渲染：按 Boundary 最近邻缩放贴到画布（白底 alpha 混合）。
use tiny_skia::Pixmap;

use crate::ofd::{ImageObj, ImageData};

pub fn draw_image(pm: &mut Pixmap, img: &ImageObj, data: &ImageData, scale: f32, page_h_mm: f32) {
    draw_image_at(pm, &img.boundary, data, scale, page_h_mm);
}

/// 按 boundary 贴图（左上原点：boundary.1 为上边，y 向下）
pub fn draw_image_at(
    pm: &mut Pixmap,
    boundary: &(f32, f32, f32, f32),
    data: &ImageData,
    scale: f32,
    _page_h_mm: f32,
) {
    let dw = (boundary.2 * scale).round() as u32;
    let dh = (boundary.3 * scale).round() as u32;
    if dw == 0 || dh == 0 || data.width == 0 || data.height == 0 {
        return;
    }
    let dx = (boundary.0 * scale).round() as i32;
    let dy = (boundary.1 * scale).round() as i32;
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let stride = pm.width() as usize;
    let d = pm.data_mut();
    for ty in 0..dh {
        let sy = (ty as u64 * data.height as u64 / dh as u64) as u32;
        let py = dy + ty as i32;
        if py < 0 || py >= ph {
            continue;
        }
        for tx in 0..dw {
            let sx = (tx as u64 * data.width as u64 / dw as u64) as u32;
            let px = dx + tx as i32;
            if px < 0 || px >= pw {
                continue;
            }
            let sidx = ((sy * data.width + sx) * 4) as usize;
            let r = data.rgba[sidx];
            let g = data.rgba[sidx + 1];
            let b = data.rgba[sidx + 2];
            let a = data.rgba[sidx + 3];
            let didx = (py as usize * stride + px as usize) * 4;
            let af = a as f32 / 255.0;
            let blend = |fg: u8| (fg as f32 * af + 255.0 * (1.0 - af)) as u8;
            d[didx] = blend(r);
            d[didx + 1] = blend(g);
            d[didx + 2] = blend(b);
            d[didx + 3] = 255;
        }
    }
}

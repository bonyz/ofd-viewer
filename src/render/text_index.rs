//! 逐字文字索引：为每页每个字算出 mm 矩形，供 GUI 逐字选中 / 搜索高亮精确对齐。
//! 字矩形与 render/text.rs 的逐字推进严格一致（同 CTM、同字体选择、同推进量），
//! 保证覆盖层框与渲染像素对齐。
use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use std::collections::HashMap;

use crate::ofd::{Obj, OfdDoc, Page};

const MM_PER_INCH: f32 = 25.4;
const A4_W_MM: f32 = 210.0;
const A4_H_MM: f32 = 297.0;

/// 单字矩形（mm，左上原点 y 向下）+ 所属 TextObject/字序（跨对象选区拼接用）
#[derive(Clone, Copy, Debug)]
pub struct CharBox {
    pub x_mm: f32,
    pub y_mm: f32, // 字形顶 = baseline - ascent
    pub w_mm: f32,
    pub h_mm: f32,
    pub ch: char,
    pub obj_idx: usize,
    pub char_idx: usize,
}

#[derive(Clone, Debug, Default)]
pub struct PageTextIndex {
    pub chars: Vec<CharBox>,
    pub page_w_mm: f32,
    pub page_h_mm: f32,
}

pub type DocTextIndex = Vec<PageTextIndex>;

/// 为整个文档构建逐字索引。字体加载复用 render 的 fallback。
pub fn build_index(doc: &OfdDoc, dpi: f32) -> anyhow::Result<DocTextIndex> {
    let (fonts, fallback) = super::load_font_table(doc)?;
    Ok(doc
        .pages
        .iter()
        .map(|page| build_page_index(page, dpi, &fonts, &fallback))
        .collect())
}

fn page_size_mm(page: &Page) -> (f32, f32) {
    match page.physical_box {
        Some((_, _, w, h)) => (w, h),
        None => (A4_W_MM, A4_H_MM),
    }
}

fn build_page_index(
    page: &Page,
    dpi: f32,
    fonts: &HashMap<String, FontVec>,
    fallback: &FontVec,
) -> PageTextIndex {
    let scale = dpi / MM_PER_INCH;
    let (page_w_mm, page_h_mm) = page_size_mm(page);
    let mut chars: Vec<CharBox> = Vec::new();

    for (obj_idx, obj) in page.objects.iter().enumerate() {
        let t = match obj {
            Obj::Text(t) => t,
            _ => continue,
        };
        if t.size <= 0.0 || t.text.is_empty() {
            continue;
        }
        // CTM：与 render/text.rs 完全一致的取值
        let (a, _b, _c, d, e, f) = t.ctm.unwrap_or((1.0, 0.0, 0.0, 1.0, 0.0, 0.0));
        let ax = a.abs().max(0.0001);
        let mut x_local = t.x;
        let glyphs: Vec<char> = t.text.chars().collect();
        let primary: &FontVec = fonts.get(&t.font_id).unwrap_or(fallback);
        let sx = (t.size * scale * a).abs().max(0.5);
        let sy = (t.size * scale * d).abs().max(0.5);
        let ps = PxScale { x: sx, y: sy };

        for (ci, ch) in glyphs.iter().enumerate() {
            // 字体选择与 render/mod.rs 一致：主字体缺字(.notdef)则回退
            let used: &FontVec = if primary.glyph_id(*ch).0 != 0 {
                primary
            } else {
                fallback
            };
            let scaled = used.as_scaled(ps);

            // baseline 的 mm 坐标（render: px = (boundary.0 + mx) * scale）
            let mx = a * x_local + e;
            let my = d * t.y + f;
            let baseline_x_mm = t.boundary.0 + mx;
            let baseline_y_mm = t.boundary.1 + my;

            // 字形度量（像素）换算 mm：sy 已含 |d| 缩放，除 scale 即可
            let ascent_mm = scaled.ascent() / scale;
            let descent_mm = scaled.descent().abs() / scale;

            // 推进量（mm）：i<delta_x.len() 用 delta_x，否则 glyph advance（与 render 一致）
            let advance_mm = if ci < t.delta_x.len() {
                t.delta_x[ci]
            } else {
                scaled.h_advance(scaled.glyph_id(*ch)) / scale / ax
            };

            chars.push(CharBox {
                x_mm: baseline_x_mm,
                y_mm: baseline_y_mm - ascent_mm,
                w_mm: advance_mm.abs(),
                h_mm: ascent_mm + descent_mm,
                ch: *ch,
                obj_idx,
                char_idx: ci,
            });

            x_local += advance_mm;
        }
    }

    PageTextIndex {
        chars,
        page_w_mm,
        page_h_mm,
    }
}

/// 全文档搜索：返回 (页索引, 起始字符序, 结束字符序[含]) 列表（按页顺序）
pub fn search(doc_text: &DocTextIndex, query: &str) -> Vec<(usize, usize, usize)> {
    let q: Vec<char> = query.trim().chars().collect();
    let mut out = Vec::new();
    if q.is_empty() {
        return out;
    }
    for (pi, pti) in doc_text.iter().enumerate() {
        let chars: Vec<char> = pti.chars.iter().map(|c| c.ch).collect();
        if q.len() > chars.len() {
            continue;
        }
        for i in 0..=chars.len() - q.len() {
            if chars[i..i + q.len()] == q[..] {
                out.push((pi, i, i + q.len() - 1));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试样本路径由环境变量 OFD_SAMPLE 提供，避免在源码中硬编码本地路径。
    fn sample_path() -> Option<String> {
        std::env::var("OFD_SAMPLE").ok().filter(|s| !s.is_empty())
    }

    #[test]
    fn build_index_has_text_and_rects() {
        let Some(sample) = sample_path() else {
            eprintln!("跳过：未设置 OFD_SAMPLE 环境变量");
            return;
        };
        let doc = crate::ofd::open(&sample).expect("解析失败");
        let idx = build_index(&doc, 150.0).expect("构建索引失败");
        assert_eq!(idx.len(), 5, "应 5 页索引");
        let p0 = &idx[0];
        assert!(!p0.chars.is_empty(), "第1页应有字");
        // 拼接文本应含“关于”
        let s: String = p0.chars.iter().map(|c| c.ch).collect();
        assert!(s.contains('关'), "第1页文字应含'关'，实际: {}", s.chars().take(40).collect::<String>());
        // 字矩形在页面范围内
        for c in &p0.chars {
            assert!(c.w_mm > 0.0 && c.h_mm > 0.0, "字宽高应为正: {:?}", c);
        }
    }

    #[test]
    fn search_finds_keyword() {
        let Some(sample) = sample_path() else {
            eprintln!("跳过：未设置 OFD_SAMPLE 环境变量");
            return;
        };
        let doc = crate::ofd::open(&sample).expect("解析失败");
        let idx = build_index(&doc, 150.0).expect("构建索引失败");
        let hits = search(&idx, "网络安全");
        assert!(!hits.is_empty(), "应搜到'网络安全'");
        println!("网络安全 命中 {} 处: {:?}", hits.len(), hits);
        // 命中页索引有效
        for (pi, s, e) in &hits {
            assert!(*pi < idx.len());
            assert!(s <= e);
        }
    }
}

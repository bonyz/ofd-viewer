//! 导出 PDF：每页渲染位图嵌入最小 PDF（每页一张 JPEG），无外部 PDF 库依赖。
use anyhow::{Context, Result};
use std::io::Write;

use crate::ofd;
use crate::render;

pub fn export_pdf(file_path: &str, dpi: f32, save_path: &str) -> Result<()> {
    let doc = ofd::open(file_path)?;
    let pms = render::render_doc(&doc, dpi)?;
    let scale = dpi / 25.4;

    // 每页：像素尺寸 + 点尺寸 + JPEG 字节
    let mut pages: Vec<(u32, u32, f32, f32, Vec<u8>)> = Vec::new();
    for pm in &pms {
        let png = pm.encode_png().context("PNG 编码失败")?;
        let img = image::load_from_memory(&png).context("图像解码失败")?;
        let dyn_rgb = image::DynamicImage::ImageRgb8(img.to_rgb8());
        let mut jpg = Vec::new();
        dyn_rgb
            .write_to(
                &mut std::io::Cursor::new(&mut jpg),
                image::ImageFormat::Jpeg,
            )
            .context("JPEG 编码失败")?;
        let w_pt = pm.width() as f32 / scale / 25.4 * 72.0;
        let h_pt = pm.height() as f32 / scale / 25.4 * 72.0;
        pages.push((pm.width(), pm.height(), w_pt, h_pt, jpg));
    }

    let n = pages.len();
    let mut buf: Vec<u8> = Vec::new();
    buf.extend(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n");
    let total = 2 + 3 * n; // catalog(1) + pages(2) + 每页 page/image/contents
    let mut offsets = vec![0usize; total + 1];

    let mut put = |num: usize, content: Vec<u8>| {
        offsets[num] = buf.len();
        buf.extend(format!("{} 0 obj\n", num).as_bytes());
        buf.extend(&content);
        buf.extend(b"\nendobj\n");
    };

    put(1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + 3 * i)).collect();
    put(
        2,
        format!("<</Type /Pages /Kids [{}] /Count {}>>", kids.join(" "), n).into_bytes(),
    );

    for (i, (wpx, hpx, wpt, hpt, jpg)) in pages.iter().enumerate() {
        let page_n = 3 + 3 * i;
        let image_n = 4 + 3 * i;
        let contents_n = 5 + 3 * i;
        // 图像 XObject（JPEG）
        let mut img_obj = Vec::new();
        img_obj.extend(
            format!(
                "<</Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {}>>\nstream\n",
                wpx, hpx, jpg.len()
            )
            .as_bytes(),
        );
        img_obj.extend(jpg);
        img_obj.extend(b"\nendstream");
        put(image_n, img_obj);
        // 页
        put(
            page_n,
            format!(
                "<</Type /Page /Parent 2 0 R /MediaBox [0 0 {:.2} {:.2}] /Resources <</XObject <</Im0 {} 0 R>>>> /Contents {} 0 R>>",
                wpt, hpt, image_n, contents_n
            )
            .into_bytes(),
        );
        // 内容流：把图像缩放到页面大小
        let cs = format!("q\n{:.2} 0 0 {:.2} 0 0 cm\n/Im0 Do\nQ", wpt, hpt);
        let mut c_obj = Vec::new();
        c_obj.extend(format!("<</Length {}>>\nstream\n", cs.len()).as_bytes());
        c_obj.extend(cs.as_bytes());
        c_obj.extend(b"\nendstream");
        put(contents_n, c_obj);
    }

    let xref_off = buf.len();
    buf.extend(format!("xref\n0 {}\n", total + 1).as_bytes());
    buf.extend(b"0000000000 65535 f \r\n");
    for num in 1..=total {
        buf.extend(format!("{:010} 00000 n \r\n", offsets[num]).as_bytes());
    }
    buf.extend(
        format!(
            "trailer\n<</Size {} /Root 1 0 R>>\nstartxref\n{}\n%%EOF",
            total + 1,
            xref_off
        )
        .as_bytes(),
    );

    std::fs::File::create(save_path)?.write_all(&buf)?;
    Ok(())
}

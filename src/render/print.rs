//! Windows 原生打印：打印对话框（PrintDlgW）+ GDI StretchDIBits 逐页整页打印。
//! 仅 Windows 编译（Cargo target-specific 依赖）。
#![cfg(windows)]

use anyhow::{anyhow, Result};
use tiny_skia::Pixmap;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    DeleteDC, GetDeviceCaps, SetStretchBltMode, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER,
    DIB_RGB_COLORS, HALFTONE, HDC, HORZRES, VERTRES,
};
use windows::Win32::Storage::Xps::{EndDoc, EndPage, StartDocW, StartPage, DOCINFOW};
use windows::Win32::UI::Controls::Dialogs::{PrintDlgW, PRINTDLGW, PD_RETURNDC};

/// 打印多个 Pixmap：每页以纸张物理像素整页铺满（白底黑字）。
pub fn print_pixmaps(pages: &[Pixmap]) -> Result<()> {
    if pages.is_empty() {
        return Err(anyhow!("没有可打印的页"));
    }
    unsafe {
        let hdc = match pick_printer_dc()? {
            Some(dc) => dc,
            None => return Ok(()), // 用户取消
        };
        let paper_w = GetDeviceCaps(Some(hdc), HORZRES);
        let paper_h = GetDeviceCaps(Some(hdc), VERTRES);
        SetStretchBltMode(hdc, HALFTONE);

        let doc_name: Vec<u16> = "OFD 文档\0".encode_utf16().collect();
        let di = DOCINFOW {
            cbSize: std::mem::size_of::<DOCINFOW>() as i32,
            lpszDocName: PCWSTR(doc_name.as_ptr()),
            ..Default::default()
        };
        StartDocW(hdc, &di);

        for pm in pages {
            StartPage(hdc);
            let dib = pixmap_to_dib(pm);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: pm.width() as i32,
                    biHeight: -(pm.height() as i32), // 负值=top-down（行序与 tiny-skia 一致）
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: 0, // BI_RGB
                    biSizeImage: (pm.width() * pm.height() * 4) as u32,
                    ..Default::default()
                },
                bmiColors: [Default::default()],
            };
            StretchDIBits(
                hdc,
                0,
                0,
                paper_w,
                paper_h,
                0,
                0,
                pm.width() as i32,
                pm.height() as i32,
                Some(dib.as_ptr() as *const _),
                &bmi,
                DIB_RGB_COLORS,
                windows::Win32::Graphics::Gdi::SRCCOPY,
            );
            EndPage(hdc);
        }
        EndDoc(hdc);
        let _ = DeleteDC(hdc);
    }
    Ok(())
}

/// 弹打印对话框拿打印机 DC；用户取消返回 None。
unsafe fn pick_printer_dc() -> Result<Option<HDC>> {
    let mut pd = PRINTDLGW::default();
    pd.lStructSize = std::mem::size_of::<PRINTDLGW>() as u32;
    pd.hwndOwner = HWND::default();
    pd.Flags = PD_RETURNDC;
    pd.nCopies = 1;
    if PrintDlgW(&mut pd).as_bool() && !pd.hDC.is_invalid() {
        Ok(Some(pd.hDC))
    } else {
        Ok(None) // 用户取消或失败，静默
    }
}

/// Pixmap(RGBA premultiplied) -> BGRA 白底 top-down DIB（alpha=0 填白，其余 unpremultiply）。
fn pixmap_to_dib(pm: &Pixmap) -> Vec<u8> {
    let n = (pm.width() as usize) * (pm.height() as usize);
    let mut dib = vec![0u8; n * 4];
    for (i, p) in pm.pixels().iter().enumerate() {
        let a = p.alpha() as f32;
        if a == 0.0 {
            dib[i * 4] = 255;
            dib[i * 4 + 1] = 255;
            dib[i * 4 + 2] = 255;
        } else {
            let f = 255.0 / a;
            dib[i * 4] = (p.blue() as f32 * f).min(255.0) as u8; // B
            dib[i * 4 + 1] = (p.green() as f32 * f).min(255.0) as u8; // G
            dib[i * 4 + 2] = (p.red() as f32 * f).min(255.0) as u8; // R
        }
        dib[i * 4 + 3] = 255;
    }
    dib
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_white_on_transparent_bgra() {
        let mut pm = tiny_skia::Pixmap::new(2, 2).unwrap();
        pm.fill(tiny_skia::Color::TRANSPARENT);
        let dib = pixmap_to_dib(&pm);
        // alpha=0 应为白底 BGRA = [255,255,255,255]
        assert_eq!(&dib[0..4], &[255, 255, 255, 255]);
    }

    #[test]
    fn dib_black_opaque_bgra() {
        let mut pm = tiny_skia::Pixmap::new(1, 1).unwrap();
        pm.fill(tiny_skia::Color::BLACK); // RGBA(0,0,0,255) premul=(0,0,0,255)
        let dib = pixmap_to_dib(&pm);
        // 黑字 BGRA = [0,0,0,255]
        assert_eq!(&dib[0..4], &[0, 0, 0, 255]);
    }
}

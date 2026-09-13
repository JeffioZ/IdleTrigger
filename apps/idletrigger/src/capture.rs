//! Window client-area capture to BMP — ported from the old platform crate
//! (its only remaining consumer is the devtools capture mode).

use std::io;
use std::path::Path;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HGDIOBJ, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

/// Captures `hwnd`'s client area and writes it as a 24-bit BMP file.
pub fn capture_client_bmp(hwnd: HWND, out_path: &Path) -> io::Result<()> {
    unsafe {
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect).map_err(|e| io::Error::other(e.to_string()))?;
        let width = (rect.right - rect.left).max(1) as usize;
        let height = (rect.bottom - rect.top).max(1) as usize;

        let window_dc = GetDC(Some(hwnd));
        if window_dc.is_invalid() {
            return Err(io::Error::other("GetDC returned a null DC"));
        }
        let mem_dc = CreateCompatibleDC(Some(window_dc));
        if mem_dc.is_invalid() {
            let _ = ReleaseDC(Some(hwnd), window_dc);
            return Err(io::Error::other("CreateCompatibleDC returned a null DC"));
        }
        let bitmap = CreateCompatibleBitmap(window_dc, width as i32, height as i32);
        if bitmap.is_invalid() {
            let _ = ReleaseDC(Some(hwnd), window_dc);
            return Err(io::Error::other("CreateCompatibleBitmap failed"));
        }
        let old_obj = SelectObject(mem_dc, HGDIOBJ(bitmap.0));

        let blit = BitBlt(
            mem_dc,
            0,
            0,
            width as i32,
            height as i32,
            Some(window_dc),
            0,
            0,
            SRCCOPY | CAPTUREBLT,
        );

        let mut pixels = vec![0u8; width * height * 4];
        let mut copied_rows = 0;
        if blit.is_ok() {
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    biHeight: -(height as i32), // top-down rows
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: 0, // BI_RGB
                    ..Default::default()
                },
                ..Default::default()
            };
            copied_rows = GetDIBits(
                mem_dc,
                bitmap,
                0,
                height as u32,
                Some(pixels.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            );
        }

        let _ = SelectObject(mem_dc, old_obj);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem_dc);
        let _ = ReleaseDC(Some(hwnd), window_dc);

        if let Err(err) = blit {
            return Err(io::Error::other(format!("BitBlt failed: {err}")));
        }
        if copied_rows as usize != height {
            return Err(io::Error::other(format!(
                "GetDIBits copied {copied_rows} of {height} rows"
            )));
        }

        write_bmp_24(out_path, width, height, &pixels)
    }
}

/// Writes a 24-bit bottom-up BMP from top-down BGRA pixels.
fn write_bmp_24(path: &Path, width: usize, height: usize, bgra: &[u8]) -> io::Result<()> {
    let row_bytes = width * 3;
    let padding = (4 - (row_bytes % 4)) % 4;
    let stride = row_bytes + padding;
    let pixel_bytes = stride * height;
    let file_size = 54 + pixel_bytes;

    let mut out = Vec::with_capacity(file_size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());

    for y in (0..height).rev() {
        for x in 0..width {
            let i = (y * width + x) * 4;
            out.push(bgra[i]); // B
            out.push(bgra[i + 1]); // G
            out.push(bgra[i + 2]); // R
        }
        out.extend(std::iter::repeat_n(0, padding));
    }

    std::fs::write(path, out)
}

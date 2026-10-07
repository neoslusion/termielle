//! Print an isolated, hidden native preferences window to BMP. Never shows or
//! focuses a window, saves a profile, launches apps or changes the taskbar.
use termielle_app::{preferences::PreferencesWindow, window::OverlayWindow};
use windows::Win32::{
    Foundation::{LPARAM, RECT, WPARAM},
    Graphics::Gdi::*,
    UI::WindowsAndMessaging::*,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args().nth(1).ok_or("output BMP path required")?;
    let page = std::env::args()
        .nth(2)
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0)
        .min(4);
    let config = termielle_core::AppConfig::default();
    let owner = OverlayWindow::create(&config, true)?;
    let preferences = PreferencesWindow::create(owner.hwnd(), owner.wake_handle(), &config.island)?;
    let pages = unsafe { GetDlgItem(Some(preferences.hwnd()), 101) }?;
    unsafe {
        SendMessageW(pages, CB_SETCURSEL, Some(WPARAM(page)), None);
        SendMessageW(
            preferences.hwnd(),
            WM_COMMAND,
            Some(WPARAM(101 | ((CBN_SELCHANGE as usize) << 16))),
            Some(LPARAM(pages.0 as isize)),
        );
    }
    let mut rect = RECT::default();
    unsafe { GetWindowRect(preferences.hwnd(), &mut rect) }?;
    let width = (rect.right - rect.left) as u32;
    let height = (rect.bottom - rect.top) as u32;
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("no desktop DC".into());
        }
        let dc = CreateCompatibleDC(Some(screen));
        if dc.is_invalid() {
            let _ = ReleaseDC(None, screen);
            return Err("no memory DC".into());
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let dib = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let previous = SelectObject(dc, HGDIOBJ(dib.0));
        std::ptr::write_bytes(bits.cast::<u8>(), 240, (width * height * 4) as usize);
        SendMessageW(
            preferences.hwnd(),
            WM_PRINT,
            Some(WPARAM(dc.0 as usize)),
            Some(LPARAM(
                (PRF_CLIENT | PRF_NONCLIENT | PRF_CHILDREN | PRF_ERASEBKGND) as isize,
            )),
        );
        let pixels =
            std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize).to_vec();
        SelectObject(dc, previous);
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = DeleteDC(dc);
        let _ = ReleaseDC(None, screen);
        let stride = (width * 3 + 3) & !3;
        let size = 54 + stride * height;
        let mut bmp = Vec::with_capacity(size as usize);
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&size.to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&width.to_le_bytes());
        bmp.extend_from_slice(&height.to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        for y in (0..height).rev() {
            for x in 0..width {
                let i = ((y * width + x) * 4) as usize;
                bmp.extend_from_slice(&pixels[i..i + 3]);
            }
            bmp.extend(std::iter::repeat_n(0, (stride - width * 3) as usize));
        }
        std::fs::write(output, bmp)?;
    }
    Ok(())
}

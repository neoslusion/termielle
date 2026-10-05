//! Narrow live diagnostic, never a desktop-wide/terminal-body capture.
//! Default: exact old capture path over the installed bar's 260x45 center.
//! --owned: briefly show a nonactivating 64x32 marker in the top bar, verify
//! scoped self-exclusion and restoration, then destroy it. No config/tray,
//! taskbar, focus, or power changes. Requires an interactive modern desktop.
use termielle_app::{
    animation::FrameBuffer,
    backdrop::{Backdrop, capture_backdrop, capture_backdrop_excluding},
    window::OverlayWindow,
};
use windows::{
    Win32::{
        Foundation::RECT,
        UI::{
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
            WindowsAndMessaging::{
                FindWindowW, GetForegroundWindow, GetWindowDisplayAffinity, GetWindowRect,
                HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos,
            },
        },
    },
    core::{PCWSTR, w},
};
fn write_bmp(path: &str, bg: &Backdrop) -> std::io::Result<()> {
    let stride = (bg.width * 3 + 3) & !3;
    let size = 54 + stride * bg.height;
    let mut bmp = Vec::with_capacity(size as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&size.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&bg.width.to_le_bytes());
    bmp.extend_from_slice(&bg.height.to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    bmp.extend_from_slice(&[0; 24]);
    for y in (0..bg.height).rev() {
        for x in 0..bg.width {
            let i = ((y * bg.width + x) * 4) as usize;
            bmp.extend_from_slice(&bg.pixels[i..i + 3]);
        }
        bmp.extend(std::iter::repeat_n(0, (stride - bg.width * 3) as usize));
    }
    std::fs::write(path, bmp)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    let output = args.first().ok_or("output BMP path required")?;
    if args.iter().any(|s| s == "--owned") {
        let foreground = unsafe { GetForegroundWindow() };
        let mut window = OverlayWindow::create(&termielle_core::AppConfig::default(), true)?;
        let marker = [7, 43, 247, 255];
        let frame = FrameBuffer {
            width: 64,
            height: 32,
            pixels_pbgra: marker.repeat(64 * 32),
            delay_ms: 0,
            loop_index: 0,
            scale: 1.0,
        };
        window.present(&frame)?;
        unsafe {
            SetWindowPos(
                window.hwnd(),
                Some(HWND_TOPMOST),
                32,
                6,
                64,
                32,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        }?;
        unsafe { windows::Win32::Graphics::Dwm::DwmFlush() }?;
        let mut rect = RECT::default();
        unsafe { GetWindowRect(window.hwnd(), &mut rect) }?;
        let before = capture_backdrop(rect.left, rect.top, 64, 32, [0, 0, 0, 255])
            .ok_or("raw copy unavailable")?;
        assert!(
            before.pixels.chunks_exact(4).all(|p| p == marker),
            "marker must be captured by the old path"
        );
        let owner = window.hwnd().0 as isize;
        let capture = std::thread::spawn(move || {
            capture_backdrop_excluding(owner, rect.left, rect.top, 64, 32, [0, 0, 0, 255])
        });
        // Same topology as production: capture worker + responsive GUI owner.
        while !capture.is_finished() {
            use windows::Win32::UI::WindowsAndMessaging::{
                DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
            };
            let mut message = MSG::default();
            while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                let _ = unsafe { TranslateMessage(&message) };
                unsafe { DispatchMessageW(&message) };
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let excluded = capture
            .join()
            .map_err(|_| "capture worker panicked")?
            .ok_or("scoped exclusion unavailable")?;
        assert!(
            !excluded.pixels.chunks_exact(4).all(|p| p == marker),
            "must see behind the marker"
        );
        assert!(
            excluded.pixels.chunks_exact(4).any(|p| p[..3] != [0; 3]),
            "must not substitute a black rectangle"
        );
        let mut affinity = 0;
        unsafe { GetWindowDisplayAffinity(window.hwnd(), &mut affinity) }?;
        assert_eq!(affinity, 0);
        let restored = capture_backdrop(rect.left, rect.top, 64, 32, [0; 4])
            .ok_or("restored copy unavailable")?;
        assert!(
            restored.pixels.chunks_exact(4).all(|p| p == marker),
            "ordinary captures must include our window again"
        );
        assert_eq!(
            foreground,
            unsafe { GetForegroundWindow() },
            "probe must not steal focus"
        );
        window.destroy();
        write_bmp(output, &excluded)?;
        write_bmp(&format!("{output}.self.bmp"), &before)?;
        println!(
            "PASS: old path captures self; scoped path sees underlying desktop (not black); original capture policy/focus restored; marker destroyed."
        );
    } else {
        let hwnd = unsafe { FindWindowW(w!("termielle_overlay"), PCWSTR::null()) }?;
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect) }?;
        let width = 260;
        let height = 45;
        let x = (rect.left + rect.right) / 2 - width / 2;
        let bg = capture_backdrop(x, rect.top, width as u32, height as u32, [40, 40, 40, 255])
            .ok_or("capture unavailable")?;
        write_bmp(output, &bg)?;
        println!(
            "Captured only the center bar strip at ({x}, {}), {width}x{height}",
            rect.top
        );
    }
    Ok(())
}

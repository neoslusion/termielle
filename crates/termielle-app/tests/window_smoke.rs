use std::thread;
use std::time::Duration;

use termielle_app::animation::FrameBuffer;
use termielle_app::window::{OverlayWindow, Rect, position_within_work_area};
use termielle_core::{AppConfig, WindowPosition};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongW, WS_EX_NOACTIVATE};

const RED_PIXEL: [u8; 4] = [0, 0, 255, 255];

fn red_frame(width: u32, height: u32) -> FrameBuffer {
    let mut pixels_pbgra = Vec::with_capacity((width * height) as usize * 4);
    for _ in 0..(width * height) {
        pixels_pbgra.extend_from_slice(&RED_PIXEL);
    }
    FrameBuffer {
        width,
        height,
        pixels_pbgra,
        delay_ms: 0,
        loop_index: 0,
        scale: 1.0,
    }
}

#[test]
fn window_smoke_present_and_destroy() {
    let mut window = OverlayWindow::create(&AppConfig::default(), true).expect("create window");
    let style = unsafe { GetWindowLongW(window.hwnd(), GWL_EXSTYLE) };
    assert_ne!(
        style as u32 & WS_EX_NOACTIVATE.0,
        0,
        "overlay must not steal terminal focus"
    );
    window.present(&red_frame(2, 2)).expect("present frame");
    thread::sleep(Duration::from_millis(1));
    let (rect, monitor) = window.position();
    assert!(rect.right > rect.left);
    assert!(rect.bottom > rect.top);
    assert!(!monitor.is_empty());
    window.destroy();
}

#[test]
fn window_position_roundtrip_restores_placement() {
    let mut window = OverlayWindow::create(&AppConfig::default(), true).expect("create window");
    window.present(&red_frame(360, 360)).expect("present frame");
    let (rect, monitor) = window.position();
    drop(window);

    let config = AppConfig {
        position: Some(WindowPosition {
            monitor,
            x_logical: rect.left,
            y_logical: rect.top,
        }),
        ..AppConfig::default()
    };
    let mut restored = OverlayWindow::create(&config, true).expect("restore window");
    restored
        .present(&red_frame(360, 360))
        .expect("present frame");
    let (rect2, _) = restored.position();
    assert_eq!(rect, rect2);
    restored.destroy();
}

#[test]
fn window_invalid_persisted_position_clamps_to_available_work_area() {
    let mut fresh = OverlayWindow::create(&AppConfig::default(), true).expect("create window");
    fresh.present(&red_frame(360, 360)).expect("present frame");
    let (_, monitor) = fresh.position();
    fresh.destroy();

    let config = AppConfig {
        position: Some(WindowPosition {
            monitor,
            x_logical: -5000,
            y_logical: -5000,
        }),
        ..AppConfig::default()
    };
    let mut bogus = OverlayWindow::create(&config, true).expect("create window");
    bogus.present(&red_frame(360, 360)).expect("present frame");
    let (bogus_rect, _) = bogus.position();
    // Off-screen coordinates can resolve to a different monitor than a fresh
    // (0,0) window, notably when a display sits left of primary. The contract is
    // safe placement in an available work area, not always the primary corner.
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    assert!(
        unsafe {
            GetMonitorInfoW(
                MonitorFromWindow(bogus.hwnd(), MONITOR_DEFAULTTONEAREST),
                &mut info,
            )
        }
        .as_bool()
    );
    bogus.destroy();
    assert!(position_within_work_area(
        (bogus_rect.left, bogus_rect.top),
        (bogus_rect.width(), bogus_rect.height()),
        Rect {
            left: info.rcWork.left,
            top: info.rcWork.top,
            right: info.rcWork.right,
            bottom: info.rcWork.bottom
        }
    ));
}

#[test]
fn window_presented_rect_matches_frame_size() {
    // Frames arrive authored at device pixels: the window takes their size
    // as-is instead of scaling them.
    let mut window = OverlayWindow::create(&AppConfig::default(), true).expect("create window");
    window.present(&red_frame(720, 720)).expect("present frame");
    let (rect, _) = window.position();
    assert_eq!(rect.width(), 720);
    assert_eq!(rect.height(), 720);
    window.destroy();
}

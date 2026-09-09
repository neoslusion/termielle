use std::thread;
use std::time::Duration;

use termielle_app::animation::FrameBuffer;
use termielle_app::window::OverlayWindow;
use termielle_core::AppConfig;
use windows::Win32::System::Threading::{GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources};

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

/// `present` must not grow the process's GDI object count.
///
/// This lives in its own test binary so no other test's window can inflate
/// the count mid-run, and it counts before the window exists and after it is
/// destroyed. On desktops without DWM (CI runners use the basic display
/// driver) a layered window is realized as GDI objects during its lifetime,
/// which would look like a leak to a count taken while the window is alive.
#[test]
fn window_present_does_not_leak_gdi_objects() {
    let count = || unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
    let baseline = count();

    let mut window = OverlayWindow::create(&AppConfig::default(), true).expect("create window");
    let frame = red_frame(2, 2);
    for _ in 0..240 {
        window.present(&frame).expect("present frame");
    }
    window.destroy();

    // Let a desktop that releases window surfaces one beat later settle.
    thread::sleep(Duration::from_millis(100));
    let after = count();
    assert!(
        after <= baseline + 8,
        "present leaked GDI objects: {baseline} before, {after} after"
    );
}

use termielle_app::{
    backdrop::{Backdrop, capture_backdrop_excluding},
    window::OverlayWindow,
};
use termielle_core::AppConfig;
use windows::Win32::UI::WindowsAndMessaging::{GetDesktopWindow, GetWindowDisplayAffinity};

#[test]
fn scoped_capture_restores_the_original_policy_and_rejects_foreign_owners() {
    let window = OverlayWindow::create(&AppConfig::default(), true).unwrap();
    let mut before = 0;
    unsafe { GetWindowDisplayAffinity(window.hwnd(), &mut before) }.unwrap();
    // Off-screen copy: no actual desktop contents or visible test windows.
    let _ = capture_backdrop_excluding(
        window.hwnd().0 as isize,
        i32::MAX - 16,
        i32::MAX - 16,
        8,
        8,
        [11, 22, 33, 255],
    );
    let mut after = 0;
    unsafe { GetWindowDisplayAffinity(window.hwnd(), &mut after) }.unwrap();
    assert_eq!(before, after);
    assert!(
        capture_backdrop_excluding(unsafe { GetDesktopWindow() }.0 as isize, 0, 0, 8, 8, [0; 4])
            .is_none()
    );
}
#[test]
fn display_recovery_rejects_old_capture_epochs() {
    let mut window = OverlayWindow::create(&AppConfig::default(), true).unwrap();
    let owner = window.hwnd().0 as isize;
    let epoch = window.backdrop_generation();
    window.invalidate_display();
    assert_ne!(window.backdrop_generation(), epoch);
    assert!(!window.set_backdrop(Backdrop {
        capture_token: Some((owner, epoch)),
        ..Default::default()
    }));
    assert!(window.set_backdrop(Backdrop {
        capture_token: Some((owner, window.backdrop_generation())),
        ..Default::default()
    }));
    assert!(!window.set_backdrop(Backdrop {
        capture_token: Some((owner + 1, window.backdrop_generation())),
        ..Default::default()
    }));
}
#[test]
fn changing_material_or_disabling_blur_invalidates_pending_captures() {
    let mut window = OverlayWindow::create(&AppConfig::default(), true).unwrap();
    let epoch = window.backdrop_generation();
    let mut glass = termielle_core::GlassConfig::default();
    glass.tint[0] ^= 16;
    window.set_glass(&glass);
    assert_ne!(epoch, window.backdrop_generation());
    let epoch = window.backdrop_generation();
    window.set_frosted_enabled(false);
    assert_ne!(epoch, window.backdrop_generation());
    let epoch = window.backdrop_generation();
    window.set_frosted_enabled(false);
    assert_eq!(epoch, window.backdrop_generation());
}

//! Isolated hidden windows only. Never launches a real application, hides the
//! native taskbar, or changes the installed process/configuration.
use termielle_app::{
    app_navigation,
    window::{OverlayWindow, WindowEvent},
};
use termielle_core::{AppConfig, AppLaunchTarget, PinnedApp};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongW, PostMessageW, WM_KEYDOWN, WS_EX_NOACTIVATE,
};
fn event(window: &mut OverlayWindow) -> WindowEvent {
    window.set_timer(Some(2000)).unwrap();
    loop {
        if let Some(event) = window.next_event().unwrap() {
            return event;
        }
    }
}
#[test]
fn chooser_keys_route_through_the_native_pump_and_restore_nonactivating_style() {
    std::thread::spawn(|| {
        let mut window = OverlayWindow::create(&AppConfig::default(), true).unwrap();
        assert_ne!(
            unsafe { GetWindowLongW(window.hwnd(), GWL_EXSTYLE) } & WS_EX_NOACTIVATE.0 as i32,
            0
        );
        window.set_navigation_focus(true);
        assert_eq!(
            unsafe { GetWindowLongW(window.hwnd(), GWL_EXSTYLE) } & WS_EX_NOACTIVATE.0 as i32,
            0
        );
        unsafe { PostMessageW(Some(window.hwnd()), WM_KEYDOWN, WPARAM(40), LPARAM(0)) }.unwrap();
        assert_eq!(event(&mut window), WindowEvent::NavigationKey(40));
        window.set_navigation_focus(false);
        assert_ne!(
            unsafe { GetWindowLongW(window.hwnd(), GWL_EXSTYLE) } & WS_EX_NOACTIVATE.0 as i32,
            0
        );
        window.destroy();
    })
    .join()
    .unwrap();
}
#[test]
fn close_validates_the_native_owner_and_posts_a_normal_close_instead_of_killing() {
    std::thread::spawn(|| {
        let mut window = OverlayWindow::create(&AppConfig::default(), true).unwrap();
        assert!(!app_navigation::close_window(
            window.hwnd().0 as isize,
            std::process::id() + 1
        ));
        assert!(app_navigation::close_window(
            window.hwnd().0 as isize,
            std::process::id()
        ));
        assert_eq!(event(&mut window), WindowEvent::Quit);
        window.destroy();
    })
    .join()
    .unwrap();
}
#[test]
fn a_missing_executable_reports_async_failure_without_launching_anything() {
    std::thread::spawn(|| {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.exe");
        assert!(!missing.exists());
        let mut window = OverlayWindow::create(&AppConfig::default(), true).unwrap();
        app_navigation::launch(
            PinnedApp {
                name: "Missing test app".into(),
                target: AppLaunchTarget::Executable(missing.to_string_lossy().into_owned()),
            },
            window.wake_handle(),
        );
        assert!(matches!(event(&mut window),WindowEvent::AppLaunchResult(code) if code!=0));
        window.destroy();
    })
    .join()
    .unwrap();
}

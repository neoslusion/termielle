//! Hidden owned windows only; never changes display modes, focus or taskbar.
use termielle_app::{desktop, window::OverlayWindow};
use termielle_core::{AppConfig, MonitorSelection};
#[test]
fn named_primary_and_missing_display_policy_resolve_without_profile_mutation() {
    std::thread::spawn(|| {
        let displays = desktop::displays();
        assert!(!displays.is_empty());
        let primary = displays.iter().find(|m| m.primary).unwrap();
        let mut config = AppConfig::default();
        let mut window = OverlayWindow::create(&config, true).unwrap();
        for display in &displays {
            config.island.monitor = MonitorSelection::Named(display.device.clone());
            window.configure_monitor(&config.island);
            let bounds = window.monitor_bounds();
            assert_eq!(
                (bounds.left, bounds.top, bounds.right, bounds.bottom),
                display.bounds
            );
        }
        config.island.monitor = MonitorSelection::Named(r"\\.\DISPLAY-MISSING".into());
        let before = config.clone();
        window.configure_monitor(&config.island);
        let bounds = window.monitor_bounds();
        assert_eq!(
            (bounds.left, bounds.top, bounds.right, bounds.bottom),
            primary.bounds
        );
        assert_eq!(config, before);
        assert!(window.set_fullscreen_suppressed(true));
        window.invalidate_display();
        window.configure_monitor(&config.island);
        assert!(window.is_fullscreen_suppressed());
        assert!(window.set_fullscreen_suppressed(false));
        assert!(
            !unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(window.hwnd()) }
                .as_bool(),
            "diagnostics must remain hidden through recovery"
        );
    })
    .join()
    .unwrap();
}

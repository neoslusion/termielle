//! Isolated power control: no installed profile, task settings or global input.
use std::{process::Command, sync::mpsc, time::Duration};
use termielle_app::{
    power,
    window::{OverlayWindow, WindowEvent},
};
use termielle_core::AppConfig;

#[test]
fn disabled_startup_exits_cleanly_before_profile_or_ipc_initialization() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("custom.json");
    std::fs::write(&config, b"not valid JSON: must remain untouched").unwrap();
    let pipe = format!(r"\\.\pipe\termielle-power-test-{}", std::process::id());
    for args in [vec!["--disable"], vec![]] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_termielle-app"));
        let mut child = command
            .args(args)
            .arg("--config")
            .arg(&config)
            .arg("--pipe")
            .arg(&pipe)
            .env("USERPROFILE", home.path())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "disabled startup must not remain resident"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(status.success());
        assert!(power::is_disabled(&home.path().join(".termielle")).unwrap());
        assert_eq!(
            std::fs::read(&config).unwrap(),
            b"not valid JSON: must remain untouched"
        );
        assert!(!home.path().join(".termielle/config.json").exists());
        assert!(!home.path().join(".termielle/events.log").exists());
        assert!(
            termielle_ipc::EventClient::new(&pipe, Duration::from_millis(20))
                .send(b"{}\n")
                .is_err(),
            "off must not bind the event pipe"
        );
    }
}

#[test]
fn off_signal_routes_to_quit_not_chooser_dismissal() {
    std::thread::spawn(|| {
        let dir = tempfile::tempdir().unwrap();
        let mut window = OverlayWindow::create(&AppConfig::default(), true).unwrap();
        window.set_navigation_focus(true);
        let wake = window.wake_handle();
        let (tx, rx) = mpsc::channel();
        let watch = power::Watch::spawn(dir.path(), move || {
            wake.post_power_off().unwrap();
            tx.send(()).unwrap();
        })
        .unwrap();
        power::set_enabled(dir.path(), false).unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let event = loop {
            if let Some(event) = window.next_event().unwrap() {
                break event;
            }
        };
        assert!(
            matches!(event, WindowEvent::Quit),
            "whole-app off must bypass focus dismissal"
        );
        drop(watch);
        window.destroy();
    })
    .join()
    .unwrap();
}

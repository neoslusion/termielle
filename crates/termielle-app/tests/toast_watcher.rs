//! Toast watcher live-thread test: the real STA polling loop runs against
//! the real notification listener. Passes on access grant (polls alive) and
//! on denial (clean silent exit) — both are correct fail-open behavior. A
//! thread panic fails loudly via the join below.

use termielle_core::{AppConfig, IslandLayout};

#[test]
fn toast_watcher_thread_survives_real_polls() {
    let mut config = AppConfig::default();
    config.island.layout = IslandLayout::Island;
    let window = termielle_app::window::OverlayWindow::create(&config, true).unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<termielle_app::toast::ToastBatch>();
    let handle = termielle_app::toast::spawn_toast_watcher(tx, window.wake_handle());
    // Baseline poll is immediate; two more follow on the 3 s cadence.
    std::thread::sleep(std::time::Duration::from_millis(7500));
    for batch in rx.try_iter() {
        for event in &batch.events {
            assert!(!event.display_title().is_empty());
            assert!(event.display_subtitle().chars().count() <= 96);
        }
    }
    // A finished thread must have exited cleanly (access denial path); a
    // live one keeps polling. Joining is safe only in the finished case.
    if handle.is_finished() {
        handle.join().unwrap();
    }
    window.destroy();
}

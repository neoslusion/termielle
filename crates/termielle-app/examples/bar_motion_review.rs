//! Live render/present timing probe. Does not save config or reserve desktop space.
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use termielle_app::app::Controller;
use termielle_app::window::{AnimationClock, OverlayWindow, WindowEvent};
use termielle_core::{AppConfig, AssetCatalog, IslandLayout};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn main() {
    let mut config = AppConfig::default();
    config.island.layout = IslandLayout::Bar;
    config.island.bar.reserve_space = false;
    config.island.bar.replace_taskbar = false;
    if std::env::args().any(|argument| argument == "--macchiato") {
        termielle_app::theme::apply_theme(&mut config.island, "catppuccin-macchiato");
    }
    let controls = std::env::args().any(|argument| argument == "--controls");
    let mut window = OverlayWindow::create(&config, false).unwrap();
    let metrics =
        termielle_app::bar::metrics::Service::spawn(window.wake_handle(), config.island.clone());
    if std::env::args().any(|arg| arg == "--primary") {
        window.pin_primary_monitor();
    }
    let assets = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut controller = Controller::new_with_island(
        5000,
        60000,
        AssetCatalog::new(vec![assets]),
        false,
        None,
        config.island,
    );
    controller.set_dpi_scale(window.dpi_scale());
    controller
        .set_bar_width((window.monitor_width() as f32 / controller.render_scale()).round() as u32);
    controller.enable_display_pacing();
    window
        .present_with_anchor(controller.current_frame(), Some((true, 0)))
        .unwrap();
    let (x, y, width, _) = window.last_dest();
    let glass = &controller.island_config().glass;
    let envelope = ((controller.island_config().bar.height + 320) as f32
        * controller.render_scale())
    .ceil() as u32;
    if let Some(mut backdrop) = termielle_app::backdrop::capture_backdrop_excluding(
        window.hwnd().0 as isize,
        x,
        y,
        width,
        envelope,
        glass.tint,
    ) {
        termielle_app::backdrop::blur_soft(&mut backdrop, glass.blur_radius);
        termielle_app::backdrop::desaturate(&mut backdrop, 0.15);
        window.set_backdrop(backdrop);
    }
    unsafe {
        use windows::Win32::Graphics::Gdi::*;
        let monitor = MonitorFromWindow(window.hwnd(), MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        let mut mode = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info.monitorInfo).as_bool()
            && EnumDisplaySettingsW(
                windows::core::PCWSTR(info.szDevice.as_ptr()),
                ENUM_CURRENT_SETTINGS,
                &mut mode,
            )
            .as_bool()
        {
            println!(
                "Display: {}x{} at {}Hz",
                mode.dmPelsWidth, mode.dmPelsHeight, mode.dmDisplayFrequency
            );
        }
    }
    let clock = AnimationClock::spawn(window.wake_handle());
    let mut costs = Vec::new();
    let mut paints = Vec::new();
    let mut presents = Vec::new();
    let mut intervals = Vec::new();
    let mut last_present = Instant::now();
    let mut due = now_ms();
    clock.arm(Some(due));
    while costs.len() < 240 {
        match window.next_event() {
            Ok(Some(WindowEvent::Quit)) | Err(_) => break,
            Ok(Some(WindowEvent::AnimationFrame)) => {}
            Ok(Some(_)) | Ok(None) => continue,
        }
        let now = now_ms();
        if now < due {
            continue;
        }
        let start = Instant::now();
        if let Some(snapshot) = metrics.take_snapshot() {
            controller.set_bar_metrics(snapshot, now);
        }
        if costs.len() % 60 == 0 {
            if controls {
                let (_, x, y, width, height) = controller
                    .click_regions()
                    .iter()
                    .find(|(id, ..)| *id == termielle_app::app::HIT_CARD_PANEL)
                    .copied()
                    .expect("Control Center bar entry");
                let scale = controller.render_scale();
                controller.handle_click(
                    ((x + width as i32 / 2) as f32 * scale).round() as i32,
                    ((y + height as i32 / 2) as f32 * scale).round() as i32,
                    now,
                );
            } else {
                controller.toggle_expand(now);
            }
        }
        controller.on_timer(now);
        paints.push(start.elapsed().as_secs_f64() * 1000.0);
        let present_start = Instant::now();
        window
            .present_with_anchor(controller.current_frame(), Some((true, 0)))
            .unwrap();
        presents.push(present_start.elapsed().as_secs_f64() * 1000.0);
        costs.push(start.elapsed().as_secs_f64() * 1000.0);
        intervals.push(last_present.elapsed().as_secs_f64() * 1000.0);
        last_present = Instant::now();
        due = now_ms();
        clock.arm(Some(due));
    }
    window.destroy();
    if costs.is_empty() {
        panic!("no frames presented");
    }
    costs.sort_by(f64::total_cmp);
    paints.sort_by(f64::total_cmp);
    presents.sort_by(f64::total_cmp);
    intervals.sort_by(f64::total_cmp);
    let n = costs.len();
    println!(
        "{n} frames; render+present median {:.2}ms p95 {:.2}ms; presentation interval median {:.2}ms p95 {:.2}ms",
        costs[n / 2],
        costs[n * 95 / 100],
        intervals[n / 2],
        intervals[n * 95 / 100]
    );
    println!(
        "paint median {:.2}ms p95 {:.2}ms; present median {:.2}ms p95 {:.2}ms",
        paints[n / 2],
        paints[n * 95 / 100],
        presents[n / 2],
        presents[n * 95 / 100]
    );
}

use termielle_app::{
    app::{AlertKind, ClickOutcome, Controller, HIT_CARD_NOTIFICATIONS, HIT_NOTIFICATIONS_CLEAR},
    window::{OverlayWindow, WindowEvent},
};
use termielle_core::{AppConfig, AssetCatalog, BarPosition, IslandLayout};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP};

fn present(window: &mut OverlayWindow, controller: &Controller) {
    window.present(controller.current_frame()).unwrap();
    window.set_hit_targets(controller.click_regions(), controller.render_scale());
}

fn post_click_part(window: &OverlayWindow, message: u32, point: (i32, i32)) {
    let packed = (point.0 as u32 & 0xffff) | ((point.1 as u32 & 0xffff) << 16);
    unsafe {
        PostMessageW(
            Some(window.hwnd()),
            message,
            WPARAM(usize::from(message == WM_LBUTTONDOWN)),
            LPARAM(packed as isize),
        )
    }
    .unwrap();
}

fn pump(window: &mut OverlayWindow, controller: &mut Controller, release: bool) {
    window.set_timer(Some(2_000)).unwrap();
    loop {
        match window.next_event().unwrap() {
            Some(WindowEvent::PointerDown(column, row)) => {
                controller.navigation_pointer_down(column, row, 2_000);
            }
            Some(WindowEvent::PressChanged(pressed)) => {
                controller.set_pressed(pressed, 2_010);
                if pressed && !release {
                    present(window, controller);
                    return;
                }
            }
            Some(WindowEvent::PointerCancelled) => {
                controller.cancel_navigation_drag(2_010);
            }
            Some(WindowEvent::ClickAt(column, row)) if release => {
                let result = controller
                    .finish_navigation_pointer(column, row, 2_100)
                    .unwrap_or_else(|| controller.handle_click(column, row, 2_100));
                assert_eq!(result, ClickOutcome::NotificationsCleared);
                present(window, controller);
                return;
            }
            Some(WindowEvent::Timer) => panic!("native notification click timed out"),
            _ => {}
        }
        present(window, controller);
    }
}

#[test]
fn clear_all_reaches_the_controller_through_the_native_press_release_pump() {
    std::thread::spawn(|| {
        for position in [BarPosition::Top, BarPosition::Bottom] {
            for scale in [1.0, 1.5] {
                let mut config = AppConfig::default();
                config.island.layout = IslandLayout::Bar;
                config.island.bar.position = position;
                config.island.face_animated = false;
                let mut controller = Controller::new_with_island(
                    5_000,
                    60_000,
                    AssetCatalog::new(Vec::new()),
                    true,
                    None,
                    config.island.clone(),
                );
                controller.set_bar_width(900);
                controller.set_dpi_scale(scale);
                controller.trigger_alert(
                    AlertKind::System,
                    "First message",
                    "Messages: First",
                    [255; 4],
                    6_000,
                    1_000,
                    "native-clear:first",
                );
                let clock = controller
                    .click_regions()
                    .iter()
                    .find(|hit| hit.0 == HIT_CARD_NOTIFICATIONS && hit.3 < 100)
                    .copied()
                    .expect("bar clock hit is available on both edges");
                controller.handle_click(
                    ((clock.1 as f32 + clock.3 as f32 / 2.0) * scale).round() as i32,
                    ((clock.2 as f32 + clock.4 as f32 / 2.0) * scale).round() as i32,
                    1_100,
                );
                let clear = controller
                    .click_regions()
                    .iter()
                    .find(|hit| hit.0 == HIT_NOTIFICATIONS_CLEAR)
                    .copied()
                    .unwrap();
                let point = (
                    ((clear.1 as f32 + clear.3 as f32 / 2.0) * scale).round() as i32,
                    ((clear.2 as f32 + clear.4 as f32 / 2.0) * scale).round() as i32,
                );
                assert!(controller.point_over_open_popup(point));
                let mut window = OverlayWindow::create(&config, true).unwrap();
                window.set_island(true);
                window.set_bar(true);
                present(&mut window, &controller);
                post_click_part(&window, WM_LBUTTONDOWN, point);
                pump(&mut window, &mut controller, false);
                controller.trigger_alert(
                    AlertKind::System,
                    "Second message",
                    "Messages: Second",
                    [255; 4],
                    6_000,
                    2_020,
                    "native-clear:second",
                );
                assert!(controller.point_over_open_popup(point));
                present(&mut window, &controller);
                post_click_part(&window, WM_LBUTTONUP, point);
                pump(&mut window, &mut controller, true);
                assert_eq!(controller.recent_notification_count(), 0);
                assert_eq!(controller.unread_notification_count(), 0);
                assert!(controller.is_notification_center_open());
                assert!(
                    !controller
                        .click_regions()
                        .iter()
                        .any(|hit| hit.0 == HIT_NOTIFICATIONS_CLEAR)
                );
                window.destroy();
            }
        }
    })
    .join()
    .unwrap();
}

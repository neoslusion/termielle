use termielle_app::{
    preferences::{PreferencesWindow, model::Request},
    theme,
    window::OverlayWindow,
};
use termielle_core::{AppConfig, IslandConfig};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{
        BM_SETCHECK, CB_SETCURSEL, CBN_SELCHANGE, GetDlgItem, SendMessageW, WM_COMMAND,
    },
};

fn control(preferences: &PreferencesWindow, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(preferences.hwnd()), id) }.unwrap()
}

fn select(preferences: &PreferencesWindow, id: i32, index: usize) {
    let child = control(preferences, id);
    unsafe {
        SendMessageW(child, CB_SETCURSEL, Some(WPARAM(index)), None);
        SendMessageW(
            preferences.hwnd(),
            WM_COMMAND,
            Some(WPARAM(id as usize | ((CBN_SELCHANGE as usize) << 16))),
            Some(LPARAM(child.0 as isize)),
        );
    }
}

fn click(preferences: &PreferencesWindow, id: usize) {
    unsafe {
        SendMessageW(preferences.hwnd(), WM_COMMAND, Some(WPARAM(id)), None);
    }
}

fn preview(preferences: &PreferencesWindow) -> IslandConfig {
    click(preferences, 130);
    match preferences.take_request() {
        Some(Request::Preview(draft)) => *draft,
        other => panic!("Expected preview, got {other:?}"),
    }
}

#[test]
fn theme_picker_applies_every_preset_without_stale_material() {
    std::thread::spawn(|| {
        let mut config = AppConfig::default();
        theme::select_theme(&mut config.island, "macos-dark");
        config.island.y_offset = 37;
        let owner = OverlayWindow::create(&config, true).unwrap();
        let preferences =
            PreferencesWindow::create(owner.hwnd(), owner.wake_handle(), &config.island).unwrap();
        for (index, name) in [
            "auto",
            "macos-dark",
            "catppuccin-macchiato",
            "liquid-dark",
            "light",
            "midnight",
            "transparent",
        ]
        .iter()
        .enumerate()
        {
            select(&preferences, 105, index);
            let draft = preview(&preferences);
            let mut expected = config.island.clone();
            theme::select_theme(&mut expected, name);
            assert_eq!(draft.theme, *name);
            assert_eq!(draft.glass, expected.glass, "{name}");
            assert_eq!(draft.y_offset, 37);
            assert_eq!(draft.bar, config.island.bar);
        }
    })
    .join()
    .unwrap();
}

#[test]
fn explicit_glass_modes_survive_apply_resolution_and_restart() {
    std::thread::spawn(|| {
        for name in ["transparent", "macos-dark"] {
            let mut config = AppConfig::default();
            theme::select_theme(&mut config.island, name);
            let owner = OverlayWindow::create(&config, true).unwrap();
            let mut preferences =
                PreferencesWindow::create(owner.hwnd(), owner.wake_handle(), &config.island)
                    .unwrap();
            for recording_safe in [false, true] {
                preferences.revert(&config.island);
                select(&preferences, 108, usize::from(recording_safe));
                let draft = preview(&preferences);
                let bytes = serde_json::to_vec(&draft).unwrap();
                let mut restarted: IslandConfig = serde_json::from_slice(&bytes).unwrap();
                theme::resolve_theme(&mut restarted);
                assert_eq!(draft.glass.blur_radius == 0, recording_safe);
                assert_eq!(restarted.glass.blur_radius == 0, recording_safe, "{name}");
            }
        }
    })
    .join()
    .unwrap();
}

#[test]
fn returning_to_custom_animation_after_preview_restores_original_physics() {
    std::thread::spawn(|| {
        let mut config = AppConfig::default();
        config.island.animation_ms = 517;
        config.island.collapse_ms = 313;
        config.island.alert_ms = 147;
        config.island.spring_bounce = 0.27;
        let owner = OverlayWindow::create(&config, true).unwrap();
        let preferences =
            PreferencesWindow::create(owner.hwnd(), owner.wake_handle(), &config.island).unwrap();
        for index in 1..5 {
            select(&preferences, 162, index);
            let preset = preview(&preferences);
            assert_eq!(
                termielle_app::preferences::model::animation_feel(&preset),
                index
            );
        }
        select(&preferences, 162, 0);
        assert_eq!(preview(&preferences), config.island);
    })
    .join()
    .unwrap();
}

#[test]
fn appearance_and_behavior_controls_round_trip_independently() {
    std::thread::spawn(|| {
        let config = AppConfig::default();
        let owner = OverlayWindow::create(&config, true).unwrap();
        let mut preferences =
            PreferencesWindow::create(owner.hwnd(), owner.wake_handle(), &config.island).unwrap();
        for id in [106, 107, 109, 161] {
            for value in [false, true] {
                preferences.revert(&config.island);
                unsafe {
                    SendMessageW(
                        control(&preferences, id),
                        BM_SETCHECK,
                        Some(WPARAM(usize::from(value))),
                        None,
                    )
                };
                let draft = preview(&preferences);
                let mut expected = config.island.clone();
                match id {
                    106 => expected.expand_on_hover = value,
                    107 => expected.face_animated = value,
                    109 => expected.show_name = value,
                    161 => expected.hide_on_fullscreen = value,
                    _ => unreachable!(),
                }
                assert_eq!(draft, expected, "checkbox {id}={value}");
            }
        }
        for (id, count) in [(102, 4), (103, 2), (104, 3), (160, 3)] {
            for index in 0..count {
                preferences.revert(&config.island);
                select(&preferences, id, index);
                let draft = preview(&preferences);
                let mut expected = config.island.clone();
                match id {
                    102 => {
                        expected.layout = [
                            termielle_core::IslandLayout::Bar,
                            termielle_core::IslandLayout::Notch,
                            termielle_core::IslandLayout::Island,
                            termielle_core::IslandLayout::Classic,
                        ][index]
                    }
                    103 => {
                        expected.bar.position = [
                            termielle_core::BarPosition::Top,
                            termielle_core::BarPosition::Bottom,
                        ][index]
                    }
                    104 => expected.bar.height = [28, 36, 44][index],
                    160 => {
                        expected.monitor = [
                            termielle_core::MonitorSelection::Automatic,
                            termielle_core::MonitorSelection::Primary,
                            termielle_core::MonitorSelection::Pointer,
                        ][index]
                            .clone()
                    }
                    _ => unreachable!(),
                }
                assert_eq!(draft, expected, "combo {id}={index}");
            }
        }
    })
    .join()
    .unwrap();
}

#[test]
fn pin_buttons_reorder_remove_and_revert_without_launching() {
    std::thread::spawn(|| {
        let mut config = AppConfig::default();
        config.island.bar.pinned_apps = (0..3)
            .map(|index| termielle_core::PinnedApp {
                name: format!("App {index}"),
                target: termielle_core::AppLaunchTarget::Executable(format!(
                    "C:/Apps/App{index}.exe"
                )),
            })
            .collect();
        let owner = OverlayWindow::create(&config, true).unwrap();
        let mut preferences =
            PreferencesWindow::create(owner.hwnd(), owner.wake_handle(), &config.island).unwrap();
        unsafe { SendMessageW(control(&preferences, 120), 0x0186, Some(WPARAM(1)), None) };
        click(&preferences, 121);
        assert_eq!(preview(&preferences).bar.pinned_apps[0].name, "App 1");
        click(&preferences, 122);
        assert_eq!(
            preview(&preferences).bar.pinned_apps,
            config.island.bar.pinned_apps
        );
        click(&preferences, 123);
        assert_eq!(preview(&preferences).bar.pinned_apps.len(), 2);
        preferences.revert(&config.island);
        assert_eq!(preview(&preferences), config.island);
    })
    .join()
    .unwrap();
}

#[test]
fn unavailable_controls_are_disabled_and_follow_layout_changes() {
    std::thread::spawn(|| {
        use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
        let mut config = AppConfig::default();
        config.island.widgets.retain(|widget| widget != "face");
        let owner = OverlayWindow::create(&config, true).unwrap();
        let preferences =
            PreferencesWindow::create(owner.hwnd(), owner.wake_handle(), &config.island).unwrap();
        let enabled = |id| unsafe { IsWindowEnabled(control(&preferences, id)) }.as_bool();
        assert!(!enabled(107));
        assert!(!enabled(161));
        assert!(enabled(103));
        select(&preferences, 102, 1);
        assert!(!enabled(103));
        assert!(!enabled(104));
        assert!(!enabled(111));
        assert!(enabled(161));
        assert!(enabled(105));
        select(&preferences, 102, 3);
        for id in [105, 108, 106, 107, 109, 160, 161, 162] {
            assert!(!enabled(id));
        }
        select(&preferences, 102, 0);
        assert!(enabled(105));
        assert!(enabled(111));
        assert!(!enabled(161));
        assert!(!enabled(107));
    })
    .join()
    .unwrap();
}

#[test]
fn recording_safe_stays_explicit_when_the_theme_changes() {
    std::thread::spawn(|| {
        let mut config = AppConfig::default();
        theme::select_theme(&mut config.island, "macos-dark");
        let owner = OverlayWindow::create(&config, true).unwrap();
        let preferences =
            PreferencesWindow::create(owner.hwnd(), owner.wake_handle(), &config.island).unwrap();
        select(&preferences, 108, 1);
        assert_eq!(preview(&preferences).glass.blur_radius, 0);
        select(&preferences, 105, 4);
        let mut draft = preview(&preferences);
        assert_eq!(draft.theme, "light");
        assert_eq!(draft.glass.blur_radius, 0);
        assert_eq!(
            draft.glass.backdrop,
            termielle_core::BackdropMode::RecordingSafe
        );
        theme::select_theme(&mut draft, "midnight");
        assert_eq!(draft.glass.blur_radius, 0);
        assert_eq!(draft.glass.tint[..3], [12, 18, 32]);
    })
    .join()
    .unwrap();
}

#[test]
fn bar_item_edits_preserve_order_and_report_invalid_input_without_requests() {
    std::thread::spawn(|| {
        use windows::{
            Win32::UI::WindowsAndMessaging::{GetWindowTextW, SetWindowTextW},
            core::PCWSTR,
        };
        let config = AppConfig::default();
        let owner = OverlayWindow::create(&config, true).unwrap();
        let preferences =
            PreferencesWindow::create(owner.hwnd(), owner.wake_handle(), &config.island).unwrap();
        let set = |id, value: &str| {
            let wide: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
            unsafe { SetWindowTextW(control(&preferences, id), PCWSTR(wide.as_ptr())) }.unwrap();
        };
        set(111, "window, apps, workspaces");
        set(112, "clock, cpu, control_center, network, volume");
        let draft = preview(&preferences);
        assert_eq!(draft.bar.modules_left, ["window", "apps", "workspaces"]);
        assert_eq!(
            draft.bar.modules_right,
            ["clock", "cpu", "control_center", "network", "volume"]
        );
        for invalid in [
            "clock, control_center, unknown",
            "clock, control_center, clock",
            "clock",
            "",
        ] {
            set(112, invalid);
            click(&preferences, 131);
            assert!(preferences.take_request().is_none());
            let mut text = [0u16; 256];
            let count = unsafe { GetWindowTextW(control(&preferences, 140), &mut text) };
            assert!(count > 0);
        }
    })
    .join()
    .unwrap();
}

#[test]
fn reselecting_the_original_theme_resets_stale_overrides_but_no_op_apply_does_not() {
    std::thread::spawn(|| {
        let mut config = AppConfig::default();
        config.island.theme = "macos-dark".into();
        config.island.glass.tint = [71, 82, 93, 204];
        let owner = OverlayWindow::create(&config, true).unwrap();
        let preferences =
            PreferencesWindow::create(owner.hwnd(), owner.wake_handle(), &config.island).unwrap();
        assert_eq!(preview(&preferences), config.island);
        select(&preferences, 105, 4);
        preview(&preferences);
        select(&preferences, 105, 1);
        let draft = preview(&preferences);
        let mut expected = config.island.clone();
        theme::select_theme(&mut expected, "macos-dark");
        assert_eq!(draft, expected);
    })
    .join()
    .unwrap();
}

#[test]
fn all_system_buttons_request_the_correct_action_without_activating_windows() {
    std::thread::spawn(|| {
        use termielle_app::bar::shell::ShellAction;
        let config = AppConfig::default();
        let owner = OverlayWindow::create(&config, true).unwrap();
        let preferences = PreferencesWindow::create(owner.hwnd(), owner.wake_handle(), &config.island).unwrap();
        for (id, expected) in [(150, ShellAction::Sound), (151, ShellAction::Network), (152, ShellAction::Power), (153, ShellAction::Clock), (154, ShellAction::TaskView)] {
            click(&preferences, id);
            assert!(matches!(preferences.take_request(), Some(Request::Shell(actual)) if actual == expected));
        }
        click(&preferences, 155);
        assert!(matches!(preferences.take_request(), Some(Request::TurnOff)));
        assert_eq!(preview(&preferences), config.island);
    }).join().unwrap();
}

#[test]
fn pin_actions_are_disabled_without_selection_and_at_list_boundaries() {
    std::thread::spawn(|| {
        use windows::Win32::UI::{
            Input::KeyboardAndMouse::IsWindowEnabled,
            WindowsAndMessaging::{LB_SETCURSEL, LBN_SELCHANGE},
        };
        let mut config = AppConfig::default();
        config.island.bar.pinned_apps = (0..2)
            .map(|index| termielle_core::PinnedApp {
                name: format!("App {index}"),
                target: termielle_core::AppLaunchTarget::Executable(format!(
                    "C:/Apps/App{index}.exe"
                )),
            })
            .collect();
        let owner = OverlayWindow::create(&config, true).unwrap();
        let preferences =
            PreferencesWindow::create(owner.hwnd(), owner.wake_handle(), &config.island).unwrap();
        let enabled = |id| unsafe { IsWindowEnabled(control(&preferences, id)) }.as_bool();
        for id in [121, 122, 123] {
            assert!(!enabled(id));
        }
        let list = control(&preferences, 120);
        for index in [0, 1] {
            unsafe {
                SendMessageW(list, LB_SETCURSEL, Some(WPARAM(index)), None);
                SendMessageW(
                    preferences.hwnd(),
                    WM_COMMAND,
                    Some(WPARAM(120 | ((LBN_SELCHANGE as usize) << 16))),
                    Some(LPARAM(list.0 as isize)),
                );
            }
            assert_eq!(enabled(121), index > 0);
            assert_eq!(enabled(122), index == 0);
            assert!(enabled(123));
        }
    })
    .join()
    .unwrap();
}

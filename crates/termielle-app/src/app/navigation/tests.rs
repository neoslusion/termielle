use super::model::Action;
use super::*;
use crate::tasks::{WindowInfo, WorkerUpdate};
use termielle_core::{
    AppLaunchTarget, AssetCatalog, BarPosition, IslandConfig, IslandLayout, PinnedApp,
};

fn app(name: &str) -> PinnedApp {
    PinnedApp {
        name: name.into(),
        target: AppLaunchTarget::Executable(format!("C:\\Apps\\{name}.exe")),
    }
}
fn win(hwnd: isize, name: &str) -> WindowInfo {
    WindowInfo {
        hwnd,
        process_id: hwnd as u32 + 20,
        title: format!("Document {hwnd}"),
        application: Some(app(name)),
        minimized: hwnd % 2 == 0,
    }
}
fn controller(position: BarPosition, width: u32, scale: f32) -> Controller {
    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.position = position;
    island.bar.modules_right.clear();
    island.bar.modules_left = vec!["apps".into()];
    let mut c = Controller::new_with_island(
        5000,
        60000,
        AssetCatalog::new(Vec::new()),
        true,
        None,
        island,
    );
    c.set_bar_width(width);
    c.set_dpi_scale(scale);
    c
}
fn update(c: &mut Controller, windows: Vec<WindowInfo>) {
    c.set_task_update_at(
        WorkerUpdate {
            windows,
            ..Default::default()
        },
        1000,
    );
}
fn click(c: &mut Controller, action: Action, context: bool) -> ClickOutcome {
    let hit = c
        .icon_hits
        .iter()
        .copied()
        .find(|h| c.navigation.action(h.0) == Some(action.clone()))
        .expect("painted action");
    let scale = c.render_scale();
    let x = ((hit.1 + hit.3 as i32 / 2) as f32 * scale).round() as i32;
    let y = ((hit.2 + hit.4 as i32 / 2) as f32 * scale).round() as i32;
    if context {
        c.handle_context_click(x, y, 1001)
    } else {
        c.handle_click(x, y, 1001)
    }
}
fn rail_point(c: &Controller, name: &str) -> (i32, i32) {
    let hit = c
        .icon_hits
        .iter()
        .find(|h| c.navigation.action(h.0) == Some(Action::Group(app(name).key())))
        .unwrap();
    let s = c.render_scale();
    (
        ((hit.1 + hit.3 as i32 / 2) as f32 * s).round() as i32,
        ((hit.2 + hit.4 as i32 / 2) as f32 * s).round() as i32,
    )
}
#[test]
fn dragging_pins_commits_once_by_identity_at_both_edges_and_dpi_scales() {
    for edge in [BarPosition::Top, BarPosition::Bottom] {
        for scale in [1.0, 1.25, 2.0] {
            let mut c = controller(edge, 1000, scale);
            c.restore_navigation_pins(vec![app("A"), app("B"), app("C")], 1000);
            update(&mut c, vec![win(1, "A"), win(2, "B"), win(3, "C")]);
            let start = rail_point(&c, "A");
            let end = rail_point(&c, "C");
            c.navigation_pointer_down(start.0, start.1, 1001);
            assert!(c.navigation_pointer_motion(end.0, end.1, 1002));
            assert!(c.wants_navigation_focus());
            assert_eq!(
                c.pinned_apps()[0].name,
                "A",
                "draft drag must not reorder yet"
            );
            assert_eq!(
                c.finish_navigation_pointer(end.0, end.1, 1003),
                Some(ClickOutcome::NavigationPinsChanged)
            );
            assert_eq!(
                c.pinned_apps()
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>(),
                ["B", "C", "A"]
            );
            assert!(!c.wants_navigation_focus());
        }
    }
}
#[test]
fn escape_or_lost_capture_cancels_drag_and_suppresses_the_release_click() {
    let mut c = controller(BarPosition::Top, 1000, 1.0);
    c.restore_navigation_pins(vec![app("A"), app("B")], 1000);
    update(&mut c, vec![win(1, "A"), win(2, "B")]);
    let start = rail_point(&c, "A");
    let end = rail_point(&c, "B");
    c.navigation_pointer_down(start.0, start.1, 1001);
    c.navigation_pointer_motion(end.0, end.1, 1002);
    assert_eq!(
        c.handle_navigation_key(27, 1003),
        ClickOutcome::NavigationChanged
    );
    assert_eq!(
        c.finish_navigation_pointer(end.0, end.1, 1004),
        Some(ClickOutcome::NavigationChanged)
    );
    assert_eq!(c.pinned_apps()[0].name, "A");
    c.navigation_pointer_down(start.0, start.1, 1005);
    assert_eq!(
        c.finish_navigation_pointer(start.0, start.1, 1006),
        None,
        "ordinary clicks still route normally"
    );
}
#[test]
fn dragging_outside_and_external_pin_changes_do_not_commit_stale_drops() {
    let mut c = controller(BarPosition::Top, 1000, 1.0);
    c.restore_navigation_pins(vec![app("A"), app("B")], 1000);
    update(&mut c, vec![win(1, "A"), win(2, "B")]);
    let start = rail_point(&c, "A");
    c.navigation_pointer_down(start.0, start.1, 1001);
    c.navigation_pointer_motion(999, 999, 1002);
    assert_eq!(
        c.finish_navigation_pointer(999, 999, 1003),
        Some(ClickOutcome::NavigationChanged)
    );
    assert_eq!(c.pinned_apps()[0].name, "A");
    let end = rail_point(&c, "B");
    c.navigation_pointer_down(start.0, start.1, 1004);
    c.navigation_pointer_motion(end.0, end.1, 1005);
    c.island.bar.pinned_apps.reverse();
    assert_eq!(
        c.finish_navigation_pointer(end.0, end.1, 1006),
        Some(ClickOutcome::NavigationChanged)
    );
    assert_eq!(c.pinned_apps()[0].name, "B");
}
#[test]
fn tooltips_use_current_identity_and_disappear_during_press_and_popups() {
    let mut c = controller(BarPosition::Top, 1000, 1.25);
    update(&mut c, vec![win(1, "A"), win(2, "A")]);
    assert!(
        c.navigation_tooltips()
            .iter()
            .any(|h| h.text.contains("A — 2 windows") && h.text.contains("choose"))
    );
    let p = rail_point(&c, "A");
    c.navigation_pointer_down(p.0, p.1, 1001);
    assert!(c.navigation_tooltips().is_empty());
    c.finish_navigation_pointer(p.0, p.1, 1002);
    click(&mut c, Action::Group(app("A").key()), false);
    assert!(c.navigation_tooltips().is_empty());
}
#[test]
fn launch_ack_does_not_allow_duplicate_launch_before_window_appears_or_timeout() {
    let mut c = controller(BarPosition::Top, 1000, 1.0);
    c.restore_navigation_pins(vec![app("A")], 1000);
    assert!(matches!(
        click(&mut c, Action::Group(app("A").key()), false),
        ClickOutcome::LaunchApp(_)
    ));
    c.app_launch_finished(0, 1020);
    assert!(!matches!(
        click(&mut c, Action::Group(app("A").key()), false),
        ClickOutcome::LaunchApp(_)
    ));
    assert!(c.next_deadline_ms().is_some());
    c.on_timer(12_000);
    assert!(matches!(
        click(&mut c, Action::Group(app("A").key()), false),
        ClickOutcome::LaunchApp(_)
    ));
}
#[test]
fn launch_completion_observes_window_identity_and_failed_dispatch_clears_pending() {
    let mut c = controller(BarPosition::Top, 1000, 1.0);
    c.restore_navigation_pins(vec![app("A")], 1000);
    click(&mut c, Action::Group(app("A").key()), false);
    c.app_launch_finished(0, 1020);
    update(&mut c, vec![win(1, "A")]);
    assert!(c.refresh_pending_launches(1030));
    assert!(c.navigation.pending.is_empty());
    c.navigation_action(Action::Launch(app("A")), 100, 1040);
    c.app_launch_finished(-1, 1050);
    assert!(c.navigation.pending.is_empty());
}
#[test]
fn close_and_dismiss_actions_are_keyboard_accessible_but_shields_are_not() {
    assert!(Action::Close(1, 2).keyboard_accessible());
    assert!(Action::Dismiss.keyboard_accessible());
    assert!(!Action::Shield.keyboard_accessible());
}

#[test]
fn overflow_reaches_every_iconless_app_in_narrow_top_bottom_and_scaled_layouts() {
    for position in [BarPosition::Top, BarPosition::Bottom] {
        for scale in [1.0, 2.0] {
            let mut c = controller(position, 360, scale);
            update(
                &mut c,
                (1..=18).map(|i| win(i, &format!("App {i:02}"))).collect(),
            );
            assert!(!c.island.bar.replace_taskbar);
            click(&mut c, Action::Overflow, false);
            let mut seen = Vec::new();
            for page in 0..5 {
                assert!(c.is_navigation_open());
                for hit in &c.icon_hits {
                    if let Some(Action::Group(key)) = c.navigation.action(hit.0) {
                        seen.push(key);
                    }
                }
                if page < 4 {
                    click(&mut c, Action::Page(page + 1), false);
                }
            }
            seen.sort();
            seen.dedup();
            assert_eq!(seen.len(), 18);
            assert!(c.point_over_open_popup({
                let h = c.icon_hits[0];
                ((h.1 as f32 * scale) as i32, (h.2 as f32 * scale) as i32)
            }));
            assert!(c.close_navigation(1100));
            assert!(!c.is_navigation_open());
        }
    }
}
#[test]
fn single_window_activates_and_multi_window_opens_identity_based_chooser() {
    let mut c = controller(BarPosition::Top, 1280, 1.0);
    update(
        &mut c,
        vec![win(1, "Editor"), win(2, "Browser"), win(3, "Browser")],
    );
    assert_eq!(
        click(&mut c, Action::Group(app("Editor").key()), false),
        ClickOutcome::ActivateAssociatedWindow(1, 21)
    );
    assert_eq!(
        click(&mut c, Action::Group(app("Browser").key()), false),
        ClickOutcome::NavigationChanged
    );
    assert!(c.is_navigation_open());
    assert_eq!(
        click(&mut c, Action::Focus(3, 23), false),
        ClickOutcome::ActivateAssociatedWindow(3, 23)
    );
    assert!(!c.is_navigation_open());
}
#[test]
fn right_click_pin_unpin_and_closed_pin_launch_do_not_change_taskbar_preference() {
    let mut c = controller(BarPosition::Top, 1280, 1.0);
    update(&mut c, vec![win(1, "Editor")]);
    click(&mut c, Action::Group(app("Editor").key()), true);
    assert_eq!(
        click(&mut c, Action::Pin(app("Editor")), false),
        ClickOutcome::NavigationPinsChanged
    );
    assert_eq!(c.pinned_apps(), &[app("Editor")]);
    assert!(c.close_navigation(1010));
    update(&mut c, vec![]);
    assert!(
        !c.navigation.launching,
        "background updates must never launch pins"
    );
    assert_eq!(
        click(&mut c, Action::Group(app("Editor").key()), false),
        ClickOutcome::LaunchApp(app("Editor"))
    );
    c.app_launch_finished(0, 1020);
    click(&mut c, Action::Group(app("Editor").key()), true);
    assert_eq!(
        click(&mut c, Action::Unpin(app("Editor").key()), false),
        ClickOutcome::NavigationPinsChanged
    );
    assert!(c.pinned_apps().is_empty());
    assert!(!c.island.bar.replace_taskbar);
}
#[test]
fn old_window_action_cannot_target_a_reused_handle_with_a_different_pid() {
    let mut c = controller(BarPosition::Top, 1280, 1.0);
    update(&mut c, vec![win(1, "Editor"), win(2, "Editor")]);
    click(&mut c, Action::Group(app("Editor").key()), false);
    let stale = Action::Focus(1, 21);
    let mut reused = win(1, "Editor");
    reused.process_id = 99;
    update(&mut c, vec![reused, win(2, "Editor")]);
    assert_eq!(
        c.navigation_action(stale, 100, 1010),
        ClickOutcome::NavigationChanged
    );
    assert_eq!(
        click(&mut c, Action::Focus(1, 99), false),
        ClickOutcome::ActivateAssociatedWindow(1, 99)
    );
}
#[test]
fn keyboard_selection_tracks_window_identity_and_pages_without_a_destructive_default() {
    let mut c = controller(BarPosition::Bottom, 1280, 1.0);
    update(&mut c, (1..=9).map(|i| win(i, "Editor")).collect());
    click(&mut c, Action::Group(app("Editor").key()), false);
    assert_eq!(c.navigation.selected, Some(Action::Focus(1, 21)));
    c.handle_navigation_key(40, 1010);
    assert_eq!(c.navigation.selected, Some(Action::Focus(2, 22)));
    c.handle_navigation_key(34, 1020);
    assert_eq!(c.navigation.popup.as_ref().unwrap().page, 1);
    assert_eq!(
        c.handle_navigation_key(13, 1030),
        ClickOutcome::ActivateAssociatedWindow(5, 25)
    );
    assert!(!c.is_navigation_open());
}
#[test]
fn settings_and_launcher_are_reachable_without_the_windows_tray() {
    let mut c = controller(BarPosition::Top, 640, 1.0);
    update(&mut c, vec![win(1, "Editor")]);
    click(&mut c, Action::Overflow, false);
    assert_eq!(
        click(&mut c, Action::Settings, false),
        ClickOutcome::OpenSettings
    );
    assert!(!c.is_navigation_open());
    click(&mut c, Action::Overflow, false);
    assert_eq!(
        click(&mut c, Action::Launcher, false),
        ClickOutcome::Shell(crate::bar::shell::ShellAction::Search)
    );
}
#[test]
fn chooser_dismissal_leaves_agent_card_and_control_center_independent() {
    let mut c = controller(BarPosition::Top, 1280, 1.0);
    update(&mut c, vec![win(1, "Editor"), win(2, "Editor")]);
    c.toggle_expand(900);
    c.open_control_panel(950);
    click(&mut c, Action::Group(app("Editor").key()), false);
    assert!(c.collapse_if_expanded(1100));
    assert!(c.is_panel_open());
    assert!(c.is_manually_expanded());
    assert!(!c.is_navigation_open());
}
#[test]
fn visible_rows_work_mid_resize_and_hidden_footer_never_has_hit_targets() {
    for position in [BarPosition::Top, BarPosition::Bottom] {
        let mut c = controller(position, 1280, 1.0);
        update(&mut c, (1..=6).map(|i| win(i, "Editor")).collect());
        click(&mut c, Action::Group(app("Editor").key()), false);
        c.current = c.render_bar(
            c.state,
            1280,
            c.island.bar.height + super::super::bar::types::BAR_POPUP_GAP + 120,
            1010,
        );
        assert!(
            c.icon_hits
                .iter()
                .any(|h| c.navigation.action(h.0) == Some(Action::Focus(1, 21)))
        );
        assert!(
            !c.icon_hits
                .iter()
                .any(|h| matches!(c.navigation.action(h.0), Some(Action::Pin(_))))
        );
        assert_eq!(
            click(&mut c, Action::Focus(1, 21), false),
            ClickOutcome::ActivateAssociatedWindow(1, 21)
        );
    }
}
#[test]
fn launch_and_save_failures_have_feedback_without_automatic_focus_or_lost_pins() {
    let mut c = controller(BarPosition::Top, 1280, 1.0);
    c.restore_navigation_pins(vec![app("Editor")], 1000);
    c.app_launch_finished(-1, 1100);
    assert!(c.is_navigation_open());
    assert!(!c.wants_navigation_focus());
    assert_eq!(c.pinned_apps(), &[app("Editor")]);
    assert!(
        c.navigation
            .notice
            .as_ref()
            .unwrap()
            .contains("Couldn't open")
    );
    click(&mut c, Action::Dismiss, false);
    assert!(!c.is_navigation_open());
}
#[test]
fn opaque_popup_background_blocks_underlying_card_actions() {
    let mut c = controller(BarPosition::Top, 640, 1.0);
    update(&mut c, vec![win(1, "Editor")]);
    click(&mut c, Action::Overflow, false);
    let (x, y, w, h) = c.navigation_rect(640, c.navigation.height(), 0);
    c.icon_hits.push((
        crate::app::types::HIT_MEDIA_PLAY_PAUSE,
        x + 4,
        y + 42,
        w - 8,
        12,
    ));
    assert_eq!(c.handle_click(x + 8, y + 44, 1010), ClickOutcome::None);
    assert!(h > 0);
}

#[test]
fn title_and_foreground_updates_do_not_move_existing_window_rows() {
    let mut c = controller(BarPosition::Top, 1280, 1.0);
    update(&mut c, vec![win(1, "Editor"), win(2, "Editor")]);
    click(&mut c, Action::Group(app("Editor").key()), false);
    let mut renamed = win(1, "Editor");
    renamed.title = "Z — renamed".into();
    update(&mut c, vec![win(3, "Editor"), win(2, "Editor"), renamed]);
    assert_eq!(
        c.navigation.groups[0]
            .windows
            .iter()
            .map(|w| w.hwnd)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(c.navigation.selected, Some(Action::Focus(1, 21)));
}

#[test]
fn unidentified_host_windows_are_not_guessed_to_be_the_same_application() {
    let mut c = controller(BarPosition::Top, 1280, 1.0);
    let mut a = win(1, "A");
    let mut b = win(2, "B");
    a.application = None;
    b.application = None;
    b.process_id = a.process_id;
    update(&mut c, vec![a, b]);
    assert_eq!(c.navigation.groups.len(), 2);
    assert!(c.navigation.groups.iter().all(|g| g.target.is_none()));
}

#[test]
fn launch_completion_refreshes_buttons_without_replacing_a_reopened_chooser() {
    let mut c = controller(BarPosition::Top, 1280, 1.0);
    update(&mut c, vec![win(1, "Editor"), win(2, "Editor")]);
    c.navigation.launching = true;
    click(&mut c, Action::Group(app("Editor").key()), true);
    assert!(
        !c.icon_hits
            .iter()
            .any(|h| matches!(c.navigation.action(h.0), Some(Action::Launch(_))))
    );
    c.app_launch_finished(0, 1020);
    assert!(
        c.icon_hits
            .iter()
            .any(|h| matches!(c.navigation.action(h.0), Some(Action::Launch(_))))
    );
    c.app_launch_finished(-1, 1030);
    assert!(c.wants_navigation_focus());
    assert!(matches!(
        c.navigation.popup.as_ref().unwrap().mode,
        super::model::Mode::App(_)
    ));
    c.handle_navigation_key(9, 1040);
    c.handle_navigation_key(265, 1041);
    assert_eq!(c.navigation.selected, Some(Action::Focus(1, 21)));
}

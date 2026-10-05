#![allow(clippy::field_reassign_with_default)]

use std::borrow::Cow;
use std::path::Path;

use termielle_app::app::Controller;
use termielle_core::{AssetCatalog, EventKind, EventMessage, IslandConfig, IslandLayout, Source};

fn source(word: &str) -> Source {
    Source::parse(word).unwrap()
}
fn catalog() -> AssetCatalog {
    AssetCatalog::new(Vec::new())
}
fn event(session: &str, kind: EventKind, at: u64) -> EventMessage {
    EventMessage {
        version: termielle_core::PROTOCOL_VERSION,
        source: source("claude"),
        session_id: session.to_string(),
        event: kind,
        timestamp_ms: at,
    }
}

fn island_140_320() -> IslandConfig {
    IslandConfig {
        layout: IslandLayout::Island,
        collapsed_width: 140,
        expanded_width: 320,
        height: 36,
        corner_radius: 18,
        animation_ms: 100,
        collapse_ms: 100,
        alert_ms: 100,
        auto_hide: true,
        ..Default::default()
    }
}

fn visible_idle_island() -> IslandConfig {
    IslandConfig {
        auto_hide: false,
        minimal_width: 140,
        ..island_140_320()
    }
}

#[test]
fn switcher_hover_repaints_on_entry_transfer_and_exit() {
    use termielle_app::tasks::{TaskIcon, WorkerUpdate};
    let mut config = island_140_320();
    config.widgets = vec!["tasks".into()];
    config.show_tasks = true;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, config);
    c.set_task_update(WorkerUpdate {
        tasks: (1..=2)
            .map(|hwnd| TaskIcon {
                hwnd,
                title: format!("Window {hwnd}"),
                width: 1,
                height: 1,
                pixels_pbgra: vec![100, 100, 100, 255],
            })
            .collect(),
        ..Default::default()
    });
    c.toggle_expand(10000);
    // Two centered 44px tiles, separated by 12px, in a 320px card.
    let unhovered = c.current_frame().pixels_pbgra.clone();
    assert!(c.set_hover_point(Some((120, 70))));
    assert!(
        c.current_frame().pixels_pbgra != unhovered,
        "hover must repaint the tile and caption"
    );
    assert!(!c.set_hover_point(Some((121, 71))));
    assert!(c.set_hover_point(Some((180, 70))));
    assert!(c.set_hover_point(None));
    assert!(
        c.current_frame().pixels_pbgra == unhovered,
        "leaving restores the idle tiles"
    );
    assert!(!c.set_hover_point(None));
}

#[test]
fn island_starts_hidden_and_promotes_to_compact_live_on_prompt() {
    let mut island = IslandConfig::default();
    island.layout = IslandLayout::Island;
    island.minimal_width = 72;
    island.collapsed_width = 140;
    island.expanded_width = 320;
    island.height = 36;
    island.corner_radius = 18;
    island.animation_ms = 100; // fast for test
    island.collapse_ms = 100;
    island.alert_ms = 100;
    island.auto_hide = true;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    assert!(c.is_island());
    // macOS / iOS auto-hide: idle with nothing live is hidden (2px top-edge sensor).
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);

    // Prompt -> Thinking promotes to the compact live pill. The real
    // island never auto-expands: live activities ride in the compact pill.
    let actions = c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    assert_eq!(
        actions.visible_state,
        Some(termielle_core::VisualState::Thinking)
    );
    assert!(actions.present_frame);
    // Spring morph in progress toward the compact live pill.
    assert!(c.next_deadline_ms().unwrap() <= 10016);
    let actions = c.on_timer(10050);
    assert!(actions.present_frame);
    assert!(c.current_frame().height > 2);
    // Settled at the compact live pill, hugging its content (face + dot).
    let _ = c.on_timer(10200);
    assert_eq!(c.current_frame().width, 72);
    assert_eq!(c.current_frame().height, 36);
    // Motion ticks pace the thinking bounce ahead of the hold.
    assert_eq!(c.next_deadline_ms(), Some(10216)); // motion tick, then thinking hold at 11000
}

#[test]
fn island_notch_is_attached_and_floating_is_not() {
    let mut notch_cfg = IslandConfig::default();
    notch_cfg.layout = IslandLayout::Notch;
    let mut island_cfg = IslandConfig::default();
    island_cfg.layout = IslandLayout::Island;
    assert!(notch_cfg.is_attached());
    assert!(!island_cfg.is_attached());
    assert!(notch_cfg.is_enabled());
    assert!(island_cfg.is_enabled());
    let mut classic = IslandConfig::default();
    classic.layout = IslandLayout::Classic;
    assert!(!classic.is_enabled());
}

#[test]
fn island_reduced_motion_is_immediate() {
    let mut island = IslandConfig::default();
    island.layout = IslandLayout::Island;
    island.minimal_width = 72;
    island.collapsed_width = 100;
    island.expanded_width = 300;
    island.animation_ms = 500;
    island.collapse_ms = 500;
    island.alert_ms = 500;
    island.auto_hide = true;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, island);
    // Idle with nothing live: hidden.
    assert_eq!(c.current_frame().width, 100);
    assert_eq!(c.current_frame().height, 2);
    c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    // Immediate, no spring: compact live pill while the agent is live.
    assert_eq!(c.current_frame().width, 72);
    assert_eq!(c.current_frame().height, 36);
    assert_eq!(c.next_deadline_ms(), Some(11000));
}

#[test]
fn island_ready_holds_compact_then_idle_hides() {
    let mut island = IslandConfig::default();
    island.layout = IslandLayout::Island;
    island.minimal_width = 72;
    island.collapsed_width = 140;
    island.expanded_width = 300;
    island.animation_ms = 50;
    island.collapse_ms = 50;
    island.alert_ms = 50;
    island.auto_hide = true;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    c.on_timer(10100); // settle at compact live pill
    assert_eq!(c.current_frame().width, 72);
    assert_eq!(c.current_frame().height, 36);

    // Turn completed -> Ready (live activity still active, stays compact)
    c.handle_event(event("s1", EventKind::TurnCompleted, 10200), 10200);
    assert_eq!(c.current_frame().width, 72);
    assert_eq!(c.current_frame().height, 36);

    // After ready hold, goes Idle -> drops all the way to hidden (iOS auto-hide).
    c.on_timer(15200); // 10200+5000 ready hold
    assert_eq!(c.visible_state(), termielle_core::VisualState::Idle);
    // Spring collapse toward hidden.
    assert!(c.next_deadline_ms().is_some());
    c.on_timer(15260);
    c.on_timer(15400);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);
}

#[test]
fn classic_controller_still_uses_fallback_size() {
    let c = Controller::new(5000, 60000, catalog(), false, None);
    assert!(!c.is_island());
    assert_eq!(c.current_frame().width, 360);
    assert_eq!(c.current_frame().height, 360);
}

#[test]
fn empty_hover_stays_hidden_and_click_opens() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    // Idle with nothing live rests hidden.
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);

    assert!(!c.set_hover(true, 10000));
    c.on_timer(10160);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);

    assert_eq!(
        c.handle_click(70, 1, 10200),
        termielle_app::app::ClickOutcome::Expanded
    );
    c.on_timer(10360);
    assert_eq!(c.current_frame().height, 154);
    assert_eq!(
        c.handle_click(70, 18, 10400),
        termielle_app::app::ClickOutcome::Collapsed
    );
    c.on_timer(10560);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);
}

#[test]
fn hover_is_ignored_when_expand_on_hover_is_off() {
    let mut cfg = island_140_320();
    cfg.expand_on_hover = false;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    assert!(!c.set_hover(true, 10000));
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);
}

#[test]
fn click_extends_to_tall_card_and_second_click_collapses() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);

    // Click to extend vertically and horizontally into tall card!
    assert_eq!(
        c.handle_click(70, 1, 10200),
        termielle_app::app::ClickOutcome::Expanded
    );
    c.on_timer(10360);
    assert_eq!(c.current_frame().width, 320);
    assert_eq!(c.current_frame().height, 154);

    // Hover leave while manually extended: stays pinned open!
    assert!(!c.set_hover(false, 10400));
    assert_eq!(c.current_frame().width, 320);
    assert_eq!(c.current_frame().height, 154);

    // Second click collapses back to hidden.
    assert_eq!(
        c.handle_click(70, 18, 10500),
        termielle_app::app::ClickOutcome::Collapsed
    );
    c.on_timer(10660);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2); // unhovered, so returns to hidden
}

#[test]
fn set_task_update_repaints_only_when_visible() {
    use termielle_app::tasks::{MediaInfo, WorkerUpdate};
    let update = || WorkerUpdate {
        windows: Vec::new(),
        media: Some(MediaInfo {
            title: "demo".into(),
            artist: "artist".into(),
            app: "player".into(),
            playing: true,
            thumbnail: None,
        }),
        tasks: Vec::new(),
        backdrop: None,
    };
    // Collapsed idle with no music widget: store but no repaint.
    let mut cfg = island_140_320();
    cfg.widgets.retain(|w| w == "face");
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    assert!(!c.set_task_update(update()));

    // With music widget enabled, media arrival reveals and repaints the pill.
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    assert!(c.set_task_update(update()));
    c.on_timer(10160);
    assert_eq!(c.current_frame().height, 36);
    assert!(!c.set_task_update(update()));
}

#[test]
fn island_without_face_widget_still_renders() {
    let mut cfg = island_140_320();
    cfg.widgets.retain(|w| w != "face");
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    // Idle rests hidden even without a face.
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);
    assert!(!c.set_hover(true, 10000));
    assert_eq!(c.current_frame().height, 2);
    assert_eq!(
        c.handle_click(70, 1, 10000),
        termielle_app::app::ClickOutcome::Expanded
    );
    c.on_timer(10160);
    assert_eq!(c.current_frame().width, 320);
    assert_eq!(c.current_frame().height, 154);
}

#[test]
fn task_switcher_honors_max_thumbnails() {
    use termielle_app::tasks::{TaskIcon, WorkerUpdate};
    let mut cfg = island_140_320();
    cfg.widgets = vec!["tasks".into()];
    cfg.show_tasks = true;
    cfg.max_thumbnails = 2;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, cfg);
    c.set_task_update(WorkerUpdate {
        media: None,
        tasks: (1..=4)
            .map(|hwnd| TaskIcon {
                hwnd,
                title: format!("Window {hwnd}"),
                width: 1,
                height: 1,
                pixels_pbgra: vec![200, 120, 40, 255],
            })
            .collect(),
        backdrop: None,
        ..Default::default()
    });
    c.toggle_expand(1000);

    assert_eq!(
        c.handle_click(132, 80, 1010),
        termielle_app::app::ClickOutcome::ActivateWindow(1)
    );
    assert_eq!(
        c.handle_click(188, 80, 1020),
        termielle_app::app::ClickOutcome::ActivateWindow(2)
    );
    assert_eq!(
        c.handle_click(76, 80, 1030),
        termielle_app::app::ClickOutcome::Collapsed
    );
}

/// Writes a 2x2 infinite-loop GIF where every frame covers the whole canvas.
fn write_gif(dir: &Path, name: &str, frames: &[(u16, u8)]) {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).unwrap();
    {
        let palette = [255, 0, 0, 0, 0, 255, 0, 255, 0];
        let mut encoder = gif::Encoder::new(&mut file, 2, 2, &palette).unwrap();
        encoder.set_repeat(gif::Repeat::Infinite).unwrap();
        for &(delay_hundredths, index) in frames {
            let mut frame = gif::Frame {
                width: 2,
                height: 2,
                left: 0,
                top: 0,
                delay: delay_hundredths,
                dispose: gif::DisposalMethod::Any,
                transparent: None,
                ..gif::Frame::default()
            };
            frame.buffer = Cow::Owned(vec![index; 4]);
            encoder.write_frame(&frame).unwrap();
        }
    }
    file.sync_all().unwrap();
}

#[test]
fn animated_face_repaints_on_its_own_deadline() {
    let dir = tempfile::tempdir().unwrap();
    write_gif(dir.path(), "standby.gif", &[(4, 0), (4, 1), (4, 2)]);
    let catalog = AssetCatalog::new(vec![dir.path().to_path_buf()]);
    let mut cfg = visible_idle_island();
    cfg.face_animated = true;
    let mut c = Controller::new_with_island(5000, 60000, catalog, false, None, cfg);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 36);

    // Idle standalone faces are intentionally paced at 10 fps.
    let actions = c.on_timer(10260);
    assert!(actions.present_frame);
}

#[test]
fn static_face_never_advances() {
    let dir = tempfile::tempdir().unwrap();
    write_gif(dir.path(), "standby.gif", &[(4, 0), (4, 1)]);
    let catalog = AssetCatalog::new(vec![dir.path().to_path_buf()]);
    let mut cfg = island_140_320();
    cfg.face_animated = false;
    let mut c = Controller::new_with_island(5000, 60000, catalog, false, None, cfg);
    c.set_hover(true, 10000);
    c.on_timer(10160);
    let first = c.current_frame().pixels_pbgra.clone();
    c.on_timer(10220);
    c.on_timer(20220);
    assert_eq!(
        c.current_frame().pixels_pbgra,
        first,
        "static face must not advance"
    );
}

#[test]
fn idle_bar_face_stops_repainting_after_its_first_loop() {
    let directory = tempfile::tempdir().unwrap();
    write_gif(directory.path(), "standby.gif", &[(2, 0), (2, 1)]);
    let config = IslandConfig {
        layout: IslandLayout::Bar,
        face_animated: true,
        forward_toasts: false,
        ..IslandConfig::default()
    };
    let mut controller = Controller::new_with_island(
        5_000,
        60_000,
        AssetCatalog::new(vec![directory.path().to_path_buf()]),
        false,
        None,
        config,
    );
    assert!(controller.on_timer(20).present_frame);
    assert!(controller.on_timer(40).present_frame);
    assert!(!controller.on_timer(60).present_frame);
    assert_eq!(controller.next_deadline_ms(), Some(2_000));
    let settled = controller.current_frame().clone();
    assert!(!controller.on_timer(1_000).present_frame);
    assert_eq!(controller.current_frame(), &settled);
}

#[test]
fn island_morphs_height_and_width_on_expand() {
    let mut c =
        Controller::new_with_island(5000, 60000, catalog(), false, None, visible_idle_island());
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 36);

    // Click extends vertically and horizontally into tall card
    assert_eq!(
        c.handle_click(70, 18, 10200),
        termielle_app::app::ClickOutcome::Expanded
    );
    c.on_timer(10360);
    assert_eq!(c.current_frame().width, 320);
    assert_eq!(c.current_frame().height, 154);

    // Click collapses back to pill format
    assert_eq!(
        c.handle_click(70, 18, 10400),
        termielle_app::app::ClickOutcome::Collapsed
    );
    c.on_timer(10560);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 36);

    assert!(!c.set_hover(false, 10600));
    c.on_timer(10760);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 36);
}

#[test]
fn island_notification_alert_expands_and_expires() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);

    // NeedsInput event arrives: triggers notification alert banner
    c.handle_event(event("s1", EventKind::NeedsInput, 10000), 10000);
    c.on_timer(10200);

    // Island auto-expands in both dimensions to show the notification alert
    assert!(c.current_frame().width >= 320);
    assert_eq!(c.current_frame().height, 96);

    // After 3500ms, alert expires and if state goes idle, collapses back
    c.on_timer(13600);
    c.handle_event(event("s1", EventKind::TurnCompleted, 13700), 13700);
    c.on_timer(18800); // after ready hold
    c.on_timer(19100);
    assert_eq!(c.current_frame().height, 2);
}

#[test]
fn alert_click_dismisses_and_returns_to_the_compact_surface() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, island_140_320());
    c.handle_event(event("s1", EventKind::NeedsInput, 10000), 10000);

    assert_eq!(
        c.handle_click(10, 10, 10200),
        termielle_app::app::ClickOutcome::AlertDismiss
    );
    assert_eq!(c.current_frame().height, 36);
}

#[test]
fn bar_alert_click_returns_to_the_bar() {
    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, island);
    c.set_bar_width(1920);
    c.handle_event(event("s1", EventKind::NeedsInput, 10000), 10000);

    assert_eq!(
        c.handle_click(960, 80, 10200),
        termielle_app::app::ClickOutcome::AlertDismiss
    );
    assert_eq!(c.current_frame().height, 36);
}

#[test]
fn bar_alert_grows_from_the_resting_pill_and_returns() {
    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    island.face_animated = false;
    let mut controller = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    controller.set_bar_width(1920);
    let pill_center = (960usize, 18usize);
    let alpha_at_pill = |controller: &Controller| {
        let frame = controller.current_frame();
        frame.pixels_pbgra[(pill_center.1 * frame.width as usize + pill_center.0) * 4 + 3]
    };
    assert!(alpha_at_pill(&controller) > 0);

    controller.handle_event(event("s1", EventKind::NeedsInput, 10_000), 10_000);
    for tick in (10_016..10_800).step_by(16) {
        controller.on_timer(tick);
    }
    assert!(controller.current_frame().height > 36);
    assert!(
        alpha_at_pill(&controller) > 0,
        "the alert stays attached to the pill"
    );

    controller.handle_click(960, 80, 10_800);
    for tick in (10_816..11_600).step_by(16) {
        controller.on_timer(tick);
    }
    assert_eq!(controller.current_frame().height, 36);
    assert!(
        alpha_at_pill(&controller) > 0,
        "the pill returns after dismissal"
    );
}

#[test]
fn bar_alert_and_control_center_keep_separate_surfaces() {
    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    island.face_animated = false;
    let mut controller = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    controller.set_bar_width(1536);
    let control_center = control_center_point(&controller);
    controller.handle_click(control_center.0, control_center.1, 1_000);
    for tick in (1_016..1_800).step_by(16) {
        controller.on_timer(tick);
    }
    assert!(controller.is_panel_open());

    controller.handle_event(event("s1", EventKind::NeedsInput, 2_000), 2_000);
    for tick in (2_016..2_800).step_by(16) {
        controller.on_timer(tick);
    }
    let frame = controller.current_frame();
    let alpha = |x: usize, y: usize| frame.pixels_pbgra[(y * frame.width as usize + x) * 4 + 3];
    assert!(alpha(768, 18) > 0, "the alert grows from the center pill");
    assert!(alpha(768, 80) > 0, "the alert stays centered on the notch");
    assert!(
        alpha(control_center.0 as usize, 80) > 0,
        "Control Center stays visible beside the alert"
    );
    assert!(controller.is_panel_open());
    assert!(
        controller
            .click_regions()
            .iter()
            .any(|(_, x, y, _, _)| *x > 1000 && *y > 120),
        "Control Center controls remain clickable during the alert"
    );

    controller.on_timer(6_000);
    assert!(controller.is_panel_open());
    let frame = controller.current_frame();
    let center_alpha = frame.pixels_pbgra[(80 * frame.width as usize + 768) * 4 + 3];
    assert_eq!(center_alpha, 0, "the expired alert leaves no center ghost");
}

#[test]
fn bar_popup_alert_timeout_drains_at_frame_rate() {
    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    island.face_animated = false;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.set_bar_width(1920);
    c.handle_event(event("s1", EventKind::NeedsInput, 10_000), 10_000);
    c.on_timer(10_200);
    assert!(
        c.current_frame().height > 36,
        "the alert should open the popup card"
    );

    // The remaining life has to drain between frames. Riding the bar's
    // two-second metrics refresh makes the countdown read as a stutter.
    let first = c.current_frame().pixels_pbgra.clone();
    c.on_timer(10_216);
    let second = c.current_frame().pixels_pbgra.clone();
    assert_ne!(
        first, second,
        "visible alert timeout must repaint every frame"
    );
    assert!(
        c.next_deadline_ms().is_some_and(|due| due <= 10_216 + 16),
        "countdown must schedule the next frame"
    );
}

#[test]
fn alert_countdown_stops_once_the_banner_is_gone() {
    use termielle_app::app::ClickOutcome;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.handle_event(event("s1", EventKind::NeedsInput, 10_000), 10_000);
    c.on_timer(10_016);
    assert!(
        c.next_deadline_ms().is_some_and(|due| due <= 10_016 + 16),
        "a visible banner schedules its countdown"
    );

    assert_eq!(c.handle_click(10, 10, 10_200), ClickOutcome::AlertDismiss);
    // Leave the input-pulse state, then let the dismiss morph and the ready
    // hold run out, so the countdown is the only thing left that could keep
    // the surface repainting.
    c.handle_event(event("s1", EventKind::TurnCompleted, 10_300), 10_300);
    for tick in (10_400..20_000).step_by(16) {
        c.on_timer(tick);
    }
    let settled_at = 20_000;
    let deadline = c.next_deadline_ms();
    assert!(
        deadline.is_none_or(|due| due > settled_at + 16),
        "a dismissed banner must not keep per-frame repaints: {deadline:?}"
    );
    assert!(
        !c.on_timer(settled_at + 16).present_frame,
        "a settled surface must stay quiet"
    );
}

#[test]
fn media_transport_hits_match_the_refined_control_row() {
    use termielle_app::tasks::{MediaInfo, WorkerUpdate};

    let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, island_140_320());
    c.set_task_update(WorkerUpdate {
        media: Some(MediaInfo {
            title: "Song".into(),
            artist: "Artist".into(),
            app: "Player".into(),
            playing: true,
            ..Default::default()
        }),
        ..Default::default()
    });
    c.toggle_expand(10000);

    assert_eq!(
        c.handle_click(108, 148, 10200),
        termielle_app::app::ClickOutcome::MediaPrev
    );
    assert_eq!(
        c.handle_click(160, 148, 10200),
        termielle_app::app::ClickOutcome::MediaToggle
    );
    assert_eq!(
        c.handle_click(212, 148, 10200),
        termielle_app::app::ClickOutcome::MediaNext
    );
}

#[test]
fn island_minimal_mode_when_auto_hide_disabled() {
    let mut cfg = island_140_320();
    cfg.auto_hide = false;
    let c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    // When auto_hide is off, idle rests at minimal dot
    assert_eq!(c.current_frame().width, 72);
    assert_eq!(c.current_frame().height, 36);
}

#[test]
fn island_compact_media_sizing() {
    use termielle_app::tasks::{MediaInfo, WorkerUpdate};
    let mut cfg = island_140_320();
    cfg.collapsed_width = 140;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    assert_eq!(c.current_frame().height, 2);

    // When media starts playing, island automatically rests in compact pill format!
    c.set_task_update(WorkerUpdate {
        media: Some(MediaInfo {
            title: "Song".into(),
            artist: "Artist".into(),
            app: "Spotify".into(),
            playing: true,
            thumbnail: None,
        }),
        tasks: Vec::new(),
        backdrop: None,
        ..Default::default()
    });
    c.on_timer(10050);
    c.on_timer(10200);
    // Width hugs the live content: idle label width (140) plus the media
    // art + equalizer, minus the shared edge padding.
    assert_eq!(c.current_frame().width, 186);
    assert_eq!(c.current_frame().height, 36);
}

#[test]
fn island_extended_notch_stays_flush_with_transparent_corners() {
    let mut cfg = IslandConfig::default();
    cfg.layout = IslandLayout::Notch;
    cfg.collapsed_width = 140;
    cfg.expanded_width = 320;
    cfg.y_offset = 0;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, cfg);

    // Idle notch: attached
    assert_eq!(c.island_anchor(), Some((true, 0)));

    // User clicks -> expands to full dashboard card
    c.handle_click(10, 10, 1000);
    assert_eq!(
        c.presentation(),
        termielle_app::animation::notch::Presentation::Expanded
    );

    // The anchor never moves during or after a morph: the notch grows
    // downward from the bezel, like the real one, instead of popping
    // detached. The window y stays 0 in every presentation.
    assert_eq!(c.island_anchor(), Some((true, 0)));

    let frame = c.current_frame();
    assert_eq!(frame.width, 320);
    assert_eq!(frame.height, 154);
    // Flat top edge fused with the bezel: the top corners are square.
    // (The right sample sits one pixel inboard: the SDF's anti-aliased
    // edge lands exactly on the last column.)
    assert!(frame.pixels_pbgra[3] > 180); // top-left
    assert!(frame.pixels_pbgra[((320 - 2) * 4) + 3] > 180); // top-right
    // The bottom corners round away to transparency.
    let bottom_left_idx = ((153 * 320) * 4) + 3;
    assert_eq!(frame.pixels_pbgra[bottom_left_idx], 0); // bottom-left
    let bottom_right_idx = ((153 * 320 + 319) * 4) + 3;
    assert_eq!(frame.pixels_pbgra[bottom_right_idx], 0); // bottom-right

    // Center interior pixel is opaque/translucent glass
    let center_idx = ((77 * 320 + 160) * 4) + 3;
    assert!(frame.pixels_pbgra[center_idx] > 180);
}

#[test]
fn split_island_two_blobs_when_agent_and_media_both_live() {
    use termielle_app::tasks::{MediaInfo, WorkerUpdate};
    let mut cfg = island_140_320();
    cfg.animation_ms = 50;
    cfg.collapse_ms = 50;
    cfg.alert_ms = 50;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);

    // Agent goes live: compact live pill.
    c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    let _ = c.on_timer(10120);
    assert_eq!(c.current_frame().width, 72);
    assert_eq!(c.current_frame().height, 36);

    // Media starts playing: the island splits — primary blob, 9px gap,
    // media blob — exactly like two live activities on the real island.
    c.set_task_update(WorkerUpdate {
        media: Some(MediaInfo {
            title: "Song".into(),
            artist: "Artist".into(),
            app: "Player".into(),
            playing: true,
            thumbnail: None,
        }),
        tasks: Vec::new(),
        backdrop: None,
        ..Default::default()
    });
    let _ = c.on_timer(10200);
    let _ = c.on_timer(10400);
    let frame = c.current_frame();
    // Union width: primary (66) + gap (9) + media (54).
    assert_eq!(frame.width, 129);
    assert_eq!(frame.height, 36);
    let alpha_at = |x: u32, y: u32| frame.pixels_pbgra[(((y * frame.width + x) * 4) + 3) as usize];
    // Both blob interiors are opaque...
    assert!(alpha_at(20, 18) > 150, "primary blob must render");
    assert!(alpha_at(102, 18) > 150, "media blob must render");
    // ...and the gap between them is fully transparent: two blobs.
    for gap_x in [67u32, 70, 74] {
        assert_eq!(alpha_at(gap_x, 18), 0, "gap must be transparent at {gap_x}");
    }
    // Once separation settles it must be removed, or procedural motion stays
    // gated forever on a zero-velocity Some spring.
    let settled = c.current_frame().pixels_pbgra.clone();
    c.on_timer(10520);
    assert_ne!(
        settled,
        c.current_frame().pixels_pbgra,
        "thinking/equalizer motion must continue after the split settles"
    );
}

#[test]
fn paused_media_stays_visible_and_can_resume() {
    use termielle_app::tasks::{MediaInfo, WorkerUpdate};
    let mut cfg = island_140_320();
    cfg.animation_ms = 50;
    cfg.collapse_ms = 50;
    cfg.alert_ms = 50;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    c.set_task_update(WorkerUpdate {
        media: Some(MediaInfo {
            title: "Song".into(),
            artist: "Artist".into(),
            app: "Player".into(),
            playing: true,
            thumbnail: None,
        }),
        tasks: Vec::new(),
        backdrop: None,
        ..Default::default()
    });
    let _ = c.on_timer(10400);
    c.set_task_update(WorkerUpdate {
        media: Some(MediaInfo {
            title: "Song".into(),
            artist: "Artist".into(),
            app: "Player".into(),
            playing: false,
            thumbnail: None,
        }),
        tasks: Vec::new(),
        backdrop: None,
        ..Default::default()
    });
    for t in (10800..11200).step_by(16) {
        let _ = c.on_timer(t);
    }
    let frame = c.current_frame();
    assert_eq!((frame.width, frame.height), (129, 36));
    let alpha_at = |x: u32, y: u32| frame.pixels_pbgra[(((y * frame.width + x) * 4) + 3) as usize];
    assert!(
        alpha_at(20, 18) > 150,
        "paused primary module must remain visible"
    );
    assert!(
        alpha_at(102, 18) > 150,
        "paused media module must remain visible"
    );
    assert_eq!(
        c.handle_click(102, 18, 10900),
        termielle_app::app::ClickOutcome::MediaToggle
    );
}

#[test]
fn media_arriving_during_compact_morph_retargets_the_active_spring() {
    use termielle_app::tasks::{MediaInfo, WorkerUpdate};
    let mut cfg = island_140_320();
    cfg.animation_ms = 100;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);

    c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    assert!(c.set_task_update(WorkerUpdate {
        media: Some(MediaInfo {
            title: "Song".into(),
            artist: "Artist".into(),
            app: "Player".into(),
            playing: true,
            thumbnail: None,
        }),
        tasks: Vec::new(),
        backdrop: None,
        ..Default::default()
    }));

    // Still inside the one-second thinking hold: only the worker-driven
    // retarget may move the pill to the two-blob geometry. A later reducer
    // state flip must not be required to repair the interrupted morph.
    for now in (10016..10400).step_by(16) {
        c.on_timer(now);
    }
    assert_eq!(
        (c.current_frame().width, c.current_frame().height),
        (129, 36)
    );
}

#[test]
fn duplicate_critical_events_do_not_requeue_an_alert() {
    for kind in [EventKind::NeedsInput, EventKind::TurnFailed] {
        let mut c =
            Controller::new_with_island(5000, 60000, catalog(), true, None, island_140_320());
        c.handle_event(event("s1", kind, 10000), 10000);
        assert_eq!(c.current_frame().height, 96);
        assert_eq!(
            c.handle_click(10, 10, 10100),
            termielle_app::app::ClickOutcome::AlertDismiss
        );

        c.handle_event(event("s1", kind, 10000), 10200);
        assert_eq!(
            c.current_frame().height,
            36,
            "duplicate {kind:?} must not resurrect its banner"
        );
    }
}

#[test]
fn replay_cannot_resurrect_a_session_older_than_the_stale_horizon() {
    const STALE_SESSION_MS: u64 = 4 * 60 * 60 * 1000;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, island_140_320());
    c.handle_event(
        event("old", EventKind::NeedsInput, 1000),
        1000 + STALE_SESSION_MS + 1,
    );

    assert_eq!(c.visible_state(), termielle_core::VisualState::Idle);
    assert_eq!(c.current_frame().height, 2);
}

#[test]
fn same_size_state_changes_crossfade_content_without_container_restart() {
    let mut cfg = island_140_320();
    cfg.face_animated = false;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    for now in (10016..10500).step_by(16) {
        c.on_timer(now);
    }
    let thinking = c.current_frame().pixels_pbgra.clone();

    c.on_timer(11000);
    let transition_start = c.current_frame().pixels_pbgra.clone();
    c.on_timer(11070);
    let transition_mid = c.current_frame().pixels_pbgra.clone();
    c.on_timer(11160);
    let transition_end = c.current_frame().pixels_pbgra.clone();

    assert_ne!(
        thinking, transition_start,
        "new state must not pop in directly"
    );
    assert_ne!(
        transition_start, transition_mid,
        "content must blend after the state change"
    );
    assert_ne!(
        transition_mid, transition_end,
        "content must settle after the interruption"
    );
    assert_eq!(
        (c.current_frame().width, c.current_frame().height),
        (72, 36)
    );
}

#[test]
fn press_swell_grows_the_pill_and_release_restores_it() {
    let mut c =
        Controller::new_with_island(5000, 60000, catalog(), false, None, visible_idle_island());
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 36);

    // Pointer down: the pill swells a few percent, like under a fingertip.
    assert!(c.set_pressed(true, 10200));
    let _ = c.on_timer(10360);
    assert_eq!(c.current_frame().width, 144);
    assert_eq!(c.current_frame().height, 38);

    // Release: settles back to the resting pill.
    assert!(c.set_pressed(false, 10400));
    let _ = c.on_timer(10600);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 36);
}

#[test]
fn notch_material_is_true_black_island_is_glass() {
    // Attached notch: opaque true black, fused with the bezel.
    let mut notch = visible_idle_island();
    notch.layout = IslandLayout::Notch;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, notch);
    let frame = c.current_frame();
    let idx = ((18 * frame.width + 20) * 4) as usize;
    assert_eq!(frame.pixels_pbgra[idx..idx + 4], [0, 0, 0, 255]);

    // Floating island: glass, not black.
    let mut island = visible_idle_island();
    island.layout = IslandLayout::Island;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    let frame = c.current_frame();
    let idx = ((18 * frame.width + 20) * 4) as usize;
    let pixel = &frame.pixels_pbgra[idx..idx + 4];
    assert!(pixel[3] > 150);
    assert!(pixel[0] + pixel[1] + pixel[2] > 30, "glass tint must show");
}

#[test]
fn render_scale_defaults_to_one() {
    let c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    assert_eq!(c.render_scale(), 1.0);
}

#[test]
fn render_scale_multiplies_dpi_and_user_zoom() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.set_dpi_scale(1.5);
    assert_eq!(c.render_scale(), 1.5);
    c.set_user_scale(2.0);
    assert_eq!(c.render_scale(), 3.0);
}

#[test]
fn render_scale_ignores_dpi_when_opted_out() {
    let mut cfg = island_140_320();
    cfg.scale_with_dpi = false;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    c.set_dpi_scale(2.0);
    assert_eq!(c.render_scale(), 1.0);
    c.set_user_scale(1.5);
    assert_eq!(c.render_scale(), 1.5);
}

#[test]
fn render_scale_rejects_garbage_and_clamps() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.set_dpi_scale(f32::NAN);
    c.set_dpi_scale(0.0);
    c.set_dpi_scale(-2.0);
    c.set_user_scale(f32::INFINITY);
    assert_eq!(c.render_scale(), 1.0);
    c.set_dpi_scale(100.0);
    assert_eq!(c.render_scale(), 4.0);
    c.set_user_scale(100.0);
    assert_eq!(c.render_scale(), 4.0);
}

#[test]
fn physical_click_maps_to_logical_hit_rect() {
    // Same scenario as `click_extends_to_tall_card`, but the client reports
    // physical pixels at 200%: (140, 36) must land on logical (70, 18).
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.set_hover(true, 10000);
    c.on_timer(10160);
    c.set_dpi_scale(2.0);
    assert_eq!(
        c.handle_click(140, 36, 10200),
        termielle_app::app::ClickOutcome::Expanded
    );
    c.on_timer(10360);
    assert_eq!(
        c.handle_click(140, 36, 10500),
        termielle_app::app::ClickOutcome::Collapsed
    );
}

#[test]
fn island_anchor_scales_y_offset_with_dpi() {
    let mut cfg = island_140_320();
    cfg.y_offset = 12;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    assert_eq!(
        c.island_anchor(),
        Some((false, 0)),
        "hidden sensor stays on the bezel"
    );
    assert!(!c.set_hover(true, 0));
    assert_eq!(c.island_anchor(), Some((false, 0)));
    c.toggle_expand(0);
    assert_eq!(c.island_anchor(), Some((false, 12)));
    c.set_dpi_scale(2.0);
    assert_eq!(c.island_anchor(), Some((false, 24)));
    // Attached notches stay flush regardless of scale.
    let mut notch = island_140_320();
    notch.layout = IslandLayout::Notch;
    let mut n = Controller::new_with_island(5000, 60000, catalog(), false, None, notch);
    n.set_dpi_scale(2.0);
    assert_eq!(n.island_anchor(), Some((true, 0)));
}

#[test]
fn manual_expansion_survives_mid_turn_flips() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    c.set_hover(true, 10000);
    c.on_timer(10160);
    assert_eq!(
        c.handle_click(70, 18, 10200),
        termielle_app::app::ClickOutcome::Expanded
    );
    c.on_timer(10360);
    assert_eq!(c.current_frame().width, 320);
    assert_eq!(c.current_frame().height, 154);

    // Thinking -> Working must not collapse the user's open card.
    c.on_timer(11200);
    assert_eq!(c.visible_state(), termielle_core::VisualState::Working);
    assert_eq!(c.current_frame().width, 320);
    assert_eq!(c.current_frame().height, 154);
    // Still the user's card: the next click collapses, not re-expands.
    assert_eq!(
        c.handle_click(70, 18, 11300),
        termielle_app::app::ClickOutcome::Collapsed
    );
}

#[test]
fn ending_the_session_retires_the_open_card_but_completion_keeps_it_available() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    c.set_hover(true, 10000);
    c.on_timer(10160);
    assert_eq!(
        c.handle_click(70, 18, 10200),
        termielle_app::app::ClickOutcome::Expanded
    );
    c.on_timer(10360);
    assert_eq!(c.current_frame().height, 154);

    // The turn completes: the card stays open through Ready ...
    c.handle_event(event("s1", EventKind::TurnCompleted, 10400), 10400);
    c.on_timer(10600);
    // ... and remains available as a finished session after Ready expires.
    c.on_timer(15600);
    assert_eq!(c.visible_state(), termielle_core::VisualState::Idle);
    assert!(c.is_manually_expanded());
    assert_eq!(c.current_frame().height, 154);
    // Explicit session end removes it and retires the card.
    c.handle_event(event("s1", EventKind::SessionEnded, 15600), 15600);
    c.set_hover(false, 15600);
    c.on_timer(15760);
    c.on_timer(16000);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);
}

#[test]
fn critical_agent_alert_yields_back_to_a_deferred_system_toast() {
    use termielle_app::app::AlertKind;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, island_140_320());
    assert!(c.trigger_alert(
        AlertKind::System,
        "System",
        "Background notification",
        [80, 160, 255, 255],
        6000,
        10000,
        "toast:1",
    ));
    c.handle_event(event("s1", EventKind::NeedsInput, 10100), 10100);

    assert_eq!(
        c.handle_click(10, 10, 10200),
        termielle_app::app::ClickOutcome::AlertDismiss
    );
    assert_eq!(
        c.current_frame().height,
        96,
        "the preempted system toast must resume in front"
    );
}
#[test]

fn stale_replayed_needs_input_raises_no_banner() {
    // Journal replay on startup: a days-old needs_input still folds into
    // state (the face shows waiting) but must not resurrect its banner.
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.handle_event(event("s1", EventKind::NeedsInput, 10000), 200000);
    // Settle past the morph end (but inside the 3.5 s banner life, so a
    // buggy banner would be fully up): the pill must stay compact.
    c.on_timer(200200);
    c.on_timer(200600);
    assert_eq!(c.visible_state(), termielle_core::VisualState::NeedsInput);
    assert_eq!(c.current_frame().height, 36);
}

#[test]
fn alert_freshness_boundary_still_banners() {
    // Exactly at the freshness horizon the event is still news.
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.handle_event(event("s1", EventKind::NeedsInput, 10000), 70000);
    c.on_timer(70200);
    assert_eq!(c.current_frame().width, 320);
    assert_eq!(c.current_frame().height, 96);
}

#[test]
fn alert_queue_plays_second_banner_after_first_expires() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    // Two sessions need input 400 ms apart: the first banner shows.
    c.handle_event(event("s1", EventKind::NeedsInput, 10000), 10000);
    c.handle_event(event("s2", EventKind::NeedsInput, 10400), 10400);
    c.on_timer(10600);
    assert_eq!(c.current_frame().width, 320);
    assert_eq!(c.current_frame().height, 96);
    let first = c.current_frame().pixels_pbgra.clone();

    // Past the first banner's 3.5 s life the second takes over instead of
    // collapsing: same card, different session badge.
    c.on_timer(13600);
    c.on_timer(13700);
    assert_eq!(c.current_frame().width, 320);
    assert_eq!(c.current_frame().height, 96);
    assert_ne!(
        c.current_frame().pixels_pbgra,
        first,
        "second banner must replace the first"
    );

    // Both turns complete: after the holds the island hides with no banner.
    c.handle_event(event("s1", EventKind::TurnCompleted, 14000), 14000);
    c.handle_event(event("s2", EventKind::TurnCompleted, 14100), 14100);
    c.on_timer(20000);
    c.on_timer(20200);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);
}

#[test]
fn queued_alert_timeout_starts_when_it_reaches_the_front() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, island_140_320());
    c.handle_event(event("s1", EventKind::NeedsInput, 10000), 10000);
    c.handle_event(event("s2", EventKind::NeedsInput, 13000), 13000);

    // The first banner owns the screen until 13.5 s. The second then receives
    // its own full 3.5 s lifetime rather than silently aging out in queue.
    c.on_timer(13600);
    assert_eq!(c.current_frame().height, 96);
    c.on_timer(16400);
    assert_eq!(c.current_frame().height, 96);
    c.on_timer(16900);
    assert_eq!(c.current_frame().height, 96);
    c.on_timer(17500);
    assert_eq!(c.current_frame().height, 36);
}
#[test]
fn collapse_settles_on_its_own_faster_timing() {
    // Expand slowly (500 ms), collapse fast (50 ms): the collapse must be
    // settled 200 ms later, which the shared slow spring could never do.
    // No agent events: NeedsInput would raise its own alert card and hide
    // the manual-expansion target this measures.
    let mut cfg = visible_idle_island();
    cfg.animation_ms = 500;
    cfg.collapse_ms = 50;
    cfg.alert_ms = 50;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    for t in (10000..12000).step_by(50) {
        c.on_timer(t);
    }
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 36);
    assert_eq!(
        c.handle_click(70, 18, 12100),
        termielle_app::app::ClickOutcome::Expanded
    );
    for t in (12100..13500).step_by(50) {
        c.on_timer(t);
    }
    assert_eq!(c.current_frame().width, 320);
    assert_eq!(c.current_frame().height, 154);
    c.set_hover(false, 13500);
    assert_eq!(
        c.handle_click(70, 18, 13600),
        termielle_app::app::ClickOutcome::Collapsed
    );
    for t in (13600..13800).step_by(50) {
        c.on_timer(t);
    }
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 36);
}

#[test]
fn thinking_dots_bounce_between_motion_ticks() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    // Settle the morph with fine steps; still Thinking (hold fires at 11000).
    for t in (10000..10400).step_by(50) {
        c.on_timer(t);
    }
    let a = c.current_frame().pixels_pbgra.clone();
    for t in (10400..10550).step_by(50) {
        c.on_timer(t);
    }
    let b = c.current_frame().pixels_pbgra.clone();
    assert_eq!(c.visible_state(), termielle_core::VisualState::Thinking);
    assert_ne!(a, b, "thinking bounce must repaint between motion ticks");
}

#[test]
fn failed_shake_settles_pixel_stable() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    c.handle_event(event("s1", EventKind::TurnFailed, 10100), 10100);
    assert_eq!(c.visible_state(), termielle_core::VisualState::Failed);
    // Dismiss the failure banner first: a live timeout keeps its hairline
    // moving, so the shake cannot be measured through it. The banner's hit
    // target exists once the card has actually painted.
    c.on_timer(10200);
    assert_eq!(
        c.handle_click(10, 10, 10200),
        termielle_app::app::ClickOutcome::AlertDismiss
    );
    // Past the dismiss morph and the 300 ms shake window: locked to zero.
    for t in (10250..11000).step_by(50) {
        c.on_timer(t);
    }
    let a = c.current_frame().pixels_pbgra.clone();
    for t in (11000..11150).step_by(50) {
        c.on_timer(t);
    }
    assert_eq!(
        a,
        c.current_frame().pixels_pbgra,
        "settled failed frames must be identical"
    );
}

#[test]
fn ready_sparkle_appears_then_vanishes() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());

    c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    c.set_hover(true, 10000);
    c.on_timer(10160);
    assert_eq!(
        c.handle_click(70, 18, 10200),
        termielle_app::app::ClickOutcome::Expanded
    );
    c.handle_event(event("s1", EventKind::TurnCompleted, 10300), 10300);
    assert_eq!(c.visible_state(), termielle_core::VisualState::Ready);
    // Settled card, sparkles mid-flight (600 ms window from 10300).
    for t in (10300..10600).step_by(50) {
        c.on_timer(t);
    }
    let a = c.current_frame().pixels_pbgra.clone();
    // Past the flight: gone, and the frames stop changing entirely.
    for t in (10600..11200).step_by(50) {
        c.on_timer(t);
    }
    let b = c.current_frame().pixels_pbgra.clone();
    assert_ne!(a, b, "sparkles must show then leave");
    for t in (11200..11350).step_by(50) {
        c.on_timer(t);
    }
    assert_eq!(
        b,
        c.current_frame().pixels_pbgra,
        "post-sparkle frames must be identical"
    );
}

#[test]
fn render_scale_authors_frames_at_device_pixels() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.set_dpi_scale(1.25);
    // The construction frame was built at 1.0: refresh re-authors it.
    assert!(c.refresh_scale(1000));
    // Hidden sensor: 140x2 logical becomes 175x3 device pixels.
    assert_eq!(c.current_frame().width, 175);
    assert_eq!(c.current_frame().height, 3);
    // Layout still reads logical: targets are untouched by the scale.
    assert_eq!(c.target_size(termielle_core::VisualState::Idle), (140, 2));
    // A second refresh is a no-op: dimensions already match.
    assert!(!c.refresh_scale(1000));
}

#[test]
fn render_scale_compact_pill_keeps_content_and_morphs_logical() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.set_dpi_scale(1.25);
    let _ = c.refresh_scale(9000);
    let _ = c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    let _ = c.on_timer(10050);
    let _ = c.on_timer(10200);
    // Settled compact pill: 72x36 logical authored at 90x45 device pixels.
    let frame = c.current_frame();
    assert_eq!(frame.width, 90);
    assert_eq!(frame.height, 45);
    let solid = frame
        .pixels_pbgra
        .chunks_exact(4)
        .filter(|px| px[3] > 150)
        .count();
    assert!(solid > 500, "compact pill must render content at scale");
}

#[test]
fn collapse_if_expanded_collapses_tall_card() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    // Initially hidden / idle
    assert!(!c.is_manually_expanded());
    assert!(!c.collapse_if_expanded(1000));

    // Expand via click
    assert!(c.toggle_expand(1000));
    assert!(c.is_manually_expanded());

    // Settle into expanded tall card
    let _ = c.on_timer(1050);
    let _ = c.on_timer(1200);
    assert_eq!(c.current_frame().width, 320);

    // Call collapse_if_expanded (simulating click outside / Esc)
    assert!(c.collapse_if_expanded(1300));
    assert!(!c.is_manually_expanded());

    // Settle back to hidden (140x2)
    let _ = c.on_timer(1350);
    let _ = c.on_timer(1500);
    assert_eq!(c.current_frame().height, 2);
}

#[test]
fn bar_layout_sizing_and_interaction() {
    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        collapsed_width: 140,
        expanded_width: 360,
        height: 36,
        ..Default::default()
    };
    island.bar.height = 36;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    assert!(c.is_island());
    c.set_bar_width(1920);

    // In compact bar layout, target size is full width (1920) by bar height (36)
    let (w, h) = c.target_size(termielle_core::VisualState::Idle);
    assert_eq!(w, 1920);
    assert_eq!(h, 36);

    // Frame rendered at 1920x36
    let frame = c.current_frame();
    assert_eq!(frame.width, 1920);
    assert_eq!(frame.height, 36);

    // Toggle expand: morphs downward
    assert!(c.toggle_expand(1000));
    assert!(c.is_manually_expanded());

    let (exp_w, exp_h) = c.target_size(termielle_core::VisualState::Idle);
    assert_eq!(exp_w, 1920);
    assert_eq!(
        exp_h, 206,
        "Today popup includes a transparent six-pixel breathing gap"
    );

    // Settle spring
    for t in 1..20 {
        let _ = c.on_timer(1000 + t * 50);
    }
    assert_eq!(c.current_frame().width, 1920);
    assert_eq!(c.current_frame().height, exp_h);
    let frame = c.current_frame();
    for y in 36..42 {
        let alpha = frame.pixels_pbgra[((y * frame.width + 960) * 4 + 3) as usize];
        assert_eq!(alpha, 0, "popup must not visually touch the bar strip");
    }

    // Collapse
    assert!(c.collapse_if_expanded(2500));
    for t in 1..20 {
        let _ = c.on_timer(2500 + t * 50);
    }
    assert_eq!(c.current_frame().width, 1920);
    assert_eq!(c.current_frame().height, 36);
}

#[test]
fn static_bar_still_has_a_periodic_metrics_deadline() {
    let config = IslandConfig {
        layout: IslandLayout::Bar,
        face_animated: false,
        ..Default::default()
    };
    let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, config);
    assert_eq!(c.next_deadline_ms(), Some(2_000));
    assert!(c.on_timer(10000).present_frame);
    assert_eq!(c.next_deadline_ms(), Some(12_000));
}

#[test]
fn bar_requires_an_explicit_click_to_open_and_collapse() {
    let island = IslandConfig {
        layout: IslandLayout::Bar,
        expand_on_hover: true,
        ..Default::default()
    };
    let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, island);
    c.set_bar_width(1920);
    c.set_hover_point(Some((960, 18)));
    assert!(!c.set_hover(true, 750));
    assert_eq!(c.current_frame().height, 36);
    c.on_timer(1000);
    assert_eq!(c.current_frame().height, 36);
    assert_eq!(
        c.handle_click(960, 18, 1010),
        termielle_app::app::ClickOutcome::Expanded
    );
    for t in (1010..1800).step_by(16) {
        c.on_timer(t);
    }
    assert!(c.current_frame().height > 36);
    assert_eq!(
        c.handle_click(960, 18, 1810),
        termielle_app::app::ClickOutcome::Collapsed
    );
    for t in (1810..2200).step_by(16) {
        c.on_timer(t);
    }
    assert_eq!(c.current_frame().height, 36);
}

#[test]
fn display_paced_motion_requests_next_refresh_without_fixed_fps_cap() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.enable_display_pacing();
    c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    assert!(c.next_deadline_ms().unwrap() <= 10001);
    c.on_timer(10200);
    assert_eq!(c.next_deadline_ms(), Some(10200));
}

#[test]
fn bar_material_is_pixel_stable_through_expansion_and_collapse() {
    for position in [
        termielle_core::BarPosition::Top,
        termielle_core::BarPosition::Bottom,
    ] {
        let mut island = IslandConfig {
            layout: IslandLayout::Bar,
            ..Default::default()
        };
        island.bar.position = position;
        island.bar.modules_left.clear();
        island.bar.modules_right.clear();
        let bar_h = island.bar.height;
        let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
        c.set_bar_width(1920);
        let resting = c.current_frame().clone();
        let verify = |frame: &termielle_app::animation::FrameBuffer| {
            let row_y = if position == termielle_core::BarPosition::Top {
                0
            } else {
                frame.height - bar_h
            };
            for y in 0..bar_h {
                for x in [20, 400, 600, 1300, 1800] {
                    let a = ((y * resting.width + x) * 4) as usize;
                    let b = (((y + row_y) * frame.width + x) * 4) as usize;
                    assert_eq!(
                        &resting.pixels_pbgra[a..a + 4],
                        &frame.pixels_pbgra[b..b + 4],
                        "bar changed at ({x}, {y}) with frame height {}",
                        frame.height
                    );
                }
            }
        };
        c.toggle_expand(10000);
        for t in (0..800).step_by(8) {
            c.on_timer(10000 + t);
            verify(c.current_frame());
        }
        c.collapse_if_expanded(11000);
        for t in (0..800).step_by(8) {
            c.on_timer(11000 + t);
            verify(c.current_frame());
        }
        assert_eq!(c.current_frame().height, bar_h);
    }
}

#[test]
fn bar_layout_click_dispatch() {
    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    island.bar.modules_left = vec!["workspaces".into(), "window".into()];
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.set_bar_width(1920);
    c.set_bar_metrics(
        termielle_app::bar::metrics::Snapshot {
            workspaces: termielle_app::bar::workspaces::WorkspaceSnapshot {
                total: 2,
                active: 1,
            },
            ..Default::default()
        },
        0,
    );

    // Clicking center island pill toggles expansion
    let pill_cx = 1920 / 2;
    let outcome = c.handle_click(pill_cx, 18, 1000);
    assert_eq!(outcome, termielle_app::app::ClickOutcome::Expanded);

    // Clicking again collapses
    let outcome = c.handle_click(pill_cx, 18, 1100);
    assert_eq!(outcome, termielle_app::app::ClickOutcome::Collapsed);

    // Click on workspace 1 (near left margin)
    // First workspace pill is at x=12, y=6..30, w=26
    let outcome = c.handle_click(20, 18, 1200);
    assert_eq!(
        outcome,
        termielle_app::app::ClickOutcome::WorkspaceSwitch(1)
    );
}

#[test]
fn right_metric_damage_preserves_left_pixels_and_workspace_hits() {
    use termielle_app::bar::metrics::Snapshot;
    use termielle_app::bar::workspaces::WorkspaceSnapshot;

    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    island.bar.modules_left = vec!["workspaces".into(), "window".into()];
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.set_bar_width(1920);
    c.set_bar_metrics(
        Snapshot {
            workspaces: WorkspaceSnapshot {
                total: 2,
                active: 1,
            },
            window_title: "editor — project".into(),
            time_str: "12:00".into(),
            ..Default::default()
        },
        1000,
    );
    let before = c.current_frame().pixels_pbgra.clone();

    assert!(c.set_bar_metrics(
        Snapshot {
            workspaces: WorkspaceSnapshot {
                total: 2,
                active: 1
            },
            window_title: "editor — project".into(),
            time_str: "12:01".into(),
            ..Default::default()
        },
        2000,
    ));
    let after = &c.current_frame().pixels_pbgra;

    let stride = 1920 * 4;
    let left_unchanged = (0..36).all(|y| {
        before[y * stride..y * stride + 1700 * 4] == after[y * stride..y * stride + 1700 * 4]
    });
    assert!(
        left_unchanged,
        "left zone pixels must survive right-only damage"
    );
    assert!(
        (0..36).any(|y| before[y * stride + 1740 * 4..(y + 1) * stride]
            != after[y * stride + 1740 * 4..(y + 1) * stride]),
        "right zone must repaint"
    );
    assert_eq!(
        c.handle_click(20, 18, 2100),
        termielle_app::app::ClickOutcome::WorkspaceSwitch(1),
        "cached workspace hit regions must survive partial rendering"
    );
}

#[test]
fn bottom_bar_center_hit_tracks_the_shifted_strip() {
    let island = IslandConfig {
        layout: IslandLayout::Bar,
        bar: termielle_core::BarConfig {
            position: termielle_core::BarPosition::Bottom,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, island);
    c.set_bar_width(1920);
    c.toggle_expand(1000);
    let popup_h = c.current_frame().height - 36;

    assert_eq!(
        c.handle_click(960, popup_h as i32 + 18, 1010),
        termielle_app::app::ClickOutcome::Collapsed
    );
}

#[test]
fn bar_margin_insets_glass_and_keeps_gap_click_through() {
    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    island.bar.edge_to_edge = false;
    island.bar.margin = 8;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.set_bar_width(1920);
    // Window stays full-width; only the glass insets.
    let frame = c.current_frame();
    assert_eq!((frame.width, frame.height), (1920, 36));
    let alpha_at = |x: u32, y: u32| frame.pixels_pbgra[((y * frame.width + x) * 4 + 3) as usize];
    // Margin gap stays transparent (click-through)...
    assert_eq!(alpha_at(2, 2), 0);
    assert_eq!(alpha_at(2, 18), 0);
    // ...while the inset bar body renders.
    assert!(alpha_at(960, 18) > 150);
    assert_eq!(
        c.handle_click(2, 18, 1000),
        termielle_app::app::ClickOutcome::None
    );
}

#[test]
fn bar_module_list_gates_volume_hit() {
    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    island.bar.modules_right = vec!["volume".to_string()];
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.set_bar_width(1920);
    assert!(c.volume_at(1896, 18));
    assert!(!c.volume_at(960, 18));
    assert_eq!(
        c.handle_click(1896, 18, 1000),
        termielle_app::app::ClickOutcome::VolumeToggle
    );

    let mut island2 = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island2.bar.height = 36;
    island2.bar.modules_right = vec!["clock".to_string()];
    c.set_island_config(island2, 1100);
    assert!(!c.volume_at(1896, 18));
    // The clock opens recent notifications rather than the sound control.
    assert_eq!(
        c.handle_click(1896, 18, 1200),
        termielle_app::app::ClickOutcome::PanelToggled
    );
    assert!(c.is_notification_center_open());
}

/// Replacement mode always has an app-launcher entry and navigation menu,
/// even when the optional apps module wasn't listed.
#[test]
fn replacement_mode_has_one_launcher_and_keeps_the_center_independent() {
    use termielle_app::app::ClickOutcome;
    use termielle_app::bar::shell::ShellAction;

    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    island.bar.replace_taskbar = true;
    island.bar.modules_left = vec!["workspaces".to_string()];
    island.bar.modules_right = vec!["clock".to_string()];
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.set_bar_width(1920);

    assert_eq!(
        c.handle_click(44, 18, 1000),
        ClickOutcome::Shell(ShellAction::Search)
    );
    assert_eq!(
        c.handle_click(98, 18, 1010),
        ClickOutcome::NavigationChanged
    );
    assert!(c.is_navigation_open());
    c.close_navigation(1050);

    // The center pill and the right zone are untouched by replacement mode.
    assert_eq!(c.handle_click(960, 18, 1100), ClickOutcome::Expanded);
    c.set_island_config(
        IslandConfig {
            layout: IslandLayout::Bar,
            bar: termielle_core::BarConfig {
                height: 36,
                replace_taskbar: false,
                modules_left: vec!["workspaces".to_string()],
                modules_right: vec!["clock".to_string()],
                ..Default::default()
            },
            ..Default::default()
        },
        2000,
    );
    assert_eq!(c.handle_click(960, 18, 2100), ClickOutcome::Collapsed);
    assert_ne!(
        c.handle_click(28, 18, 2200),
        ClickOutcome::Shell(ShellAction::Start),
        "no shell controls while the native taskbar is kept"
    );
}

/// Narrow layouts reserve overflow instead of silently dropping open apps.
#[test]
fn left_navigation_stays_clear_of_the_center_and_keeps_overflow_reachable() {
    use termielle_app::app::ClickOutcome;
    use termielle_app::tasks::{WindowInfo, WorkerUpdate};
    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.replace_taskbar = true;
    island.bar.modules_right = vec!["clock".into()];
    for width in [640, 1920] {
        let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, island.clone());
        c.set_bar_width(width);
        c.set_task_update_at(
            WorkerUpdate {
                windows: (1..=18)
                    .map(|i| WindowInfo {
                        hwnd: i,
                        process_id: i as u32,
                        title: format!("Window {i}"),
                        minimized: false,
                        application: Some(termielle_core::PinnedApp {
                            name: format!("App {i:02}"),
                            target: termielle_core::AppLaunchTarget::Executable(format!(
                                "C:\\Apps\\App{i:02}.exe"
                            )),
                        }),
                    })
                    .collect(),
                ..Default::default()
            },
            1000,
        );
        let rail = c
            .click_regions()
            .iter()
            .copied()
            .filter(|h| h.0 <= -1000 && h.0 > -2000)
            .collect::<Vec<_>>();
        assert!(rail.len() >= 2);
        assert!(
            rail.iter()
                .all(|h| h.1 + h.3 as i32 <= width as i32 / 2 - 98)
        );
        let (_, x, y, w, h) = *rail.last().unwrap();
        assert_eq!(
            c.handle_click(x + w as i32 / 2, y + h as i32 / 2, 1100),
            ClickOutcome::NavigationChanged
        );
        assert!(c.is_navigation_open());
        assert_eq!(
            c.handle_click(width as i32 / 2, 18, 1200),
            ClickOutcome::Expanded
        );
    }
}

/// The strip's Control Center entry toggles its own panel while the pill
/// retains its own click behavior.
#[test]
fn strip_control_center_toggles_the_panel() {
    use termielle_app::app::ClickOutcome;

    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.set_bar_width(1920);
    let collapsed_h = c.current_frame().height;

    // The glyph sits at the pill's right end, ahead of the pill's own hit.
    assert_eq!(
        c.handle_click(control_center_point(&c).0, control_center_point(&c).1, 1000),
        ClickOutcome::PanelToggled
    );
    for tick in (1000..1400).step_by(16) {
        c.on_timer(tick);
    }
    assert!(
        c.current_frame().height > 120,
        "the panel is a tall card: {}",
        c.current_frame().height
    );

    // The strip entry stays live while the card is open, and it is a real
    // toggle now: it used to live inside the pill, where opening the card made
    // it unreachable, and it could only ever mean "open".
    assert_eq!(
        c.handle_click(control_center_point(&c).0, control_center_point(&c).1, 1500),
        ClickOutcome::PanelToggled,
        "the strip entry must close the panel it opened, as a panel"
    );
    let frame = c.current_frame();
    let panel_alpha = frame.pixels_pbgra[(80 * frame.width as usize + 1800) * 4 + 3];
    assert!(
        panel_alpha > 0,
        "the panel retracts rather than disappearing instantly"
    );
    for tick in (1500..2100).step_by(16) {
        c.on_timer(tick);
    }
    assert_eq!(
        c.current_frame().height,
        collapsed_h,
        "the second press of the strip entry closes the panel"
    );

    // Open it again, then open the island without dismissing the panel.
    assert_eq!(
        c.handle_click(control_center_point(&c).0, control_center_point(&c).1, 2200),
        ClickOutcome::PanelToggled
    );
    for tick in (2200..2600).step_by(16) {
        c.on_timer(tick);
    }
    assert!(c.current_frame().height > 120, "the panel is open again");

    assert_eq!(c.handle_click(960, 18, 2700), ClickOutcome::Expanded);
    assert!(c.is_panel_open(), "the pill must not dismiss the panel");
    assert!(
        c.is_manually_expanded(),
        "the pill must still open its own card"
    );
}

/// Control Center and the pill are two surfaces, not one surface with two
/// bodies. They used to share everything: opening the panel set
/// `manually_expanded` and painted inside the island's card, so the pill's
/// header sat on the panel, the pill's hover could take it away, and
/// dismissing "the card" dismissed the panel.
#[test]
fn the_panel_is_a_surface_of_its_own() {
    use termielle_app::app::ClickOutcome;

    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.set_bar_width(1536);
    let entry = control_center_point(&c);
    // The pill's own centre, read from the frame: a hard-coded x is wrong for
    // any bar width but one, and a miss returns `None` rather than a hit.
    let (_, px, py, pw, ph) = c
        .click_regions()
        .iter()
        .find(|(id, ..)| *id == termielle_app::bar::HIT_BAR_TERMIELLE_MODULE)
        .copied()
        .expect("the collapsed pill installs its own hit");
    let pill = (px + pw as i32 / 2, py + ph as i32 / 2);

    c.handle_click(entry.0, entry.1, 1000);
    assert!(c.is_panel_open(), "the strip entry opens the panel");
    assert!(
        !c.is_manually_expanded(),
        "the panel must not be the island's manually expanded card"
    );
    for tick in (1000..1400).step_by(16) {
        c.on_timer(tick);
    }

    // The pill's hover is a gesture on the island. It must not reach across
    // and take the panel away.
    c.set_hover(true, 1500);
    c.on_timer(1500 + 2000);
    assert!(
        c.is_panel_open(),
        "hovering the pill must not close a separate panel"
    );

    assert_eq!(c.handle_click(pill.0, pill.1, 5000), ClickOutcome::Expanded);
    assert!(c.is_panel_open(), "the pill must not dismiss the panel");
    assert!(c.is_manually_expanded());
    assert!(c.point_over_open_popup(pill));
    assert_eq!(
        c.handle_click(pill.0, pill.1, 5_050),
        ClickOutcome::Collapsed
    );
    assert!(
        !c.is_manually_expanded(),
        "the pill only closes its own card"
    );

    // The panel is dismissed the way a panel is: its own key, which is the one
    // shared path, and it leaves the island exactly as it found it. The old
    // panel branch also cleared the island's hover deadline and set its
    // suppression flag, so a dismissal reached into the other surface's
    // state machine.
    c.set_hover(false, 5_100);
    for tick in (5_100..6_000).step_by(16) {
        c.on_timer(tick);
    }
    assert!(
        c.collapse_if_expanded(6_000),
        "Escape dismisses whichever surface is showing"
    );
    for tick in (6_000..6_600).step_by(16) {
        c.on_timer(tick);
    }
    assert!(!c.is_panel_open(), "Escape closes the panel");
    assert!(
        !c.is_manually_expanded(),
        "and leaves the island's card shut"
    );
}

/// The two surfaces have to be able to open and close without the other one
/// feeling it. Each test here fails if one surface's lifecycle reaches into
/// the other's state.
mod surfaces_are_independent {
    use super::*;
    use termielle_app::app::ClickOutcome;

    fn bar() -> IslandConfig {
        let mut island = IslandConfig {
            layout: IslandLayout::Bar,
            ..Default::default()
        };
        island.bar.height = 36;
        island
    }

    fn settle(c: &mut Controller, from: u64, to: u64) {
        for tick in (from..to).step_by(16) {
            c.on_timer(tick);
        }
    }

    /// The Control Center's close must not reach into the island. It used to
    /// clear the island's hover deadline and set its suppression flag, so
    /// dismissing the panel also re-armed or disarmed the island's hover.
    #[test]
    fn closing_the_panel_leaves_the_island_hover_alone() {
        let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, bar());
        c.set_bar_width(1536);
        let entry = control_center_point(&c);

        c.handle_click(entry.0, entry.1, 1_000);
        settle(&mut c, 1_000, 1_400);
        assert!(c.is_panel_open());

        // The pointer is on the panel's control, so the island sees "outside".
        // Nothing there may arm the island's dwell.
        c.set_hover(false, 1_500);

        assert!(c.collapse_if_expanded(2_000), "the panel dismisses");
        // Well past the dwell and the grace. If the panel's close left an
        // island dwell armed, it fires in here and the card opens behind the
        // dismissal that was supposed to leave the island at rest.
        settle(&mut c, 2_000, 4_000);
        assert!(!c.is_panel_open());
        assert!(
            !c.is_manually_expanded() && c.current_frame().height < 100,
            "dismissing the panel must not leave the island opening behind it"
        );
    }

    /// The island's hover machine must keep running while the panel is up.
    ///
    /// `set_hover` used to return early whenever the panel was open, so the
    /// pointer leaving the pill did not clear the island's suppression and a
    /// dwell armed just before the panel opened sat pending until it closed.
    /// The panel's close then also set that suppression itself, so between the
    /// two the island could not hover-open at all until the pointer left -
    /// which is the panel steering the island's own gesture.
    ///
    /// The observable difference: the pointer travels off the pill and back
    /// while the panel is up, and the island opens its card on the panel's
    /// close, by its own dwell. Freeze the machine and the leave never lands,
    /// the suppression stands, and the card never opens.
    #[test]
    fn the_island_keeps_hovering_across_the_panels_open_period() {
        let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, bar());
        c.set_bar_width(1536);
        c.set_task_update_at(
            termielle_app::tasks::WorkerUpdate {
                media: Some(termielle_app::tasks::MediaInfo {
                    title: "Test song".into(),
                    playing: true,
                    ..Default::default()
                }),
                ..Default::default()
            },
            0,
        );
        let entry = control_center_point(&c);

        c.handle_click(entry.0, entry.1, 1_000);
        settle(&mut c, 1_000, 1_400);
        assert!(c.is_panel_open());

        // A dwell is already running when the panel opened, and the panel
        // dismisses the island, so it starts clean.
        c.set_hover(false, 1_500);
        c.set_hover(true, 1_600);
        settle(&mut c, 1_600, 2_000);
        assert!(c.is_panel_open(), "the island's hover takes nothing away");

        // The pointer is on the pill and stays there. The island's own dwell
        // runs on the panel's open period like any other.
        c.collapse_if_expanded(2_100);
        settle(&mut c, 2_100, 4_000);
        assert!(!c.is_panel_open());
        assert!(
            c.is_manually_expanded() || c.current_frame().height > 100,
            "the island's hover was frozen while the panel was up, so it can \
             never open on its own again: the panel is steering the island's \
             gesture"
        );
    }

    /// Opening the panel dismisses the island through the island's own close,
    /// and the island is left at rest rather than half-torn-down.
    #[test]
    fn opening_and_closing_the_panel_preserves_the_island_card() {
        let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, bar());
        c.set_bar_width(1536);
        let (_, px, py, pw, ph) = c
            .click_regions()
            .iter()
            .find(|(id, ..)| *id == termielle_app::bar::HIT_BAR_TERMIELLE_MODULE)
            .copied()
            .expect("the pill installs its own hit");
        let pill = (px + pw as i32 / 2, py + ph as i32 / 2);

        c.handle_click(pill.0, pill.1, 1_000);
        assert!(c.is_manually_expanded(), "the pill opens the island's card");
        let entry = control_center_point(&c);
        assert_eq!(
            c.handle_click(entry.0, entry.1, 1_200),
            ClickOutcome::PanelToggled
        );
        assert!(c.is_panel_open());
        assert!(
            c.is_manually_expanded(),
            "opening Control Center must not dismiss the island"
        );
        settle(&mut c, 1_200, 1_800);
        assert!(
            c.click_regions()
                .iter()
                .any(|(_, x, y, _, _)| *x > pill.0 + 180 && *y > 36),
            "the Control Center must remain interactive beside the island"
        );
        let frame = c.current_frame();
        let alpha = |x: usize| frame.pixels_pbgra[(80 * frame.width as usize + x) * 4 + 3];
        assert!(alpha(pill.0 as usize) > 0, "the island card stays visible");
        assert!(
            alpha(entry.0 as usize) > 0,
            "the Control Center stays visible beside it"
        );
        assert!(
            !c.point_over_notch_surface_is(entry),
            "the Control Center is outside the island's hover surface"
        );

        c.collapse_if_expanded(2_000);
        settle(&mut c, 2_000, 2_600);
        assert!(!c.is_panel_open());
        assert!(
            c.is_manually_expanded(),
            "dismissing the panel must leave the island's card alone"
        );
    }

    #[test]
    fn bottom_bar_keeps_both_popovers_attached_to_the_strip() {
        let mut island = bar();
        island.bar.position = termielle_core::BarPosition::Bottom;
        let mut controller =
            Controller::new_with_island(5000, 60000, catalog(), false, None, island);
        controller.set_bar_width(1536);
        let pill = controller
            .click_regions()
            .iter()
            .find(|(id, ..)| *id == termielle_app::bar::HIT_BAR_TERMIELLE_MODULE)
            .map(|(_, x, y, w, h)| (x + *w as i32 / 2, y + *h as i32 / 2))
            .expect("pill hit region");
        controller.handle_click(pill.0, pill.1, 1_000);
        settle(&mut controller, 1_000, 1_600);
        let entry = control_center_point(&controller);
        controller.handle_click(entry.0, entry.1, 1_600);
        settle(&mut controller, 1_600, 2_200);

        let frame = controller.current_frame();
        let bar_y = frame.height as usize - 36;
        let alpha = |x: usize, y: usize| frame.pixels_pbgra[(y * frame.width as usize + x) * 4 + 3];
        assert!(alpha(pill.0 as usize, bar_y - 30) > 0);
        assert!(alpha(entry.0 as usize, bar_y - 30) > 0);
        assert!(controller.point_over_notch_surface_is((pill.0, (bar_y - 30) as i32)));
        assert!(!controller.point_over_notch_surface_is((entry.0, (bar_y - 30) as i32)));
    }

    /// The island's "is the pointer over me" test must not be widened by the
    /// panel being open. Hover asks about the surface it drives, so a pointer
    /// on the panel's control is outside the island even though it is inside
    /// the window.
    #[test]
    fn the_islands_surface_test_ignores_the_panel() {
        let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, bar());
        c.set_bar_width(1536);
        let entry = control_center_point(&c);
        let (ex, ey) = entry;

        assert!(
            !c.point_over_notch_surface_is((ex, ey)),
            "the panel's control is not the island's surface"
        );
        c.handle_click(entry.0, entry.1, 1_000);
        settle(&mut c, 1_000, 1_400);
        assert!(
            !c.point_over_notch_surface_is((ex, ey)),
            "and it stops being the island's surface when the panel opens"
        );
    }
}

/// A module drawn on transparent glass is only clickable on the pixels that
/// happened to get inked. The control center is two thin slider strokes, so the
/// gaps between them passed the click through to the bar underneath and the
/// target collapsed onto the ink: the control only answered on its strokes, not
/// on the slot the layout reserved for it.
///
/// The fix marks the declared regions in the alpha map - the same 1/255 trick
/// the top sensor strip already uses - so the module's own slot is the target.
/// It must not widen anything: the neighbours keep their own clicks.
mod declared_regions_are_clickable {
    use termielle_app::window::{HitTestResult, alpha_hit_test, mark_hit_targets_for_test};

    fn blank(w: u32, h: u32) -> (Vec<u8>, Vec<u8>) {
        (
            vec![0u8; w as usize * h as usize * 4],
            vec![0u8; w as usize * h as usize],
        )
    }

    fn hit(alpha: &[u8], w: u32, x: i32, y: i32) -> bool {
        alpha_hit_test(alpha, w, x, y) == HitTestResult::Caption
    }

    #[test]
    fn an_empty_gap_inside_a_region_still_receives_the_click() {
        let (mut dst, mut alpha) = blank(64, 16);
        // Only two strokes were inked, as a two-slider glyph would be.
        for y in 4..8 {
            for x in [20, 30] {
                dst[(y * 64 + x) * 4 + 3] = 255;
            }
        }
        for i in 0..alpha.len() {
            alpha[i] = dst[i * 4 + 3];
        }
        assert!(
            !hit(&alpha, 64, 25, 6),
            "before: the gap between the strokes is click-through"
        );

        mark_hit_targets_for_test(&mut dst, &mut alpha, 64, 16, &[(16, 2, 20, 12)]);

        assert!(
            hit(&alpha, 64, 25, 6),
            "the gap inside the region is a target"
        );
        assert!(hit(&alpha, 64, 20, 6), "and so is the ink");
        // The plate is 1/255: enough for DWM, invisible on screen.
        assert_eq!(dst[(6 * 64 + 25) * 4 + 3], 1);
        // A stroke keeps its real alpha, so nothing is dimmed.
        assert_eq!(dst[(6 * 64 + 20) * 4 + 3], 255);
    }

    #[test]
    fn neighbouring_modules_keep_their_own_clicks() {
        let (mut dst, mut alpha) = blank(64, 16);
        // The marking must not spill past the region it was given: the clock
        // and the tray sit next to the control center, and swallowing their
        // clicks would be a worse bug than the small target.
        mark_hit_targets_for_test(&mut dst, &mut alpha, 64, 16, &[(16, 2, 20, 12)]);
        assert!(hit(&alpha, 64, 20, 6), "inside the region");
        assert!(!hit(&alpha, 64, 15, 6), "one pixel to the left is not");
        assert!(!hit(&alpha, 64, 36, 6), "one pixel to the right is not");
        assert!(!hit(&alpha, 64, 20, 1), "above the region is not");
        assert!(!hit(&alpha, 64, 20, 14), "below the region is not");
    }

    #[test]
    fn a_region_clipped_by_the_window_edge_marks_only_what_fits() {
        // The right zone's last module can sit flush against the frame edge,
        // and a region that starts off-screen must not underflow the index.
        let (mut dst, mut alpha) = blank(64, 16);
        mark_hit_targets_for_test(&mut dst, &mut alpha, 64, 16, &[(60, 4, 20, 8)]);
        assert!(
            hit(&alpha, 64, 62, 6),
            "the part inside the window is marked"
        );
        assert_eq!(dst[0], 0, "and the far side of the window is untouched");
    }
}

/// The panel must be closable by its own icon. It used to be impossible:
/// opening it no longer set `manually_expanded`, so the surface test fell
/// back to "is the cursor on the pill" - the icon is not the pill - and the
/// very click that opened the panel then read as a click outside and closed
/// it again. Every press re-opened it, which looked like the toggle was
/// stuck open.
#[test]
fn the_panel_closes_on_the_click_that_opened_it() {
    use termielle_app::app::ClickOutcome;

    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.set_bar_width(1536);
    let entry = control_center_point(&c);

    c.handle_click(entry.0, entry.1, 1000);
    for tick in (1000..1400).step_by(16) {
        c.on_timer(tick);
    }
    assert!(c.is_panel_open());
    let (_, icon_x, icon_y, icon_w, icon_h) = c
        .click_regions()
        .iter()
        .find(|(id, ..)| *id == termielle_app::app::HIT_CARD_PANEL)
        .copied()
        .expect("the strip entry is live while the panel is open");
    let on_icon = (icon_x + icon_w as i32 / 2, icon_y + icon_h as i32 / 2);
    assert!(
        c.point_over_bar_surface_is(on_icon),
        "the icon that opened the panel is inside the surface it opened"
    );

    assert_eq!(
        c.handle_click(on_icon.0, on_icon.1, 2000),
        ClickOutcome::PanelToggled
    );
    for tick in (2000..2400).step_by(16) {
        c.on_timer(tick);
    }
    assert!(!c.is_panel_open(), "the second press must close the panel");

    // And a third press opens it again: a toggle, not a one-way door.
    assert_eq!(
        c.handle_click(entry.0, entry.1, 3000),
        ClickOutcome::PanelToggled
    );
    assert!(c.is_panel_open());
}

/// The pill lives in the strip and the panel hangs below it, so opening the
/// panel must leave the pill on screen. They shared one morph driver, so the
/// panel's height read as "the island has expanded" and the pill faded out
/// under it.
#[test]
fn the_pill_stays_on_screen_while_the_panel_is_open() {
    use termielle_app::app::ClickOutcome;

    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.set_bar_width(1536);

    // The resting pill paints pixels in the strip: the face and the label.
    let lit = |f: &termielle_app::animation::FrameBuffer, x: u32, y: u32| -> usize {
        let mut n = 0;
        for yy in y..y + 30 {
            for xx in x..x + 180 {
                let i = (yy as usize * f.width as usize + xx as usize) * 4;
                if f.pixels_pbgra[i + 3] > 8 {
                    n += 1;
                }
            }
        }
        n
    };
    let (_, pill_x, pill_y, pill_w, _) = c
        .click_regions()
        .iter()
        .find(|(id, ..)| *id == termielle_app::bar::HIT_BAR_TERMIELLE_MODULE)
        .copied()
        .expect("the collapsed pill installs its own hit");
    let resting = lit(c.current_frame(), pill_x as u32, pill_y as u32);

    let entry = control_center_point(&c);
    c.handle_click(entry.0, entry.1, 1000);
    for tick in (1000..1400).step_by(16) {
        c.on_timer(tick);
    }
    assert!(c.is_panel_open());
    let with_panel = lit(c.current_frame(), pill_x as u32, pill_y as u32);
    assert!(
        with_panel >= resting * 9 / 10,
        "the pill must stay painted while the panel is open: {resting} then {with_panel}"
    );
    let _ = (pill_w, ClickOutcome::PanelToggled);
}

/// macOS drops the Control Center out of its menu-bar icon, not out of the
/// middle of the bar. The card therefore anchors to the entry's own click
/// region, and only the island's own card stays centred.
#[test]
fn the_panel_hangs_from_the_control_center_icon() {
    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.set_bar_width(1536);
    let (_, icon_x, _, icon_w, _) = c
        .click_regions()
        .iter()
        .find(|(id, ..)| *id == termielle_app::app::HIT_CARD_PANEL)
        .copied()
        .expect("the strip must carry a Control Center entry");
    let icon_centre = icon_x + icon_w as i32 / 2;
    assert!(
        icon_centre > 1536 * 2 / 3,
        "the icon belongs on the right of the strip, not the centre"
    );

    c.handle_click(icon_centre, 18, 1000);
    for tick in (1000..1400).step_by(16) {
        c.on_timer(tick);
    }
    let (_, tile_x, _, tile_w, _) = c
        .click_regions()
        .iter()
        .find(|(id, ..)| *id == termielle_app::bar::shell::ShellAction::Network.hit_id())
        .copied()
        .expect("the open panel installs its quick controls");
    let card_left = tile_x - 16;
    let card_centre = card_left + tile_w as i32 + 20;
    assert!(
        (card_centre - icon_centre).abs() < tile_w as i32,
        "the panel should hang from the icon: card centre {card_centre}, icon {icon_centre}"
    );
    assert!(
        card_left > 1536 / 2,
        "the panel must sit right of centre, not under the pill: left {card_left}"
    );
}

/// Quick controls hand off to the matching Windows settings pages. Sound
/// controls own live volume while the native taskbar and tray remain intact.
#[test]
fn panel_quick_controls_and_sound_targets_work() {
    use termielle_app::app::ClickOutcome;
    use termielle_app::bar::metrics::Snapshot;
    use termielle_app::bar::shell::ShellAction;
    use termielle_app::bar::volume::VolumeSnapshot;

    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.set_bar_width(1920);
    c.set_bar_metrics(
        Snapshot {
            volume: VolumeSnapshot {
                level: 60,
                muted: false,
            },
            ..Default::default()
        },
        1000,
    );
    assert_eq!(
        c.handle_click(control_center_point(&c).0, control_center_point(&c).1, 1000),
        ClickOutcome::PanelToggled
    );
    for tick in (1000..1400).step_by(16) {
        c.on_timer(tick);
    }

    // The panel hangs from the Control Center icon, so its card is no longer
    // centred and the row coordinates cannot be recomputed from the strip
    // width. Every target is read back out of the frame's click regions
    // instead: a hand-computed position that misses reports `None` or, worse,
    // a different control's outcome, and the test would pass for the wrong
    // reason.
    let regions = c.click_regions().to_vec();
    let region = |id: isize| {
        regions
            .iter()
            .find(|(found, ..)| *found == id)
            .copied()
            .unwrap_or_else(|| panic!("frame must install region {id}"))
    };
    for action in [
        ShellAction::Network,
        ShellAction::Bluetooth,
        ShellAction::Focus,
        ShellAction::Display,
        ShellAction::Settings,
    ] {
        let (_, x, y, w, h) = region(action.hit_id());
        assert_eq!(
            c.handle_click(x + w as i32 / 2, y + h as i32 / 2, 1500),
            ClickOutcome::Shell(action)
        );
    }
    let (_, track_x, track_y, track_w, track_h) =
        region(termielle_app::app::HIT_PANEL_VOLUME_TRACK);
    assert_eq!(
        c.handle_click(
            track_x + track_w as i32 / 2,
            track_y + track_h as i32 / 2,
            1600
        ),
        ClickOutcome::VolumeSet(50),
        "the slider sets an absolute level"
    );
    assert_eq!(
        c.handle_click(track_x, track_y + track_h as i32 / 2, 1601),
        ClickOutcome::VolumeSet(0)
    );
    assert_eq!(
        c.handle_click(
            track_x + track_w as i32 - 1,
            track_y + track_h as i32 / 2,
            1602
        ),
        ClickOutcome::VolumeSet(100)
    );
    assert!(
        c.panel_volume_at(track_x + 10, track_y + track_h as i32 / 2),
        "the volume track answers the wheel"
    );
    let (_, mute_x, mute_y, mute_w, mute_h) = regions
        .iter()
        .rev()
        .find(|(id, ..)| *id == termielle_app::bar::HIT_BAR_VOLUME_TOGGLE)
        .copied()
        .expect("panel speaker must be clickable");
    assert_eq!(
        c.handle_click(mute_x + mute_w as i32 / 2, mute_y + mute_h as i32 / 2, 1650),
        ClickOutcome::VolumeToggle,
        "the speaker glyph is the mute button"
    );
    assert!(
        !c.click_regions()
            .iter()
            .any(|(id, ..)| *id == termielle_app::app::HIT_MEDIA_PLAY_PAUSE),
        "an empty media card must not offer fake playback"
    );
    c.set_task_update(termielle_app::tasks::WorkerUpdate {
        media: Some(termielle_app::tasks::MediaInfo {
            title: "Test track".into(),
            artist: "Test artist".into(),
            playing: true,
            ..Default::default()
        }),
        ..Default::default()
    });
    let (_, play_x, play_y, play_w, play_h) = c
        .click_regions()
        .iter()
        .find(|(id, ..)| *id == termielle_app::app::HIT_MEDIA_PLAY_PAUSE)
        .copied()
        .expect("a live media session gets a playback button");
    assert_eq!(
        c.handle_click(play_x + play_w as i32 / 2, play_y + play_h as i32 / 2, 1700),
        ClickOutcome::MediaToggle
    );
}

#[test]
fn clock_opens_recent_notifications_and_clear_all() {
    use termielle_app::app::{
        AlertKind, ClickOutcome, HIT_CARD_NOTIFICATIONS, HIT_NOTIFICATIONS_CLEAR,
    };

    let island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    let mut controller = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    controller.set_bar_width(1536);
    controller.trigger_alert(
        AlertKind::System,
        "A message",
        "First notification",
        [200, 120, 80, 255],
        6000,
        1000,
        "toast:one",
    );
    controller.trigger_alert(
        AlertKind::System,
        "Another message",
        "Second notification",
        [200, 120, 80, 255],
        6000,
        1010,
        "toast:two",
    );
    assert_eq!(controller.recent_notification_count(), 2);
    assert_eq!(controller.unread_notification_count(), 2);

    let (_, clock_x, clock_y, clock_w, clock_h) = controller
        .click_regions()
        .iter()
        .find(|(id, ..)| *id == HIT_CARD_NOTIFICATIONS)
        .copied()
        .expect("clock has its own hit");
    assert_eq!(
        controller.handle_click(
            clock_x + clock_w as i32 / 2,
            clock_y + clock_h as i32 / 2,
            1100
        ),
        ClickOutcome::PanelToggled
    );
    for tick in (1100..1500).step_by(16) {
        controller.on_timer(tick);
    }
    assert!(controller.is_notification_center_open());
    assert_eq!(controller.unread_notification_count(), 0);
    let (_, clear_x, clear_y, clear_w, clear_h) = controller
        .click_regions()
        .iter()
        .find(|(id, ..)| *id == HIT_NOTIFICATIONS_CLEAR)
        .copied()
        .expect("recent notifications have a clear action");
    assert_eq!(
        controller.handle_click(
            clear_x + clear_w as i32 / 2,
            clear_y + clear_h as i32 / 2,
            1510
        ),
        ClickOutcome::NotificationsCleared
    );
    assert!(controller.is_notification_center_open());
    assert_eq!(controller.recent_notification_count(), 0);
    assert!(
        !controller
            .click_regions()
            .iter()
            .any(|(id, ..)| *id == HIT_NOTIFICATIONS_CLEAR)
    );
    let (_, close_x, close_y, close_w, close_h) = controller
        .click_regions()
        .iter()
        .find(|(id, _, y, ..)| *id == HIT_CARD_NOTIFICATIONS && *y > 36)
        .copied()
        .expect("empty Notifications still has a close button");
    assert_eq!(
        controller.handle_click(
            close_x + close_w as i32 / 2,
            close_y + close_h as i32 / 2,
            1600
        ),
        ClickOutcome::PanelToggled
    );
    assert!(!controller.is_notification_center_open());
}

#[test]
fn notification_center_only_claims_its_popup_and_bar_entries_for_outside_clicks() {
    use termielle_app::app::{ClickOutcome, HIT_CARD_NOTIFICATIONS};

    for position in [
        termielle_core::BarPosition::Top,
        termielle_core::BarPosition::Bottom,
    ] {
        let mut island = IslandConfig {
            layout: IslandLayout::Bar,
            ..Default::default()
        };
        island.bar.position = position;
        let mut controller =
            Controller::new_with_island(5000, 60000, catalog(), false, None, island);
        controller.set_bar_width(1536);
        let (_, clock_x, clock_y, clock_width, clock_height) = controller
            .click_regions()
            .iter()
            .find(|(id, ..)| *id == HIT_CARD_NOTIFICATIONS)
            .copied()
            .expect("clock opens Notifications");
        let clock_point = (
            clock_x + clock_width as i32 / 2,
            clock_y + clock_height as i32 / 2,
        );
        assert_eq!(
            controller.handle_click(clock_point.0, clock_point.1, 1_000),
            ClickOutcome::PanelToggled
        );
        for tick in (1_000..1_500).step_by(16) {
            controller.on_timer(tick);
        }
        assert!(controller.is_notification_center_open());
        let panel_y = 60;
        assert!(controller.point_over_open_popup(clock_point));
        assert!(controller.point_over_open_popup((clock_point.0, panel_y)));
        assert!(!controller.point_over_open_popup((10, panel_y)));
        let (_, pill_x, pill_y, pill_width, pill_height) = controller
            .click_regions()
            .iter()
            .find(|(id, ..)| *id == termielle_app::bar::HIT_BAR_TERMIELLE_MODULE)
            .copied()
            .expect("pill remains visible beside Notifications");
        assert!(!controller.point_over_open_popup((
            pill_x + pill_width as i32 / 2,
            pill_y + pill_height as i32 / 2,
        )));
        assert!(controller.point_over_bar_surface_is((10, panel_y)));
        assert!(controller.collapse_if_expanded(1_600));
        assert!(!controller.is_notification_center_open());
    }
}

#[test]
fn control_center_and_notifications_switch_without_closing_the_notch() {
    use termielle_app::app::{ClickOutcome, HIT_CARD_NOTIFICATIONS};

    let island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    let mut controller = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    controller.set_bar_width(1536);
    let (_, pill_x, pill_y, pill_w, pill_h) = controller
        .click_regions()
        .iter()
        .find(|(id, ..)| *id == termielle_app::bar::HIT_BAR_TERMIELLE_MODULE)
        .copied()
        .expect("notch has its own hit");
    assert_eq!(
        controller.handle_click(pill_x + pill_w as i32 / 2, pill_y + pill_h as i32 / 2, 1000),
        ClickOutcome::Expanded
    );
    let (_, clock_x, clock_y, clock_w, clock_h) = controller
        .click_regions()
        .iter()
        .find(|(id, ..)| *id == HIT_CARD_NOTIFICATIONS)
        .copied()
        .expect("clock has its own hit");
    assert_eq!(
        controller.handle_click(
            clock_x + clock_w as i32 / 2,
            clock_y + clock_h as i32 / 2,
            1100
        ),
        ClickOutcome::PanelToggled
    );
    assert!(controller.is_manually_expanded());
    assert!(controller.is_notification_center_open());
    let center = control_center_point(&controller);
    assert_eq!(
        controller.handle_click(center.0, center.1, 1200),
        ClickOutcome::PanelToggled
    );
    assert!(controller.is_manually_expanded());
    assert!(controller.is_panel_open());
    assert!(!controller.is_notification_center_open());
}

/// The glyph's x follows the pill's right end, so the tests never hard-code
/// the pill's own width.
/// Centre of the Control Center's click region, read from the frame the
/// controller just painted. The entry is a strip module in the right zone, so
/// asking the frame where it is beats recomputing the right-zone layout here -
/// a wrong constant would silently click the volume or clock module instead,
/// and `handle_click` would happily report that as a hit.
fn control_center_point(c: &Controller) -> (i32, i32) {
    let (_, x, y, w, h) = c
        .click_regions()
        .iter()
        .find(|(id, ..)| *id == termielle_app::app::HIT_CARD_PANEL)
        .copied()
        .expect("the bar must install a Control Center click region");
    (x + w as i32 / 2, y + h as i32 / 2)
}

#[test]
fn bar_controls_remain_live_during_island_morph() {
    use termielle_app::app::ClickOutcome;
    for position in [
        termielle_core::BarPosition::Top,
        termielle_core::BarPosition::Bottom,
    ] {
        let mut island = IslandConfig {
            layout: IslandLayout::Bar,
            ..Default::default()
        };
        island.bar.position = position;
        island.bar.modules_right = vec!["volume".into()];
        let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
        c.set_bar_width(1920);
        c.toggle_expand(1000);
        for now in [1016, 1080, 1200, 1600] {
            c.on_timer(now);
            let y = if position == termielle_core::BarPosition::Bottom {
                c.current_frame().height as i32 - 18
            } else {
                18
            };
            assert!(c.volume_at(1896, y));
            assert_eq!(c.handle_click(1896, y, now), ClickOutcome::VolumeToggle);
        }
    }
}

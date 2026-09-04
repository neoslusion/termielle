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

#[test]
fn island_starts_minimal_and_promotes_on_prompt() {
    let mut island = IslandConfig::default();
    island.layout = IslandLayout::Island;
    island.minimal_width = 72;
    island.collapsed_width = 140;
    island.expanded_width = 320;
    island.height = 36;
    island.corner_radius = 18;
    island.animation_ms = 100; // fast for test
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    assert!(c.is_island());
    // iOS minimal presentation: idle with nothing live rests small.
    assert_eq!(c.current_frame().width, 72);
    assert_eq!(c.current_frame().height, 36);
    // Prompt -> Thinking promotes to the compact pill.
    let actions = c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    assert_eq!(
        actions.visible_state,
        Some(termielle_core::VisualState::Thinking)
    );
    assert!(actions.present_frame);
    // Spring morph in progress toward compact.
    assert!(c.next_deadline_ms().unwrap() <= 10016);
    let actions = c.on_timer(10050);
    assert!(actions.present_frame);
    assert!(c.current_frame().width > 72);
    // Settled at compact. The spring already snapped during the 10050 tick,
    // so nothing is scheduled besides the reducer's thinking hold.
    let _ = c.on_timer(10160);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.next_deadline_ms(), Some(11000)); // thinking hold
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
    let classic = IslandConfig::default();
    assert!(!classic.is_enabled());
}

#[test]
fn island_reduced_motion_is_immediate() {
    let mut island = IslandConfig::default();
    island.layout = IslandLayout::Island;
    island.minimal_width = 72;
    island.collapsed_width = 100;
    island.expanded_width = 200;
    island.animation_ms = 500;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), true, None, island);
    // Idle with nothing live: minimal.
    assert_eq!(c.current_frame().width, 72);
    c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    // Immediate, no spring: compact while the agent is live.
    assert_eq!(c.current_frame().width, 100);
    assert_eq!(c.next_deadline_ms(), Some(11000));
}

#[test]
fn island_ready_expands_and_idle_collapses() {
    let mut island = IslandConfig::default();
    island.layout = IslandLayout::Island;
    island.minimal_width = 72;
    island.collapsed_width = 140;
    island.expanded_width = 300;
    island.animation_ms = 50;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.handle_event(event("s1", EventKind::PromptSubmitted, 10000), 10000);
    c.on_timer(10100); // settle at compact
    assert_eq!(c.current_frame().width, 140);
    // Turn completed -> Ready (session still live, still compact)
    c.handle_event(event("s1", EventKind::TurnCompleted, 10200), 10200);
    assert_eq!(c.current_frame().width, 140);
    // After ready hold, goes Idle -> drop to minimal (iOS auto-hide).
    c.on_timer(15200); // 10200+5000 ready hold
    assert_eq!(c.visible_state(), termielle_core::VisualState::Idle);
    // Spring collapse toward minimal.
    assert!(c.next_deadline_ms().is_some());
    c.on_timer(15260);
    c.on_timer(15400);
    assert_eq!(c.current_frame().width, 72);
}

#[test]
fn classic_controller_still_uses_fallback_size() {
    let c = Controller::new(5000, 60000, catalog(), false, None);
    assert!(!c.is_island());
    assert_eq!(c.current_frame().width, 360);
    assert_eq!(c.current_frame().height, 360);
}

fn island_140_320() -> IslandConfig {
    IslandConfig {
        layout: IslandLayout::Island,
        collapsed_width: 140,
        expanded_width: 320,
        height: 36,
        corner_radius: 18,
        animation_ms: 100,
        ..Default::default()
    }
}

#[test]
fn hover_expands_idle_and_leave_collapses() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    // Idle with nothing live rests minimal.
    assert_eq!(c.current_frame().width, 72);
    assert!(c.set_hover(true, 10000));
    // Morph in flight toward the expanded dashboard.
    let actions = c.on_timer(10050);
    assert!(actions.present_frame);
    assert!(c.current_frame().width > 72);
    c.on_timer(10120);
    assert_eq!(c.current_frame().width, 320);
    // Leaving collapses back to minimal (no live sessions).
    assert!(c.set_hover(false, 10200));
    c.on_timer(10320);
    assert_eq!(c.current_frame().width, 72);
}

#[test]
fn hover_is_ignored_when_expand_on_hover_is_off() {
    let mut cfg = island_140_320();
    cfg.expand_on_hover = false;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    assert!(!c.set_hover(true, 10000));
    assert_eq!(c.current_frame().width, 72);
}

#[test]
fn hover_does_not_collapse_a_manually_toggled_pill() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    assert!(c.toggle_expand(10000));
    c.on_timer(10120);
    assert_eq!(c.current_frame().width, 320);
    // Hover leave while manually expanded: no visible change.
    assert!(!c.set_hover(false, 10200));
    assert_eq!(c.current_frame().width, 320);
    // Manual toggle collapses even though the cursor is "inside".
    c.set_hover(true, 10250);
    assert!(c.toggle_expand(10300));
    c.on_timer(10420);
    // Idle with nothing live: back to minimal.
    assert_eq!(c.current_frame().width, 72);
}

#[test]
fn set_task_update_repaints_only_when_visible() {
    use termielle_app::tasks::{TaskIcon, WorkerUpdate};
    let update = || WorkerUpdate {
        icons: vec![TaskIcon {
            hwnd: 42,
            title: "demo".into(),
            width: 8,
            height: 8,
            pixels_pbgra: vec![255u8; 8 * 8 * 4],
        }],
        media: None,
        backdrop: None,
    };
    // Collapsed idle with no visible dynamic widgets: store but no repaint.
    let mut cfg = island_140_320();
    cfg.widgets.retain(|w| w == "face");
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    assert!(!c.set_task_update(update()));
    // Expand, then the worker round repaints.
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.set_hover(true, 10000);
    c.on_timer(10120);
    assert_eq!(c.current_frame().width, 320);
    assert!(c.set_task_update(update()));
}

#[test]
fn island_without_face_widget_still_renders() {
    let mut cfg = island_140_320();
    cfg.widgets.retain(|w| w != "face");
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    // Idle rests minimal (glass dot) even without a face.
    assert_eq!(c.current_frame().width, 72);
    assert_eq!(c.current_frame().height, 36);
    assert!(c.set_hover(true, 10000));
    c.on_timer(10120);
    assert_eq!(c.current_frame().width, 320);
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
fn animated_face_advances_on_its_own_deadline() {
    let dir = tempfile::tempdir().unwrap();
    write_gif(dir.path(), "standby.gif", &[(4, 0), (4, 1)]);
    let catalog = AssetCatalog::new(vec![dir.path().to_path_buf()]);
    let mut cfg = island_140_320();
    cfg.face_animated = true;
    let mut c = Controller::new_with_island(5000, 60000, catalog, false, None, cfg);
    // Minimal idle still composites the face (frame 0, red).
    assert_eq!(c.current_frame().width, 72);
    let first = c.current_frame().pixels_pbgra.clone();
    // Past the 40ms face delay: the face (red -> blue) must have advanced
    // and repainted, independent of any agent activity.
    let actions = c.on_timer(10050);
    assert!(actions.present_frame);
    assert_ne!(
        c.current_frame().pixels_pbgra,
        first,
        "animated face must advance"
    );
}

#[test]
fn static_face_never_advances() {
    let dir = tempfile::tempdir().unwrap();
    write_gif(dir.path(), "standby.gif", &[(4, 0), (4, 1)]);
    let catalog = AssetCatalog::new(vec![dir.path().to_path_buf()]);
    let mut cfg = island_140_320();
    cfg.collapsed_width = 220;
    cfg.face_animated = false;
    let mut c = Controller::new_with_island(5000, 60000, catalog, false, None, cfg);
    let first = c.current_frame().pixels_pbgra.clone();
    c.on_timer(10050);
    c.on_timer(20050);
    assert_eq!(
        c.current_frame().pixels_pbgra,
        first,
        "static face must not advance"
    );
}

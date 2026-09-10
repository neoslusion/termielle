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
    assert_eq!(c.next_deadline_ms(), Some(10250)); // motion tick, then thinking hold at 11000
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
fn hover_pops_out_to_pill_and_leave_collapses() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    // Idle with nothing live rests hidden.
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);

    // Hover pops it out in pill format!
    assert!(c.set_hover(true, 10000));
    let actions = c.on_timer(10050);
    assert!(actions.present_frame);
    assert!(c.current_frame().height > 2);
    c.on_timer(10160);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 36);

    // Leaving collapses back to hidden (no live sessions).
    assert!(c.set_hover(false, 10200));
    c.on_timer(10360);
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
    // Hover pops out into pill format
    c.set_hover(true, 10000);
    c.on_timer(10160);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 36);

    // Click to extend vertically and horizontally into tall card!
    assert_eq!(
        c.handle_click(70, 18, 10200),
        termielle_app::app::ClickOutcome::Expanded
    );
    c.on_timer(10360);
    assert_eq!(c.current_frame().width, 320);
    assert_eq!(c.current_frame().height, 154);

    // Hover leave while manually extended: stays pinned open!
    assert!(!c.set_hover(false, 10400));
    assert_eq!(c.current_frame().width, 320);
    assert_eq!(c.current_frame().height, 154);

    // Second click collapses back to pill format!
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

    // With music widget enabled and hovered (pill format visible), media arrival repaints.
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.set_hover(true, 10000);
    c.on_timer(10160);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 36);
    assert!(c.set_task_update(update()));
}

#[test]
fn island_without_face_widget_still_renders() {
    let mut cfg = island_140_320();
    cfg.widgets.retain(|w| w != "face");
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    // Idle rests hidden even without a face.
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);
    assert!(c.set_hover(true, 10000));
    c.on_timer(10160);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 36);
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
    // Pop out into pill format via hover
    c.set_hover(true, 10000);
    c.on_timer(10160);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 36);
    let first = c.current_frame().pixels_pbgra.clone();

    // Past the 40ms face delay: the face (red -> blue) must advance and repaint.
    let actions = c.on_timer(10220);
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
fn island_morphs_height_and_width_on_expand() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    // Starts hidden
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);

    // Hover pops out to pill format
    assert!(c.set_hover(true, 10000));
    c.on_timer(10160);
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

    // Leave collapses both width and height to hidden
    assert!(c.set_hover(false, 10600));
    c.on_timer(10760);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);
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
    assert_eq!(c.current_frame().height, 124); // Alert banner layout is 124px tall

    // After 3500ms, alert expires and if state goes idle, collapses back
    c.on_timer(13600);
    c.handle_event(event("s1", EventKind::TurnCompleted, 13700), 13700);
    c.on_timer(18800); // after ready hold
    c.on_timer(19100);
    assert_eq!(c.current_frame().height, 2);
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
}

#[test]
fn split_island_merges_back_when_media_stops() {
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
    });
    let _ = c.on_timer(10400);

    // Media stops: the blobs flow back together into one pill.
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
    });
    let _ = c.on_timer(10600);
    let _ = c.on_timer(10800);
    let frame = c.current_frame();
    assert_eq!(frame.width, 72);
    assert_eq!(frame.height, 36);
    // No transparent gap inside the merged pill.
    let alpha_at = |x: u32, y: u32| frame.pixels_pbgra[(((y * frame.width + x) * 4) + 3) as usize];
    for x in [30u32, 36, 42] {
        assert!(alpha_at(x, 18) > 150, "merged pill must be solid at {x}");
    }
}

#[test]
fn press_swell_grows_the_pill_and_release_restores_it() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    c.set_hover(true, 10000);
    let _ = c.on_timer(10160);
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
    let mut notch = island_140_320();
    notch.layout = IslandLayout::Notch;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, notch);
    c.set_hover(true, 10000);
    let _ = c.on_timer(10160);
    let frame = c.current_frame();
    let idx = ((18 * frame.width + 70) * 4) as usize;
    assert_eq!(frame.pixels_pbgra[idx..idx + 4], [0, 0, 0, 255]);

    // Floating island: glass, not black.
    let mut island = island_140_320();
    island.layout = IslandLayout::Island;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.set_hover(true, 10000);
    let _ = c.on_timer(10160);
    let frame = c.current_frame();
    let idx = ((18 * frame.width + 70) * 4) as usize;
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
fn ending_the_turn_retires_the_open_card() {
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
    // ... and retires once the ready hold expires into Idle. Leaving the
    // hover lets it fall all the way back to the hidden sensor.
    c.on_timer(15600);
    assert_eq!(c.visible_state(), termielle_core::VisualState::Idle);
    c.set_hover(false, 15600);
    c.on_timer(15760);
    c.on_timer(16000);
    assert_eq!(c.current_frame().width, 140);
    assert_eq!(c.current_frame().height, 2);
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
    assert_eq!(c.current_frame().height, 124);
}

#[test]
fn alert_queue_plays_second_banner_after_first_expires() {
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island_140_320());
    // Two sessions need input 400 ms apart: the first banner shows.
    c.handle_event(event("s1", EventKind::NeedsInput, 10000), 10000);
    c.handle_event(event("s2", EventKind::NeedsInput, 10400), 10400);
    c.on_timer(10600);
    assert_eq!(c.current_frame().width, 320);
    assert_eq!(c.current_frame().height, 124);
    let first = c.current_frame().pixels_pbgra.clone();

    // Past the first banner's 3.5 s life the second takes over instead of
    // collapsing: same card, different session badge.
    c.on_timer(13600);
    c.on_timer(13700);
    assert_eq!(c.current_frame().width, 320);
    assert_eq!(c.current_frame().height, 124);
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
fn collapse_settles_on_its_own_faster_timing() {
    // Expand slowly (500 ms), collapse fast (50 ms): the collapse must be
    // settled 200 ms later, which the shared slow spring could never do.
    // No agent events: NeedsInput would raise its own alert card and hide
    // the manual-expansion target this measures.
    let mut cfg = island_140_320();
    cfg.animation_ms = 500;
    cfg.collapse_ms = 50;
    cfg.alert_ms = 50;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, cfg);
    c.set_hover(true, 10000);
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
    assert_eq!(c.current_frame().height, 2);
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
    // Past morph end and the 300 ms shake window: locked to zero.
    for t in (10100..10600).step_by(50) {
        c.on_timer(t);
    }
    let a = c.current_frame().pixels_pbgra.clone();
    for t in (10600..10750).step_by(50) {
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
    assert!(exp_h > 36);

    // Settle spring
    for t in 1..20 {
        let _ = c.on_timer(1000 + t * 50);
    }
    assert_eq!(c.current_frame().width, 1920);
    assert_eq!(c.current_frame().height, exp_h);

    // Collapse
    assert!(c.collapse_if_expanded(2500));
    for t in 1..20 {
        let _ = c.on_timer(2500 + t * 50);
    }
    assert_eq!(c.current_frame().width, 1920);
    assert_eq!(c.current_frame().height, 36);
}

#[test]
fn bar_layout_click_dispatch() {
    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.height = 36;
    let mut c = Controller::new_with_island(5000, 60000, catalog(), false, None, island);
    c.set_bar_width(1920);

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
    // A click in the gap hits nothing and takes the bar miss path.
    assert_eq!(
        c.handle_click(2, 18, 1000),
        termielle_app::app::ClickOutcome::Expanded
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
    // Volume listed alone: pill at 1822..1900 toggles mute. Battery is gated
    // by the list too, so this holds with or without hardware batteries.
    assert_eq!(
        c.handle_click(1861, 18, 1000),
        termielle_app::app::ClickOutcome::VolumeToggle
    );
    // Volume unlisted: the same point is bare glass (clock carries no hit)
    // and takes the bar miss path instead.
    let mut island2 = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island2.bar.height = 36;
    island2.bar.modules_right = vec!["clock".to_string()];
    c.set_island_config(island2, 1100);
    assert_eq!(
        c.handle_click(1861, 18, 1200),
        termielle_app::app::ClickOutcome::Expanded
    );
}

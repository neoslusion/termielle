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
    island.expanded_width = 300;
    island.animation_ms = 500;
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

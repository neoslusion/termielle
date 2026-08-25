//! Tests for the [`Controller`] that ties the reducer, the animation pipeline,
//! and the Win32 timer into one state machine.
//!
//! Every timestamp here is a fake Unix epoch millisecond clock. No Win32 calls
//! happen: the window is not created and the pipe is never touched.

use std::borrow::Cow;
use std::path::Path;

use termielle_app::app::{Controller, FALLBACK_FRAME_SIZE};
use termielle_core::{AssetCatalog, EventKind, EventMessage, Source, VisualState};

const S1: &str = "session-a";
const S2: &str = "session-b";

/// Parses a test source word, panicking when it is invalid.
fn source(word: &str) -> Source {
    Source::parse(word).unwrap()
}

/// Busy-stall used by every test; must stay far above the thinking hold so
/// the per-state deadlines below stay observable.
const BUSY_STALL_MS: u64 = 60_000;

/// File name the catalog expects for the Thinking animation.
const THINKING_GIF: &str = "ai_thingking.gif";

/// A catalog that resolves nothing: every state renders procedurally.
fn catalog() -> AssetCatalog {
    AssetCatalog::new(Vec::new())
}

/// A catalog holding a two-frame GIF for Thinking only (red, then blue,
/// 40 hundredths of a second per frame) so state deadlines stay observable.
///
/// Returns the temp dir too: dropping it deletes the GIFs.
fn gif_catalog() -> (tempfile::TempDir, AssetCatalog) {
    let dir = tempfile::tempdir().unwrap();
    write_gif(dir.path(), THINKING_GIF, &[(40, 0), (40, 1)]);
    let catalog = AssetCatalog::new(vec![dir.path().to_path_buf()]);
    (dir, catalog)
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

fn prompt_event(session: &str, at: u64) -> EventMessage {
    event(session, EventKind::PromptSubmitted, at)
}

fn completed_event(session: &str, at: u64) -> EventMessage {
    event(session, EventKind::TurnCompleted, at)
}

fn needs_input_event(session: &str, at: u64) -> EventMessage {
    event(session, EventKind::NeedsInput, at)
}

fn session_started_event(session: &str, at: u64) -> EventMessage {
    event(session, EventKind::SessionStarted, at)
}

fn session_ended_event(session: &str, at: u64) -> EventMessage {
    event(session, EventKind::SessionEnded, at)
}

/// Writes a 2x2 infinite-loop GIF where every frame covers the whole canvas.
fn write_gif(dir: &Path, name: &str, frames: &[(u16, u8)]) -> std::path::PathBuf {
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
    path
}

#[test]
fn swaps_thinking_to_working_without_a_fixed_poll_loop() {
    let mut controller = Controller::new(5_000, BUSY_STALL_MS, catalog(), false, None);
    let actions = controller.handle_event(prompt_event(S1, 10_000), 10_000);
    assert_eq!(actions.visible_state, Some(VisualState::Thinking));
    assert!(actions.present_frame);
    assert_eq!(controller.next_deadline_ms(), Some(11_000));

    let actions = controller.on_timer(11_000);
    assert_eq!(actions.visible_state, Some(VisualState::Working));
    assert!(actions.present_frame);
    assert_eq!(actions.next_deadline_ms, controller.next_deadline_ms());
    assert_eq!(controller.next_deadline_ms(), Some(10_000 + BUSY_STALL_MS));
}

#[test]
fn duplicate_completion_does_not_restart_the_ready_timer() {
    let mut controller = Controller::new(5_000, BUSY_STALL_MS, catalog(), false, None);
    controller.handle_event(completed_event(S1, 20_000), 20_000);

    let actions = controller.handle_event(completed_event(S1, 20_000), 21_000);
    assert_eq!(actions.visible_state, None);
    assert!(!actions.present_frame);
    assert_eq!(controller.next_deadline_ms(), Some(25_000));
    assert_eq!(actions.next_deadline_ms, Some(25_000));
}

#[test]
fn missing_assets_render_the_procedural_fallback() {
    let mut controller = Controller::new(5_000, BUSY_STALL_MS, catalog(), false, None);
    controller.handle_event(prompt_event(S1, 10_000), 10_000);

    let frame = controller.current_frame();
    assert_eq!(frame.width, FALLBACK_FRAME_SIZE);
    assert_eq!(frame.height, FALLBACK_FRAME_SIZE);
    assert_eq!(frame.delay_ms, 0);
    assert_eq!(frame.alpha_at(frame.width / 2, frame.height / 2), 255);
    assert_eq!(controller.next_deadline_ms(), Some(11_000));
}

#[test]
fn a_corrupt_asset_falls_back_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(THINKING_GIF), b"this is not a gif image").unwrap();
    let catalog = AssetCatalog::new(vec![dir.path().to_path_buf()]);

    let mut controller = Controller::new(5_000, BUSY_STALL_MS, catalog, false, None);
    let actions = controller.handle_event(prompt_event(S1, 10_000), 10_000);
    assert_eq!(actions.visible_state, Some(VisualState::Thinking));
    assert!(actions.error_code.is_some());
    let frame = controller.current_frame();
    assert_eq!(frame.delay_ms, 0);
    assert_eq!(frame.alpha_at(frame.width / 2, frame.height / 2), 255);
    assert_eq!(controller.next_deadline_ms(), Some(11_000));
}

#[test]
fn animation_deadline_drives_the_timer_before_the_state_deadline() {
    let (_dir, catalog) = gif_catalog();
    let mut controller = Controller::new(5_000, BUSY_STALL_MS, catalog, false, None);
    controller.handle_event(prompt_event(S1, 10_000), 10_000);

    // The first animation frame is due before the thinking -> working flip.
    assert_eq!(controller.next_deadline_ms(), Some(10_400));

    let actions = controller.on_timer(10_400);
    assert_eq!(actions.visible_state, None);
    assert!(actions.present_frame);
    // Frame two (blue) has been composited. BGRA: blue = [255, 0, 0, 255].
    assert!(
        controller
            .current_frame()
            .contains_pixel_bgra([255, 0, 0, 255])
    );
    assert_eq!(actions.next_deadline_ms, controller.next_deadline_ms());
    assert_eq!(controller.next_deadline_ms(), Some(10_800));

    let actions = controller.on_timer(10_800);
    assert!(actions.present_frame);
    // The state deadline is now nearer than the next animation frame.
    assert_eq!(controller.next_deadline_ms(), Some(11_000));

    let actions = controller.on_timer(11_000);
    assert_eq!(actions.visible_state, Some(VisualState::Working));
    assert!(actions.present_frame);
    assert_eq!(controller.next_deadline_ms(), Some(10_000 + BUSY_STALL_MS));
}

#[test]
fn a_fixed_frame_interval_overrides_the_gif_delays() {
    let (_dir, catalog) = gif_catalog();
    let mut controller = Controller::new(5_000, BUSY_STALL_MS, catalog, false, Some(100));
    controller.handle_event(prompt_event(S1, 10_000), 10_000);

    // The GIF's first frame is due at +400 ms, but the fixed interval wins.
    assert_eq!(controller.next_deadline_ms(), Some(10_100));
    let actions = controller.on_timer(10_100);
    assert!(actions.present_frame);
    // The next frame is due another 100 ms later, not at the GIF's +800.
    assert_eq!(controller.next_deadline_ms(), Some(10_200));
}

#[test]
fn reduced_motion_shows_the_first_frame_without_a_frame_deadline() {
    let (_dir, catalog) = gif_catalog();
    let mut controller = Controller::new(5_000, BUSY_STALL_MS, catalog, true, None);

    let actions = controller.handle_event(prompt_event(S1, 10_000), 10_000);
    assert_eq!(actions.visible_state, Some(VisualState::Thinking));
    assert!(actions.present_frame);
    // The first frame is red (BGRA: [0, 0, 255, 255]); blue must never show.
    assert!(
        controller
            .current_frame()
            .contains_pixel_bgra([0, 0, 255, 255])
    );
    // Only the reducer deadline is scheduled: there is no per-frame timer.
    assert_eq!(controller.next_deadline_ms(), Some(11_000));

    // A timer firing between state deadlines does not repaint.
    let actions = controller.on_timer(10_999);
    assert_eq!(actions.visible_state, None);
    assert!(!actions.present_frame);

    let actions = controller.on_timer(11_000);
    assert_eq!(actions.visible_state, Some(VisualState::Working));
}

#[test]
fn needs_input_outranks_a_thinking_session() {
    let mut controller = Controller::new(5_000, BUSY_STALL_MS, catalog(), false, None);
    controller.handle_event(prompt_event(S1, 10_000), 10_000);

    let actions = controller.handle_event(needs_input_event(S2, 10_500), 10_500);
    assert_eq!(actions.visible_state, Some(VisualState::NeedsInput));
    assert!(actions.present_frame);

    // The thinking session's deadline fires underneath: NeedsInput outranks
    // Working, so the visible state cannot move, but the pending deadline is
    // now the thinking session's busy-stall moment instead of its 11_000 hold.
    let actions = controller.on_timer(11_000);
    assert_eq!(actions.visible_state, None);
    assert!(!actions.present_frame);
    assert_eq!(controller.next_deadline_ms(), Some(10_000 + BUSY_STALL_MS));
}

#[test]
fn session_ended_returns_to_the_previous_state() {
    let mut controller = Controller::new(5_000, BUSY_STALL_MS, catalog(), false, None);
    controller.handle_event(prompt_event(S1, 10_000), 10_000);
    controller.handle_event(needs_input_event(S2, 10_500), 10_500);

    let actions = controller.handle_event(session_ended_event(S2, 10_600), 10_600);
    assert_eq!(actions.visible_state, Some(VisualState::Thinking));
    assert_eq!(controller.next_deadline_ms(), Some(11_000));
}

#[test]
fn a_late_event_advances_due_transitions_first() {
    let mut controller = Controller::new(5_000, BUSY_STALL_MS, catalog(), false, None);
    controller.handle_event(prompt_event(S1, 10_000), 10_000);

    // The same prompt re-delivered after its thinking hold elapsed: time passed,
    // so the session must already have moved to Working.
    let actions = controller.handle_event(prompt_event(S1, 10_000), 12_000);
    assert_eq!(actions.visible_state, Some(VisualState::Working));
    assert_eq!(controller.next_deadline_ms(), Some(10_000 + BUSY_STALL_MS));
}

#[test]
fn idle_has_no_pending_deadline() {
    let controller = Controller::new(5_000, BUSY_STALL_MS, catalog(), false, None);
    assert_eq!(controller.next_deadline_ms(), None);
    let frame = controller.current_frame();
    assert_eq!(frame.width, FALLBACK_FRAME_SIZE);
    assert_eq!(frame.delay_ms, 0);

    // A session opening while idle stays idle: the Idle animation was already
    // loaded at startup, so nothing is reloaded or repainted.
    let mut controller = controller;
    let actions = controller.handle_event(session_started_event(S1, 10_000), 10_000);
    assert_eq!(actions.visible_state, None);
    assert!(!actions.present_frame);
    // Only the stale-drop deadline remains, hours away.
    assert!(
        controller
            .next_deadline_ms()
            .is_some_and(|at| at > 11_000_000)
    );
}

#[test]
fn a_duplicate_delivery_does_not_restart_the_animation() {
    let (_dir, catalog) = gif_catalog();
    let mut controller = Controller::new(5_000, BUSY_STALL_MS, catalog, false, None);
    controller.handle_event(prompt_event(S1, 10_000), 10_000);
    assert_eq!(controller.next_deadline_ms(), Some(10_400));

    let actions = controller.handle_event(prompt_event(S1, 10_000), 10_500);
    assert_eq!(actions.visible_state, None);
    assert!(!actions.present_frame);
    assert_eq!(controller.next_deadline_ms(), Some(10_400));
}

#[test]
fn fallback_to_still_replaces_the_active_animation() {
    let (_dir, catalog) = gif_catalog();
    let mut controller = Controller::new(5_000, BUSY_STALL_MS, catalog, false, None);
    controller.handle_event(prompt_event(S1, 10_000), 10_000);
    assert_eq!(controller.next_deadline_ms(), Some(10_400));

    controller.fallback_to_still();
    assert_eq!(controller.next_deadline_ms(), Some(11_000));
    let frame = controller.current_frame();
    assert_eq!(frame.delay_ms, 0);
    assert_eq!(frame.alpha_at(frame.width / 2, frame.height / 2), 255);
}

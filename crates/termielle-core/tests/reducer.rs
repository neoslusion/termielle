use termielle_core::{
    ApplyOutcome, EventKind, EventMessage, PROTOCOL_VERSION, SessionReducer, Source, VisualState,
};

const FOUR_HOURS_MS: u64 = 14_400_000;
const BUSY_STALL_MS: u64 = 60_000;

fn message(source: Source, session: &str, event: EventKind, at: u64) -> EventMessage {
    EventMessage {
        version: PROTOCOL_VERSION,
        source,
        session_id: session.into(),
        event,
        timestamp_ms: at,
    }
}

#[test]
fn thinking_becomes_working_after_one_second() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    reducer.apply(message(
        Source::Codex,
        "one",
        EventKind::PromptSubmitted,
        10_000,
    ));
    assert_eq!(reducer.visible_state(), VisualState::Thinking);
    assert_eq!(reducer.next_deadline_ms(), Some(11_000));
    assert!(!reducer.advance(10_999));
    assert!(reducer.advance(11_000));
    assert_eq!(reducer.visible_state(), VisualState::Working);
}

#[test]
fn thinking_started_holds_the_thinking_face_past_the_heuristic() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    reducer.apply(message(
        Source::Opencode,
        "one",
        EventKind::PromptSubmitted,
        10_000,
    ));
    // A real reasoning signal arrives; the one-second heuristic deadline must
    // no longer advance the overlay.
    reducer.apply(message(
        Source::Opencode,
        "one",
        EventKind::ThinkingStarted,
        10_100,
    ));
    assert_eq!(reducer.visible_state(), VisualState::Thinking);
    assert!(reducer.next_deadline_ms().unwrap() > 11_000);

    assert!(!reducer.advance(30_000));
    assert_eq!(reducer.visible_state(), VisualState::Thinking);

    // Reasoning ends: working, with no timer.
    reducer.apply(message(
        Source::Opencode,
        "one",
        EventKind::ThinkingEnded,
        30_100,
    ));
    assert_eq!(reducer.visible_state(), VisualState::Working);
}

#[test]
fn thinking_end_without_a_start_just_works() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    reducer.apply(message(
        Source::Opencode,
        "one",
        EventKind::ThinkingEnded,
        10_000,
    ));
    assert_eq!(reducer.visible_state(), VisualState::Working);
    assert!(reducer.next_deadline_ms().unwrap() > 60_000);
}

#[test]
fn priority_is_needs_input_failed_ready_running_idle() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    reducer.apply(message(
        Source::Codex,
        "run",
        EventKind::PromptSubmitted,
        1_000,
    ));
    reducer.apply(message(
        Source::Claude,
        "ready",
        EventKind::TurnCompleted,
        2_000,
    ));
    assert_eq!(reducer.visible_state(), VisualState::Ready);
    reducer.apply(message(
        Source::Codex,
        "failed",
        EventKind::TurnFailed,
        3_000,
    ));
    assert_eq!(reducer.visible_state(), VisualState::Failed);
    reducer.apply(message(Source::Claude, "ask", EventKind::NeedsInput, 4_000));
    assert_eq!(reducer.visible_state(), VisualState::NeedsInput);
}

#[test]
fn duplicate_completion_is_idempotent_and_old_events_are_ignored() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    let done = message(Source::Codex, "one", EventKind::TurnCompleted, 20_000);
    assert_eq!(reducer.apply(done.clone()), ApplyOutcome::Changed);
    assert_eq!(reducer.apply(done), ApplyOutcome::Duplicate);
    assert_eq!(
        reducer.apply(message(
            Source::Codex,
            "one",
            EventKind::PromptSubmitted,
            19_999
        )),
        ApplyOutcome::Stale
    );
    assert_eq!(reducer.next_deadline_ms(), Some(25_000));
}

#[test]
fn ready_expires_and_session_end_removes_state() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    reducer.apply(message(
        Source::Codex,
        "one",
        EventKind::TurnCompleted,
        30_000,
    ));
    assert!(reducer.advance(35_000));
    assert_eq!(reducer.visible_state(), VisualState::Idle);
    reducer.apply(message(
        Source::Codex,
        "one",
        EventKind::SessionEnded,
        36_000,
    ));
    assert_eq!(reducer.session_count(), 0);
}

#[test]
fn equal_priority_states_resolve_by_most_recent_activity() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    reducer.apply(message(
        Source::Claude,
        "first",
        EventKind::PromptSubmitted,
        1_000,
    ));
    reducer.apply(message(
        Source::Codex,
        "second",
        EventKind::PromptSubmitted,
        4_000,
    ));

    // "first" crosses its one-second deadline and becomes Working, but Thinking and
    // Working share priority 2, so the more recently active "second" still wins.
    assert!(!reducer.advance(2_000));
    assert_eq!(reducer.visible_state(), VisualState::Thinking);

    // Once "second" also crosses its deadline both sessions are Working.
    assert!(reducer.advance(5_000));
    assert_eq!(reducer.visible_state(), VisualState::Working);
}

#[test]
fn session_started_registers_an_idle_session() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    assert_eq!(
        reducer.apply(message(
            Source::Claude,
            "one",
            EventKind::SessionStarted,
            1_000
        )),
        ApplyOutcome::Changed
    );
    assert_eq!(reducer.session_count(), 1);
    assert_eq!(reducer.visible_state(), VisualState::Idle);
    assert_eq!(reducer.next_deadline_ms(), Some(1_000 + FOUR_HOURS_MS));
}

#[test]
fn a_new_prompt_leaves_needs_input() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    reducer.apply(message(Source::Codex, "one", EventKind::NeedsInput, 1_000));
    assert_eq!(reducer.visible_state(), VisualState::NeedsInput);
    assert_eq!(
        reducer.apply(message(
            Source::Codex,
            "one",
            EventKind::PromptSubmitted,
            2_000
        )),
        ApplyOutcome::Changed
    );
    assert_eq!(reducer.visible_state(), VisualState::Thinking);
    assert_eq!(reducer.next_deadline_ms(), Some(3_000));
}

#[test]
fn an_accepted_event_hidden_behind_a_busier_session_is_unchanged() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    reducer.apply(message(Source::Claude, "ask", EventKind::NeedsInput, 1_000));
    assert_eq!(reducer.visible_state(), VisualState::NeedsInput);
    assert_eq!(reducer.next_deadline_ms(), Some(1_000 + FOUR_HOURS_MS));

    // Accepted, but a lower-priority session with a later stale deadline moves
    // neither the visible state nor the next deadline.
    assert_eq!(
        reducer.apply(message(
            Source::Codex,
            "new",
            EventKind::SessionStarted,
            2_000
        )),
        ApplyOutcome::Unchanged
    );
    assert_eq!(reducer.session_count(), 2);
    assert_eq!(reducer.visible_state(), VisualState::NeedsInput);
    assert_eq!(reducer.next_deadline_ms(), Some(1_000 + FOUR_HOURS_MS));
}

#[test]
fn sessions_expire_after_four_idle_hours() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    reducer.apply(message(Source::Claude, "one", EventKind::NeedsInput, 1_000));
    assert_eq!(reducer.next_deadline_ms(), Some(1_000 + FOUR_HOURS_MS));

    assert!(!reducer.advance(FOUR_HOURS_MS + 999));
    assert_eq!(reducer.session_count(), 1);
    assert_eq!(reducer.visible_state(), VisualState::NeedsInput);

    assert!(reducer.advance(FOUR_HOURS_MS + 1_000));
    assert_eq!(reducer.session_count(), 0);
    assert_eq!(reducer.visible_state(), VisualState::Idle);
    assert_eq!(reducer.next_deadline_ms(), None);
}

#[test]
fn a_busy_session_decays_to_idle_after_the_stall_without_events() {
    let mut reducer = SessionReducer::new(5_000, BUSY_STALL_MS);
    reducer.apply(message(
        Source::Codex,
        "one",
        EventKind::PromptSubmitted,
        10_000,
    ));
    assert!(reducer.advance(11_000));
    assert_eq!(reducer.visible_state(), VisualState::Working);
    // The stall deadline, not the four-hour stale window, is now nearest.
    assert_eq!(reducer.next_deadline_ms(), Some(10_000 + BUSY_STALL_MS));

    assert!(!reducer.advance(10_000 + BUSY_STALL_MS - 1));
    assert_eq!(reducer.visible_state(), VisualState::Working);

    assert!(reducer.advance(10_000 + BUSY_STALL_MS));
    assert_eq!(reducer.visible_state(), VisualState::Idle);
    // The record survives until the stale window drops it.
    assert_eq!(reducer.session_count(), 1);
    assert_eq!(reducer.next_deadline_ms(), Some(10_000 + FOUR_HOURS_MS));
}

#[test]
fn an_event_refreshes_the_busy_stall() {
    let mut reducer = SessionReducer::new(5_000, BUSY_STALL_MS);
    reducer.apply(message(
        Source::Codex,
        "one",
        EventKind::PromptSubmitted,
        10_000,
    ));
    assert!(reducer.advance(11_000));
    assert_eq!(reducer.visible_state(), VisualState::Working);

    // A second prompt re-arms both the thinking hold and the stall.
    reducer.apply(message(
        Source::Codex,
        "one",
        EventKind::PromptSubmitted,
        30_000,
    ));
    assert_eq!(reducer.visible_state(), VisualState::Thinking);
    assert_eq!(reducer.next_deadline_ms(), Some(31_000));
    assert!(reducer.advance(31_000));
    assert_eq!(reducer.visible_state(), VisualState::Working);

    // The original stall moment passes without decaying: only the refreshed
    // one counts.
    assert!(!reducer.advance(10_000 + BUSY_STALL_MS));
    assert_eq!(reducer.visible_state(), VisualState::Working);
    assert!(reducer.advance(30_000 + BUSY_STALL_MS));
    assert_eq!(reducer.visible_state(), VisualState::Idle);
}

#[test]
fn needs_input_is_not_busy_stalled() {
    let mut reducer = SessionReducer::new(5_000, BUSY_STALL_MS);
    reducer.apply(message(
        Source::Claude,
        "ask",
        EventKind::NeedsInput,
        10_000,
    ));
    // No stall deadline: NeedsInput waits for an event or the stale window.
    assert_eq!(reducer.next_deadline_ms(), Some(10_000 + FOUR_HOURS_MS));
    assert!(!reducer.advance(10_000 + BUSY_STALL_MS));
    assert_eq!(reducer.visible_state(), VisualState::NeedsInput);
}

#[test]
fn completing_a_turn_clears_the_busy_stall() {
    let mut reducer = SessionReducer::new(5_000, BUSY_STALL_MS);
    reducer.apply(message(
        Source::Codex,
        "one",
        EventKind::PromptSubmitted,
        10_000,
    ));
    assert_eq!(reducer.next_deadline_ms(), Some(11_000));
    assert_eq!(
        reducer.apply(message(
            Source::Codex,
            "one",
            EventKind::TurnCompleted,
            10_500
        )),
        ApplyOutcome::Changed
    );
    // Ready holds for the ready hold, then Idle; the stall is gone.
    assert_eq!(reducer.next_deadline_ms(), Some(15_500));
    assert!(reducer.advance(15_500));
    assert_eq!(reducer.visible_state(), VisualState::Idle);
    assert!(!reducer.advance(10_000 + BUSY_STALL_MS));
    assert_eq!(reducer.visible_state(), VisualState::Idle);
}

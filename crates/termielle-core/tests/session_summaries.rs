use termielle_core::{
    ApplyOutcome, EventKind, EventMessage, PROTOCOL_VERSION, SessionReducer, Source, VisualState,
};

fn event(source: &str, id: &str, kind: EventKind, at: u64) -> EventMessage {
    EventMessage {
        version: PROTOCOL_VERSION,
        source: Source::parse(source).unwrap(),
        session_id: id.into(),
        event: kind,
        timestamp_ms: at,
    }
}

#[test]
fn snapshots_are_complete_deterministic_and_match_primary_priority() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    for (source, id, kind, at) in [
        ("codex", "same", EventKind::ThinkingEnded, 10_000),
        ("claude", "same", EventKind::ThinkingEnded, 10_000),
        ("claude", "ask", EventKind::NeedsInput, 9_000),
        ("opencode", "failed", EventKind::TurnFailed, 9_500),
        ("codex", "done", EventKind::TurnCompleted, 10_500),
        ("codex", "newer", EventKind::ThinkingEnded, 11_000),
    ] {
        reducer.apply(event(source, id, kind, at));
    }
    let entries = reducer.session_summaries();
    assert_eq!(entries.len(), 6);
    assert_eq!(
        entries
            .iter()
            .map(|s| (s.source.as_str(), s.session_id.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("claude", "ask"),
            ("opencode", "failed"),
            ("codex", "done"),
            ("codex", "newer"),
            ("claude", "same"),
            ("codex", "same")
        ]
    );
    let primary = reducer.primary_session().unwrap();
    assert_eq!(
        (
            entries[0].source.clone(),
            entries[0].session_id.clone(),
            entries[0].state
        ),
        primary
    );
}

#[test]
fn state_age_uses_event_and_scheduled_times_not_the_time_of_a_late_tick() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    reducer.apply(event("claude", "a", EventKind::PromptSubmitted, 10_000));
    assert_eq!(reducer.session_summaries()[0].state_since_ms, 10_000);
    reducer.advance(15_000);
    let working = &reducer.session_summaries()[0];
    assert_eq!(working.state, VisualState::Working);
    assert_eq!(working.state_since_ms, 11_000);
    reducer.apply(event("claude", "a", EventKind::ThinkingEnded, 16_000));
    assert_eq!(reducer.session_summaries()[0].state_since_ms, 11_000);
    reducer.advance(80_000);
    let idle = &reducer.session_summaries()[0];
    assert_eq!(idle.state, VisualState::Idle);
    assert_eq!(idle.state_since_ms, 76_000);
}

#[test]
fn completed_sessions_keep_the_completion_identity_after_ready_hold() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    reducer.apply(event("codex", "done", EventKind::TurnCompleted, 10_000));
    reducer.advance(15_000);
    let summary = &reducer.session_summaries()[0];
    assert_eq!(summary.state, VisualState::Idle);
    assert_eq!(summary.last_event, EventKind::TurnCompleted);
    assert_eq!(summary.last_activity_ms, 10_000);
    assert_eq!(summary.state_since_ms, 15_000);
    reducer.apply(event("codex", "done", EventKind::SessionEnded, 16_000));
    assert!(reducer.session_summaries().is_empty());
}

#[test]
fn revision_tracks_secondary_changes_but_not_duplicate_stale_or_rejected_events() {
    let mut reducer = SessionReducer::new(5_000, 60_000);
    reducer.apply(event("claude", "ask", EventKind::NeedsInput, 10_000));
    reducer.apply(event("codex", "run", EventKind::PromptSubmitted, 10_000));
    let before = reducer.revision();
    assert!(!reducer.advance(11_000)); // Primary remains NeedsInput.
    assert_ne!(before, reducer.revision());
    let before = reducer.revision();
    assert_eq!(
        reducer.apply(event("codex", "run", EventKind::PromptSubmitted, 10_000)),
        ApplyOutcome::Duplicate
    );
    assert_eq!(
        reducer.apply(event("codex", "run", EventKind::TurnCompleted, 9_999)),
        ApplyOutcome::Stale
    );
    assert_eq!(
        reducer.apply_at(
            event("codex", "run", EventKind::TurnCompleted, 100_000),
            11_000
        ),
        ApplyOutcome::Rejected
    );
    assert_eq!(before, reducer.revision());
    reducer.advance(14_411_000);
    assert!(reducer.session_summaries().is_empty());
    assert_ne!(before, reducer.revision());
}

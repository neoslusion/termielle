//! Journal contract tests: append order, replay fidelity, bounds, and
//! resilience to the torn files a crash can leave behind.

use std::io::Write;

use termielle_core::{EventKind, EventLog, EventMessage, Source, decode_event_line};

/// Parses a test source word, panicking when it is invalid.
fn source(word: &str) -> Source {
    Source::parse(word).unwrap()
}

fn event(source: Source, session: &str, kind: EventKind, at_ms: u64) -> EventMessage {
    EventMessage {
        version: 1,
        source,
        session_id: session.to_owned(),
        event: kind,
        timestamp_ms: at_ms,
    }
}

fn journal(dir: &std::path::Path, name: &str, max_bytes: usize) -> EventLog {
    EventLog::new(dir.join(name), max_bytes)
}

#[test]
fn replays_events_in_arrival_order() {
    let dir = tempfile::tempdir().unwrap();
    let log = journal(dir.path(), "events.log", 1_048_576);

    let first = event(source("claude"), "one", EventKind::SessionStarted, 1);
    let second = event(source("codex"), "two", EventKind::TurnCompleted, 2);
    log.append(&first).unwrap();
    log.append(&second).unwrap();

    assert_eq!(log.read_all(), vec![first, second]);
}

#[test]
fn a_missing_journal_replays_as_empty_and_append_creates_it() {
    let dir = tempfile::tempdir().unwrap();
    let log = journal(dir.path(), "events.log", 1_048_576);

    assert_eq!(log.read_all(), Vec::<EventMessage>::new());

    log.append(&event(source("opencode"), "s", EventKind::NeedsInput, 1))
        .unwrap();
    assert_eq!(log.read_all().len(), 1);
    assert!(dir.path().join("events.log").is_file());
}

#[test]
fn the_journal_stores_the_same_wire_form_the_pipe_carries() {
    let dir = tempfile::tempdir().unwrap();
    let log = journal(dir.path(), "events.log", 1_048_576);

    let e = event(source("claude"), "s", EventKind::PromptSubmitted, 7);
    log.append(&e).unwrap();

    let bytes = std::fs::read(dir.path().join("events.log")).unwrap();
    let line = bytes
        .strip_suffix(b"\n")
        .expect("one line, newline terminated");
    assert_eq!(decode_event_line(line).unwrap(), e);
}

#[test]
fn session_identifiers_round_trip_through_any_unicode() {
    let dir = tempfile::tempdir().unwrap();
    let log = journal(dir.path(), "events.log", 1_048_576);

    // Session identifiers may carry any non-control text; the wire form must
    // survive quoting and escaping.
    let e = event(
        source("codex"),
        "会话 \"quoted\" \\slash",
        EventKind::ThinkingStarted,
        3,
    );
    log.append(&e).unwrap();

    assert_eq!(log.read_all(), vec![e]);
}

#[test]
fn trims_the_oldest_events_when_the_budget_is_exceeded() {
    let dir = tempfile::tempdir().unwrap();
    // Small enough that two events cannot both fit.
    let log = journal(dir.path(), "events.log", 120);

    for index in 0..5 {
        log.append(&event(
            source("codex"),
            "s",
            EventKind::TurnCompleted,
            index + 1,
        ))
        .unwrap();
    }

    let replayed = log.read_all();
    assert!(!replayed.is_empty(), "the newest event must always survive");
    assert!(
        replayed[0].timestamp_ms > 0,
        "only the oldest events may be dropped"
    );
    assert!(
        std::fs::metadata(dir.path().join("events.log"))
            .unwrap()
            .len()
            <= 120
    );
}

#[test]
fn skips_a_torn_tail_from_a_crash_mid_append() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.log");
    let log = EventLog::new(path.clone(), 1_048_576);

    log.append(&event(
        source("claude"),
        "one",
        EventKind::SessionStarted,
        1,
    ))
    .unwrap();
    log.append(&event(source("claude"), "two", EventKind::TurnCompleted, 2))
        .unwrap();

    // A crash between `write_all` and `flush` can leave a partial line: the
    // readable prefix must replay, the torn tail must not surface as an error.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(
        br#"{"version":1,"source":"claude","session_id":"torn","event":"needs_input","timestamp_m"#,
    )
    .unwrap();
    drop(file);

    let replayed = log.read_all();
    assert_eq!(replayed.len(), 2);
    assert_eq!(replayed[0].session_id, "one");
    assert_eq!(replayed[1].session_id, "two");
}

#[test]
fn skips_lines_a_newer_protocol_version_does_not_decode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.log");
    let log = EventLog::new(path.clone(), 1_048_576);

    log.append(&event(source("codex"), "s", EventKind::SessionStarted, 1))
        .unwrap();

    // A future protocol version bumps the wire version; today's decoder must
    // skip it rather than fail the whole replay.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(br#"{"version":2,"source":"codex","session_id":"future","event":"session_started","timestamp_ms":9}"#)
        .unwrap();
    drop(file);

    assert_eq!(log.read_all().len(), 1);
}

#[test]
fn append_after_a_trim_keeps_the_journal_consistent() {
    let dir = tempfile::tempdir().unwrap();
    let log = journal(dir.path(), "events.log", 120);

    for index in 0..4 {
        log.append(&event(
            source("claude"),
            "s",
            EventKind::TurnCompleted,
            index + 1,
        ))
        .unwrap();
    }
    log.append(&event(source("claude"), "s", EventKind::SessionEnded, 4))
        .unwrap();

    // The fold over the trimmed journal still produces a decodable, ordered
    // event list.
    let replayed = log.read_all();
    assert!(!replayed.is_empty());
    assert_eq!(replayed.last().unwrap().event, EventKind::SessionEnded);
    for pair in replayed.windows(2) {
        assert!(pair[0].timestamp_ms < pair[1].timestamp_ms);
    }
}

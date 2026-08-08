use termielle_core::{
    EventKind, EventMessage, MAX_EVENT_BYTES, PROTOCOL_VERSION, ProtocolError, Source,
    decode_event_line, encode_event_line,
};

fn event() -> EventMessage {
    EventMessage {
        version: PROTOCOL_VERSION,
        source: Source::Codex,
        session_id: "thr_123".into(),
        event: EventKind::PromptSubmitted,
        timestamp_ms: 1_785_682_800_000,
    }
}

#[test]
fn round_trips_one_newline_terminated_event() {
    let encoded = encode_event_line(&event()).unwrap();
    assert_eq!(encoded.last(), Some(&b'\n'));
    assert_eq!(decode_event_line(&encoded).unwrap(), event());
}

#[test]
fn rejects_oversized_input_before_json_parsing() {
    let input = vec![b' '; MAX_EVENT_BYTES + 1];
    assert_eq!(decode_event_line(&input), Err(ProtocolError::TooLarge));
}

#[test]
fn rejects_unknown_fields_and_versions() {
    let extra = br#"{"version":1,"source":"codex","session_id":"x","event":"session_started","timestamp_ms":1,"prompt":"secret"}"#;
    assert!(matches!(
        decode_event_line(extra),
        Err(ProtocolError::Json(_))
    ));

    let mut wrong = event();
    wrong.version = 2;
    assert_eq!(
        encode_event_line(&wrong),
        Err(ProtocolError::UnsupportedVersion(2))
    );
}

#[test]
fn rejects_empty_long_or_control_character_session_ids() {
    for id in [String::new(), "x".repeat(129), "bad\nsession".into()] {
        let mut value = event();
        value.session_id = id;
        assert!(matches!(
            encode_event_line(&value),
            Err(ProtocolError::InvalidSessionId)
        ));
    }
}

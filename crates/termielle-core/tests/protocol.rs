use termielle_core::{
    EventKind, EventMessage, MAX_EVENT_BYTES, MAX_SOURCE_BYTES, PROTOCOL_VERSION, ProtocolError,
    Source, decode_event_line, encode_event_line,
};

fn event() -> EventMessage {
    EventMessage {
        version: PROTOCOL_VERSION,
        source: Source::parse("codex").unwrap(),
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

#[test]
fn accepts_any_well_formed_source_word_on_the_wire() {
    // Sources are open: a new agent CLI needs no overlay change. The word is
    // all the wire carries, and it must survive the round trip unchanged.
    for word in [
        "claude",
        "codex",
        "opencode",
        "agy",
        "opencode2",
        "gemini",
        "a1-b_c",
    ] {
        let mut value = event();
        value.source = Source::parse(word).unwrap();
        let encoded = encode_event_line(&value).unwrap();
        assert_eq!(decode_event_line(&encoded).unwrap(), value);
    }

    // The historical enum spellings decode to exactly the same words, so
    // journals written before sources were opened still replay.
    let legacy = br#"{"version":1,"source":"codex","session_id":"x","event":"turn_completed","timestamp_ms":7}"#;
    let decoded = decode_event_line(legacy).unwrap();
    assert_eq!(decoded.source.as_str(), "codex");
}

#[test]
fn rejects_malformed_source_words_at_the_boundary() {
    let oversized = "x".repeat(MAX_SOURCE_BYTES + 1);
    for bad in [
        "",
        "Claude",
        "claude code",
        "-lead",
        "_lead",
        "clau.de",
        oversized.as_str(),
    ] {
        let line = format!(
            r#"{{"version":1,"source":"{bad}","session_id":"x","event":"turn_completed","timestamp_ms":7}}"#
        );
        assert_eq!(
            decode_event_line(line.as_bytes()),
            Err(ProtocolError::InvalidSource),
            "for {bad:?}"
        );
    }
}

#[test]
fn accepts_a_source_word_at_exactly_the_protocol_limit() {
    let at_limit = "x".repeat(MAX_SOURCE_BYTES);
    let mut value = event();
    value.source = Source::parse(&at_limit).unwrap();
    let encoded = encode_event_line(&value).unwrap();
    assert_eq!(
        decode_event_line(&encoded).unwrap().source.as_str(),
        at_limit
    );
}

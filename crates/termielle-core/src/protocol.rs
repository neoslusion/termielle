pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_EVENT_BYTES: usize = 4096;
pub const MAX_SESSION_ID_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Claude,
    Codex,
    Opencode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    SessionStarted,
    PromptSubmitted,
    NeedsInput,
    TurnCompleted,
    TurnFailed,
    SessionEnded,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventMessage {
    pub version: u8,
    pub source: Source,
    pub session_id: String,
    pub event: EventKind,
    pub timestamp_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProtocolError {
    #[error("event line exceeds {MAX_EVENT_BYTES} bytes")]
    TooLarge,
    #[error("invalid event JSON: {0}")]
    Json(String),
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u8),
    #[error("invalid session ID")]
    InvalidSessionId,
    #[error("timestamp must be nonzero")]
    InvalidTimestamp,
}

pub fn decode_event_line(input: &[u8]) -> Result<EventMessage, ProtocolError> {
    if input.len() > MAX_EVENT_BYTES {
        return Err(ProtocolError::TooLarge);
    }

    let json = input.strip_suffix(b"\n").unwrap_or(input);
    let event: EventMessage =
        serde_json::from_slice(json).map_err(|error| ProtocolError::Json(error.to_string()))?;

    if json.contains(&b'\n') || json.contains(&b'\r') {
        return Err(ProtocolError::Json(
            "event line contains an embedded newline".into(),
        ));
    }

    validate_event(&event)?;
    Ok(event)
}

pub fn encode_event_line(event: &EventMessage) -> Result<Vec<u8>, ProtocolError> {
    let mut encoded =
        serde_json::to_vec(event).map_err(|error| ProtocolError::Json(error.to_string()))?;
    encoded.push(b'\n');

    if encoded.len() > MAX_EVENT_BYTES {
        return Err(ProtocolError::TooLarge);
    }

    validate_event(event)?;
    Ok(encoded)
}

fn validate_event(event: &EventMessage) -> Result<(), ProtocolError> {
    if event.version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion(event.version));
    }

    if event.session_id.is_empty()
        || event.session_id.len() > MAX_SESSION_ID_BYTES
        || event.session_id.chars().any(char::is_control)
    {
        return Err(ProtocolError::InvalidSessionId);
    }

    if event.timestamp_ms == 0 {
        return Err(ProtocolError::InvalidTimestamp);
    }

    Ok(())
}

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_EVENT_BYTES: usize = 4096;
pub const MAX_SESSION_ID_BYTES: usize = 128;

/// The longest accepted [`Source`] word.
pub const MAX_SOURCE_BYTES: usize = 32;

/// The agent a session belongs to: any short lowercase word (`[a-z0-9_-]+`),
/// such as `claude`, `codex`, `opencode`, `agy`, or `gemini`.
///
/// The wire form is the plain string, so values written before the type was
/// opened up still replay from old journals, and a new agent needs no overlay
/// change — the source exists only to keep sessions from different agents
/// apart, which is why adding one is not a protocol-version bump.
///
/// Serde derives bypass [`Source::parse`], so a deserialized source is *not*
/// trustworthy until [`validate_event`] has re-checked it; every read path
/// (wire decode, journal replay) goes through that validation.
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Source(String);

impl Source {
    /// Validates and wraps a source word. A word is valid when it is
    /// nonempty, at most [`MAX_SOURCE_BYTES`] bytes, starts with a lowercase
    /// ASCII letter or digit, and continues with lowercase ASCII letters,
    /// digits, `-`, or `_`: no control characters, whitespace, or uppercase,
    /// so a source can never split or disguise itself on the wire.
    pub fn parse(raw: &str) -> Result<Self, ProtocolError> {
        if raw.is_empty() || raw.len() > MAX_SOURCE_BYTES {
            return Err(ProtocolError::InvalidSource);
        }

        let mut characters = raw.chars();
        let first = characters.next().unwrap_or('?');
        let rest_ok = characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-'
                || character == '_'
        });
        if !(first.is_ascii_lowercase() || first.is_ascii_digit()) || !rest_ok {
            return Err(ProtocolError::InvalidSource);
        }

        Ok(Self(raw.to_owned()))
    }

    /// The wire form of this source.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    SessionStarted,
    PromptSubmitted,
    /// The agent is actively reasoning; held until [`Self::ThinkingEnded`].
    ThinkingStarted,
    /// Reasoning finished; the agent is generating or running tools.
    ThinkingEnded,
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
    #[error("invalid source identifier")]
    InvalidSource,
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

    // Re-validated here because serde derives construct `Source` without
    // going through `parse`: a hostile or outdated emitter must not be able
    // to smuggle a malformed source word onto the pipe or into the journal.
    Source::parse(&event.source.0)?;

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

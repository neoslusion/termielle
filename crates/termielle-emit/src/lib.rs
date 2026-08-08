//! Fail-open hook emitter for Claude Code and Codex CLI.
//!
//! Agent hooks invoke this process with a fixed source and event name, a hook
//! document on stdin or the command line, and nothing else. The document is
//! reduced to a session identifier, one event line is sent over the overlay
//! pipe, and the neutral hook response is always written before exit. A missing
//! overlay, a malformed document, or a pipe failure never delays or breaks the
//! agent: those paths exit `0` after writing [`neutral_hook_output`]. Only a
//! broken installation (an invalid command line) exits `2`.
//!
//! Nothing from the hook document ever reaches the pipe or the log except the
//! session identifier: no prompts, assistant text, tool payloads, or paths.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use termielle_core::{
    EventKind, EventMessage, MAX_EVENT_BYTES, MAX_SESSION_ID_BYTES, PROTOCOL_VERSION, Source,
    encode_event_line,
};
use termielle_ipc::{DEFAULT_PIPE_NAME, PipeClient};

/// Hook documents are bounded by the same ceiling as wire events: anything an
/// agent could hand us that matters fits well inside 4 KiB, and a document
/// larger than the wire ceiling could never become a valid event anyway.
pub const MAX_HOOK_INPUT_BYTES: usize = MAX_EVENT_BYTES;

/// The overlay must never be able to stall an agent: if it has not answered in
/// this long, the emitter gives up and fails open.
const PIPE_TIMEOUT: Duration = Duration::from_millis(20);

/// The response every agent hook API parses. Written on every path, including
/// usage errors, so a broken installation still leaves the agent running.
pub const fn neutral_hook_output() -> &'static [u8] {
    b"{}\n"
}

/// Why a hook document could not be turned into an event.
///
/// Every variant is a fixed classification with no payload, so a failure can be
/// swallowed or logged without ever leaking what the agent was doing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EmitError {
    /// The document has none of the accepted session identifier keys.
    #[error("hook document carries no session identifier")]
    MissingSessionId,
    /// A present session identifier is empty, overlong, or contains control
    /// characters, so it cannot be carried on the wire.
    #[error("session identifier is invalid")]
    InvalidSessionId,
    /// The document exceeds the protocol ceiling.
    #[error("hook input exceeds {MAX_HOOK_INPUT_BYTES} bytes")]
    InputTooLarge,
    /// The document is empty, not JSON, or its session identifier is not a
    /// string. Kept as one classification: none of these cases is actionable.
    #[error("hook input is not usable JSON")]
    MalformedInput,
    /// The caller supplied a zero clock reading, which the protocol forbids.
    #[error("timestamp is zero")]
    InvalidTimestamp,
    /// The hook's stdin could not be read at all.
    #[error("could not read hook input")]
    ReadFailed,
}

/// Where the hook document comes from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputSource {
    Stdin,
    Argv,
}

/// The parsed, validated command line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmitArgs {
    pub source: Source,
    pub event: EventKind,
    pub input: InputSource,
    pub pipe: Option<String>,
    pub document: Option<String>,
}

/// A command line that cannot be an installed hook. These are the only errors
/// that exit `2`: they mean the installation itself is wrong.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum UsageError {
    #[error("unknown flag")]
    UnknownFlag,
    #[error("a required flag is missing")]
    MissingFlag,
    #[error("a flag has an unknown value")]
    UnknownValue,
    #[error("a flag is missing its value")]
    MissingValue,
    #[error("a flag was given more than once")]
    RepeatedFlag,
    #[error("a JSON document was given without --input argv")]
    UnexpectedPositional,
    #[error("an argument is not valid UTF-8")]
    NonUtf8,
}

/// Reduces a hook document to the only information it may contribute: the
/// session identifier. Everything else in the document is ignored, and the
/// event kind comes from the command line, never from the document, so a newer
/// or hostile hook payload cannot steer the overlay into a state the installed
/// hook did not ask for.
pub fn normalize_hook_event(
    source: Source,
    event: EventKind,
    input: &[u8],
    now_ms: u64,
) -> Result<EventMessage, EmitError> {
    if input.len() > MAX_HOOK_INPUT_BYTES {
        return Err(EmitError::InputTooLarge);
    }
    if input.is_empty() {
        return Err(EmitError::MalformedInput);
    }

    let value: serde_json::Value =
        serde_json::from_slice(input).map_err(|_| EmitError::MalformedInput)?;

    // The clock check precedes session extraction so a zero reading is always
    // reported as the clock error, never masked by a document problem.
    if now_ms == 0 {
        return Err(EmitError::InvalidTimestamp);
    }

    let session_id = session_id_from_document(&value)?;

    Ok(EventMessage {
        version: PROTOCOL_VERSION,
        source,
        session_id,
        event,
        timestamp_ms: now_ms,
    })
}

/// Extracts the session identifier, preferring `session_id` (Claude Code),
/// then `thread-id` (Codex), then `thread_id` (older Codex). A present
/// identifier must be a string and pass the wire validation; anything else is
/// unusable.
fn session_id_from_document(value: &serde_json::Value) -> Result<String, EmitError> {
    for key in ["session_id", "thread-id", "thread_id"] {
        let Some(found) = value.get(key) else {
            continue;
        };
        let text = found.as_str().ok_or(EmitError::MalformedInput)?;
        if text.is_empty()
            || text.len() > MAX_SESSION_ID_BYTES
            || text.chars().any(char::is_control)
        {
            return Err(EmitError::InvalidSessionId);
        }
        return Ok(text.to_owned());
    }
    Err(EmitError::MissingSessionId)
}

/// Reads the hook document, stopping at the ceiling so an oversized document is
/// classified without being pulled into memory.
///
/// Reading one byte past the ceiling is deliberate: `refuses_to_read_past_the_ceiling`
/// needs that byte to tell "exactly at the limit" from "larger than the limit",
/// and agent hooks always close stdin, so the extra read returns EOF promptly.
pub fn read_bounded(source: &mut impl Read) -> Result<Vec<u8>, EmitError> {
    let mut buffer = Vec::with_capacity(MAX_HOOK_INPUT_BYTES);
    let mut chunk = [0u8; 1024];

    loop {
        let read = source.read(&mut chunk).map_err(|_| EmitError::ReadFailed)?;
        if read == 0 {
            break;
        }
        if buffer.len() + read > MAX_HOOK_INPUT_BYTES {
            return Err(EmitError::InputTooLarge);
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    Ok(buffer)
}

/// Parses the documented flag grammar, validated at the end so the positional
/// JSON document is accepted in any position alongside `--input argv`.
pub fn parse_args(args: Vec<OsString>) -> Result<EmitArgs, UsageError> {
    let mut source = None;
    let mut event = None;
    let mut input = None;
    let mut pipe = None;
    let mut document = None;

    let mut position = args.iter();
    while let Some(raw) = position.next() {
        let Some(arg) = raw.to_str() else {
            return Err(UsageError::NonUtf8);
        };

        let Some(flag) = arg.strip_prefix("--") else {
            if document.is_some() {
                return Err(UsageError::UnexpectedPositional);
            }
            document = Some(arg.to_owned());
            continue;
        };

        // The flag name is judged before its value, so an unknown flag is
        // reported even when nothing follows it.
        let value = match flag {
            "source" | "event" | "input" | "pipe" => {
                position.next().ok_or(UsageError::MissingValue)?
            }
            _ => return Err(UsageError::UnknownFlag),
        };
        let value = value.to_str().ok_or(UsageError::NonUtf8)?;

        match flag {
            "source" => {
                if source.is_some() {
                    return Err(UsageError::RepeatedFlag);
                }
                source = Some(match value {
                    "claude" => Source::Claude,
                    "codex" => Source::Codex,
                    "opencode" => Source::Opencode,
                    _ => return Err(UsageError::UnknownValue),
                });
            }
            "event" => {
                if event.is_some() {
                    return Err(UsageError::RepeatedFlag);
                }
                event = Some(match value {
                    "session_started" => EventKind::SessionStarted,
                    "prompt_submitted" => EventKind::PromptSubmitted,
                    "thinking_started" => EventKind::ThinkingStarted,
                    "thinking_ended" => EventKind::ThinkingEnded,
                    "needs_input" => EventKind::NeedsInput,
                    "turn_completed" => EventKind::TurnCompleted,
                    "turn_failed" => EventKind::TurnFailed,
                    "session_ended" => EventKind::SessionEnded,
                    _ => return Err(UsageError::UnknownValue),
                });
            }
            "input" => {
                if input.is_some() {
                    return Err(UsageError::RepeatedFlag);
                }
                input = Some(match value {
                    "stdin" => InputSource::Stdin,
                    "argv" => InputSource::Argv,
                    _ => return Err(UsageError::UnknownValue),
                });
            }
            "pipe" => {
                if pipe.is_some() {
                    return Err(UsageError::RepeatedFlag);
                }
                pipe = Some(value.to_owned());
            }
            _ => unreachable!("flag membership checked above"),
        }
    }

    let source = source.ok_or(UsageError::MissingFlag)?;
    let event = event.ok_or(UsageError::MissingFlag)?;
    let input = input.ok_or(UsageError::MissingFlag)?;
    if document.is_some() && input != InputSource::Argv {
        return Err(UsageError::UnexpectedPositional);
    }

    Ok(EmitArgs {
        source,
        event,
        input,
        pipe,
        document,
    })
}

/// Executes one emitter run against in-memory IO, returning the process exit
/// code. The neutral hook response is written first so every path below leaves
/// the agent with a parsable hook output.
pub fn run(args: Vec<OsString>, stdin: &mut impl Read, stdout: &mut impl Write) -> i32 {
    if stdout.write_all(neutral_hook_output()).is_err() {
        return 0;
    }

    let parsed = match parse_args(args) {
        Ok(parsed) => parsed,
        // A misinstalled hook is the one failure the agent should hear about.
        Err(_) => return 2,
    };

    let document: &[u8];
    let stdin_buffer: Vec<u8>;
    match parsed.input {
        InputSource::Argv => {
            // An argv hook that fired without its document is an unusable hook,
            // not a broken installation: fail open like any other bad document.
            let Some(value) = parsed.document.as_deref() else {
                return 0;
            };
            document = value.as_bytes();
        }
        InputSource::Stdin => match read_bounded(stdin) {
            Ok(bytes) => {
                stdin_buffer = bytes;
                document = &stdin_buffer;
            }
            Err(_) => return 0,
        },
    }

    let event = match normalize_hook_event(parsed.source, parsed.event, document, now_ms()) {
        Ok(event) => event,
        Err(_) => return 0,
    };
    let line = match encode_event_line(&event) {
        Ok(line) => line,
        Err(_) => return 0,
    };

    let pipe = parsed.pipe.as_deref().unwrap_or(DEFAULT_PIPE_NAME);
    let client = PipeClient::new(pipe, PIPE_TIMEOUT);
    // A missing overlay, a timeout, and every other pipe failure are all
    // fail-open outcomes: the agent must never wait on or break over the pet.
    let _ = client.send(&line);
    0
}

/// Milliseconds since the Unix epoch, or a nonzero stand-in when the clock is
/// unusable, so a normalized event always satisfies the protocol's nonzero
/// timestamp rule.
fn now_ms() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as u64,
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use termielle_core::{
        EventKind, MAX_EVENT_BYTES, MAX_SESSION_ID_BYTES, PROTOCOL_VERSION, Source,
        encode_event_line,
    };

    use crate::{
        EmitArgs, EmitError, InputSource, MAX_HOOK_INPUT_BYTES, UsageError, neutral_hook_output,
        normalize_hook_event, parse_args, read_bounded, run,
    };

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    /// A pipe name no overlay can be listening on, so the `run` tests exercise
    /// the fail-open path instead of reaching a developer's live overlay.
    fn dead_pipe_name(label: &str) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        format!(
            r"\\.\pipe\termielle-emit-unit-{}-{}-{}",
            std::process::id(),
            label,
            NEXT.fetch_add(1, Ordering::Relaxed),
        )
    }

    // -- neutral output ----------------------------------------------------

    #[test]
    fn neutral_hook_output_is_exactly_an_empty_json_object() {
        assert_eq!(neutral_hook_output(), b"{}\n");
    }

    // -- normalization -----------------------------------------------------

    #[test]
    fn reads_claude_session_id_from_stdin_json() {
        let input = br#"{"session_id":"claude-1","hook_event_name":"UserPromptSubmit","prompt":"must-not-leak"}"#;
        let event =
            normalize_hook_event(Source::Claude, EventKind::PromptSubmitted, input, 42).unwrap();
        assert_eq!(event.session_id, "claude-1");
        let encoded = serde_json::to_string(&event).unwrap();
        assert!(!encoded.contains("must-not-leak"));
    }

    #[test]
    fn reads_codex_notify_thread_id_without_copying_messages() {
        let input = br#"{"type":"agent-turn-complete","thread-id":"thr-9","last-assistant-message":"private"}"#;
        let event =
            normalize_hook_event(Source::Codex, EventKind::TurnCompleted, input, 43).unwrap();
        assert_eq!(event.session_id, "thr-9");
        assert!(!serde_json::to_string(&event).unwrap().contains("private"));
    }

    #[test]
    fn carries_only_the_protocol_fields_and_the_callers_own_arguments() {
        // Nothing from the hook document may reach the event except the session
        // identifier. In particular the event kind comes from the command line,
        // not from `hook_event_name`, so a newer or hostile document cannot
        // steer the overlay into a state the installed hook did not ask for.
        let input = br#"{"session_id":"s","hook_event_name":"SessionEnd","cwd":"C:\\secret"}"#;
        let event =
            normalize_hook_event(Source::Claude, EventKind::PromptSubmitted, input, 7).unwrap();

        assert_eq!(event.version, PROTOCOL_VERSION);
        assert_eq!(event.source, Source::Claude);
        assert_eq!(event.session_id, "s");
        assert_eq!(event.event, EventKind::PromptSubmitted);
        assert_eq!(event.timestamp_ms, 7);
        assert!(!serde_json::to_string(&event).unwrap().contains("secret"));
    }

    #[test]
    fn prefers_session_id_over_both_thread_id_spellings() {
        let input = br#"{"thread_id":"third","thread-id":"second","session_id":"first"}"#;
        let event =
            normalize_hook_event(Source::Codex, EventKind::SessionStarted, input, 1).unwrap();
        assert_eq!(event.session_id, "first");
    }

    #[test]
    fn prefers_the_hyphenated_thread_id_over_the_underscored_one() {
        let input = br#"{"thread_id":"third","thread-id":"second"}"#;
        let event =
            normalize_hook_event(Source::Codex, EventKind::SessionStarted, input, 1).unwrap();
        assert_eq!(event.session_id, "second");
    }

    #[test]
    fn accepts_the_underscored_thread_id_as_the_last_resort() {
        let input = br#"{"thread_id":"third"}"#;
        let event =
            normalize_hook_event(Source::Codex, EventKind::SessionStarted, input, 1).unwrap();
        assert_eq!(event.session_id, "third");
    }

    #[test]
    fn rejects_a_document_carrying_no_session_identifier() {
        let input = br#"{"hook_event_name":"Stop","prompt":"must-not-leak"}"#;
        let error =
            normalize_hook_event(Source::Claude, EventKind::TurnCompleted, input, 1).unwrap_err();
        assert_eq!(error, EmitError::MissingSessionId);
        assert!(!error.to_string().contains("must-not-leak"));
    }

    #[test]
    fn rejects_an_empty_session_identifier() {
        let input = br#"{"session_id":""}"#;
        let error =
            normalize_hook_event(Source::Claude, EventKind::TurnCompleted, input, 1).unwrap_err();
        assert_eq!(error, EmitError::InvalidSessionId);
    }

    #[test]
    fn rejects_a_session_identifier_past_the_protocol_limit() {
        let oversized = "x".repeat(MAX_SESSION_ID_BYTES + 1);
        let input = format!(r#"{{"session_id":"{oversized}"}}"#);
        let error = normalize_hook_event(
            Source::Claude,
            EventKind::TurnCompleted,
            input.as_bytes(),
            1,
        )
        .unwrap_err();
        assert_eq!(error, EmitError::InvalidSessionId);
    }

    #[test]
    fn accepts_a_session_identifier_at_exactly_the_protocol_limit() {
        let at_limit = "x".repeat(MAX_SESSION_ID_BYTES);
        let input = format!(r#"{{"session_id":"{at_limit}"}}"#);
        let event = normalize_hook_event(
            Source::Claude,
            EventKind::TurnCompleted,
            input.as_bytes(),
            1,
        )
        .unwrap();
        assert_eq!(event.session_id, at_limit);
    }

    #[test]
    fn rejects_a_session_identifier_containing_a_control_character() {
        // A newline would split one event line into two on the wire, so it must
        // never reach the encoder.
        let input = br#"{"session_id":"a\nb"}"#;
        let error =
            normalize_hook_event(Source::Claude, EventKind::TurnCompleted, input, 1).unwrap_err();
        assert_eq!(error, EmitError::InvalidSessionId);
    }

    #[test]
    fn rejects_input_past_the_protocol_ceiling() {
        let mut input = br#"{"session_id":"s","pad":""#.to_vec();
        input.resize(MAX_HOOK_INPUT_BYTES + 1, b'x');
        input.extend_from_slice(br#""}"#);

        let error =
            normalize_hook_event(Source::Claude, EventKind::TurnCompleted, &input, 1).unwrap_err();

        assert_eq!(error, EmitError::InputTooLarge);
        assert_eq!(MAX_HOOK_INPUT_BYTES, MAX_EVENT_BYTES);
    }

    #[test]
    fn accepts_input_at_exactly_the_protocol_ceiling() {
        let prefix = br#"{"session_id":"s","pad":""#;
        let suffix = br#""}"#;
        let mut input = prefix.to_vec();
        input.resize(MAX_HOOK_INPUT_BYTES - suffix.len(), b'x');
        input.extend_from_slice(suffix);
        assert_eq!(input.len(), MAX_HOOK_INPUT_BYTES);

        let event =
            normalize_hook_event(Source::Claude, EventKind::TurnCompleted, &input, 1).unwrap();

        assert_eq!(event.session_id, "s");
    }

    #[test]
    fn rejects_input_that_is_not_json() {
        let error = normalize_hook_event(Source::Claude, EventKind::TurnCompleted, b"not json", 1)
            .unwrap_err();
        assert_eq!(error, EmitError::MalformedInput);
    }

    #[test]
    fn rejects_empty_input() {
        // A hook invoked with no stdin at all must fail open rather than invent
        // a session identifier.
        let error =
            normalize_hook_event(Source::Claude, EventKind::TurnCompleted, b"", 1).unwrap_err();
        assert_eq!(error, EmitError::MalformedInput);
    }

    #[test]
    fn rejects_a_session_identifier_that_is_not_a_string() {
        let input = br#"{"session_id":17}"#;
        let error =
            normalize_hook_event(Source::Claude, EventKind::TurnCompleted, input, 1).unwrap_err();
        assert_eq!(error, EmitError::MalformedInput);
    }

    #[test]
    fn rejects_a_zero_timestamp() {
        // The protocol rejects a zero timestamp at encode time, so catching it
        // here keeps the invariant that a normalized event always encodes.
        let error =
            normalize_hook_event(Source::Claude, EventKind::TurnCompleted, br#"{"a":1}"#, 0)
                .unwrap_err();
        assert_eq!(error, EmitError::InvalidTimestamp);
    }

    #[test]
    fn every_normalized_event_encodes_within_the_protocol() {
        let at_limit = "x".repeat(MAX_SESSION_ID_BYTES);
        let input = format!(r#"{{"session_id":"{at_limit}"}}"#);
        let event = normalize_hook_event(
            Source::Codex,
            EventKind::SessionEnded,
            input.as_bytes(),
            u64::MAX,
        )
        .unwrap();

        let line = encode_event_line(&event).expect("a normalized event must encode");

        assert!(line.ends_with(b"\n"));
        assert!(line.len() <= MAX_EVENT_BYTES);
    }

    // -- bounded reads -----------------------------------------------------

    #[test]
    fn reads_a_document_at_the_ceiling() {
        let source = vec![b'x'; MAX_HOOK_INPUT_BYTES];
        let read = read_bounded(&mut source.as_slice()).unwrap();
        assert_eq!(read.len(), MAX_HOOK_INPUT_BYTES);
    }

    #[test]
    fn refuses_to_read_past_the_ceiling() {
        // One byte past the ceiling is enough to classify the document; the rest
        // is never pulled into memory.
        let source = vec![b'x'; MAX_HOOK_INPUT_BYTES * 4];
        let error = read_bounded(&mut source.as_slice()).unwrap_err();
        assert_eq!(error, EmitError::InputTooLarge);
    }

    // -- command line ------------------------------------------------------

    #[test]
    fn parses_the_full_documented_grammar() {
        let parsed = parse_args(args(&[
            "--source",
            "codex",
            "--event",
            "turn_completed",
            "--input",
            "argv",
            "--pipe",
            r"\\.\pipe\custom",
            r#"{"thread-id":"t"}"#,
        ]))
        .unwrap();

        assert_eq!(
            parsed,
            EmitArgs {
                source: Source::Codex,
                event: EventKind::TurnCompleted,
                input: InputSource::Argv,
                pipe: Some(r"\\.\pipe\custom".to_owned()),
                document: Some(r#"{"thread-id":"t"}"#.to_owned()),
            }
        );
    }

    #[test]
    fn leaves_the_pipe_unset_when_it_is_not_given() {
        let parsed = parse_args(args(&[
            "--source",
            "claude",
            "--event",
            "session_started",
            "--input",
            "stdin",
        ]))
        .unwrap();

        assert_eq!(parsed.pipe, None);
        assert_eq!(parsed.document, None);
        assert_eq!(parsed.source, Source::Claude);
        assert_eq!(parsed.input, InputSource::Stdin);
    }

    #[test]
    fn parses_every_documented_event_keyword() {
        let expected = [
            ("session_started", EventKind::SessionStarted),
            ("prompt_submitted", EventKind::PromptSubmitted),
            ("thinking_started", EventKind::ThinkingStarted),
            ("thinking_ended", EventKind::ThinkingEnded),
            ("needs_input", EventKind::NeedsInput),
            ("turn_completed", EventKind::TurnCompleted),
            ("turn_failed", EventKind::TurnFailed),
            ("session_ended", EventKind::SessionEnded),
        ];

        for (keyword, kind) in expected {
            let parsed = parse_args(args(&[
                "--source", "claude", "--event", keyword, "--input", "stdin",
            ]))
            .unwrap_or_else(|error| panic!("{keyword} must parse, got {error}"));
            assert_eq!(parsed.event, kind);
            // The CLI keyword and the wire encoding must stay the same word, or
            // a hook fixture and the overlay would disagree about an event.
            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                format!("\"{keyword}\"")
            );
        }
    }

    #[test]
    fn rejects_an_unknown_flag() {
        let error = parse_args(args(&[
            "--source",
            "claude",
            "--event",
            "needs_input",
            "--input",
            "stdin",
            "--verbose",
        ]))
        .unwrap_err();
        assert_eq!(error, UsageError::UnknownFlag);
    }

    #[test]
    fn rejects_a_missing_required_flag() {
        for incomplete in [
            vec!["--event", "needs_input", "--input", "stdin"],
            vec!["--source", "claude", "--input", "stdin"],
            vec!["--source", "claude", "--event", "needs_input"],
            vec![],
        ] {
            let error = parse_args(args(&incomplete)).unwrap_err();
            assert_eq!(error, UsageError::MissingFlag, "for {incomplete:?}");
        }
    }

    #[test]
    fn rejects_an_unknown_value_for_a_known_flag() {
        for bad in [
            vec![
                "--source",
                "gemini",
                "--event",
                "needs_input",
                "--input",
                "stdin",
            ],
            vec![
                "--source",
                "claude",
                "--event",
                "tool_used",
                "--input",
                "stdin",
            ],
            vec![
                "--source",
                "claude",
                "--event",
                "needs_input",
                "--input",
                "file",
            ],
        ] {
            let error = parse_args(args(&bad)).unwrap_err();
            assert_eq!(error, UsageError::UnknownValue, "for {bad:?}");
        }
    }

    #[test]
    fn rejects_a_flag_without_a_value() {
        let error = parse_args(args(&[
            "--source",
            "claude",
            "--event",
            "needs_input",
            "--input",
        ]))
        .unwrap_err();
        assert_eq!(error, UsageError::MissingValue);
    }

    #[test]
    fn rejects_a_repeated_flag() {
        let error = parse_args(args(&[
            "--source",
            "claude",
            "--source",
            "codex",
            "--event",
            "needs_input",
            "--input",
            "stdin",
        ]))
        .unwrap_err();
        assert_eq!(error, UsageError::RepeatedFlag);
    }

    #[test]
    fn rejects_a_positional_document_without_argv_input() {
        // Order must not matter: the positional is judged once the whole command
        // line has been read, not at the moment it is seen.
        let error = parse_args(args(&[
            "--source",
            "claude",
            r#"{"session_id":"s"}"#,
            "--event",
            "needs_input",
            "--input",
            "stdin",
        ]))
        .unwrap_err();
        assert_eq!(error, UsageError::UnexpectedPositional);
    }

    #[test]
    fn rejects_a_second_positional_document() {
        let error = parse_args(args(&[
            "--source",
            "claude",
            "--event",
            "needs_input",
            "--input",
            "argv",
            "{}",
            "{}",
        ]))
        .unwrap_err();
        assert_eq!(error, UsageError::UnexpectedPositional);
    }

    #[cfg(windows)]
    #[test]
    fn rejects_an_argument_that_is_not_utf8() {
        use std::os::windows::ffi::OsStringExt;

        // An unpaired surrogate: a real Windows command line can carry one, and
        // it has no UTF-8 form, so it can be neither a keyword nor a JSON
        // document.
        let mut line = args(&["--source", "claude", "--event", "needs_input", "--input"]);
        line.push(OsString::from_wide(&[0xD800]));

        let error = parse_args(line).unwrap_err();

        assert_eq!(error, UsageError::NonUtf8);
    }

    // -- orchestration -----------------------------------------------------

    #[test]
    fn writes_neutral_output_before_rejecting_a_usage_error() {
        // A misinstalled hook must still leave the agent with a parsable hook
        // response, or a broken install would break the agent too.
        let mut stdout = Vec::new();
        let code = run(args(&["--nonsense"]), &mut b"".as_slice(), &mut stdout);

        assert_eq!(code, 2);
        assert_eq!(stdout, b"{}\n");
    }

    #[test]
    fn exits_zero_when_no_overlay_is_listening() {
        let pipe = dead_pipe_name("absent");
        let mut stdout = Vec::new();
        let started = std::time::Instant::now();

        let code = run(
            args(&[
                "--source",
                "claude",
                "--event",
                "prompt_submitted",
                "--input",
                "argv",
                "--pipe",
                &pipe,
                r#"{"session_id":"s"}"#,
            ]),
            &mut b"".as_slice(),
            &mut stdout,
        );

        assert_eq!(code, 0);
        assert_eq!(stdout, b"{}\n");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "a missing overlay must fail open promptly, took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn exits_zero_on_malformed_hook_input() {
        let pipe = dead_pipe_name("malformed");
        let mut stdout = Vec::new();

        let code = run(
            args(&[
                "--source",
                "codex",
                "--event",
                "turn_completed",
                "--input",
                "stdin",
                "--pipe",
                &pipe,
            ]),
            &mut b"not json at all".as_slice(),
            &mut stdout,
        );

        assert_eq!(code, 0);
        assert_eq!(stdout, b"{}\n");
    }

    #[test]
    fn exits_zero_when_the_argv_document_is_absent() {
        // `--input argv` with nothing after it is a hook that fired without a
        // payload, not a broken installation, so it fails open like any other
        // unusable document.
        let pipe = dead_pipe_name("argv-empty");
        let mut stdout = Vec::new();

        let code = run(
            args(&[
                "--source",
                "codex",
                "--event",
                "turn_completed",
                "--input",
                "argv",
                "--pipe",
                &pipe,
            ]),
            &mut b"".as_slice(),
            &mut stdout,
        );

        assert_eq!(code, 0);
        assert_eq!(stdout, b"{}\n");
    }

    #[test]
    fn never_reads_stdin_when_the_document_came_from_argv() {
        // A hook configured with `--input argv` may be launched with no stdin
        // writer at all; reading it could block the agent indefinitely.
        let pipe = dead_pipe_name("argv-only");
        let mut stdout = Vec::new();
        let mut stdin = FailingReader;

        let code = run(
            args(&[
                "--source",
                "codex",
                "--event",
                "turn_completed",
                "--input",
                "argv",
                "--pipe",
                &pipe,
                r#"{"thread-id":"t"}"#,
            ]),
            &mut stdin,
            &mut stdout,
        );

        assert_eq!(code, 0);
        assert_eq!(stdout, b"{}\n");
    }

    /// Panics if anything reads it, proving the argv path never touches stdin.
    struct FailingReader;

    impl std::io::Read for FailingReader {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("stdin must not be read when the document came from argv");
        }
    }
}

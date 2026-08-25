//! End-to-end fixture check: every command string checked into
//! `integrations/` must be an executable emitter invocation that delivers the
//! right (source, event, session) pair over a real named pipe, and the hook
//! payload must never leak beyond the session identifier.
//!
//! The checked-in fixtures are Windows command lines (`command_windows`,
//! `termielle-emit.exe`), so this suite runs on Windows only.

#![cfg(windows)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;

use termielle_core::{EventKind, Source, decode_event_line};
use termielle_ipc::EventServer;

fn fixture(relative: &str) -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Parses a test source word, panicking when it is invalid.
fn source(word: &str) -> Source {
    Source::parse(word).unwrap()
}

/// One fixture hook: its command line and the event it must deliver.
struct Hook {
    command: String,
    source: Source,
    event: EventKind,
}

/// Every `command_windows` line from `integrations/codex/hooks.toml`,
/// including the legacy notify fallback.
fn codex_hooks() -> Vec<Hook> {
    let text = std::fs::read_to_string(fixture("integrations/codex/hooks.toml")).unwrap();
    let value: toml::Value = toml::from_str(&text).unwrap();

    let mut hooks = Vec::new();
    for (name, groups) in value["hooks"].as_table().unwrap() {
        let event = match name.as_str() {
            "SessionStart" => EventKind::SessionStarted,
            "UserPromptSubmit" => EventKind::PromptSubmitted,
            "PermissionRequest" => EventKind::NeedsInput,
            "Stop" => EventKind::TurnCompleted,
            "SessionEnd" => EventKind::SessionEnded,
            other => panic!("unexpected codex hook event {other}"),
        };
        for group in groups.as_array().unwrap() {
            for handler in group["hooks"].as_array().unwrap() {
                hooks.push(Hook {
                    command: handler["command_windows"].as_str().unwrap().to_string(),
                    source: source("codex"),
                    event,
                });
            }
        }
    }

    // Legacy notify fallback: an argv array, not a [[hooks.*]] table.
    let notify: Vec<&str> = value["notify"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| part.as_str().unwrap())
        .collect();
    hooks.push(Hook {
        command: notify.join(" "),
        source: source("codex"),
        event: EventKind::TurnCompleted,
    });
    hooks
}

/// Every `command` line from `integrations/claude/settings.fragment.json`
/// and the native plugin manifest, which carry the same hooks.
fn claude_hooks() -> Vec<Hook> {
    let mut hooks = Vec::new();
    for relative in [
        "integrations/claude/settings.fragment.json",
        "integrations/claude/.claude-plugin/plugin.json",
    ] {
        let text = std::fs::read_to_string(fixture(relative)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();

        for (name, groups) in value["hooks"].as_object().unwrap() {
            let event = match name.as_str() {
                "SessionStart" => EventKind::SessionStarted,
                "UserPromptSubmit" => EventKind::PromptSubmitted,
                "PermissionRequest" => EventKind::NeedsInput,
                "Stop" => EventKind::TurnCompleted,
                "StopFailure" => EventKind::TurnFailed,
                "SessionEnd" => EventKind::SessionEnded,
                other => panic!("unexpected claude hook event {other}"),
            };
            for group in groups.as_array().unwrap() {
                for handler in group["hooks"].as_array().unwrap() {
                    hooks.push(Hook {
                        command: handler["command"].as_str().unwrap().to_string(),
                        source: source("claude"),
                        event,
                    });
                }
            }
        }
    }
    hooks
}

/// Every `emit("<event>", ...)` mapping from an opencode plugin file. Both
/// the V1 and the OpenCode-2 plugin are checked; they differ only in their
/// `--source` word.
///
/// The plugin builds each invocation as
/// `termielle-emit.exe --source <word> --event <event> --input argv
/// {"session_id":"..."}`, so the fixture reconstructs the same command line
/// and verifies it delivers over a real pipe.
fn opencode_hooks(relative: &str, word: &str) -> Vec<Hook> {
    let text = std::fs::read_to_string(fixture(relative)).unwrap();
    let mut hooks = Vec::new();
    for captured in text.split("emit(\"").skip(1) {
        let wire = captured.split('"').next().unwrap();
        let event = match wire {
            "prompt_submitted" => EventKind::PromptSubmitted,
            "thinking_started" => EventKind::ThinkingStarted,
            "thinking_ended" => EventKind::ThinkingEnded,
            "needs_input" => EventKind::NeedsInput,
            "turn_completed" => EventKind::TurnCompleted,
            "turn_failed" => EventKind::TurnFailed,
            "session_ended" => EventKind::SessionEnded,
            other => panic!("unexpected opencode plugin event {other}"),
        };
        hooks.push(Hook {
            command: format!("termielle-emit.exe --source {word} --event {wire} --input argv"),
            source: source(word),
            event,
        });
    }
    hooks
}

/// Every command from `integrations/agy/hooks.fragment.json`, with the same
/// `{{TERMIELLE_EMIT}}` substitution to the PATH exe that the installer
/// performs against the real absolute path.
fn agy_hooks() -> Vec<Hook> {
    let text = std::fs::read_to_string(fixture("integrations/agy/hooks.fragment.json")).unwrap();
    let substituted = text.replace("{{TERMIELLE_EMIT}}", "termielle-emit.exe");
    let value: serde_json::Value = serde_json::from_str(&substituted).unwrap();

    let handlers = &value["termielle"];
    assert!(
        handlers.is_object(),
        "the agy fragment must keep its `termielle` handler"
    );
    let mut hooks = Vec::new();
    for (name, entries) in handlers.as_object().unwrap() {
        let event = match name.as_str() {
            "PreInvocation" => EventKind::ThinkingStarted,
            "PostInvocation" => EventKind::ThinkingEnded,
            "Stop" => EventKind::TurnCompleted,
            other => panic!("unexpected agy hook event {other}"),
        };
        for entry in entries.as_array().unwrap() {
            hooks.push(Hook {
                command: entry["command"].as_str().unwrap().to_string(),
                source: source("agy"),
                event,
            });
        }
    }
    hooks
}

/// The payload shape each source hands the hook, with a secret that must
/// never reach the wire. Returns (bytes, input_source, session_id).
fn payload(hook: &Hook) -> (Vec<u8>, &'static str, String) {
    let session = format!("fixture-{}-{:x}", hook.source, hook.event_key());
    // The wire name as plain text: `to_string` would serialize the enum as a
    // JSON string, quotes included, and break the document.
    let name = serde_json::to_string(&hook.event).unwrap();
    let name = name.trim_matches('"');
    match (hook.source.as_str(), hook.event) {
        // The notify fallback is the one argv hook: Codex passes the
        // agent-turn-complete JSON as the first command-line argument.
        ("codex", EventKind::TurnCompleted) if hook.command.contains("--input argv") => (
            format!(r#"{{"type":"agent-turn-complete","thread-id":"{session}","last-assistant-message":"secret"}}"#)
                .into_bytes(),
            "argv",
            session,
        ),
        ("codex", _) => (
            format!(r#"{{"type":"{name}","session_id":"{session}","prompt":"secret"}}"#)
                .into_bytes(),
            "stdin",
            session,
        ),
        ("claude", _) => (
            format!(
                r#"{{"session_id":"{session}","hook_event_name":"{name}","transcript_path":"C:\\secret"}}"#
            )
            .into_bytes(),
            "stdin",
            session,
        ),
        // Antigravity's Stop document carries only `conversationId`; the
        // emitter must find that identifier without any session key.
        ("agy", EventKind::TurnCompleted) => (
            format!(
                r#"{{"conversationId":"{session}","terminationReason":"NO_TOOL_CALL","transcriptPath":"C:\\secret"}}"#
            )
            .into_bytes(),
            "stdin",
            session,
        ),
        ("agy", _) => (
            format!(
                r#"{{"session_id":"{session}","hookEventName":"{name}","toolCall":{{"args":{{"CommandLine":"secret"}}}}}}"#
            )
            .into_bytes(),
            "stdin",
            session,
        ),
        (_, _) => (
            format!(r#"{{"session_id":"{session}","prompt":"secret"}}"#).into_bytes(),
            "argv",
            session,
        ),
    }
}

impl Hook {
    fn event_key(&self) -> u8 {
        match self.event {
            EventKind::SessionStarted => 0,
            EventKind::PromptSubmitted => 1,
            EventKind::ThinkingStarted => 6,
            EventKind::ThinkingEnded => 7,
            EventKind::NeedsInput => 2,
            EventKind::TurnCompleted => 3,
            EventKind::TurnFailed => 4,
            EventKind::SessionEnded => 5,
        }
    }
}

/// Runs every fixture command against one live pipe and checks the delivered
/// event. The emitter exe path replaces the fixture's PATH reference, and
/// `--pipe` pins the test's unique pipe so a developer's live overlay is
/// never reached.
#[test]
fn fixture_commands_deliver_the_right_events_over_a_real_pipe() {
    let pipe = termielle_ipc::test_endpoint("fixture");

    let server = EventServer::bind(&pipe).expect("bind test pipe");
    let (sender, receiver) = mpsc::channel();
    let (ready_sender, ready) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = ready_sender.send(());
        loop {
            match server.receive_one() {
                Ok(line) => match decode_event_line(&line) {
                    Ok(event) => {
                        if sender.send(event).is_err() {
                            return;
                        }
                    }
                    Err(_) => return,
                },
                Err(_) => return,
            }
        }
    });
    ready.recv().expect("server thread started");

    let emitter = std::env::var_os("CARGO_BIN_EXE_termielle-emit").expect("emitter binary");
    let mut hooks = codex_hooks();
    hooks.extend(claude_hooks());
    hooks.extend(opencode_hooks(
        "integrations/opencode/termielle.plugin.ts",
        "opencode",
    ));
    hooks.extend(opencode_hooks(
        "integrations/opencode2/termielle.plugin.ts",
        "opencode2",
    ));
    hooks.extend(agy_hooks());
    assert_eq!(
        hooks.len(),
        39,
        "six codex, twelve claude (fragment and plugin manifest), nine \
         opencode V1 plus nine OpenCode-2 plugin hooks, and three agy"
    );

    for (index, hook) in hooks.iter().enumerate() {
        let (document, input, expected_session) = payload(hook);

        let mut args: Vec<String> = hook
            .command
            .split_whitespace()
            .map(str::to_string)
            .collect();
        assert_eq!(
            args[0], "termielle-emit.exe",
            "fixture must call the PATH exe"
        );
        args[0] = emitter.to_string_lossy().into_owned();
        args.push("--pipe".into());
        args.push(pipe.clone());

        let mut child = Command::new(&args[0])
            .args(&args[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("spawn {}: {error}", hook.command));
        if input == "stdin" {
            child.stdin.take().unwrap().write_all(&document).unwrap();
        } else {
            // argv hooks pass the document as a positional argument, exactly
            // like Codex does with the notify payload.
            let json = String::from_utf8(document).unwrap();
            args.insert(1, json.clone());
            let _ = args;
            child = Command::new(&args[0])
                .args(&args[1..])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            // Close stdin immediately: argv hooks must not wait on it.
            child.stdin.take().unwrap().write_all(b"").unwrap();
        }

        let output = child.wait_with_output().expect("emitter finishes");
        assert_eq!(output.status.code(), Some(0), "exit for {}", hook.command);
        assert_eq!(
            output.stdout, b"{}\n",
            "neutral response for {}",
            hook.command
        );

        let event = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap_or_else(|error| {
                panic!(
                    "hook #{index} {} did not deliver an event: {error}",
                    hook.command
                )
            });
        let serialized = serde_json::to_string(&event).unwrap();
        assert_eq!(event.source, hook.source, "source for {}", hook.command);
        assert_eq!(event.event, hook.event, "event for {}", hook.command);
        assert_eq!(event.session_id, expected_session);
        assert!(
            !serialized.contains("secret"),
            "payload content leaked onto the wire: {serialized}"
        );
    }
}

//! Process-level verification of the real `termielle-emit.exe` binary.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::mpsc;

use termielle_core::{EventKind, decode_event_line};
use termielle_ipc::{EventServer, test_endpoint};

fn unique_pipe_name(label: &str) -> String {
    test_endpoint(&format!("emit-{label}"))
}

/// A `termielle-emit.exe` command with piped stdio and no implicit arguments,
/// so each test can add exactly what it exercises.
fn emitter() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_termielle-emit"));
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command
}

#[test]
fn a_real_process_delivers_one_claude_event_to_the_server() {
    let name = unique_pipe_name("deliver");
    let (ready_tx, ready_rx) = mpsc::channel();
    let server_name = name.clone();
    let server = std::thread::spawn(move || {
        let server = EventServer::bind(&server_name).expect("bind");
        ready_tx.send(()).unwrap();
        server.receive_one().expect("receive")
    });
    ready_rx.recv().unwrap();

    let mut child = emitter()
        .args([
            "--source",
            "claude",
            "--event",
            "prompt_submitted",
            "--input",
            "stdin",
            "--pipe",
            &name,
        ])
        .spawn()
        .expect("spawn emitter");

    {
        let mut stdin = child.stdin.take().expect("piped stdin");
        stdin
            .write_all(br#"{"session_id":"claude-1","hook_event_name":"UserPromptSubmit","prompt":"must-not-leak"}"#)
            .expect("write hook document");
    }

    let output = child.wait_with_output().expect("wait for emitter");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"{}\n");
    assert!(output.stderr.is_empty());

    let received = server.join().expect("server thread");
    let event = decode_event_line(&received).expect("decodable event");
    assert_eq!(event.event, EventKind::PromptSubmitted);
    assert_eq!(event.session_id, "claude-1");
    assert!(
        !received
            .windows(14)
            .any(|window| window == b"must-not-leak")
    );
}

#[test]
fn a_real_process_delivers_one_opencode_event_from_an_argv_document() {
    let name = unique_pipe_name("opencode");
    let (ready_tx, ready_rx) = mpsc::channel();
    let server_name = name.clone();
    let server = std::thread::spawn(move || {
        let server = EventServer::bind(&server_name).expect("bind");
        ready_tx.send(()).unwrap();
        server.receive_one().expect("receive")
    });
    ready_rx.recv().unwrap();

    let mut child = emitter()
        .args([
            "--source",
            "opencode",
            "--event",
            "needs_input",
            "--input",
            "argv",
            "--pipe",
            &name,
            r#"{"session_id":"ses_plugin-1","prompt":"must-not-leak"}"#,
        ])
        .spawn()
        .expect("spawn emitter");

    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(b"")
        .unwrap();

    let output = child.wait_with_output().expect("wait for emitter");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"{}\n");
    assert!(output.stderr.is_empty());

    let received = server.join().expect("server thread");
    let event = decode_event_line(&received).expect("decodable event");
    assert_eq!(event.event, EventKind::NeedsInput);
    assert_eq!(event.session_id, "ses_plugin-1");
    assert!(
        !received
            .windows(14)
            .any(|window| window == b"must-not-leak")
    );
}

#[test]
fn a_real_process_fails_open_when_no_overlay_is_listening() {
    let name = unique_pipe_name("absent");

    let output = emitter()
        .args([
            "--source",
            "codex",
            "--event",
            "turn_completed",
            "--input",
            "argv",
            "--pipe",
            &name,
            r#"{"thread-id":"thr-9"}"#,
        ])
        .output()
        .expect("run emitter");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"{}\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn a_real_process_reports_usage_errors_without_delivering() {
    let output = emitter()
        .args(["--nonsense"])
        .output()
        .expect("run emitter");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"{}\n");
    assert!(output.stderr.is_empty());
}

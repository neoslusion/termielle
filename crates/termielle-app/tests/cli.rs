//! End-to-end checks of the diagnostics CLI surface: the smoke test against a
//! unique pipe (with and without an external event) and the acknowledgement
//! file that drives benchmark latency measurements.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn app_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_termielle-app"))
}

/// The emitter lives in the same target directory as the app.
fn emit_binary() -> PathBuf {
    app_binary().with_file_name("termielle-emit.exe")
}

fn unique_pipe(label: &str) -> String {
    format!(
        r"\\.\pipe\termielle-cli-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Sends one codex `prompt_submitted` event over the pipe; that is the only
/// event that reaches the `Thinking` state the smoke test waits for. The wire
/// payload carries no secrets beyond the session identifier.
fn emit_prompt_submitted(pipe: &str, session: &str) -> bool {
    let payload = format!(r#"{{"type":"prompt_submitted","session_id":"{session}","prompt":"x"}}"#);
    Command::new(emit_binary())
        .args([
            "--source",
            "codex",
            "--event",
            "prompt_submitted",
            "--input",
            "stdin",
        ])
        .arg("--pipe")
        .arg(pipe)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(payload.as_bytes())?;
            child.wait()
        })
        .is_ok_and(|status| status.success())
}

/// Polls `child` for exit until `deadline`, killing it if it overruns.
fn wait_exit(mut child: Child, deadline: Instant) -> std::process::ExitStatus {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                panic!("process did not exit within the deadline");
            }
            Err(error) => panic!("wait failed: {error}"),
        }
    }
}

fn ack_has_state(path: &Path, state: &str) -> bool {
    fs::read_to_string(path).is_ok_and(|text| text.contains(&format!(r#""state":"{state}""#)))
}

/// Waits until the ack file records `state`, failing with its content.
fn wait_for_ack_state(path: &Path, state: &str, seconds: u64) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if ack_has_state(path, state) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!(
        "ack file {:?} never recorded {state}; got: {:?}",
        path,
        fs::read_to_string(path)
    );
}

/// Smoke mode on a unique pipe passes only when an external event reaches the
/// `Thinking` state; the emitter delivers it over the real transport.
#[test]
fn smoke_test_passes_on_an_external_event_over_a_custom_pipe() {
    let pipe = unique_pipe("smoke-ext");
    let child = Command::new(app_binary())
        .args(["--smoke-test", "--pipe"])
        .arg(&pipe)
        .spawn()
        .expect("app starts");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        emit_prompt_submitted(&pipe, "cli-ext"),
        "emitter must deliver the event"
    );
    let status = wait_exit(child, Instant::now() + Duration::from_secs(15));
    assert_eq!(
        Some(0),
        status.code(),
        "smoke must exit 0 on an external event"
    );
}

/// Smoke mode with no event at all times out with exit code 1.
#[test]
fn smoke_test_without_an_event_times_out() {
    let pipe = unique_pipe("smoke-none");
    let child = Command::new(app_binary())
        .args(["--smoke-test", "--pipe"])
        .arg(&pipe)
        .spawn()
        .expect("app starts");
    let status = wait_exit(child, Instant::now() + Duration::from_secs(12));
    assert_eq!(Some(1), status.code(), "smoke must exit 1 on timeout");
}

/// A smoke run against a held instance mutex exits 3, so the doctor never
/// mistakes a silent second-instance exit for a passing check.
#[test]
fn smoke_test_with_a_held_instance_exits_3() {
    let holder = Command::new(app_binary())
        .spawn()
        .expect("holder app starts");
    // Give the holder a moment to claim the default mutex and pipe.
    std::thread::sleep(Duration::from_millis(300));
    let mut holder = holder;
    let child = Command::new(app_binary())
        .args(["--smoke-test"])
        .spawn()
        .expect("second app starts");
    let status = wait_exit(child, Instant::now() + Duration::from_secs(10));
    assert_eq!(
        Some(3),
        status.code(),
        "smoke against a held instance must exit 3"
    );
    let _ = holder.kill();
    let _ = holder.wait();
}

/// Normal mode records every presented state in the ack file, one JSON line
/// per present, so the benchmark can time event-to-visible latency.
#[test]
fn ack_file_records_visible_state_transitions() {
    let pipe = unique_pipe("ack");
    let ack = std::env::temp_dir().join(format!(
        "termielle-ack-{}-{}.jsonl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_file(&ack);

    let child = Command::new(app_binary())
        .args(["--pipe"])
        .arg(&pipe)
        .arg("--ack-file")
        .arg(&ack)
        .spawn()
        .expect("app starts");

    wait_for_ack_state(&ack, "idle", 15);
    assert!(
        emit_prompt_submitted(&pipe, "cli-ack"),
        "emitter must deliver the event"
    );
    wait_for_ack_state(&ack, "thinking", 15);

    let mut child = child;
    let _ = child.kill();
    let _ = child.wait();

    let lines = fs::read_to_string(&ack).expect("ack file readable");
    let first = lines.lines().next().expect("first ack line");
    assert!(
        first.starts_with(r#"{"state":"idle","ts":"#),
        "ack line must be JSON with state and ts: {first}"
    );
    let _ = fs::remove_file(&ack);
}

//! Transport contract tests, run against whichever backend the platform
//! provides: the Windows named pipe or the Unix domain socket. The contract is
//! identical: one connection carries one bounded, newline-terminated line and
//! is then torn down.

use std::io::Write;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use termielle_ipc::{EventClient, EventServer, IpcError, test_endpoint};

/// The protocol's per-message ceiling, restated so the transport tests do not
/// depend on the core crate.
const MAX_EVENT_BYTES: usize = 4096;

#[test]
fn sends_exactly_one_bounded_event_to_the_server() {
    let name = test_endpoint("roundtrip");
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let server_name = name.clone();
    let server = std::thread::spawn(move || {
        let server = EventServer::bind(&server_name).unwrap();
        ready_tx.send(()).unwrap();
        server.receive_one().unwrap()
    });
    ready_rx.recv().unwrap();

    let line = b"{\"version\":1,\"source\":\"codex\",\"session_id\":\"one\",\"event\":\"session_started\",\"timestamp_ms\":1}\n";
    EventClient::new(&name, std::time::Duration::from_millis(20))
        .send(line)
        .unwrap();
    assert_eq!(server.join().unwrap(), line);
}

#[test]
fn reports_a_missing_endpoint_without_blocking_past_the_timeout() {
    let name = test_endpoint("absent");
    let started = Instant::now();

    let result = EventClient::new(&name, Duration::from_millis(20)).send(b"{}\n");

    assert!(matches!(result, Err(IpcError::NotFound)), "got {result:?}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a missing endpoint must fail open quickly, took {:?}",
        started.elapsed()
    );
}

#[test]
fn rejects_an_oversized_line_before_touching_the_endpoint() {
    // The endpoint is never created, so a client that opened first would
    // report `NotFound`. Seeing `TooLarge` proves the size check runs before
    // the open.
    let name = test_endpoint("oversize-client");
    let line = vec![b'x'; MAX_EVENT_BYTES + 1];

    let result = EventClient::new(&name, Duration::from_millis(20)).send(&line);

    assert!(matches!(result, Err(IpcError::TooLarge)), "got {result:?}");
}

#[test]
fn accepts_a_line_at_exactly_the_protocol_limit() {
    let name = test_endpoint("limit");
    let (ready_tx, ready_rx) = mpsc::channel();
    let server_name = name.clone();
    let server = thread::spawn(move || {
        let server = EventServer::bind(&server_name).unwrap();
        ready_tx.send(()).unwrap();
        server.receive_one().unwrap()
    });
    ready_rx.recv().unwrap();

    let mut line = vec![b'x'; MAX_EVENT_BYTES - 1];
    line.push(b'\n');
    EventClient::new(&name, Duration::from_millis(20))
        .send(&line)
        .unwrap();

    assert_eq!(server.join().unwrap(), line);
}

#[test]
fn serves_two_sequential_clients() {
    let name = test_endpoint("sequential");
    let (ready_tx, ready_rx) = mpsc::channel();
    let (line_tx, line_rx) = mpsc::channel();
    let server_name = name.clone();
    let server = thread::spawn(move || {
        let server = EventServer::bind(&server_name).unwrap();
        ready_tx.send(()).unwrap();
        line_tx.send(server.receive_one().unwrap()).unwrap();
        line_tx.send(server.receive_one().unwrap()).unwrap();
    });
    ready_rx.recv().unwrap();

    let first_line = b"{\"version\":1,\"source\":\"codex\",\"session_id\":\"one\",\"event\":\"session_started\",\"timestamp_ms\":1}\n";
    let second_line = b"{\"version\":1,\"source\":\"claude\",\"session_id\":\"two\",\"event\":\"turn_completed\",\"timestamp_ms\":2}\n";

    EventClient::new(&name, Duration::from_millis(20))
        .send(first_line)
        .unwrap();
    assert_eq!(line_rx.recv().unwrap(), first_line);

    // A client that connects and closes before the server serves the
    // connection is a ghost: the server discards it and waits for the next
    // client. Retry the delivery until the server confirms receipt, then
    // verify both lines in the expected order.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        EventClient::new(&name, Duration::from_millis(20))
            .send(second_line)
            .unwrap();
        match line_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(line) if line == second_line => break,
            Ok(line) => panic!("unexpected line: {line:?}"),
            Err(_) if Instant::now() < deadline => {}
            Err(error) => panic!("second line was never delivered: {error}"),
        }
    }

    server.join().unwrap();
}

#[test]
fn rejects_a_line_that_overruns_the_limit_at_the_server() {
    let name = test_endpoint("oversize-server");
    let (ready_tx, ready_rx) = mpsc::channel();
    let server_name = name.clone();
    let server = thread::spawn(move || {
        let server = EventServer::bind(&server_name).unwrap();
        ready_tx.send(()).unwrap();
        server.receive_one()
    });
    ready_rx.recv().unwrap();

    // A hostile writer that ignores the client-side check: raw bytes, no newline
    // until well past the ceiling.
    let mut payload = vec![b'x'; MAX_EVENT_BYTES + 8];
    payload.push(b'\n');
    let mut raw = open_raw_writer(&name);
    let _ = raw.write_all(&payload);
    drop(raw);

    let received = server.join().unwrap();
    assert!(
        matches!(received, Err(IpcError::TooLarge)),
        "got {received:?}"
    );
}

/// Opens a raw writer against the endpoint through `std` alone, retrying while
/// the server is between connections: a file on the Windows pipe, a socket on
/// Unix.
fn open_raw_writer(endpoint: &str) -> Box<dyn Write + Send> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        #[cfg(windows)]
        let attempt = std::fs::OpenOptions::new().write(true).open(endpoint);
        #[cfg(unix)]
        let attempt = std::os::unix::net::UnixStream::connect(endpoint);

        match attempt {
            Ok(writer) => return Box::new(writer),
            Err(_) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("could not open {endpoint}: {error}"),
        }
    }
}

#[test]
fn delivers_while_the_server_is_blocked_waiting_for_a_connection() {
    // The overlay's steady state is a server waiting in its accept/connect
    // call. A client that gates on the endpoint being "available" in the way
    // of a busy instance would burn its whole budget there, so a hook could
    // never reach a live overlay; the client must rendezvous with a listening
    // server directly.
    let name = test_endpoint("connecting");
    let (ready_tx, ready_rx) = mpsc::channel();
    let server_name = name.clone();
    let server = thread::spawn(move || {
        let server = EventServer::bind(&server_name).unwrap();
        ready_tx.send(()).unwrap();
        server.receive_one().unwrap()
    });
    ready_rx.recv().unwrap();
    // Let the server thread actually reach its blocking accept.
    thread::sleep(Duration::from_millis(50));

    let line = b"{\"version\":1,\"source\":\"codex\",\"session_id\":\"connecting\",\"event\":\"session_started\",\"timestamp_ms\":1}\n";
    EventClient::new(&name, Duration::from_millis(20))
        .send(line)
        .unwrap();
    assert_eq!(server.join().unwrap(), line);
}

#[test]
fn refuses_to_bind_a_name_another_process_already_owns() {
    // The squatter here is a prior `bind`, which is exactly what a competing
    // overlay looks like: the endpoint exists and is owned by someone else's
    // live server.
    let name = test_endpoint("squatted");
    let squatter = EventServer::bind(&name).expect("squatter claims the name");

    // `EventServer` is not `Debug` (it owns raw handles), so reduce to the
    // error before asserting; the success case is a failure here anyway.
    let result = EventServer::bind(&name).err();

    assert_eq!(
        result,
        Some(IpcError::PipeNameOwned),
        "a squatted name must be reported distinctly"
    );
    // Held to the assertion so the name is still owned when the second bind runs.
    drop(squatter);
}

#[test]
fn binding_claims_the_name_before_the_first_receive() {
    // Before the fix, `bind` created nothing, so a client could find no
    // endpoint at all until the server reached `receive_one` — and a squatter
    // could take the name in that window. A writer must be able to connect the
    // instant `bind` returns, with no `receive_one` in sight.
    let name = test_endpoint("claimed-at-bind");
    let server = EventServer::bind(&name).expect("bind");

    // `open_raw_writer` panics when the endpoint cannot be opened: reaching it
    // at all proves bind left a connectable endpoint with no `receive_one` in
    // sight.
    let opened = open_raw_writer(&name);
    drop(opened);
    drop(server);
}

#[test]
fn an_unbounded_timeout_does_not_overflow_the_deadline_clock() {
    // `Instant::now() + Duration::MAX` panics. An emitter that panics on a
    // misconfigured timeout violates the fail-open contract, so the clamp is
    // exercised against a real send rather than asserted by inspection.
    let name = test_endpoint("saturating-timeout");
    let (ready_tx, ready_rx) = mpsc::channel();
    let server_name = name.clone();
    let server = thread::spawn(move || {
        let server = EventServer::bind(&server_name).unwrap();
        ready_tx.send(()).unwrap();
        server.receive_one().unwrap()
    });
    ready_rx.recv().unwrap();

    let line = b"{\"version\":1,\"source\":\"codex\",\"session_id\":\"max\",\"event\":\"session_started\",\"timestamp_ms\":1}\n";
    EventClient::new(&name, Duration::MAX).send(line).unwrap();

    assert_eq!(server.join().unwrap(), line);
}

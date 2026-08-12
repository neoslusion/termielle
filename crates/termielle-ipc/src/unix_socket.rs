//! Byte-stream Unix-domain socket client and server.
//!
//! This backend mirrors the Windows named-pipe contract exactly: one
//! connection carries one newline-terminated event line and is then torn down.
//! Where Windows expresses "current user only" through a DACL, Unix expresses
//! it through an owner-only socket mode, and where Windows reclaims nothing
//! (pipe names vanish with the last handle), Unix reclaims a crashed overlay's
//! leftover socket file at bind time — and only when no live listener answers
//! on it, so a squatter's live socket is never stolen.
//!
//! A stalled or hostile writer can occupy the single accept loop, exactly as
//! it can occupy the Windows server's current pipe instance; local writes are
//! buffered by the kernel, so a wedged overlay still cannot stall an agent.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use termielle_core::MAX_EVENT_BYTES;

use crate::IpcError;

/// How long a client pauses before retrying while the server is between
/// connections. Short enough to be invisible against a 20 ms budget, long
/// enough not to spin a core.
const RETRY_INTERVAL: Duration = Duration::from_millis(1);

/// Stand-in deadline for a timeout so large that `Instant + timeout` would
/// overflow. Effectively "never expires", which is the honest reading of such a
/// budget and, crucially, is not a panic: the emitter's fail-open contract
/// forbids taking down the agent over a bad configuration value.
const FAR_FUTURE: Duration = Duration::from_secs(60 * 60 * 24);

/// One byte past the ceiling, so an overlong line is detected rather than
/// silently truncated at the limit.
const READ_CEILING: usize = MAX_EVENT_BYTES + 1;

/// Owner-only socket mode: the overlay's event stream is user-scoped, the
/// socket-path analogue of the Windows owner-only DACL.
const SOCKET_MODE: u32 = 0o600;

/// Sends a single event line to a listening overlay.
///
/// The client never blocks longer than its timeout and never waits for the
/// reader to consume the bytes, so a wedged overlay cannot stall an agent.
pub struct EventClient {
    endpoint: String,
    timeout: Duration,
}

impl EventClient {
    pub fn new(endpoint: &str, timeout: Duration) -> Self {
        Self {
            endpoint: endpoint.to_owned(),
            timeout,
        }
    }

    /// Delivers `line` over one connection, then closes it.
    ///
    /// Returns [`IpcError::NotFound`] when no overlay is listening, which
    /// callers treat as a fail-open success.
    pub fn send(&self, line: &[u8]) -> Result<(), IpcError> {
        // Checked before any syscall, so an oversized line is rejected the
        // same way whether or not an overlay happens to be running.
        if line.len() > MAX_EVENT_BYTES {
            return Err(IpcError::TooLarge);
        }

        let mut stream = self.open()?;
        stream.write_all(line).map_err(|error| os_error(&error))?;

        // No explicit shutdown: dropping the stream closes the write end, and
        // the server sees EOF once it has consumed the bytes — the same
        // delivery model as the Windows pipe.
        Ok(())
    }

    /// Connects to the endpoint, retrying while the server is between
    /// connections and never exceeding the caller's budget.
    fn open(&self) -> Result<UnixStream, IpcError> {
        let now = Instant::now();
        // A `Duration` large enough to overflow the clock would panic here, and
        // an emitter that panics is exactly what the fail-open contract rules
        // out. Such a budget means "wait indefinitely", so a far-future deadline
        // is the faithful substitute.
        let deadline = now
            .checked_add(self.timeout)
            .unwrap_or_else(|| now + FAR_FUTURE);

        loop {
            let expired = deadline.saturating_duration_since(Instant::now()).is_zero();

            match UnixStream::connect(&self.endpoint) {
                Ok(stream) => return Ok(stream),
                // A missing or dead socket file: the overlay is between
                // connections or not running at all. Yield rather than spin,
                // and keep retrying only within the budget.
                Err(error) if is_absent(&error) && !expired => {
                    std::thread::sleep(RETRY_INTERVAL);
                }
                Err(error) => return Err(classify_connect(&error)),
            }
        }
    }
}

/// Whether the connect failure means "nothing is listening": the socket file
/// is gone, or it is a stale leftover of a crashed overlay.
fn is_absent(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
    )
}

/// Classifies a connect failure for the caller. Absent endpoints are
/// [`IpcError::NotFound`], the same fail-open signal the Windows backend
/// produces for `ERROR_FILE_NOT_FOUND`; everything else carries its errno.
fn classify_connect(error: &std::io::Error) -> IpcError {
    if is_absent(error) {
        IpcError::NotFound
    } else {
        os_error(error)
    }
}

/// Listens for single-event connections on an owner-only socket.
pub struct EventServer {
    listener: UnixListener,
    /// The bound socket file. A Unix socket does not vanish with its handle
    /// the way a named pipe does, so `Drop` removes it after a clean exit;
    /// `bind` reclaims the leftover of a crashed overlay.
    endpoint: PathBuf,
}

impl EventServer {
    /// Claims `endpoint` and prepares to listen on it.
    ///
    /// The claim mirrors the Windows `FILE_FLAG_FIRST_PIPE_INSTANCE` contract:
    /// a name some live process already holds must fail here rather than let
    /// both processes receive events. The liveness check is a connect attempt
    /// — a live listener answers into its backlog, a crashed overlay's leftover
    /// file refuses. Only the dead file is unlinked and reclaimed.
    pub fn bind(endpoint: &str) -> Result<Self, IpcError> {
        let path = Path::new(endpoint);

        match fs::symlink_metadata(path) {
            Ok(_) => match UnixStream::connect(endpoint) {
                Ok(_) => return Err(IpcError::PipeNameOwned),
                Err(_) => {
                    // Stale leftover: reclaim it, then bind below.
                    let _ = fs::remove_file(path);
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(os_error(&error)),
        }

        let listener = UnixListener::bind(path).map_err(|error| os_error(&error))?;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(SOCKET_MODE));
        Ok(Self {
            listener,
            endpoint: path.to_path_buf(),
        })
    }

    /// Accepts one connection and returns the bytes it carried.
    ///
    /// The returned buffer is the line exactly as sent, including its trailing
    /// newline when the writer supplied one. Anything longer than
    /// [`MAX_EVENT_BYTES`] is rejected as [`IpcError::TooLarge`] without being
    /// returned to the caller. A connection that closes without writing a byte
    /// is a ghost: it is discarded and the server keeps listening, mirroring
    /// the Windows backend's `ERROR_NO_DATA` recovery.
    pub fn receive_one(&self) -> Result<Vec<u8>, IpcError> {
        loop {
            let (mut stream, _) = self.listener.accept().map_err(|error| os_error(&error))?;
            let line = read_line(&mut stream)?;
            if line.is_empty() {
                // The client connected and closed before writing; the kernel
                // never delivered anything, so there is nothing to report.
                continue;
            }
            return Ok(line);
        }
    }
}

impl Drop for EventServer {
    fn drop(&mut self) {
        // Only removes the file if it still exists; `bind`'s liveness check
        // prevents this from ever racing a live squatter, and a clean exit is
        // the common case where the file is ours.
        let _ = fs::remove_file(&self.endpoint);
    }
}

/// Reads until a newline, EOF, or one byte past the protocol ceiling.
fn read_line(stream: &mut UnixStream) -> Result<Vec<u8>, IpcError> {
    let mut line: Vec<u8> = Vec::with_capacity(READ_CEILING);
    let mut chunk = [0u8; 256];

    loop {
        let read = stream.read(&mut chunk).map_err(|error| os_error(&error))?;

        if read == 0 {
            break;
        }

        let received = &chunk[..read];
        let boundary = received
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1);

        // Bytes after the newline belong to no message on a one-event-per-
        // connection socket, so they are dropped rather than merged into it.
        line.extend_from_slice(&received[..boundary.unwrap_or(received.len())]);

        if line.len() > MAX_EVENT_BYTES {
            return Err(IpcError::TooLarge);
        }

        if boundary.is_some() {
            break;
        }
    }

    Ok(line)
}

/// Carries the raw errno, falling back to a nonzero code when the OS failed
/// to record one.
fn os_error(error: &std::io::Error) -> IpcError {
    IpcError::Os(error.raw_os_error().unwrap_or(1) as u32)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;

    /// The protocol's per-message ceiling, restated so the transport tests do
    /// not depend on the core crate.
    const MAX_EVENT_BYTES: usize = 4096;

    /// A unique socket path under the system temp directory.
    fn unique_endpoint(label: &str) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir()
            .join(format!(
                "termielle-unix-{}-{}-{}.sock",
                std::process::id(),
                label,
                NEXT.fetch_add(1, Ordering::Relaxed),
            ))
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn sends_exactly_one_bounded_event_to_the_server() {
        let name = unique_endpoint("roundtrip");
        let (ready_tx, ready_rx) = mpsc::channel();
        let server_name = name.clone();
        let server = std::thread::spawn(move || {
            let server = EventServer::bind(&server_name).unwrap();
            ready_tx.send(()).unwrap();
            server.receive_one().unwrap()
        });
        ready_rx.recv().unwrap();

        let line = b"{\"version\":1,\"source\":\"codex\",\"session_id\":\"one\",\"event\":\"session_started\",\"timestamp_ms\":1}\n";
        EventClient::new(&name, Duration::from_millis(20))
            .send(line)
            .unwrap();
        assert_eq!(server.join().unwrap(), line);
    }

    #[test]
    fn reports_a_missing_socket_without_blocking_past_the_timeout() {
        let name = unique_endpoint("absent");
        let started = Instant::now();

        let result = EventClient::new(&name, Duration::from_millis(20)).send(b"{}\n");

        assert!(matches!(result, Err(IpcError::NotFound)), "got {result:?}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "a missing socket must fail open quickly, took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_stale_socket_file_is_reported_as_not_found() {
        // A crashed overlay leaves its socket file behind. Connecting to it
        // must look like "no overlay" to the fail-open emitter, not like an
        // errno the caller has never heard of.
        let name = unique_endpoint("stale");
        // `std`'s listener does not unlink the file on drop, which is exactly
        // what a crash leaves behind: the file, with nobody listening.
        let dead = std::os::unix::net::UnixListener::bind(&name).expect("bind");
        drop(dead);

        let result = EventClient::new(&name, Duration::from_millis(20)).send(b"{}\n");
        assert!(matches!(result, Err(IpcError::NotFound)), "got {result:?}");
    }

    #[test]
    fn rejects_an_oversized_line_before_touching_the_socket() {
        let name = unique_endpoint("oversize-client");
        let line = vec![b'x'; MAX_EVENT_BYTES + 1];

        let result = EventClient::new(&name, Duration::from_millis(20)).send(&line);

        assert!(matches!(result, Err(IpcError::TooLarge)), "got {result:?}");
    }

    #[test]
    fn accepts_a_line_at_exactly_the_protocol_limit() {
        let name = unique_endpoint("limit");
        let (ready_tx, ready_rx) = mpsc::channel();
        let server_name = name.clone();
        let server = std::thread::spawn(move || {
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
        let name = unique_endpoint("sequential");
        let (ready_tx, ready_rx) = mpsc::channel();
        let (line_tx, line_rx) = mpsc::channel();
        let server_name = name.clone();
        let server = std::thread::spawn(move || {
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

        // A client that connects and closes before the server accepts it is a
        // ghost: the server discards it and keeps listening. Retry the second
        // delivery until the server confirms receipt.
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
    fn a_ghost_connection_never_reaches_the_caller() {
        // The overlay must not report an empty line for a client that opened
        // and closed without writing; receive_one has to skip the ghost and
        // deliver the next real event.
        let name = unique_endpoint("ghost");
        let (ready_tx, ready_rx) = mpsc::channel();
        let server_name = name.clone();
        let server = std::thread::spawn(move || {
            let server = EventServer::bind(&server_name).unwrap();
            ready_tx.send(()).unwrap();
            server.receive_one().unwrap()
        });
        ready_rx.recv().unwrap();

        let ghost_name = name.clone();
        std::thread::spawn(move || {
            let _ = UnixStream::connect(&ghost_name);
        });
        std::thread::sleep(Duration::from_millis(50));

        let line = b"{\"version\":1,\"source\":\"codex\",\"session_id\":\"after-ghost\",\"event\":\"session_started\",\"timestamp_ms\":1}\n";
        EventClient::new(&name, Duration::from_millis(20))
            .send(line)
            .unwrap();

        assert_eq!(server.join().unwrap(), line);
    }

    #[test]
    fn rejects_a_line_that_overruns_the_limit_at_the_server() {
        let name = unique_endpoint("oversize-server");
        let (ready_tx, ready_rx) = mpsc::channel();
        let server_name = name.clone();
        let server = std::thread::spawn(move || {
            let server = EventServer::bind(&server_name).unwrap();
            ready_tx.send(()).unwrap();
            server.receive_one()
        });
        ready_rx.recv().unwrap();

        // A hostile writer that ignores the client-side check: raw bytes, no
        // newline until well past the ceiling.
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

    #[test]
    fn refuses_to_bind_a_name_a_live_process_already_owns() {
        // The squatter here is a prior `bind`, which is exactly what a
        // competing overlay looks like: a live listener on the same endpoint.
        let name = unique_endpoint("squatted");
        let squatter = EventServer::bind(&name).expect("squatter claims the name");

        let result = EventServer::bind(&name).err();

        assert_eq!(
            result,
            Some(IpcError::PipeNameOwned),
            "a squatted name must be reported distinctly"
        );
        // Held to the assertion so the name is still owned when the second
        // bind runs.
        drop(squatter);
    }

    #[test]
    fn reclaims_the_socket_file_of_a_crashed_overlay() {
        // A crashed overlay leaves its socket file behind with no listener.
        // `bind` must reclaim it instead of reporting the name as owned.
        let name = unique_endpoint("reclaimed");
        let path = Path::new(&name);
        // `std`'s listener does not unlink the file on drop, simulating the
        // crash leftover exactly.
        let crashed = std::os::unix::net::UnixListener::bind(&name).expect("bind");
        drop(crashed);
        assert!(
            path.exists(),
            "socket file must linger like a crash leftover"
        );

        let rebound = EventServer::bind(&name).expect("reclaim stale socket");
        assert!(path.exists());
        drop(rebound);
    }

    #[test]
    fn the_socket_file_carries_an_owner_only_mode() {
        let name = unique_endpoint("mode");
        let server = EventServer::bind(&name).expect("bind");

        let mode = fs::metadata(&name)
            .expect("socket metadata")
            .permissions()
            .mode();

        assert_eq!(
            mode & 0o777,
            SOCKET_MODE,
            "the socket must be owner-only, got mode {mode:#o}"
        );
        drop(server);
    }

    #[test]
    fn an_unbounded_timeout_does_not_overflow_the_deadline_clock() {
        // `Instant::now() + Duration::MAX` panics. An emitter that panics on a
        // misconfigured timeout violates the fail-open contract, so the clamp
        // is exercised against a real send rather than asserted by inspection.
        let name = unique_endpoint("saturating-timeout");
        let (ready_tx, ready_rx) = mpsc::channel();
        let server_name = name.clone();
        let server = std::thread::spawn(move || {
            let server = EventServer::bind(&server_name).unwrap();
            ready_tx.send(()).unwrap();
            server.receive_one().unwrap()
        });
        ready_rx.recv().unwrap();

        let line = b"{\"version\":1,\"source\":\"codex\",\"session_id\":\"max\",\"event\":\"session_started\",\"timestamp_ms\":1}\n";
        EventClient::new(&name, Duration::MAX).send(line).unwrap();

        assert_eq!(server.join().unwrap(), line);
    }

    /// Opens a raw writer against the endpoint through `std` alone, retrying
    /// while the server is between connections.
    fn open_raw_writer(endpoint: &str) -> UnixStream {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match UnixStream::connect(endpoint) {
                Ok(stream) => return stream,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("could not open {endpoint}: {error}"),
            }
        }
    }
}

//! Bounded, current-user-only transport for agent lifecycle events.
//!
//! One connection carries exactly one newline-terminated event line of at most
//! [`MAX_EVENT_BYTES`] bytes and is then closed. The transport never retains,
//! parses, or logs the payload; decoding is the caller's job.
//!
//! The transport is platform-abstracted. Windows uses an owner-only named pipe
//! (`\\.\pipe\termielle-v1`); Unix-like systems use an owner-only Unix domain
//! socket in the user's runtime or data directory. Both backends expose the
//! same [`EventClient`] and [`EventServer`] API and honor the same
//! one-connection-per-event contract, so the emitter and the overlay never
//! mention a transport.

use std::sync::atomic::{AtomicU64, Ordering};

use termielle_core::MAX_EVENT_BYTES;

#[cfg(windows)]
mod security;
#[cfg(unix)]
mod unix_socket;
#[cfg(windows)]
mod windows_pipe;

#[cfg(unix)]
pub use unix_socket::{EventClient, EventServer};
#[cfg(windows)]
pub use windows_pipe::{EventClient, EventServer};

/// The endpoint the overlay listens on when nothing is configured.
///
/// Windows keeps the historical constant so the app and diagnostics can
/// compare against it; Unix derives a per-user socket path from the
/// environment, so callers use [`default_endpoint`].
#[cfg(windows)]
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\termielle-v1";

/// The default endpoint the overlay listens on, as a pipe name on Windows and
/// a socket path on Unix.
///
/// On Unix the socket lives under `$XDG_RUNTIME_DIR` when that is set, else
/// under the user's data directory, else in the system temp directory as a
/// last resort. Every choice is per-user so a multi-user machine never mixes
/// overlays.
pub fn default_endpoint() -> String {
    #[cfg(windows)]
    {
        DEFAULT_PIPE_NAME.to_owned()
    }
    #[cfg(not(windows))]
    {
        if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
            return std::path::Path::new(&runtime)
                .join("termielle-v1.sock")
                .to_string_lossy()
                .into_owned();
        }
        if let Some(home) = std::env::var_os("HOME") {
            return std::path::Path::new(&home)
                .join(".local")
                .join("share")
                .join("termielle")
                .join("termielle-v1.sock")
                .to_string_lossy()
                .into_owned();
        }
        std::env::temp_dir()
            .join("termielle-v1.sock")
            .to_string_lossy()
            .into_owned()
    }
}

/// A unique, per-process test endpoint: a named pipe on Windows, a socket in
/// the system temp directory on Unix. Two calls never collide, and no test can
/// reach a developer's live overlay. For diagnostics harnesses and integration
/// tests only; not part of the stable API.
#[doc(hidden)]
pub fn test_endpoint(label: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let ticket = NEXT.fetch_add(1, Ordering::Relaxed);

    #[cfg(windows)]
    {
        format!(
            r"\\.\pipe\termielle-test-{}-{}-{}",
            std::process::id(),
            label,
            ticket
        )
    }
    #[cfg(not(windows))]
    {
        std::env::temp_dir()
            .join(format!(
                "termielle-test-{}-{}-{}.sock",
                std::process::id(),
                label,
                ticket
            ))
            .to_string_lossy()
            .into_owned()
    }
}

/// Why an event could not be delivered or received.
///
/// Every variant is a fixed classification or a numeric OS code. There is no
/// variant carrying a path, a payload, or any other caller data, so an error
/// can be logged verbatim without leaking what the agent was doing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum IpcError {
    /// No endpoint by that name existed within the caller's budget: the
    /// overlay is not running. Emitters treat this as success and exit
    /// quietly.
    #[error("no overlay transport is listening")]
    NotFound,
    /// The endpoint exists but every instance was already taken.
    #[error("every overlay instance is busy")]
    Busy,
    /// Another process already holds the endpoint name, so this process cannot
    /// be the overlay. Surfaced by `EventServer::bind`, which claims the name
    /// up front precisely so a squatter is rejected at startup rather than
    /// silently receiving events.
    #[error("the endpoint name is owned by another process")]
    PipeNameOwned,
    /// The endpoint exists but did not free up within the caller's budget.
    #[error("the overlay transport did not become available in time")]
    Timeout,
    /// The line is longer than the protocol allows and was never transmitted.
    #[error("event line exceeds {MAX_EVENT_BYTES} bytes")]
    TooLarge,
    /// Any other OS failure, carrying only its numeric code: a Win32 error on
    /// Windows, an errno on Unix.
    #[error("transport operation failed with OS error {0}")]
    Os(u32),
}

/// Recovers the Win32 code from a `windows` error so it can be classified.
///
/// `HRESULT::from_win32` maps a DWORD `code` to `0x8007_0000 | code`; this is
/// the inverse for facility-Win32 results and a passthrough for anything else.
#[cfg(windows)]
pub(crate) fn win32_code(error: &windows::core::Error) -> u32 {
    let hresult = error.code().0 as u32;
    if hresult & 0xFFFF_0000 == 0x8007_0000 {
        hresult & 0x0000_FFFF
    } else {
        hresult
    }
}

/// Classifies a `windows` error that has no more specific handling.
#[cfg(windows)]
pub(crate) fn from_win32(error: windows::core::Error) -> IpcError {
    IpcError::Os(win32_code(&error))
}

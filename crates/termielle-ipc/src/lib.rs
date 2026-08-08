//! Bounded, current-user-only transport for agent lifecycle events.
//!
//! One connection carries exactly one newline-terminated event line of at most
//! [`MAX_EVENT_BYTES`] bytes and is then closed. The transport never retains,
//! parses, or logs the payload; decoding is the caller's job.

use termielle_core::MAX_EVENT_BYTES;

#[cfg(windows)]
mod security;
#[cfg(windows)]
mod windows_pipe;

#[cfg(windows)]
pub use windows_pipe::{PipeClient, PipeServer};

/// The pipe the overlay listens on.
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\termielle-v1";

/// Why an event could not be delivered or received.
///
/// Every variant is a fixed classification or a numeric Win32 code. There is no
/// variant carrying a path, a payload, or any other caller data, so an error can
/// be logged verbatim without leaking what the agent was doing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum IpcError {
    /// No pipe by that name existed within the caller's budget: the overlay is
    /// not running. Emitters treat this as success and exit quietly.
    #[error("no overlay pipe is listening")]
    NotFound,
    /// The pipe exists but every instance was already taken.
    #[error("every overlay pipe instance is busy")]
    Busy,
    /// Another process already holds the pipe name, so this process cannot be
    /// the overlay. Surfaced by `PipeServer::bind`, which claims the name up
    /// front precisely so a squatter is rejected at startup rather than
    /// silently receiving events.
    #[error("pipe name is owned by another process")]
    PipeNameOwned,
    /// The pipe exists but did not free up within the caller's budget.
    #[error("the overlay pipe did not become available in time")]
    Timeout,
    /// The line is longer than the protocol allows and was never transmitted.
    #[error("event line exceeds {MAX_EVENT_BYTES} bytes")]
    TooLarge,
    /// Any other Win32 failure, carrying only its numeric code.
    #[error("pipe operation failed with Win32 error {0}")]
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

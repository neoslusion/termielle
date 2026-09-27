//! Bounded, append-only journal of the events the overlay accepted.
//!
//! The overlay is a pure fold over its event stream: a session's visual state
//! is a deterministic function of the events it has seen. Persisting those
//! events lets a restart — the tray's Restart, the crash watchdog — rebuild
//! the exact same state instead of starting over at Idle, and the reducer's
//! own staleness rules retire anything the journal outlives.
//!
//! The journal is a JSON-lines file of [`EventMessage`]s in arrival order. It
//! is bounded: once the file exceeds [`EventLog::max_bytes`], the oldest lines
//! are dropped so the disk footprint stays constant. Replay never fails: a
//! missing file, a torn tail from a crash mid-append, or a malformed line all
//! read as "nothing there" rather than an error, because the fold handles
//! anything the journal could legitimately contain.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::protocol::{EventMessage, decode_event_line, encode_event_line};

/// The default ceiling for the journal file. A few thousand events are far
/// beyond what any session's visual state can still be influenced by: the
/// reducer drops a session four hours after its last event, and the busy-stall
/// decay usually retires it long before that.
pub const DEFAULT_EVENT_LOG_MAX_BYTES: usize = 1_048_576;

#[cfg(windows)]
fn replace_atomically(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }
    let source = wide(temporary);
    let target = wide(destination);
    // SAFETY: both buffers are NUL-terminated and outlive the call.
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(std::io::Error::from)
}

#[cfg(not(windows))]
fn replace_atomically(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(temporary, destination)
}

/// Appends accepted events to one bounded JSON-lines file and replays it.
#[derive(Debug)]
pub struct EventLog {
    path: PathBuf,
    max_bytes: usize,
}

impl EventLog {
    /// A journal at `path` capped at `max_bytes`. Nothing touches the disk
    /// until the first [`EventLog::append`].
    pub fn new(path: PathBuf, max_bytes: usize) -> Self {
        Self { path, max_bytes }
    }

    /// Appends one accepted event in canonical wire form, then drops the
    /// oldest lines if the file has outgrown its budget.
    ///
    /// The append is opened per call so a crash never leaves the journal
    /// locked, and the canonical wire form means a later replay decodes with
    /// the same validation the live path applies.
    pub fn append(&self, event: &EventMessage) -> Result<(), EventLogError> {
        // `encode_event_line` already terminates the line; the journal stores
        // the same canonical wire form the pipe carries.
        let line = encode_event_line(event)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(&line)?;
        writer.flush()?;
        writer.get_ref().sync_data()?;
        drop(writer);

        let _ = self.trim();
        Ok(())
    }

    /// Every persisted event in arrival order.
    ///
    /// Lines that do not decode — a torn tail from a crash mid-append, or
    /// bytes from a newer protocol version — are skipped, so replaying an
    /// interrupted journal still recovers the state the readable prefix
    /// implies.
    pub fn read_all(&self) -> Vec<EventMessage> {
        let file = match fs::File::open(&self.path) {
            Ok(file) => file,
            Err(_) => return Vec::new(),
        };

        BufReader::new(file)
            .lines()
            .filter_map(|line| {
                let line = line.ok()?;
                decode_event_line(line.as_bytes()).ok()
            })
            .collect()
    }

    /// Drops the oldest lines until the file fits under `max_bytes`, rewriting
    /// atomically so a reader never observes a half-trimmed journal. Best
    /// effort: a failed trim is a housekeeping miss, not a journal failure.
    fn trim(&self) -> Result<(), EventLogError> {
        let size = fs::metadata(&self.path)?.len();
        if size <= self.max_bytes as u64 {
            return Ok(());
        }

        let events = self.read_all();
        let mut lines: Vec<Vec<u8>> = events
            .iter()
            .filter_map(|event| encode_event_line(event).ok())
            .collect();

        // Keep the newest lines that fit; the newest line always fits in
        // practice (lines are at most MAX_EVENT_BYTES and the budget is far
        // larger), so the journal never trims down to nothing.
        let mut kept_bytes = 0usize;
        let mut kept_count = 0usize;
        for line in lines.iter().rev() {
            if kept_bytes + line.len() > self.max_bytes && kept_count > 0 {
                break;
            }
            kept_bytes += line.len();
            kept_count += 1;
        }
        lines.drain(..lines.len() - kept_count);

        let temporary = self.path.with_extension("log.trimming");
        let result = (|| -> Result<(), EventLogError> {
            {
                let mut writer = BufWriter::new(fs::File::create(&temporary)?);
                for line in &lines {
                    writer.write_all(line)?;
                }
                writer.flush()?;
            }
            replace_atomically(&temporary, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            // A failed trim must not leak its scratch file.
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

/// Why an event could not be journaled or trimmed.
#[derive(Debug, thiserror::Error)]
pub enum EventLogError {
    #[error("event encoding failed: {0}")]
    Protocol(String),
    #[error("journal I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

impl From<crate::protocol::ProtocolError> for EventLogError {
    fn from(error: crate::protocol::ProtocolError) -> Self {
        Self::Protocol(error.to_string())
    }
}

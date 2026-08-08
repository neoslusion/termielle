use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Severity of a bounded-log record.
#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Warning,
    Error,
}

/// Which subsystem produced a record.
#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogComponent {
    Config,
    Ipc,
    Animation,
    Window,
}

/// What happened, drawn from a fixed vocabulary.
#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogEvent {
    ReadFailed,
    DecodeFailed,
    PresentFailed,
    InvalidEvent,
}

/// One log line.
///
/// Every field is an enum or a number on purpose: there is no place to put a
/// path, session identifier, prompt, or payload, so none can leak.
#[derive(serde::Serialize)]
pub struct LogRecord {
    pub timestamp_ms: u64,
    pub level: LogLevel,
    pub component: LogComponent,
    pub event: LogEvent,
    pub error_code: i32,
}

/// A size-capped JSON-lines log with a fixed number of rotated backups.
///
/// Disk use is bounded by `(backups + 1)` files of roughly `max_bytes` each.
pub struct BoundedLog {
    path: PathBuf,
    max_bytes: u64,
    backups: u8,
}

impl BoundedLog {
    pub fn new(path: PathBuf, max_bytes: u64, backups: u8) -> Self {
        Self {
            path,
            max_bytes,
            backups,
        }
    }

    /// Appends one record, rotating first when it would push the file past the
    /// limit.
    ///
    /// # Concurrency
    ///
    /// This method is not synchronized. Concurrent calls from multiple threads may
    /// interleave rotation renames and lose or duplicate a backup file. If logging
    /// from more than one thread, wrap `BoundedLog` in a `Mutex`.
    pub fn write(&self, record: &LogRecord) -> std::io::Result<()> {
        let mut line = serde_json::to_vec(record).map_err(std::io::Error::other)?;
        line.push(b'\n');

        if let Some(directory) = self.path.parent() {
            fs::create_dir_all(directory)?;
        }

        let current_bytes = fs::metadata(&self.path).map(|meta| meta.len()).unwrap_or(0);
        if current_bytes > 0 && current_bytes + line.len() as u64 > self.max_bytes {
            self.rotate()?;
        }

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(&line)
    }

    /// Shifts `termielle.log` into `termielle.log.1`, each backup one place
    /// older, and drops whatever fell off the end.
    fn rotate(&self) -> std::io::Result<()> {
        if self.backups == 0 {
            return fs::remove_file(&self.path);
        }

        let oldest = self.backup_path(self.backups);
        if oldest.is_file() {
            fs::remove_file(&oldest)?;
        }

        for index in (1..self.backups).rev() {
            let from = self.backup_path(index);
            if from.is_file() {
                fs::rename(&from, self.backup_path(index + 1))?;
            }
        }

        fs::rename(&self.path, self.backup_path(1))
    }

    fn backup_path(&self, index: u8) -> PathBuf {
        let name = self
            .path
            .file_name()
            .unwrap_or_else(|| Path::new("termielle.log").as_os_str());
        let mut name = name.to_os_string();
        name.push(format!(".{index}"));
        self.path.with_file_name(name)
    }
}

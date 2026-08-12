use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::reducer::VisualState;

/// Overlay scale bounds.
const MIN_SCALE: f32 = 0.5;
const MAX_SCALE: f32 = 2.0;
const DEFAULT_SCALE: f32 = 1.0;

/// Ready-state hold bounds, in milliseconds.
const MIN_READY_HOLD_MS: u64 = 1_000;
const MAX_READY_HOLD_MS: u64 = 30_000;
const DEFAULT_READY_HOLD_MS: u64 = 5_000;

/// Fixed animation playback rate bounds, in frames per second.
const MIN_FRAME_RATE: u32 = 1;
const MAX_FRAME_RATE: u32 = 240;

/// Busy-state stall bounds: how long a Thinking or Working session may stay
/// silent before the overlay returns to Idle, in milliseconds. The lower bound
/// keeps a briefly paused agent from flickering out; the upper bound stays
/// well under the four-hour stale window that drops session records.
const MIN_BUSY_STALL_MS: u64 = 60_000;
const MAX_BUSY_STALL_MS: u64 = 3_600_000;
const DEFAULT_BUSY_STALL_MS: u64 = 300_000;

/// Whether the user overrides the system reduced-motion preference.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReducedMotion {
    System,
    On,
    Off,
}

/// How the overlay composites its layered window.
///
/// `PerPixel` uses `UpdateLayeredWindow` with per-pixel alpha and is the
/// primary path. `ColorKey` paints normally and asks the compositor to make
/// one key color transparent; it is the fallback for drivers whose DIB
/// redirection renders black instead of the frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderMode {
    PerPixel,
    ColorKey,
}

/// Last overlay placement, in logical coordinates on a named monitor.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WindowPosition {
    pub monitor: String,
    pub x_logical: i32,
    pub y_logical: i32,
}

/// Version 1 persisted settings.
///
/// Missing fields take their default; out-of-range numbers clamp to the nearer
/// bound, so one bad value never discards the rest of the file.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub scale: f32,
    pub always_on_top: bool,
    pub reduced_motion: ReducedMotion,
    pub ready_hold_ms: u64,
    /// How long a Thinking or Working session may go silent before the overlay
    /// returns to Idle, in milliseconds. Ends a conversation that never sent a
    /// completion event without waiting for the four-hour stale window.
    pub busy_stall_ms: u64,
    pub position: Option<WindowPosition>,
    pub render: RenderMode,
    /// Fixed animation playback rate in frames per second. When set, the
    /// overlay presents animation frames at this rate instead of the GIF's
    /// own delays; the loop duration becomes frame count / rate.
    pub frame_rate: Option<u32>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            scale: DEFAULT_SCALE,
            always_on_top: true,
            reduced_motion: ReducedMotion::System,
            ready_hold_ms: DEFAULT_READY_HOLD_MS,
            busy_stall_ms: DEFAULT_BUSY_STALL_MS,
            position: None,
            render: RenderMode::PerPixel,
            frame_rate: None,
        }
    }
}

impl AppConfig {
    /// Parses configuration bytes, clamping out-of-range values.
    ///
    /// Returns [`ConfigError::Malformed`] when the bytes are not a configuration
    /// object at all; individual values never fail, they clamp.
    pub fn from_json(input: &[u8]) -> Result<Self, ConfigError> {
        let mut config: Self = serde_json::from_slice(input)
            .map_err(|error| ConfigError::Malformed(error.to_string()))?;
        config.clamp_fields();
        Ok(config)
    }

    fn clamp_fields(&mut self) {
        // `clamp` maps infinities onto the nearer bound but propagates NaN, so
        // NaN is the one scale that has to fall back instead.
        self.scale = if self.scale.is_nan() {
            DEFAULT_SCALE
        } else {
            self.scale.clamp(MIN_SCALE, MAX_SCALE)
        };
        self.ready_hold_ms = self
            .ready_hold_ms
            .clamp(MIN_READY_HOLD_MS, MAX_READY_HOLD_MS);
        self.busy_stall_ms = self
            .busy_stall_ms
            .clamp(MIN_BUSY_STALL_MS, MAX_BUSY_STALL_MS);
        self.frame_rate = self
            .frame_rate
            .map(|rate| rate.clamp(MIN_FRAME_RATE, MAX_FRAME_RATE));
    }
}

/// Why a configuration read or write could not be completed.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("malformed configuration: {0}")]
    Malformed(String),
    #[error("configuration I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Reads configuration from `path`.
///
/// A missing or malformed file yields [`AppConfig::default`]; any other read
/// failure is reported as [`ConfigError::Io`].
pub fn load_config(path: &Path) -> Result<AppConfig, ConfigError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AppConfig::default());
        }
        Err(error) => return Err(ConfigError::Io(error)),
    };

    match AppConfig::from_json(&bytes) {
        Ok(config) => Ok(config),
        Err(ConfigError::Malformed(_)) => Ok(AppConfig::default()),
        Err(error) => Err(error),
    }
}

/// Writes configuration to `path` so readers never observe a partial file.
///
/// The bytes land in a same-directory temporary file that is flushed to disk
/// before it replaces `path`. The temporary file is removed if anything fails.
/// Values are clamped on the way out, so a saved file always reloads unchanged.
pub fn save_config_atomic(path: &Path, config: &AppConfig) -> Result<(), ConfigError> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory)?;

    let mut clamped = config.clone();
    clamped.clamp_fields();

    let temporary = temporary_path(path);
    match write_and_replace(&temporary, path, &clamped) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error)
        }
    }
}

fn write_and_replace(
    temporary: &Path,
    destination: &Path,
    config: &AppConfig,
) -> Result<(), ConfigError> {
    let mut bytes = serde_json::to_vec_pretty(config)
        .map_err(|error| ConfigError::Malformed(error.to_string()))?;
    bytes.push(b'\n');

    let mut file = fs::File::create(temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);

    replace_atomically(temporary, destination)?;
    Ok(())
}

/// A unique same-directory scratch name, so concurrent saves cannot collide.
fn temporary_path(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ticket = COUNTER.fetch_add(1, Ordering::Relaxed);

    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.{ticket}.tmp", std::process::id()));
    path.with_file_name(name)
}

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

/// Where the overlay looks for the approved animation files.
///
/// Roots are searched in order, so a per-installation directory can shadow a
/// user directory without copying files.
#[derive(Clone, Debug)]
pub struct AssetCatalog {
    roots: Vec<PathBuf>,
}

impl AssetCatalog {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    /// Where the animation for `state` would live under the first root, or
    /// `None` when the state has no approved asset.
    pub fn expected_path_for(&self, state: VisualState) -> Option<PathBuf> {
        let name = asset_file_name(state)?;
        Some(self.roots.first()?.join(name))
    }

    /// The first existing approved animation for `state`, or `None` so the
    /// renderer falls back to its procedural frame.
    pub fn resolve(&self, state: VisualState) -> Option<PathBuf> {
        let name = asset_file_name(state)?;
        self.roots
            .iter()
            .map(|root| root.join(name))
            .find(|candidate| candidate.is_file())
    }
}

/// The approved file name for each visual state.
///
/// `Failed` has no dedicated artwork yet and is rendered procedurally.
fn asset_file_name(state: VisualState) -> Option<&'static str> {
    match state {
        VisualState::Idle => Some("standby.gif"),
        VisualState::Thinking => Some("ai_thingking.gif"),
        VisualState::Working => Some("ai_working.gif"),
        VisualState::NeedsInput => Some("user_typing.gif"),
        VisualState::Ready => Some("ai_complete_answer.gif"),
        VisualState::Failed => None,
    }
}

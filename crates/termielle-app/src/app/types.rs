//! Shared controller vocabulary: action sets, click outcomes, alert banners, tuning consts.

use termielle_core::VisualState;

/// Edge length of the cached termielle face. Larger than any display size
/// (faces show at <= 32 logical px, so 96 stays crisp through render scales
/// up to 3x), so the downsample always resolves real detail instead of blur.
pub(crate) const FACE_SIZE: u32 = 96;
/// How old an event may be while still raising an alert banner. Live pipe
/// delivery is instant, so anything older is a journal replay or a delayed
/// ghost — and a notification about a days-old prompt is not news. The
/// reducer still folds the event into state; only the banner is gated.
pub(crate) const ALERT_FRESHNESS_MS: u64 = 60_000;
/// Procedural-motion repaint cadence in ms (~20 fps): thinking bounce,
/// worker orbit, input pulse, celebration/shake one-shots, and the media
/// equalizer all ride this clock so they move even with no other deadline
/// pending. Cheap (cached glass, one present) and idle-silent.
pub(crate) const MOTION_TICK_MS: u64 = 50;
/// How many notification banners queue behind the showing one; arrivals
/// beyond that drop the longest-waiting unseen banner, never the showing one.
pub(crate) const MAX_QUEUED_ALERTS: usize = 3;

/// Transient alert banner displayed in the Dynamic Island on notifications.
/// Cache key for the frosted-glass layer: geometry, material, blob layout,
/// and the authoring scale — a DPI or zoom change misses and rebuilds
/// instead of presenting stale-resolution glass.
pub(crate) type GlassCacheKey = (u32, u32, u32, bool, bool, u32, i32, u32, u32);
/// Square edge length of procedural fallback frames.
pub const FALLBACK_FRAME_SIZE: u32 = 360;
/// What the GUI thread must do after one controller step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ControllerActions {
    /// Set when the visible state changed and its animation was reloaded.
    pub visible_state: Option<VisualState>,
    /// True when the frame changed and the window should repaint.
    pub present_frame: bool,
    /// Nearest reducer or animation deadline, for the single Win32 timer.
    pub next_deadline_ms: Option<u64>,
    /// Numeric code of an asset that failed to decode this step, if any.
    pub error_code: Option<i32>,
}
pub const HIT_MEDIA_PLAY_PAUSE: isize = -1;
pub const HIT_MEDIA_PREV: isize = -2;
pub const HIT_MEDIA_NEXT: isize = -3;
pub const HIT_ALERT_DISMISS: isize = -4;
/// What a click on the island should do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClickOutcome {
    /// Toggle play/pause on the system media session.
    MediaToggle,
    /// Skip to previous track.
    MediaPrev,
    /// Skip to next track.
    MediaNext,
    /// Dismiss notification banner.
    AlertDismiss,
    /// Activate a specific window (HWND).
    ActivateWindow(isize),
    /// Switch to a virtual desktop workspace by index.
    WorkspaceSwitch(u32),
    /// Toggle system audio mute.
    VolumeToggle,
    /// The dashboard opened.
    Expanded,
    /// The dashboard closed.
    Collapsed,
    /// Click landed outside any interactive element.
    None,
}
#[derive(Clone, Debug)]
pub struct AlertBanner {
    pub title: String,
    pub subtitle: String,
    pub accent: [u8; 4],
    pub expires_at_ms: u64,
    pub duration_ms: u64,
}

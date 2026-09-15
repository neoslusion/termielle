//! Bar shared types: metrics cache, hit targets, card geometry.

/// Cached system metrics for Waybar mode, refreshed on a 1-second cadence
/// instead of blocking the render loop on every frame.
#[derive(Clone, Debug)]
pub(crate) struct BarMetricsCache {
    pub last_query_ms: u64,
    pub workspaces: crate::bar::workspaces::WorkspaceSnapshot,
    pub window_title: String,
    pub time_str: String,
    pub battery: (Option<u8>, bool),
    pub volume: crate::bar::volume::VolumeSnapshot,
    pub memory_pct: u8,
    pub cpu_pct: u8,
}
impl BarMetricsCache {
    /// Empty snapshot for fully-hidden bars: renders nothing and performs
    /// no system queries. `last_query_ms` is zero so re-showing a module
    /// refetches immediately instead of serving stale emptiness for 900ms.
    pub(crate) fn empty() -> Self {
        Self {
            last_query_ms: 0,
            workspaces: crate::bar::workspaces::WorkspaceSnapshot {
                total: 0,
                active: 0,
            },
            window_title: String::new(),
            time_str: String::new(),
            battery: (None, false),
            volume: crate::bar::volume::VolumeSnapshot {
                level: 0,
                muted: true,
            },
            memory_pct: 0,
            cpu_pct: 0,
        }
    }
}
/// One bar-module hit target: (id, x, y, w, h) in frame coordinates.
/// Collected per zone and installed as [`Controller::icon_hits`] by the
/// center painter, which owns the island interaction state.
pub(crate) type BarHit = (isize, i32, i32, u32, u32);
/// Expanded-card geometry for the bar center painter: where the drop-down
/// card lives plus whether this frame is expanded at all.
pub(crate) struct BarCard {
    pub(crate) island_x: i32,
    pub(crate) island_y: i32,
    pub(crate) island_w: u32,
    pub(crate) exp_h: u32,
    pub(crate) expanded: bool,
}

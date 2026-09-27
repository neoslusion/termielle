//! Bar shared types: metrics cache, hit targets, card geometry.

use crate::animation::FrameBuffer;
pub(crate) use crate::bar::metrics::Snapshot as BarMetricsCache;
use std::rc::Rc;

/// One bar-module hit target: (id, x, y, w, h) in frame coordinates.
/// Collected per zone and installed as [`Controller::icon_hits`] by the
/// center painter, which owns the island interaction state.
pub(crate) type BarHit = (isize, i32, i32, u32, u32);
/// Cached transparent module layer for one bar zone. The frame is full
/// monitor width so zone compositing keeps one coordinate system; only the
/// configured side range is copied into the destination.
pub(crate) struct BarZoneCache {
    pub(crate) key: (u32, u32, f32, u32),
    pub(crate) frame: Rc<FrameBuffer>,
    pub(crate) hits: Vec<BarHit>,
}

/// Transparent breathing room between the persistent strip and its popup.
/// The popup is a separate island surface, not a continuation of the bar.
pub(crate) const BAR_POPUP_GAP: u32 = 6;
/// Expanded-card geometry for the bar center painter: where the drop-down
/// card lives plus whether this frame is expanded at all.
pub(crate) struct BarCard {
    pub(crate) content_w: u32,
    pub(crate) content_h: u32,
    /// Strip offset inside the current full frame; bottom-bar popups shift
    /// the strip down by the expanded height.
    pub(crate) bar_y: i32,
    pub(crate) progress: f32,
    pub(crate) island_x: i32,
    pub(crate) island_y: i32,
    pub(crate) island_w: u32,
    pub(crate) exp_h: u32,
    pub(crate) expanded: bool,
}

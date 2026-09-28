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
    /// Centre x of this zone's Control Center entry, when it drew one. The
    /// card reads it back instead of recomputing the right-zone layout, so the
    /// panel hangs from the icon the frame actually painted rather than from
    /// a second, independently-derived idea of where that icon is.
    pub(crate) control_center_x: Option<i32>,
}

/// Transparent breathing room between the persistent strip and its popup.
/// The popup is a separate island surface, not a continuation of the bar.
pub(crate) const BAR_POPUP_GAP: u32 = 6;
/// Logical pixels the left zone keeps clear of the center pill. The pill and
/// the right zone are fixed, so left-side content is laid out up to this
/// edge rather than painted past it and clipped.
pub(crate) const BAR_ZONE_GAP: u32 = 8;
/// Right-zone layout, all logical px. A module is a glyph and its number, so
/// these are the glyph box, the gap to the number, and the number's own
/// reserve. `VALUE_W` must fit the widest reading a percentage can take:
/// "100%" measures 31.9 px in Segoe UI Variable Text at 13 px bold, and
/// anything narrower gets the value ellipsized to "100" with a trailing …
pub(crate) const ICON: i32 = 14;
pub(crate) const ICON_GAP: i32 = 5;
pub(crate) const VALUE_W: u32 = 34;
/// One metric module: glyph, gap, number.
pub(crate) const METRIC_W: i32 = ICON + ICON_GAP + VALUE_W as i32;
/// Breathing room between adjacent right-zone modules.
pub(crate) const MODULE_GAP: i32 = 12;
/// The volume readout is a glyph alone; the wheel over it is the value.
pub(crate) const VOLUME_W: i32 = 15;
/// Battery is a number plus its glyph, like the other metrics.
pub(crate) const BATTERY_W: i32 = VALUE_W as i32 + ICON_GAP + ICON;
/// The Control Center entry is a glyph alone, like volume: a menu-bar item
/// that opens a panel rather than a readout.
pub(crate) const CONTROL_CENTER_W: i32 = 15;
/// The clock reads `HH:MM`, which is wider than a percentage.
pub(crate) const CLOCK_ICON: i32 = 13;
pub(crate) const CLOCK_TEXT_W: u32 = 38;
pub(crate) const CLOCK_W: i32 = CLOCK_ICON + ICON_GAP + CLOCK_TEXT_W as i32;
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
    /// Left edge of the full-size card content, in frame coordinates. Same
    /// anchor as `island_x`; carried separately because the card animates its
    /// width while the content inside it is already full size.
    pub(crate) content_x: i32,
    pub(crate) island_y: i32,
    pub(crate) island_w: u32,
    pub(crate) exp_h: u32,
    pub(crate) expanded: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The widest reading each reserve has to hold, measured with GDI in
    /// Segoe UI Variable Text: "100%" at 13 px bold is 31.9 px and "10:49"
    /// at 12 px bold is 29.2 px. A reserve narrower than these does not
    /// clip, it *ellipsizes*: the battery showed "10…" for a full charge.
    #[test]
    fn text_reserves_fit_the_widest_reading() {
        assert!(
            VALUE_W as f32 >= 31.9,
            "VALUE_W {VALUE_W} is narrower than a 100% reading"
        );
        assert!(
            CLOCK_TEXT_W as f32 >= 29.2,
            "CLOCK_TEXT_W {CLOCK_TEXT_W} is narrower than an HH:MM reading"
        );
    }

    /// A module's reserved width must be the sum of the parts it lays out, so
    /// the next module can never be drawn on top of this one's number.
    #[test]
    fn module_widths_are_the_sum_of_their_parts() {
        assert_eq!(METRIC_W, ICON + ICON_GAP + VALUE_W as i32);
        assert_eq!(BATTERY_W, VALUE_W as i32 + ICON_GAP + ICON);
        assert_eq!(CLOCK_W, CLOCK_ICON + ICON_GAP + CLOCK_TEXT_W as i32);
    }
}

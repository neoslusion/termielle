//! Where the launcher opens when it grows out of the bar's pill, and the shape
//! it has on the way.
//!
//! Everything here is plain geometry in physical pixels so it can be tested
//! without a window. The launcher is its own window (the native text box gives
//! it IME input for free), so the growth is a clip: a rounded region reveals the
//! window, starting as a pill-wide strip against the pill and ending as the whole
//! window, while the window itself slides from the pill to where it rests.

/// The bar's pill in screen coordinates, and which side of it the launcher
/// opens on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Anchor {
    /// Left, top, right, bottom of the pill.
    pub pill: (i32, i32, i32, i32),
    /// A top bar opens the launcher below its pill; a bottom bar opens it above.
    pub below: bool,
}

/// A rectangle as left, top, right, bottom.
pub(super) type Rect = (i32, i32, i32, i32);

/// Where the launcher window goes and where its reveal starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Placement {
    /// The window's left edge on screen.
    pub x: i32,
    /// The window's top edge while it is a strip against the pill, and once it
    /// has settled where it rests.
    pub y_start: i32,
    pub y_rest: i32,
    /// The reveal's first shape, in the window's own coordinates.
    pub start: Rect,
}

impl Placement {
    /// The window's top edge at `progress` (0 = against the pill, 1 = at rest).
    pub fn y_at(&self, progress: f32) -> i32 {
        (self.y_start as f32 + (self.y_rest - self.y_start) as f32 * progress).round() as i32
    }
}

/// The edge of the launcher that stays put once it has settled: its top edge
/// below a top bar, its bottom edge above a bottom bar.
///
/// It rests where the launcher always used to open, a quarter of the way into
/// the work area, which is well clear of the pill, but never nearer the bar than
/// the pill itself: a launcher cannot rest on the wrong side of it.
pub(super) fn rest_edge(anchor: Anchor, height: i32, work: Rect, gap: i32) -> i32 {
    let room = (work.3 - work.1 - height).max(0);
    if anchor.below {
        let against = anchor.pill.3 + gap;
        (work.1 + room / 4).max(against)
    } else {
        let against = anchor.pill.1 - gap;
        (work.3 - room / 4).min(against)
    }
}

/// Places a `width` x `height` window against `anchor`, kept inside `work`.
///
/// `gap` is the space left between the pill and the window and `strip` is how
/// thick the reveal starts, both in physical pixels. `rest` is the edge the
/// window settles on (see [`rest_edge`]).
pub(super) fn place(
    anchor: Anchor,
    width: i32,
    height: i32,
    work: Rect,
    gap: i32,
    strip: i32,
    rest: i32,
) -> Placement {
    let (pill_left, pill_top, pill_right, pill_bottom) = anchor.pill;
    let pill_center = (pill_left + pill_right) / 2;
    let x = clamp_into(pill_center - width / 2, work.0, work.2 - width);
    let (against, settled) = if anchor.below {
        (pill_bottom + gap, rest)
    } else {
        (pill_top - gap - height, rest - height)
    };
    let y_start = clamp_into(against, work.1, work.3 - height);
    let y_rest = clamp_into(settled, work.1, work.3 - height);

    // The strip is as wide as the pill and centred on it, so the launcher reads
    // as coming out of the pill rather than out of the middle of the window.
    let strip_width = (pill_right - pill_left).clamp(1, width);
    let strip_left = clamp_into(pill_center - strip_width / 2 - x, 0, width - strip_width);
    let strip = strip.clamp(1, height);
    let start = if anchor.below {
        (strip_left, 0, strip_left + strip_width, strip)
    } else {
        (strip_left, height - strip, strip_left + strip_width, height)
    };
    Placement {
        x,
        y_start,
        y_rest,
        start,
    }
}

/// `value` limited to `low..=high`, preferring `low` when the range is empty
/// (a window wider than the work area is pinned to its left edge).
fn clamp_into(value: i32, low: i32, high: i32) -> i32 {
    value.min(high).max(low)
}

/// The reveal's shape at `progress` (0 = the strip, 1 = the whole window).
pub(super) fn reveal_rect(start: Rect, full: Rect, progress: f32) -> Rect {
    let mix = |from: i32, to: i32| (from as f32 + (to - from) as f32 * progress).round() as i32;
    (
        mix(start.0, full.0),
        mix(start.1, full.1),
        mix(start.2, full.2),
        mix(start.3, full.3),
    )
}

/// The corner radius at `progress`, from half the strip's thickness (a pill) to
/// the radius Windows rounds a resting window with.
pub(super) fn reveal_radius(strip: i32, resting: i32, progress: f32) -> i32 {
    let from = strip as f32 / 2.0;
    (from + (resting as f32 - from) * progress).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = (0, 0, 2560, 1400);

    fn top_bar() -> Anchor {
        Anchor {
            pill: (1180, 6, 1380, 38),
            below: true,
        }
    }

    fn bottom_bar() -> Anchor {
        Anchor {
            pill: (1180, 1362, 1380, 1394),
            below: false,
        }
    }

    #[test]
    fn it_rests_a_quarter_of_the_way_down_like_it_always_did_but_never_above_the_pill() {
        // (1400 - 300) / 4 below the top of the work area.
        assert_eq!(rest_edge(top_bar(), 300, WORK, 6), 275);
        // A work area with no room: pinned against the pill, not above it.
        assert_eq!(rest_edge(top_bar(), 1400, WORK, 6), 44);
        // Mirrored for a bottom bar: a quarter of the way up from the bottom.
        assert_eq!(rest_edge(bottom_bar(), 300, WORK, 6), 1400 - 275);
        assert_eq!(rest_edge(bottom_bar(), 1400, WORK, 6), 1362 - 6);
    }

    #[test]
    fn a_top_bar_starts_under_its_pill_centred_on_it_and_slides_down_to_rest() {
        let placement = place(top_bar(), 640, 300, WORK, 6, 12, 275);
        assert_eq!(placement.x, 1280 - 320);
        assert_eq!(placement.y_start, 38 + 6);
        assert_eq!(placement.y_rest, 275);
        // The strip is the pill's width, centred on it, against the window's top edge.
        assert_eq!(placement.start, (220, 0, 420, 12));
        assert_eq!(placement.y_at(0.0), 44);
        assert_eq!(placement.y_at(1.0), 275);
        assert!(placement.y_at(0.5) > 44 && placement.y_at(0.5) < 275);
    }

    #[test]
    fn a_bottom_bar_starts_above_its_pill_with_the_strip_on_the_bottom_edge() {
        let rest = rest_edge(bottom_bar(), 300, WORK, 6);
        let placement = place(bottom_bar(), 640, 300, WORK, 6, 12, rest);
        assert_eq!(placement.y_start, 1362 - 6 - 300);
        assert_eq!(placement.y_rest, rest - 300);
        assert!(placement.y_rest < placement.y_start, "it slides up");
        assert_eq!(placement.start, (220, 288, 420, 300));
    }

    #[test]
    fn the_window_stays_on_the_work_area_and_the_strip_stays_on_the_window() {
        // A pill near the right edge: the window is pushed left, and the strip
        // follows the pill rather than the window's centre.
        let anchor = Anchor {
            pill: (2400, 6, 2540, 38),
            below: true,
        };
        let placement = place(anchor, 640, 300, WORK, 6, 12, 275);
        assert_eq!(placement.x, 2560 - 640);
        let (left, _, right, _) = placement.start;
        assert!(left >= 0 && right <= 640, "strip {left}..{right}");
        assert_eq!(right - left, 140);
        assert!(
            (placement.x + left..placement.x + right).contains(&2470),
            "the strip is under the pill"
        );

        // Too tall for the room below: pinned to the top of the work area.
        let tall = place(top_bar(), 640, 1500, WORK, 6, 12, 44);
        assert_eq!((tall.y_start, tall.y_rest), (0, 0));
    }

    #[test]
    fn a_pill_wider_than_the_window_gets_a_window_wide_strip() {
        let anchor = Anchor {
            pill: (0, 6, 1000, 38),
            below: true,
        };
        let placement = place(anchor, 640, 300, WORK, 6, 12, 275);
        assert_eq!(placement.start, (0, 0, 640, 12));
    }

    #[test]
    fn the_reveal_grows_from_the_strip_to_the_window() {
        let start = (220, 0, 420, 12);
        let full = (0, 0, 640, 300);
        assert_eq!(reveal_rect(start, full, 0.0), start);
        assert_eq!(reveal_rect(start, full, 1.0), full);
        let middle = reveal_rect(start, full, 0.5);
        assert!(middle.0 < 220 && middle.2 > 420 && middle.3 > 12 && middle.3 < 300);
        assert_eq!(reveal_radius(12, 8, 0.0), 6);
        assert_eq!(reveal_radius(12, 8, 1.0), 8);
    }
}

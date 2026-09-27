//! Battery cluster geometry for the bar's right zone.
//!
//! One source of truth for the percent text, the charging bolt, the body, its
//! nub, and the level fill. The cluster grew a charging mark next to the
//! battery, and keeping every rect in one place is what stops that mark from
//! drifting off the battery it belongs to.

/// Height of the battery body, its bolt, and the row they sit in.
pub(crate) const BATTERY_H: u32 = 10;
/// Width reserved for the percent text.
pub(crate) const BATTERY_TEXT_W: u32 = 38;
/// Width of the charging bolt's slot.
pub(crate) const BATTERY_BOLT_W: u32 = 10;
/// Width of the battery body.
pub(crate) const BATTERY_BODY_W: u32 = 22;
/// Width of the battery's terminal nub.
pub(crate) const BATTERY_NUB_W: u32 = 2;
/// Total width the module needs, including the gaps between its parts. The
/// bolt's slot is always reserved so plugging the charger in never shifts the
/// rest of the right zone.
pub(crate) const BATTERY_MODULE_W: i32 =
    (BATTERY_TEXT_W + BATTERY_BOLT_W + BATTERY_BODY_W + BATTERY_NUB_W + 12) as i32;

/// Every rect of the battery cluster, in frame coordinates.
pub(crate) struct BatteryGeometry {
    /// The charging bolt's slot, aligned with the body's box.
    pub(crate) bolt: (i32, i32, u32, u32),
    pub(crate) body: (i32, i32, u32, u32),
    pub(crate) nub: (i32, i32, u32, u32),
    pub(crate) fill: (i32, i32, u32, u32),
}

/// Lays the cluster out from `x`, vertically centered on `y`.
///
/// The bolt shares the body's top and height exactly, so the two always read
/// as one mark. `percent` only drives the level fill.
pub(crate) fn battery_geometry(x: i32, y: i32, percent: u8) -> BatteryGeometry {
    let text = x + BATTERY_TEXT_W as i32 + 4;
    let bolt_x = text;
    let body_x = bolt_x + BATTERY_BOLT_W as i32 + 2;
    let nub_x = body_x + BATTERY_BODY_W as i32 + 1;
    BatteryGeometry {
        bolt: (bolt_x, y, BATTERY_BOLT_W, BATTERY_H),
        body: (body_x, y, BATTERY_BODY_W, BATTERY_H),
        nub: (nub_x, y + 3, BATTERY_NUB_W, BATTERY_H - 6),
        fill: (
            body_x + 2,
            y + 2,
            (18 * u32::from(percent.min(100)) / 100).max(1),
            6,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charging_bolt_shares_the_battery_box() {
        let g = battery_geometry(1752, 13, 62);
        assert_eq!(g.bolt.1, g.body.1, "bolt and battery share a top edge");
        assert_eq!(g.bolt.3, g.body.3, "bolt and battery share a height");
        assert!(
            g.bolt.0 + g.bolt.2 as i32 <= g.body.0,
            "the bolt never sits on top of the battery"
        );
    }

    #[test]
    fn nub_and_fill_stay_inside_the_body() {
        let g = battery_geometry(0, 13, 100);
        let (bx, by, bw, bh) = g.body;
        assert!(g.nub.0 >= bx + bw as i32, "the nub sits past the body");
        assert_eq!(g.nub.1, by + 3);
        assert_eq!(
            g.nub.1 + g.nub.3 as i32,
            by + bh as i32 - 3,
            "the nub is centered"
        );
        assert!(g.fill.0 >= bx && g.fill.0 + g.fill.2 as i32 <= bx + bw as i32);
        assert!(g.fill.1 >= by && g.fill.1 + g.fill.3 as i32 <= by + bh as i32);
    }

    #[test]
    fn cluster_fits_the_reserved_width() {
        let g = battery_geometry(1000, 0, 50);
        let rightmost = g
            .nub
            .0
            .max(g.body.0 + g.body.2 as i32)
            .max(g.bolt.0 + g.bolt.2 as i32);
        assert!(
            rightmost - 1000 <= BATTERY_MODULE_W,
            "the cluster must fit the width the right zone reserves"
        );
    }

    #[test]
    fn empty_battery_still_draws_a_visible_fill() {
        assert!(battery_geometry(0, 0, 0).fill.2 >= 1, "0% must not vanish");
    }
}

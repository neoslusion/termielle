//! Expanded-card section: the control panel.
//!
//! The bar popup shows one body at a time. The panel is a deliberate body, not
//! a takeover: it never wins priority over an activity, and the pill's own
//! click closes it. Everything here is something Termielle can actually own —
//! the system volume, and its own settings. Windows' own quick settings stay
//! behind the one row that hands off to the shell.

use super::super::controller::Controller;
use super::super::paint::CardPaintCtx;
use super::super::types::{
    HIT_PANEL_TOGGLE_BASE, HIT_PANEL_VOLUME_DOWN, HIT_PANEL_VOLUME_UP, PanelToggle,
};
use crate::animation::FrameBuffer;
use termielle_core::IslandConfig;

/// Height of the card header the panel's rows start below.
pub(crate) const PANEL_HEADER_H: i32 = 40;
/// Height of one panel row.
pub(crate) const PANEL_ROW_H: i32 = 28;
/// Vertical gap between rows.
pub(crate) const PANEL_ROW_GAP: i32 = 8;
/// Inset from the card's left and right edges.
pub(crate) const PANEL_PAD: i32 = 18;
/// Bottom breathing room under the last row.
pub(crate) const PANEL_BOTTOM_PAD: i32 = 16;
/// Width of a toggle's switch track.
pub(crate) const PANEL_SWITCH_W: u32 = 32;
/// Height of a toggle's switch track.
pub(crate) const PANEL_SWITCH_H: u32 = 18;
/// Side length of a stepper button.
pub(crate) const PANEL_STEPPER: u32 = 28;

/// Rows in order: volume, the three Termielle toggles, then the shell
/// hand-off. The order is the panel's identity, so it lives in one list.
pub(crate) const PANEL_ROWS: [PanelRow; 5] = [
    PanelRow::Volume,
    PanelRow::Toggle(PanelToggle::HoverExpand),
    PanelRow::Toggle(PanelToggle::Face),
    PanelRow::Toggle(PanelToggle::Music),
    PanelRow::SystemTray,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanelRow {
    Volume,
    Toggle(PanelToggle),
    SystemTray,
}

/// Total card height the panel needs. One source of truth: the painter lays
/// rows out from it and `bar_expanded_height` sizes the popup from it, so the
/// card can never be shorter than its content.
pub(crate) const fn panel_height() -> u32 {
    (PANEL_HEADER_H
        + PANEL_ROW_H * PANEL_ROWS.len() as i32
        + PANEL_ROW_GAP * (PANEL_ROWS.len() as i32 - 1)
        + PANEL_BOTTOM_PAD) as u32
}

/// Top edge of row `index`, in card coordinates.
pub(crate) const fn panel_row_y(index: usize) -> i32 {
    PANEL_HEADER_H + (PANEL_ROW_H + PANEL_ROW_GAP) * index as i32
}

/// Rect of the volume row's track, given the card width. The track is also the
/// row's wheel target, so it is wider than the drawn bar.
pub(crate) fn volume_track(width: u32) -> (i32, i32, u32, u32) {
    let left = PANEL_PAD + 30;
    let right = width as i32 - PANEL_PAD - 34 - PANEL_STEPPER as i32 * 2 - 8;
    (left, panel_row_y(0) + 11, (right - left).max(0) as u32, 6)
}

impl Controller {
    /// The control panel: system volume plus Termielle's own settings, with one
    /// row that hands the rest to the Windows shell.
    pub(crate) fn paint_control_panel(
        &mut self,
        frame: &mut FrameBuffer,
        _island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        let (primary, secondary) = crate::animation::notch::ink_pair(&self.island.glass);
        let dim = self.ink_dim();
        // Card header caption, matching the media card's label. The header
        // itself is drawn by the dispatcher; this only names the body.
        let tag_x = if self.island.has_widget("face") {
            ctx.pad + 30
        } else {
            ctx.pad
        };
        crate::animation::notch::draw_text(
            frame,
            "Controls",
            tag_x,
            16,
            ctx.width.saturating_sub(tag_x as u32 + 50),
            10,
            true,
            dim,
        );

        let (level, muted) = self.panel_volume();

        for (index, row) in PANEL_ROWS.iter().enumerate() {
            let y = panel_row_y(index);
            match row {
                PanelRow::Volume => {
                    let glyph_x = PANEL_PAD;
                    let glyph_y = y + (PANEL_ROW_H - 24) / 2;
                    crate::animation::icons::draw_icon(
                        frame,
                        if muted {
                            crate::animation::icons::VOLUME_MUTED
                        } else {
                            crate::animation::icons::VOLUME
                        },
                        glyph_x,
                        glyph_y,
                        24,
                        primary,
                    );
                    // The whole glyph is the mute target; the track answers
                    // the wheel, and the two steppers own their own cells.
                    self.icon_hits.push((
                        crate::bar::HIT_BAR_VOLUME_TOGGLE,
                        glyph_x,
                        glyph_y,
                        24,
                        24,
                    ));

                    let (tx, ty, tw, th) = volume_track(ctx.width);
                    crate::animation::notch::draw_rounded_rect(
                        frame,
                        tx,
                        ty,
                        tw,
                        th,
                        3,
                        [255, 255, 255, 18],
                        [0; 4],
                    );
                    let filled = (tw * level as u32 / 100).min(tw);
                    if filled > 0 {
                        crate::animation::notch::draw_rounded_rect(
                            frame,
                            tx,
                            ty,
                            filled,
                            th,
                            3,
                            if muted {
                                [dim[0], dim[1], dim[2], 90]
                            } else {
                                primary
                            },
                            [0; 4],
                        );
                    }
                    // The track is not clickable, but registering it means
                    // the wheel finds the row in the same frame coordinates
                    // the click path already uses — no second geometry copy.
                    self.icon_hits.push((
                        crate::app::types::HIT_PANEL_VOLUME_TRACK,
                        tx,
                        panel_row_y(0),
                        tw,
                        PANEL_ROW_H as u32,
                    ));
                    let label_x = tx + tw as i32 + 8;
                    crate::animation::notch::draw_text(
                        frame,
                        &format!("{level}%"),
                        label_x,
                        y + 8,
                        34,
                        12,
                        muted,
                        if muted { dim } else { primary },
                    );

                    for (index, glyph) in ["−", "+"].iter().enumerate() {
                        let bx = ctx.width as i32
                            - PANEL_PAD
                            - PANEL_STEPPER as i32 * (2 - index as i32);
                        let hit = if index == 0 {
                            HIT_PANEL_VOLUME_DOWN
                        } else {
                            HIT_PANEL_VOLUME_UP
                        };
                        crate::animation::notch::draw_rounded_rect(
                            frame,
                            bx,
                            y,
                            PANEL_STEPPER,
                            PANEL_ROW_H as u32,
                            8,
                            [255, 255, 255, 20],
                            [255, 255, 255, 28],
                        );
                        crate::animation::notch::draw_text_in_rect(
                            frame,
                            glyph,
                            (bx, y, PANEL_STEPPER, PANEL_ROW_H as u32),
                            14,
                            false,
                            primary,
                            true,
                        );
                        self.icon_hits
                            .push((hit, bx, y, PANEL_STEPPER, PANEL_ROW_H as u32));
                    }
                }
                PanelRow::Toggle(setting) => {
                    let (label, on) = self.panel_toggle_state(*setting);
                    crate::animation::notch::draw_text(
                        frame,
                        label,
                        PANEL_PAD,
                        y + 8,
                        ctx.width
                            .saturating_sub((PANEL_PAD as u32) + PANEL_SWITCH_W + 24),
                        12,
                        false,
                        primary,
                    );
                    let sx = ctx.width as i32 - PANEL_PAD - PANEL_SWITCH_W as i32;
                    let sy = y + (PANEL_ROW_H - PANEL_SWITCH_H as i32) / 2;
                    crate::animation::notch::draw_rounded_rect(
                        frame,
                        sx,
                        sy,
                        PANEL_SWITCH_W,
                        PANEL_SWITCH_H,
                        PANEL_SWITCH_H / 2,
                        if on { primary } else { [255, 255, 255, 20] },
                        if on { [0; 4] } else { [255, 255, 255, 40] },
                    );
                    let knob = PANEL_SWITCH_H - 4;
                    let knob_x = if on {
                        sx + PANEL_SWITCH_W as i32 - knob as i32 - 2
                    } else {
                        sx + 2
                    };
                    crate::animation::notch::draw_rounded_rect(
                        frame,
                        knob_x,
                        sy + 2,
                        knob,
                        knob,
                        knob / 2,
                        if on {
                            [
                                self.island.glass.tint[0],
                                self.island.glass.tint[1],
                                self.island.glass.tint[2],
                                255,
                            ]
                        } else {
                            [255, 255, 255, 200]
                        },
                        [0; 4],
                    );
                    self.icon_hits.push((
                        HIT_PANEL_TOGGLE_BASE - *setting as isize,
                        PANEL_PAD,
                        y,
                        ctx.width - PANEL_PAD as u32 * 2,
                        PANEL_ROW_H as u32,
                    ));
                }
                PanelRow::SystemTray => {
                    // Deliberate delegation: Windows owns these toggles, and
                    // the shell's own panel is the honest way to reach them.
                    crate::animation::notch::draw_text(
                        frame,
                        "System tray",
                        PANEL_PAD,
                        y + 8,
                        ctx.width.saturating_sub((PANEL_PAD as u32) * 2 + 44),
                        12,
                        false,
                        primary,
                    );
                    crate::animation::notch::draw_text(
                        frame,
                        "Open",
                        ctx.width as i32 - PANEL_PAD - 40,
                        y + 8,
                        40,
                        12,
                        false,
                        secondary,
                    );
                    self.icon_hits.push((
                        crate::bar::shell::ShellAction::SystemTray.hit_id(),
                        PANEL_PAD,
                        y,
                        ctx.width - PANEL_PAD as u32 * 2,
                        PANEL_ROW_H as u32,
                    ));
                }
            }
        }
    }

    /// The volume level the panel shows. The bar's metrics worker carries the
    /// live snapshot; without one the row reads as muted rather than guessing.
    pub(crate) fn panel_volume(&self) -> (u8, bool) {
        self.bar_metrics_cache
            .as_ref()
            .map(|metrics| (metrics.volume.level, metrics.volume.muted))
            .unwrap_or((0, true))
    }

    /// Label and current state of one Termielle toggle. The controller's
    /// island config is the mirror of the host's config, which is the single
    /// owner both the tray menu and this panel read.
    pub(crate) fn panel_toggle_state(&self, setting: PanelToggle) -> (&'static str, bool) {
        match setting {
            PanelToggle::HoverExpand => ("Expand on hover", self.island.expand_on_hover),
            PanelToggle::Face => ("Termielle face", self.island.has_widget("face")),
            PanelToggle::Music => ("Media", self.island.has_widget("music")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_never_overlap() {
        for pair in PANEL_ROWS.windows(2) {
            let (first, second) = (pair[0], pair[1]);
            let _ = (first, second);
        }
        let mut previous_bottom = 0;
        for index in 0..PANEL_ROWS.len() {
            let top = panel_row_y(index);
            assert!(
                top >= previous_bottom,
                "row {index} overlaps the one above it"
            );
            previous_bottom = top + PANEL_ROW_H;
        }
    }

    #[test]
    fn last_row_fits_inside_the_card_height() {
        let bottom = panel_row_y(PANEL_ROWS.len() - 1) + PANEL_ROW_H;
        assert!(
            bottom + PANEL_BOTTOM_PAD <= panel_height() as i32,
            "the card would clip its last row"
        );
    }

    #[test]
    fn volume_track_clears_the_steppers() {
        let (x, _, w, _) = volume_track(420);
        let stepper_left = 420 - PANEL_PAD - PANEL_STEPPER as i32 * 2;
        assert!(
            x + w as i32 <= stepper_left,
            "the track runs under the buttons"
        );
        assert!(w > 0, "the track must have a usable width");
    }

    #[test]
    fn volume_track_survives_a_narrow_card() {
        // A card too narrow for a track must report none rather than a
        // negative width, so the painter simply draws no bar.
        let (x, _, w, _) = volume_track(90);
        assert_eq!(w, 0, "a card too narrow for a track reports none");
        assert!(x > 0 && x < 90, "the row still starts inside the card");
    }
}

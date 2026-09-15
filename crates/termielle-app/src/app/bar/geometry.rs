//! Bar geometry: strip-row metrics, pill rect, module flags, card rect.

use super::super::controller::Controller;

impl Controller {
    /// Bar strip row geometry from raw config values (all logical px):
    /// `(margin, vis_off, vis_h, pill_off, pill_h, radius)` — horizontal
    /// margin, visual-strip offset within the bar strip, visual height,
    /// module-pill offset/height centered in the visual strip, and the
    /// clamped blob radius. Pure so tests can pin it without a Controller.
    /// `render_bar` adds `bar_y`; the collapsed hover sensor uses strip-local
    /// coords directly (collapsed frames put the strip at y=0).
    pub(crate) fn bar_row_geometry(
        bar_h: u32,
        top: bool,
        edge_to_edge: bool,
        margin: u32,
        corner_radius: u32,
    ) -> (u32, i32, u32, i32, u32, u32) {
        let margin = if edge_to_edge {
            0
        } else {
            margin.min(bar_h / 2)
        };
        let vis_h = bar_h.saturating_sub(margin).max(12);
        let pill_h = (vis_h.saturating_sub(10)).max(12).min(vis_h);
        let vis_off = if top { margin as i32 } else { 0 };
        let pill_off = vis_off + ((vis_h.saturating_sub(pill_h)) / 2) as i32;
        let radius = corner_radius.min(vis_h / 2);
        (margin, vis_off, vis_h, pill_off, pill_h, radius)
    }

    /// Row geometry for the live config: (margin, vis_y, vis_h, pill_y,
    /// pill_h, bar_radius) in collapsed-frame coords. `render_bar` shifts by
    /// `bar_y` for expanded-bottom frames.
    pub(crate) fn bar_row(&self) -> (u32, i32, u32, i32, u32, u32) {
        let bar = &self.island.bar;
        let (margin, vis_off, vis_h, pill_off, pill_h, radius) = Self::bar_row_geometry(
            bar.height,
            bar.position == termielle_core::BarPosition::Top,
            bar.edge_to_edge,
            bar.margin,
            bar.corner_radius,
        );
        (margin, vis_off, vis_h, pill_off, pill_h, radius)
    }

    /// Collapsed center-pill rect (x, y, w, h), logical px. Single source for
    /// `render_bar`, its hit target, and the `set_hover` sensor.
    pub(crate) fn bar_pill_rect(&self, width: u32) -> (i32, i32, u32, u32) {
        let (_, _, _, pill_off, pill_h, _) = self.bar_row();
        let pill_w = 180u32.min(width / 3).max(120);
        let pill_cx = ((width.saturating_sub(pill_w)) / 2) as i32;
        (pill_cx, pill_off, pill_w, pill_h)
    }

    /// Whether a bar module name is listed in its zone (`left`, `center`,
    /// `right`). Unknown names are ignored; order within a zone is fixed by
    /// the renderer. An emptied zone collapses (neighbors do not reflow).
    pub(crate) fn bar_module(&self, zone: &str, name: &str) -> bool {
        let list = match zone {
            "left" => &self.island.bar.modules_left,
            "center" => &self.island.bar.modules_center,
            _ => &self.island.bar.modules_right,
        };
        list.iter().any(|m| m == name)
    }

    /// Expanded island-card rect (x, y, w, h), logical px. Width matches the
    /// `render_bar` card blob (including `.max(300)`); height derives from
    /// the same `target_size` the renderer uses, so the sensor tracks the
    /// real 154..210 card instead of a hardcoded 210.
    pub(crate) fn bar_card_rect(&self) -> (i32, i32, i32, i32) {
        let bar_h = self.island.bar.height as i32;
        let is_top = self.island.bar.position == termielle_core::BarPosition::Top;
        let island_w = self
            .island
            .expanded_width
            .min(self.bar_width.saturating_sub(40))
            .max(300) as i32;
        let island_x = ((self.bar_width as i32 - island_w) / 2).max(0);
        let exp_h = (self.target_size(self.state).1 as i32)
            .saturating_sub(bar_h)
            .max(0);
        let y = if is_top { bar_h } else { 0 };
        (island_x, y, island_w, exp_h)
    }
}

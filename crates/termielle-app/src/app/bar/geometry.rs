//! Bar geometry: strip-row metrics, pill rect, module flags, card rect.

use super::super::controller::Controller;
use super::modules::{BarZone, module_enabled};

impl Controller {
    pub(crate) fn bar_expanded_height(&self) -> u32 {
        if !self.alerts.is_empty() {
            return 124;
        }
        if self.panel_open {
            return crate::app::cards::panel::panel_height();
        }
        let tasks =
            self.island.has_widget("tasks") && self.island.show_tasks && !self.tasks.is_empty();
        match (self.media_available(), tasks) {
            (true, true) => 210,
            (true, false) => 175,
            (false, true) => 180,
            (false, false) => 76,
        }
    }

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
        let pill_w = 180.min(width.max(1).saturating_sub(16));
        let pill_cx = ((width.saturating_sub(pill_w)) / 2) as i32;
        (pill_cx, pill_off, pill_w, pill_h)
    }
    /// Whether a physical client point counts as being on the bar's surface.
    ///
    /// A bar window is mostly transparent and its hit map covers every opaque
    /// pixel - module text, an open card - so "over an opaque pixel" cannot
    /// tell the pill apart from the rest of the strip. The point arrives in
    /// physical client pixels and is mapped into frame space first, the same
    /// way `set_hover_point` maps it before testing icon hit-rects.
    ///
    /// With nothing open, only the pill counts. With a card open *by hover*,
    /// the whole frame counts: reaching a switch means travelling from the
    /// pill down a card taller than the strip, so "left the pill" is not
    /// "gone", and treating it as gone closed the card out from under the
    /// pointer. A card opened by click is deliberately excluded - there the
    /// same flag drives click-outside dismissal, which must still fire.
    pub fn point_over_bar_surface(&self, point: (i32, i32)) -> bool {
        let (x, y) = (self.to_logical(point.0), self.to_logical(point.1));
        let (pill_cx, pill_off, pill_w, pill_h) = self.bar_pill_rect(self.bar_width);
        let (width, height) = self.current_logical_size();
        let bar_h = self.island.bar.height;
        let bar_y = if self.island.bar.position == termielle_core::BarPosition::Top {
            0
        } else {
            height.saturating_sub(bar_h) as i32
        };
        let top = bar_y + pill_off;
        if x >= pill_cx && x < pill_cx + pill_w as i32 && y >= top && y < top + pill_h as i32 {
            return true;
        }
        self.hover_expanded
            && !self.manually_expanded
            && x >= 0
            && x < width as i32
            && y >= 0
            && y < height as i32
    }

    /// Whether a bar module name is listed in its zone (`left`, `center`,
    /// `right`). Unknown names are ignored; order within a zone is fixed by
    /// the renderer. An emptied zone collapses (neighbors do not reflow).
    pub(crate) fn bar_module(&self, zone: &str, name: &str) -> bool {
        let zone = match zone {
            "left" => BarZone::Left,
            "center" => BarZone::Center,
            _ => BarZone::Right,
        };
        module_enabled(&self.island.bar, zone, name)
    }
}

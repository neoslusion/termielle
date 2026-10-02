//! Bar geometry: strip-row metrics, pill rect, module flags, card rect.

use super::super::controller::Controller;
use super::modules::{BarZone, module_enabled};

impl Controller {
    pub(crate) fn bar_expanded_height(&self) -> u32 {
        let island_height = if !self.alerts.is_empty() || self.island_card_open() {
            self.bar_island_height()
        } else {
            0
        };
        let panel_height = if self.panel_open {
            match self.panel_kind {
                crate::app::types::RightPanel::Controls => crate::app::cards::panel::panel_height(),
                crate::app::types::RightPanel::Notifications => {
                    crate::app::cards::notifications::panel_height(self.recent_notifications.len())
                }
            }
        } else {
            0
        };
        island_height.max(panel_height)
    }

    pub(crate) fn bar_island_height(&self) -> u32 {
        if !self.alerts.is_empty()
            || (self.alert_pill_morphing && self.spring.is_some() && !self.island_card_open())
        {
            return crate::app::types::ALERT_HEIGHT;
        }
        let tasks =
            self.island.has_widget("tasks") && self.island.show_tasks && !self.tasks.is_empty();
        match (self.media_available(), tasks) {
            (true, true) => 210,
            (true, false) => 175,
            (false, true) => 180,
            (false, false) if self.state == termielle_core::VisualState::Idle => 164,
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
    /// Coarse bar-frame containment, including the whole frame while a popup is open.
    ///
    /// A bar window is mostly transparent and its hit map covers every opaque
    /// pixel - module text, an open card - so "over an opaque pixel" cannot
    /// tell the pill apart from the rest of the strip. The point arrives in
    /// physical client pixels and is mapped into frame space first, the same
    /// way `set_hover_point` maps it before testing icon hit-rects.
    ///
    /// With nothing open, only the pill counts. Click-outside dismissal uses
    /// [`Self::point_over_open_popup`] instead of this broad frame test.
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
        (self.panel_open || self.hover_expanded || self.manually_expanded)
            && x >= 0
            && x < width as i32
            && y >= 0
            && y < height as i32
    }

    /// Whether a click is inside the open popup or one of its own bar entries.
    pub fn point_over_open_popup(&self, point: (i32, i32)) -> bool {
        let (cursor_x, cursor_y) = (self.to_logical(point.0), self.to_logical(point.1));
        let notifications_open =
            self.panel_open && self.panel_kind == crate::app::types::RightPanel::Notifications;
        if !notifications_open && self.point_over_notch_surface(point) {
            return true;
        }
        if !self.panel_open {
            return false;
        }
        let panel_id = match self.panel_kind {
            crate::app::types::RightPanel::Controls => crate::app::types::HIT_CARD_PANEL,
            crate::app::types::RightPanel::Notifications => {
                crate::app::types::HIT_CARD_NOTIFICATIONS
            }
        };
        if self
            .icon_hits
            .iter()
            .any(|&(id, hit_x, hit_y, hit_w, hit_h)| {
                id == panel_id
                    && cursor_x >= hit_x
                    && cursor_x < hit_x + hit_w as i32
                    && cursor_y >= hit_y
                    && cursor_y < hit_y + hit_h as i32
            })
        {
            return true;
        }
        let (width, height) = self.current_logical_size();
        let bar_height = self.island.bar.height;
        let expanded_height = height.saturating_sub(bar_height + super::types::BAR_POPUP_GAP);
        let bar_y = if self.island.bar.position == termielle_core::BarPosition::Top {
            0
        } else {
            height.saturating_sub(bar_height) as i32
        };
        let panel_width = self.island.expanded_width.min(width.saturating_sub(32));
        let (panel_x, panel_y, visible_width, visible_height) =
            self.bar_panel_rect(width, expanded_height, bar_y, panel_width);
        cursor_x >= panel_x
            && cursor_x < panel_x + visible_width as i32
            && cursor_y >= panel_y
            && cursor_y < panel_y + visible_height as i32
    }

    pub(crate) fn bar_panel_rect(
        &self,
        width: u32,
        expanded_height: u32,
        bar_y: i32,
        panel_width: u32,
    ) -> (i32, i32, u32, u32) {
        let panel_height = match self.panel_kind {
            crate::app::types::RightPanel::Controls => crate::app::cards::panel::panel_height(),
            crate::app::types::RightPanel::Notifications => {
                crate::app::cards::notifications::panel_height(self.recent_notifications.len())
            }
        };
        let visible_height = expanded_height.min(panel_height);
        let anchor_x = self
            .bar_right_cache
            .as_ref()
            .and_then(|cache| match self.panel_kind {
                crate::app::types::RightPanel::Controls => cache.control_center_x,
                crate::app::types::RightPanel::Notifications => cache.clock_x,
            })
            .unwrap_or(width as i32 / 2);
        let panel_x = (anchor_x - (panel_width / 2) as i32)
            .clamp(0, (width as i32 - panel_width as i32).max(0));
        let panel_y = if self.island.bar.position == termielle_core::BarPosition::Top {
            (self.island.bar.height + super::types::BAR_POPUP_GAP) as i32
        } else {
            bar_y - super::types::BAR_POPUP_GAP as i32 - visible_height as i32
        };
        (panel_x, panel_y, panel_width, visible_height)
    }

    /// Whether the point is over the *island's* surface: its pill, or the
    /// window while the island's own card is up.
    ///
    /// [`Self::point_over_bar_surface`] answers that question for whichever
    /// surface is showing, so a click on the panel's control counted as
    /// "over the bar" - and the island read it as a pointer on its own
    /// surface. Hover input has to be asked about the surface it drives, or
    /// opening the panel silently redefines where the pill is.
    pub fn point_over_notch_surface(&self, point: (i32, i32)) -> bool {
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
        let card_w = self.island.expanded_width.min(width.saturating_sub(32));
        let card_h = self.bar_island_height();
        let card_x = (width.saturating_sub(card_w) / 2) as i32;
        let card_y = if self.island.bar.position == termielle_core::BarPosition::Top {
            bar_h + crate::app::bar::types::BAR_POPUP_GAP
        } else {
            height.saturating_sub(bar_h + crate::app::bar::types::BAR_POPUP_GAP + card_h)
        } as i32;
        (self.hover_expanded || self.manually_expanded)
            && x >= card_x
            && x < card_x + card_w as i32
            && y >= card_y
            && y < card_y + card_h as i32
    }

    /// Test-facing alias for the island's own surface test.
    pub fn point_over_notch_surface_is(&self, point: (i32, i32)) -> bool {
        self.point_over_notch_surface(point)
    }

    /// Test-facing alias for the surface test.
    pub fn point_over_bar_surface_is(&self, point: (i32, i32)) -> bool {
        self.point_over_bar_surface(point)
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

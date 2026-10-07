//! Content-fit compact geometry shared by target sizing, painting and hit maps.
//! Inspired by the reference's measured/quantized layout principle; no copied renderer.
use super::controller::Controller;
use termielle_core::VisualState;
fn quantize(width: u32) -> u32 {
    width.saturating_add(1) / 2 * 2
}
impl Controller {
    pub(crate) fn compact_face_size(&self) -> u32 {
        self.island.height.saturating_sub(8).min(28)
    }
    pub(crate) fn compact_label_x(&self) -> u32 {
        if self.island.has_widget("face") {
            12 + self.compact_face_size() + 6
        } else {
            16
        }
    }
    pub(crate) fn fitted_primary_width(&self) -> u32 {
        let agent = self.state != VisualState::Idle || self.reducer.session_count() > 0;
        let label_x = self.compact_label_x();
        if agent {
            let dots = if self.island.has_widget("agents") {
                self.reducer.session_count().clamp(1, 4) as u32
            } else {
                1
            };
            return quantize(label_x + dots * 10 + 14).max(56);
        }
        if self.media_available() {
            // Combined media layout subtracts its shared 8px edge: with no
            // face there is no primary reservation at all (minimal_width
            // still bounds the complete clickable surface).
            return if self.island.has_widget("face") {
                quantize(label_x + 14)
            } else {
                8
            };
        }
        // Preserve explicit custom widths. Only the stock 140px idle/hover
        // geometry is content-fit; disk configuration is never rewritten.
        if self.island.collapsed_width != 140 {
            return self.island.collapsed_width;
        }
        // Branding visibility is decoration-only: preserve the same hit map.
        let label =
            crate::animation::text_metrics::width("Termielle", 11, true, self.render_scale());
        quantize(label_x + label + 24).max(self.island.minimal_width)
    }
    pub(crate) fn bar_center_label(&self, state: VisualState) -> (&str, bool, bool) {
        if self.media_available() {
            let title = self
                .media
                .as_ref()
                .map(|m| m.title.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("Media");
            (title, false, false)
        } else if state != VisualState::Idle {
            (state.display_name(), false, true)
        } else if self.island.show_name {
            ("Termielle", true, false)
        } else {
            ("", true, false)
        }
    }
    pub(crate) fn fitted_bar_pill_width(&self, pill_h: u32) -> u32 {
        let (label, bold, status) = self.bar_center_label(self.state);
        let label = if self.state == VisualState::Idle && !self.media_available() {
            "Termielle"
        } else {
            label
        };
        let leading = if self.island.has_widget("face") {
            4 + pill_h.saturating_sub(4).min(22) + 6
        } else {
            10
        };
        let text = crate::animation::text_metrics::width(label, 12, bold, self.render_scale());
        quantize(leading + text + if status { 12 } else { 0 } + 10)
            // Keep room for transient volume icon/meter/value without moving
            // the hit targets when feedback appears or branding is hidden.
            .clamp((pill_h * 2).clamp(112, 180), 180)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use termielle_core::{AssetCatalog, IslandConfig, IslandLayout};
    fn controller() -> Controller {
        Controller::new_with_island(
            5000,
            60000,
            AssetCatalog::new(Vec::new()),
            true,
            None,
            IslandConfig {
                layout: IslandLayout::Bar,
                ..Default::default()
            },
        )
    }
    #[test]
    fn bar_pill_fits_content_and_retains_paint_hit_geometry() {
        let mut c = controller();
        let before = c.bar_pill_rect(800);
        assert!(before.2 < 180);
        c.render_bar(VisualState::Idle, 800, 36, 1000);
        let hit = c
            .icon_hits
            .iter()
            .find(|h| h.0 == crate::bar::HIT_BAR_TERMIELLE_MODULE)
            .unwrap();
        assert_eq!((hit.1, hit.3), (before.0, before.2));
        let old_key = c.bar_left_cache.as_ref().unwrap().key;
        c.island.show_name = false;
        assert_eq!(c.bar_pill_rect(800), before);
        c.media = Some(crate::tasks::MediaInfo {
            title: "A very long track title ".repeat(100),
            ..Default::default()
        });
        assert_eq!(c.bar_pill_rect(800).2, 180);
        c.render_bar(VisualState::Idle, 800, 36, 1000);
        let hit = c
            .icon_hits
            .iter()
            .find(|h| h.0 == crate::bar::HIT_BAR_TERMIELLE_MODULE)
            .unwrap();
        assert_eq!(hit.3, 180);
        assert_ne!(
            c.bar_left_cache.as_ref().unwrap().key,
            old_key,
            "side-zone limits must invalidate when center geometry changes"
        );
        let narrow = c.bar_pill_rect(60);
        assert!(narrow.0 >= 0 && narrow.0 + narrow.2 as i32 <= 60);
        c.clock_ms = 100;
        c.volume_feedback_deadline = Some(200);
        assert_eq!(c.bar_pill_rect(800).2, 180);
    }
    #[test]
    fn compact_avoids_empty_media_space_and_preserves_custom_width() {
        let mut c = controller();
        c.island.layout = IslandLayout::Island;
        let idle = c.fitted_primary_width();
        assert!(idle <= 140);
        c.island.collapsed_width = 216;
        assert_eq!(c.fitted_primary_width(), 216);
        c.media = Some(crate::tasks::MediaInfo::default());
        assert!(c.fitted_primary_width() < 100);
        c.media = None;
        c.island.collapsed_width = 140;
        c.island.show_name = false;
        assert_eq!(c.fitted_primary_width(), idle);
        c.media = Some(crate::tasks::MediaInfo::default());
        c.island.widgets.retain(|w| w != "face");
        assert_eq!(c.fitted_primary_width(), 8);
        assert_eq!(termielle_core::IslandConfig::default().collapsed_width, 140);
    }
}

//! Content dispatcher: routes the current presentation to its painter.

use super::super::controller::Controller;
use crate::animation::FrameBuffer;
use termielle_core::{IslandConfig, VisualState};

impl Controller {
    /// Draws the presentation content (alert card, face, session dots,
    /// media, dashboard) onto `frame`. In the split compact presentation
    /// the trailing media content is positioned inside the detached media
    /// blob instead of inline.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_content(
        &mut self,
        frame: &mut FrameBuffer,
        state: VisualState,
        island: &IslandConfig,
        presentation: crate::animation::notch::Presentation,
        width: u32,
        height: u32,
        blobs: &[crate::animation::notch::BlobRect],
        now_ms: u64,
    ) {
        use crate::animation::notch::Presentation;

        // Render according to the active iOS/macOS presentation class.
        // Each arm is a focused painter below; shared setup (glass, frame
        // size) stays with the callers.
        self.icon_hits.clear();
        // The countdown tick repaints only while a banner with a live timeout
        // is actually on screen, so record what this frame showed.
        self.alert_visible = self.paint_alert_banner(frame, island, width, height, now_ms);
        if self.alert_visible {
            return;
        }
        match presentation {
            Presentation::Hidden => {}
            Presentation::Minimal => {
                self.paint_minimal_content(frame, island, width, height, now_ms)
            }
            Presentation::Compact => {
                self.paint_compact_content(frame, state, island, width, height, blobs, now_ms);
            }
            Presentation::Expanded => {
                self.paint_expanded_content(frame, state, island, width, height, now_ms);
            }
        }
    }
}

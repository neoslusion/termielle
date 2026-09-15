//! Expanded presentation: tall-card dispatcher plus the card header row.

use super::super::controller::Controller;
use super::types::CardPaintCtx;
use crate::animation::FrameBuffer;
use termielle_core::{IslandConfig, VisualState};

impl Controller {
    /// Content painter: the expanded dashboard card (media body, agent
    /// live activity, task switcher, standby telemetry).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_expanded_content(
        &mut self,
        frame: &mut FrameBuffer,
        state: VisualState,
        island: &IslandConfig,
        width: u32,
        height: u32,
        now_ms: u64,
    ) {
        let cy = (height / 2) as i32;
        let accent = crate::system::accent_color_bgra();
        let now = now_ms;
        // A paused equalizer holds its pose instead of performing playback.
        let eq_now = if self.media_playing() { now } else { 0 };
        let pad = 18i32;

        let ctx = CardPaintCtx {
            width,
            height,
            cy,
            accent,
            now,
            eq_now,
            pad,
        };

        if height >= 85 {
            // Authentic tall card layout dropping vertically downward.
            self.paint_card_header(frame, state, island, &ctx);
            if self.media_playing() && island.has_widget("music") {
                self.paint_media_card(frame, island, &ctx);
            } else if state != VisualState::Idle {
                self.paint_agent_activity(frame, state, island, &ctx);
            } else if island.has_widget("tasks") && island.show_tasks && !self.tasks.is_empty() {
                self.paint_task_switcher(frame, island, &ctx);
            } else {
                self.paint_standby_dashboard(frame, island, &ctx);
            }
        } else {
            // Compact / transitioning view.
            self.paint_short_card(frame, state, island, &ctx);
        }
    }

    /// Expanded card header row: face glyph plus the hairline divider.
    pub(crate) fn paint_card_header(
        &self,
        frame: &mut FrameBuffer,
        state: VisualState,
        island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        // Authentic tall card layout dropping vertically downward
        // 1. Top Header Row
        if island.has_widget("face") {
            let face_size = 22i32;
            let fx = ctx.pad;
            let fy = 14;
            let (sc, _) = crate::animation::notch::accent_colors(state);
            crate::animation::notch::draw_disc(
                frame,
                fx + face_size / 2,
                fy + face_size / 2,
                (face_size / 2 + 2) as u32,
                [sc[0], sc[1], sc[2], 65],
            );
            crate::animation::notch::blit_rounded(
                frame,
                &self.face_frame,
                fx,
                fy,
                face_size as u32,
                face_size as u32,
                6,
            );
        }

        // Hairline glass divider
        crate::animation::notch::fill_rect_pub(
            frame,
            ctx.pad,
            40,
            ctx.width.saturating_sub((ctx.pad as u32) * 2),
            1,
            [255, 255, 255, 22],
        );
    }
}

//! Expanded-card section: agent live activity (sessions, status, beacon, session badge).

use super::super::controller::Controller;
use super::super::paint::CardPaintCtx;
use crate::animation::FrameBuffer;
use termielle_core::{IslandConfig, VisualState};

impl Controller {
    /// Expanded card section: agent live activity (sessions, status,
    /// beacon, completion accent, and session badge).
    pub(crate) fn paint_agent_activity(
        &mut self,
        frame: &mut FrameBuffer,
        state: VisualState,
        island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        // Minimal agent card: source plus state only. The protocol is
        // intentionally content-free, so extra prose would be misleading.
        let primary = self.reducer.primary_session();
        let agent_source = primary
            .as_ref()
            .map(|(s, _, _)| s.as_str())
            .unwrap_or("agent");
        let (sc, _) = crate::animation::notch::accent_colors(state);

        let tag_x = if island.has_widget("face") {
            ctx.pad + 30
        } else {
            ctx.pad
        };
        let header_title = agent_source.to_string();
        crate::animation::notch::draw_text(
            frame,
            &header_title,
            tag_x,
            16,
            ctx.width.saturating_sub((tag_x as u32) + 50),
            10,
            true,
            [sc[0], sc[1], sc[2], 255],
        );

        // Session count dots top right
        let sessions = self.reducer.session_count().clamp(1, 4);
        for i in 0..sessions {
            crate::animation::notch::draw_disc(
                frame,
                ctx.width as i32 - ctx.pad - (i as i32 * 10) - 4,
                24,
                3,
                [sc[0], sc[1], sc[2], sc[3]],
            );
        }

        let state_title = state.display_name();

        // Glowing state beacon disc
        let beacon_x = ctx.pad + 16;
        let beacon_y = 72;
        crate::animation::notch::draw_disc(
            frame,
            beacon_x,
            beacon_y,
            14,
            [sc[0], sc[1], sc[2], 45],
        );
        crate::animation::notch::draw_disc(
            frame,
            beacon_x,
            beacon_y,
            8,
            [sc[0], sc[1], sc[2], 255],
        );
        if state == VisualState::Ready {
            let age = ctx.now.saturating_sub(self.state_since_ms);
            for i in 0..8u32 {
                if let Some((sx, sy, sa)) = Self::sparkle_dot(beacon_x, beacon_y, i, age) {
                    crate::animation::notch::draw_disc(frame, sx, sy, 1, [sc[0], sc[1], sc[2], sa]);
                }
            }
        }
        let text_x = beacon_x + 24;
        let text_w = ctx.width.saturating_sub((text_x as u32) + (ctx.pad as u32));
        crate::animation::notch::draw_text(
            frame,
            state_title,
            text_x,
            52,
            text_w,
            15,
            true,
            self.ink(),
        );
    }
}

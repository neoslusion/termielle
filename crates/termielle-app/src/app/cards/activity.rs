//! Expanded-card section: agent live activity (sessions, status, beacon, sparkles, session badge).

use super::super::controller::Controller;
use super::super::paint::CardPaintCtx;
use crate::animation::FrameBuffer;
use termielle_core::{IslandConfig, VisualState};

impl Controller {
    /// Expanded card section: agent live activity (sessions, status,
    /// beacon, sparkles, session badge).
    pub(crate) fn paint_agent_activity(
        &mut self,
        frame: &mut FrameBuffer,
        state: VisualState,
        island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        // Agent Live Activity (e.g. Claude, Codex, Agy, OpenCode is running)
        let primary = self.reducer.primary_session();
        let agent_source = primary
            .as_ref()
            .map(|(s, _, _)| s.as_str())
            .unwrap_or("agent");
        let session_id = primary.as_ref().map(|(_, id, _)| id.as_str()).unwrap_or("");
        let (sc, _) = crate::animation::notch::accent_colors(state);

        let tag_x = if island.has_widget("face") {
            ctx.pad + 30
        } else {
            ctx.pad
        };
        let header_title = format!("Live Activity • {}", agent_source);
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

        // Main status body
        let (state_title, state_detail) = match state {
            VisualState::Idle => ("Termielle", "Ready for instructions"),
            VisualState::Thinking => (
                "Reasoning & Planning",
                "Analyzing context and constructing plan...",
            ),
            VisualState::Working => ("Executing Actions", "Running autonomous tools and edits..."),
            VisualState::NeedsInput => ("Action Required", "Waiting for confirmation or input"),
            VisualState::Ready => ("Turn Complete", "Task finished successfully!"),
            VisualState::Failed => ("Turn Failed", "Execution stopped with error"),
        };

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
        // Celebration sparkles after a turn completes; silent
        // once the 600 ms flight ends.
        if state == VisualState::Ready {
            let age = ctx.now.saturating_sub(self.state_since_ms);
            for i in 0..8u32 {
                if let Some((sx, sy, sa)) = Self::sparkle_dot(beacon_x, beacon_y, i, age) {
                    if sa > 0 {
                        crate::animation::notch::draw_disc(
                            frame,
                            sx,
                            sy,
                            1,
                            [sc[0], sc[1], sc[2], sa],
                        );
                    }
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
        crate::animation::notch::draw_text(
            frame,
            state_detail,
            text_x,
            76,
            text_w,
            11,
            false,
            self.ink_dim(),
        );

        // Session ID badge pill
        if !session_id.is_empty() && ctx.height >= 140 {
            let badge_text = format!("Session: {}", &session_id[..session_id.len().min(26)]);
            let bw = (badge_text.len() * 6 + 18) as u32;
            crate::animation::notch::fill_rect_pub(frame, text_x, 98, bw, 18, [255, 255, 255, 18]);
            crate::animation::notch::draw_text(
                frame,
                &badge_text,
                text_x + 6,
                101,
                bw - 10,
                10,
                false,
                self.ink_dim(),
            );
        }

        // Bottom ctx.accent pill bar
        if ctx.height >= 145 {
            let bar_w = 64u32;
            let bar_x = (ctx.width as i32 - bar_w as i32) / 2;
            crate::animation::notch::fill_rect_pub(
                frame,
                bar_x,
                ctx.height as i32 - 12,
                bar_w,
                3,
                [sc[0], sc[1], sc[2], 255],
            );
        }
    }
}

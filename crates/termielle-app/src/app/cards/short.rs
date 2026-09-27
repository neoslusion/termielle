//! Expanded-card fallback: compact transitioning view for short cards.

use super::super::controller::Controller;
use super::super::paint::CardPaintCtx;
use crate::animation::FrameBuffer;
use termielle_core::{IslandConfig, VisualState};

impl Controller {
    /// Expanded card fallback: compact transitioning view for short cards.
    pub(crate) fn paint_short_card(
        &mut self,
        frame: &mut FrameBuffer,
        state: VisualState,
        island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        // Compact / transitioning view
        let face_size = (ctx.height.saturating_sub(20)).min(36) as i32;
        if island.has_widget("face") {
            let fx = ctx.pad;
            let fy = ctx.cy - face_size / 2;
            let (sc, _) = crate::animation::notch::accent_colors(state);
            crate::animation::notch::draw_disc(
                frame,
                fx + face_size / 2,
                ctx.cy,
                (face_size / 2 + 3) as u32,
                [sc[0], sc[1], sc[2], 55],
            );
            crate::animation::notch::blit_rounded(
                frame,
                &self.face_frame,
                fx,
                fy,
                face_size as u32,
                face_size as u32,
                8,
            );
        }

        if self.media_available() && island.has_widget("music") {
            let art_size = 28i32;
            let media_start_x = ctx.pad + face_size + 14;
            let art_y = ctx.cy - art_size / 2;
            if let Some(thumb) = self.media.as_ref().and_then(|m| m.thumbnail.as_ref()) {
                crate::animation::notch::blit_rounded_pixels(
                    frame,
                    &thumb.pixels_pbgra,
                    thumb.width,
                    thumb.height,
                    media_start_x,
                    art_y,
                    art_size as u32,
                    art_size as u32,
                    6,
                );
            } else {
                crate::animation::notch::draw_disc(
                    frame,
                    media_start_x + art_size / 2,
                    ctx.cy,
                    (art_size / 2) as u32,
                    [ctx.accent[0], ctx.accent[1], ctx.accent[2], 230],
                );
            }
            let text_x = media_start_x + art_size + 10;
            let text_w = (ctx.width as i32 - ctx.pad - text_x).max(40) as u32;
            let title = self
                .media
                .as_ref()
                .map(|m| m.title.as_str())
                .unwrap_or("Playing");
            crate::animation::notch::draw_text(
                frame,
                title,
                text_x,
                ctx.cy - 7,
                text_w,
                11,
                true,
                self.ink(),
            );
        } else {
            let left_edge = ctx.pad + face_size + 14;
            let text_w = (ctx.width as i32 - ctx.pad - left_edge).max(40) as u32;
            let title = state.display_name();
            crate::animation::notch::draw_text(
                frame,
                title,
                left_edge,
                ctx.cy - 7,
                text_w,
                12,
                true,
                self.ink(),
            );
        }
    }
}

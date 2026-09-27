//! Expanded-card section: prominent media body (artwork, track, progress, transport controls).

use super::super::controller::Controller;
use super::super::paint::CardPaintCtx;
use super::super::types::{HIT_MEDIA_NEXT, HIT_MEDIA_PLAY_PAUSE, HIT_MEDIA_PREV};
use crate::animation::FrameBuffer;
use termielle_core::IslandConfig;

impl Controller {
    /// Expanded card section: prominent media body (artwork, track,
    /// progress, transport controls).
    pub(crate) fn paint_media_card(
        &mut self,
        frame: &mut FrameBuffer,
        island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        // Header title & mini equalizer in header row
        let tag_x = if island.has_widget("face") {
            ctx.pad + 30
        } else {
            ctx.pad
        };
        crate::animation::notch::draw_text(
            frame,
            "Media",
            tag_x,
            16,
            ctx.width.saturating_sub((tag_x as u32) + 50),
            10,
            true,
            self.ink_dim(),
        );

        // 4-bar mini equalizer in top right
        let eq_x = ctx.width as i32 - ctx.pad - 20;
        for (i, &phase) in [0u64, 180, 360, 540].iter().enumerate() {
            let t =
                ((ctx.eq_now.saturating_add(phase) % 700) as f32 / 700.0) * std::f32::consts::TAU;
            let h = (3.0 + 8.0 * (t + i as f32).sin().abs()).round() as i32;
            crate::animation::notch::fill_rect_pub(
                frame,
                eq_x + i as i32 * 5,
                26 - h,
                3,
                h as u32,
                ctx.accent,
            );
        }

        // Prominent Media Body Card:
        // High-res 64x64 Album Artwork
        let art_size = 64i32;
        let art_x = ctx.pad;
        let art_y = 48i32;

        if let Some(thumb) = self.media.as_ref().and_then(|m| m.thumbnail.as_ref()) {
            crate::animation::notch::blit_rounded_pixels(
                frame,
                &thumb.pixels_pbgra,
                thumb.width,
                thumb.height,
                art_x,
                art_y,
                art_size as u32,
                art_size as u32,
                12,
            );
        } else {
            crate::animation::notch::draw_rounded_rect(
                frame,
                art_x,
                art_y,
                art_size as u32,
                art_size as u32,
                12,
                [100, 100, 100, 50],
                [0; 4],
            );
            crate::animation::notch::draw_text(
                frame,
                "♪",
                art_x + 21,
                art_y + 14,
                32,
                28,
                false,
                self.ink_dim(),
            );
        }

        // Track title, artist, and app
        let text_x = art_x + art_size + 14;
        let text_w = ctx.width.saturating_sub((text_x as u32) + (ctx.pad as u32));
        let title = self
            .media
            .as_ref()
            .map(|m| m.title.as_str())
            .unwrap_or("Playing");
        let artist = self
            .media
            .as_ref()
            .map(|m| m.artist.as_str())
            .unwrap_or("Media");

        crate::animation::notch::draw_text(frame, title, text_x, 50, text_w, 14, true, self.ink());
        crate::animation::notch::draw_text(
            frame,
            artist,
            text_x,
            74,
            text_w,
            11,
            false,
            self.ink_dim(),
        );

        // Keep the transport row optically balanced and bottom-aligned. The
        // side controls intentionally share one radius; the primary control
        // gets a slightly larger target and fill to establish hierarchy.
        if ctx.height >= 165 {
            let ctrl_y = ctx.height.saturating_sub(27) as i32;
            let center_x = ctx.width as i32 / 2;
            let control_spacing = 52;
            let side_radius = 15u32;
            let primary_radius = 18u32;

            // Previous track button
            let prev_x = center_x - control_spacing;
            let prev_hover = self.hover_point.is_some_and(|(px, py)| {
                (px - prev_x).pow(2) + (py - ctrl_y).pow(2) <= (side_radius as i32).pow(2)
            });
            let btn_bg = if prev_hover {
                [255, 255, 255, 45]
            } else {
                [255, 255, 255, 25]
            };
            crate::animation::notch::draw_button_circle(
                frame,
                prev_x,
                ctrl_y,
                side_radius,
                btn_bg,
                [255, 255, 255, 60],
            );
            crate::animation::notch::draw_glyph_prev(frame, prev_x, ctrl_y, self.ink());
            self.icon_hits.push((
                HIT_MEDIA_PREV,
                prev_x - side_radius as i32 - 5,
                ctrl_y - side_radius as i32 - 5,
                (side_radius + 5) * 2,
                (side_radius + 5) * 2,
            ));

            // Play / Pause Button
            let play_hover = self.hover_point.is_some_and(|(px, py)| {
                (px - center_x).pow(2) + (py - ctrl_y).pow(2) <= (primary_radius as i32 + 2).pow(2)
            });
            let play_bg = if play_hover {
                [ctx.accent[0], ctx.accent[1], ctx.accent[2], 255]
            } else {
                [ctx.accent[0], ctx.accent[1], ctx.accent[2], 210]
            };
            crate::animation::notch::draw_button_circle(
                frame,
                center_x,
                ctrl_y,
                primary_radius,
                play_bg,
                [255, 255, 255, 100],
            );
            if self.media.as_ref().is_some_and(|m| m.playing) {
                crate::animation::notch::draw_glyph_pause(frame, center_x, ctrl_y, self.ink());
            } else {
                crate::animation::notch::draw_glyph_play(frame, center_x, ctrl_y, self.ink());
            }
            self.icon_hits.push((
                HIT_MEDIA_PLAY_PAUSE,
                center_x - primary_radius as i32 - 3,
                ctrl_y - primary_radius as i32 - 3,
                (primary_radius + 3) * 2,
                (primary_radius + 3) * 2,
            ));

            // Next track button
            let next_x = center_x + control_spacing;
            let next_hover = self.hover_point.is_some_and(|(px, py)| {
                (px - next_x).pow(2) + (py - ctrl_y).pow(2) <= (side_radius as i32).pow(2)
            });
            let btn_bg = if next_hover {
                [255, 255, 255, 45]
            } else {
                [255, 255, 255, 25]
            };
            crate::animation::notch::draw_button_circle(
                frame,
                next_x,
                ctrl_y,
                side_radius,
                btn_bg,
                [255, 255, 255, 60],
            );
            crate::animation::notch::draw_glyph_next(frame, next_x, ctrl_y, self.ink());
            self.icon_hits.push((
                HIT_MEDIA_NEXT,
                next_x - side_radius as i32 - 5,
                ctrl_y - side_radius as i32 - 5,
                (side_radius + 5) * 2,
                (side_radius + 5) * 2,
            ));
        }
    }
}

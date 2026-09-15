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
            "Now Playing",
            tag_x,
            16,
            ctx.width.saturating_sub((tag_x as u32) + 50),
            10,
            true,
            ctx.accent,
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
            crate::animation::notch::blit_rounded(
                frame,
                &crate::animation::FrameBuffer {
                    width: thumb.width,
                    height: thumb.height,
                    pixels_pbgra: thumb.pixels_pbgra.clone(),
                    delay_ms: 0,
                    loop_index: 0,
                    scale: 1.0,
                },
                art_x,
                art_y,
                art_size as u32,
                art_size as u32,
                12,
            );
        } else {
            crate::animation::notch::draw_disc(
                frame,
                art_x + art_size / 2,
                art_y + art_size / 2,
                (art_size / 2) as u32,
                [ctx.accent[0], ctx.accent[1], ctx.accent[2], 220],
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

        let app_name = self.media.as_ref().map(|m| m.app.as_str()).unwrap_or("");
        if !app_name.is_empty() {
            crate::animation::notch::draw_text(
                frame,
                app_name,
                text_x,
                94,
                text_w,
                10,
                false,
                [ctx.accent[0], ctx.accent[1], ctx.accent[2], 210],
            );
        }

        // Progress / timeline bar
        if ctx.height >= 145 {
            let track_y = 120i32;
            let track_w = ctx.width.saturating_sub((ctx.pad as u32) * 2);
            crate::animation::notch::fill_rect_pub(
                frame,
                ctx.pad,
                track_y,
                track_w,
                3,
                [255, 255, 255, 30],
            );
            crate::animation::notch::fill_rect_pub(
                frame,
                ctx.pad,
                track_y,
                (track_w as f32 * 0.45) as u32,
                3,
                ctx.accent,
            );
        }

        // Dedicated Media Controls Row: Prev (⏮), Play/Pause (⏯), Next (⏭)
        if ctx.height >= 165 {
            let ctrl_y = 148i32;
            let center_x = ctx.width as i32 / 2;

            // Previous Track Button
            let prev_x = center_x - 56;
            let prev_hover = self
                .hover_point
                .is_some_and(|(px, py)| (px - prev_x).pow(2) + (py - ctrl_y).pow(2) <= 16 * 16);
            let btn_bg = if prev_hover {
                [255, 255, 255, 45]
            } else {
                [255, 255, 255, 25]
            };
            crate::animation::notch::draw_button_circle(
                frame,
                prev_x,
                ctrl_y,
                14,
                btn_bg,
                [255, 255, 255, 60],
            );
            crate::animation::notch::draw_glyph_prev(frame, prev_x, ctrl_y, [255, 255, 255, 240]);
            self.icon_hits
                .push((HIT_MEDIA_PREV, prev_x - 16, ctrl_y - 16, 32, 32));

            // Play / Pause Button
            let play_hover = self
                .hover_point
                .is_some_and(|(px, py)| (px - center_x).pow(2) + (py - ctrl_y).pow(2) <= 19 * 19);
            let play_bg = if play_hover {
                [ctx.accent[0], ctx.accent[1], ctx.accent[2], 255]
            } else {
                [ctx.accent[0], ctx.accent[1], ctx.accent[2], 210]
            };
            crate::animation::notch::draw_button_circle(
                frame,
                center_x,
                ctrl_y,
                17,
                play_bg,
                [255, 255, 255, 100],
            );
            if self.media.as_ref().is_some_and(|m| m.playing) {
                crate::animation::notch::draw_glyph_pause(frame, center_x, ctrl_y, self.ink());
            } else {
                crate::animation::notch::draw_glyph_play(frame, center_x, ctrl_y, self.ink());
            }
            self.icon_hits
                .push((HIT_MEDIA_PLAY_PAUSE, center_x - 18, ctrl_y - 18, 36, 36));

            // Next Track Button
            let next_x = center_x + 56;
            let next_hover = self
                .hover_point
                .is_some_and(|(px, py)| (px - next_x).pow(2) + (py - ctrl_y).pow(2) <= 16 * 16);
            let btn_bg = if next_hover {
                [255, 255, 255, 45]
            } else {
                [255, 255, 255, 25]
            };
            crate::animation::notch::draw_button_circle(
                frame,
                next_x,
                ctrl_y,
                14,
                btn_bg,
                [255, 255, 255, 60],
            );
            crate::animation::notch::draw_glyph_next(frame, next_x, ctrl_y, [255, 255, 255, 240]);
            self.icon_hits
                .push((HIT_MEDIA_NEXT, next_x - 16, ctrl_y - 16, 32, 32));
        }
    }
}

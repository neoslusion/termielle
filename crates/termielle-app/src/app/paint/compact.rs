//! Compact presentation: face plus live agent dots, or the idle label.

use super::super::controller::Controller;
use super::super::types::HIT_MEDIA_PLAY_PAUSE;
use crate::animation::FrameBuffer;
use termielle_core::{IslandConfig, VisualState};

impl Controller {
    /// Content painter: the compact live pill (face, status, media live
    /// activity, session dots), including the two-blob split layout.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_compact_content(
        &mut self,
        frame: &mut FrameBuffer,
        state: VisualState,
        island: &IslandConfig,
        width: u32,
        height: u32,
        blobs: &[crate::animation::notch::BlobRect],
        now_ms: u64,
    ) {
        let cy = (height / 2) as i32;
        let accent = crate::system::accent_color_bgra();
        let now = now_ms;
        // A paused equalizer holds its pose instead of performing playback.
        let eq_now = if self.media_playing() { now } else { 0 };
        if blobs.len() > 1 {
            // The Dynamic Island split: agent content leads in the
            // primary blob, media lives in the detached blob, and
            // the liquid bridge between them is drawn by the
            // material. Clicks on the media blob toggle playback.
            let (primary, media_blob) = (blobs[0], blobs[1]);
            if island.has_widget("face") {
                let face_size = (height.saturating_sub(8)).min(28) as i32;
                let face_x = primary.x + 12;
                if state != VisualState::Idle {
                    let (sc, _) = crate::animation::notch::accent_colors(state);
                    crate::animation::notch::draw_disc(
                        frame,
                        face_x + face_size / 2,
                        cy,
                        (face_size / 2 + 2) as u32,
                        [sc[0], sc[1], sc[2], 50],
                    );
                }
                crate::animation::notch::blit_scaled(
                    frame,
                    &self.face_frame,
                    face_x,
                    cy - face_size / 2,
                    face_size as u32,
                    face_size as u32,
                );
            }
            if state != VisualState::Idle || self.reducer.session_count() > 0 {
                let (dot, _) = crate::animation::notch::accent_colors(state);
                let count = if island.has_widget("agents") {
                    self.reducer.session_count().clamp(1, 4)
                } else {
                    1
                };
                let mut dot_x = primary.x + primary.w as i32 - 16;
                for i in 0..count {
                    let dy = Self::think_bob(state, i, now);
                    crate::animation::notch::draw_disc(frame, dot_x, cy + dy, 3, dot);
                    dot_x -= 10;
                }
            }
            // Media blob: album art plus the equalizer bars.
            let art_size = (height.saturating_sub(12)).min(22) as i32;
            let art_x = media_blob.x + 10;
            if let Some(thumb) = self.media.as_ref().and_then(|m| m.thumbnail.as_ref()) {
                crate::animation::notch::blit_rounded_pixels(
                    frame,
                    &thumb.pixels_pbgra,
                    thumb.width,
                    thumb.height,
                    art_x,
                    cy - art_size / 2,
                    art_size as u32,
                    art_size as u32,
                    6,
                );
            } else {
                crate::animation::notch::draw_disc(
                    frame,
                    art_x + art_size / 2,
                    cy,
                    (art_size / 2) as u32,
                    accent,
                );
            }
            let eq_x = art_x + art_size + 6;
            for (i, &phase) in [0u64, 180, 360].iter().enumerate() {
                let t =
                    ((eq_now.saturating_add(phase) % 800) as f32 / 800.0) * std::f32::consts::TAU;
                let h = (3.0 + 8.0 * (t + i as f32).sin().abs()).round() as i32;
                crate::animation::notch::fill_rect_pub(
                    frame,
                    eq_x + i as i32 * 4,
                    cy + 4 - h,
                    2,
                    h as u32,
                    accent,
                );
            }
            self.icon_hits
                .push((HIT_MEDIA_PLAY_PAUSE, media_blob.x, 0, media_blob.w, height));
        } else {
            // Split Dynamic Island: Leading face, Trailing activity
            if island.has_widget("face") {
                let face_size = (height.saturating_sub(8)).min(28) as i32;
                let face_x = 12;
                if state != VisualState::Idle {
                    let (sc, _) = crate::animation::notch::accent_colors(state);
                    crate::animation::notch::draw_disc(
                        frame,
                        face_x + face_size / 2,
                        cy,
                        (face_size / 2 + 2) as u32,
                        [sc[0], sc[1], sc[2], 50],
                    );
                }
                crate::animation::notch::blit_scaled(
                    frame,
                    &self.face_frame,
                    face_x,
                    cy - face_size / 2,
                    face_size as u32,
                    face_size as u32,
                );
            }

            let mut right_cursor = width as i32 - 14;

            // Media equalizer, with the album art leading it
            if self.media_available() && island.has_widget("music") {
                let bx = right_cursor - 14;
                let art_size = (height.saturating_sub(12)).min(22) as i32;
                let art_x = bx - art_size - 6;
                if let Some(thumb) = self.media.as_ref().and_then(|m| m.thumbnail.as_ref()) {
                    crate::animation::notch::blit_rounded_pixels(
                        frame,
                        &thumb.pixels_pbgra,
                        thumb.width,
                        thumb.height,
                        art_x,
                        cy - art_size / 2,
                        art_size as u32,
                        art_size as u32,
                        6,
                    );
                } else {
                    crate::animation::notch::draw_disc(
                        frame,
                        art_x + art_size / 2,
                        cy,
                        (art_size / 2) as u32,
                        [accent[0], accent[1], accent[2], 230],
                    );
                }
                let base = cy + 7;
                for (i, &phase) in [0u64, 180, 360].iter().enumerate() {
                    let t = ((eq_now.saturating_add(phase) % 800) as f32 / 800.0)
                        * std::f32::consts::TAU;
                    let h = (3.0 + 8.0 * (t + i as f32).sin().abs()).round() as i32;
                    crate::animation::notch::fill_rect_pub(
                        frame,
                        bx + i as i32 * 5,
                        base - h,
                        3,
                        h as u32,
                        accent,
                    );
                }
                self.icon_hits
                    .push((HIT_MEDIA_PLAY_PAUSE, art_x - 2, cy - 10, 44, 20));
                right_cursor -= 22;
            }

            // Agent status beacon and dots
            if state != VisualState::Idle || self.reducer.session_count() > 0 {
                let (dot, _) = crate::animation::notch::accent_colors(state);
                let count = if island.has_widget("agents") {
                    self.reducer.session_count().clamp(1, 4)
                } else {
                    1
                };
                for i in 0..count {
                    let dy = Self::think_bob(state, i, now);
                    right_cursor -= 10;
                    crate::animation::notch::draw_disc(
                        frame,
                        right_cursor + 4,
                        cy + dy,
                        3,
                        [dot[0], dot[1], dot[2], dot[3]],
                    );
                }
            } else if state == VisualState::Idle && !self.media_available() {
                let text_x = self.compact_label_x() as i32;
                let text_w = width.saturating_sub((text_x as u32) + 24);
                if island.show_name {
                    crate::animation::notch::draw_text(
                        frame,
                        "Termielle",
                        text_x,
                        cy - 6,
                        text_w,
                        11,
                        true,
                        self.ink_dim(),
                    );
                }
                crate::animation::notch::draw_disc(
                    frame,
                    width as i32 - 16,
                    cy,
                    3,
                    [accent[0], accent[1], accent[2], 180],
                );
            }
        }
    }
}

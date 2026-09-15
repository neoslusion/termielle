//! Minimal presentation: the idle dot/face painter.

use super::super::controller::Controller;
use crate::animation::FrameBuffer;
use termielle_core::IslandConfig;

impl Controller {
    /// Content painter: the minimal dot/face presentation.
    pub(crate) fn paint_minimal_content(
        &mut self,
        frame: &mut FrameBuffer,
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
        if self.media_playing() && island.has_widget("music") {
            let art_size = (height.saturating_sub(12)).min(22) as i32;
            let art_x = (width as i32 / 2) - 22;
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
                    [accent[0], accent[1], accent[2], 255],
                );
            }
            // Mini equalizer bars
            let eq_x = (width as i32 / 2) + 6;
            for (i, &phase) in [0u64, 180, 360].iter().enumerate() {
                let t =
                    ((eq_now.saturating_add(phase) % 800) as f32 / 800.0) * std::f32::consts::TAU;
                let h = (3.0 + 5.0 * (t + i as f32).sin().abs()).round() as i32;
                crate::animation::notch::fill_rect_pub(
                    frame,
                    eq_x + i as i32 * 4,
                    cy + 4 - h,
                    2,
                    h as u32,
                    accent,
                );
            }
            self.icon_hits.push((0, art_x - 2, cy - 10, 44, 20));
        } else if island.has_widget("face") {
            let face_size = (height.saturating_sub(8)).min(32) as i32;
            crate::animation::notch::blit_scaled(
                frame,
                &self.face_frame,
                (width as i32 - face_size) / 2,
                cy - face_size / 2,
                face_size as u32,
                face_size as u32,
            );
        }
    }
}

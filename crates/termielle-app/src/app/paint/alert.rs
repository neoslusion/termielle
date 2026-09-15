//! Alert banner: transient notification card with timeout hairline.

use super::super::controller::Controller;
use crate::animation::FrameBuffer;
use termielle_core::IslandConfig;

impl Controller {
    /// Content painter 0: the active notification alert banner. Returns
    /// true when a banner showed (the caller returns early); the cloned
    /// alert unties the `alerts` borrow from the paint calls.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_alert_banner(
        &mut self,
        frame: &mut FrameBuffer,
        island: &IslandConfig,
        width: u32,
        height: u32,
        now_ms: u64,
    ) -> bool {
        let cy = (height / 2) as i32;
        let now = now_ms;
        // 0. Active Notification Alert Banner
        if let Some(alert) = self.alerts.front() {
            let pad = 18i32;

            if height >= 85 {
                // Tall card layout: drops vertically downward
                if island.has_widget("face") {
                    let face_size = 22i32;
                    let fx = pad;
                    let fy = 14;
                    crate::animation::notch::draw_disc(
                        frame,
                        fx + face_size / 2,
                        fy + face_size / 2,
                        (face_size / 2 + 2) as u32,
                        [alert.accent[0], alert.accent[1], alert.accent[2], 65],
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

                let tag_x = if island.has_widget("face") {
                    pad + 30
                } else {
                    pad
                };
                crate::animation::notch::draw_text(
                    frame,
                    "System Notification",
                    tag_x,
                    15,
                    width.saturating_sub((tag_x as u32) + 40),
                    10,
                    true,
                    [alert.accent[0], alert.accent[1], alert.accent[2], 255],
                );

                // Trailing beacon badge with soft glowing halo
                crate::animation::notch::draw_disc(
                    frame,
                    width as i32 - 24,
                    22,
                    7,
                    [alert.accent[0], alert.accent[1], alert.accent[2], 50],
                );
                crate::animation::notch::draw_disc(frame, width as i32 - 24, 22, 4, alert.accent);

                // Hairline glass separator
                crate::animation::notch::fill_rect_pub(
                    frame,
                    pad,
                    38,
                    width.saturating_sub((pad as u32) * 2),
                    1,
                    [255, 255, 255, 22],
                );

                // Main headline and detail text
                let text_w = width.saturating_sub((pad as u32) * 2);
                crate::animation::notch::draw_text(
                    frame,
                    &alert.title,
                    pad,
                    48,
                    text_w,
                    15,
                    true,
                    self.ink(),
                );
                crate::animation::notch::draw_text(
                    frame,
                    &alert.subtitle,
                    pad,
                    76,
                    text_w,
                    12,
                    false,
                    self.ink_dim(),
                );

                // Timeout hairline: the banner's remaining life, so the
                // auto-dismiss reads as intentional rather than a flicker.
                let frac = if alert.duration_ms == 0 {
                    0.0
                } else {
                    alert.expires_at_ms.saturating_sub(now) as f32 / alert.duration_ms as f32
                }
                .clamp(0.0, 1.0);
                let hair_w =
                    ((width.saturating_sub((pad as u32) * 2)) as f32 * frac).round() as u32;
                if hair_w > 0 {
                    crate::animation::notch::fill_rect_pub(
                        frame,
                        pad,
                        height as i32 - 4,
                        hair_w,
                        2,
                        [alert.accent[0], alert.accent[1], alert.accent[2], 200],
                    );
                }

                // Bottom accent pill bar
                if height >= 105 {
                    let bar_w = 44u32;
                    let bar_x = (width as i32 - bar_w as i32) / 2;
                    crate::animation::notch::fill_rect_pub(
                        frame,
                        bar_x,
                        height as i32 - 10,
                        bar_w,
                        3,
                        alert.accent,
                    );
                }
            } else {
                let face_size = (height.saturating_sub(18)).min(36) as i32;
                let fx = pad;
                let fy = cy - face_size / 2;

                if island.has_widget("face") {
                    crate::animation::notch::draw_disc(
                        frame,
                        fx + face_size / 2,
                        cy,
                        (face_size / 2 + 3) as u32,
                        [alert.accent[0], alert.accent[1], alert.accent[2], 65],
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

                let text_x = if island.has_widget("face") {
                    fx + face_size + 14
                } else {
                    pad
                };
                let text_w = width.saturating_sub((text_x as u32) + 36);

                crate::animation::notch::draw_text(
                    frame,
                    &alert.title,
                    text_x,
                    cy - 7,
                    text_w,
                    12,
                    true,
                    [255, 255, 255, 245],
                );

                // Trailing beacon badge
                crate::animation::notch::draw_disc(frame, width as i32 - 20, cy, 5, alert.accent);
            }

            return true;
        }
        false
    }
}

use super::super::controller::Controller;
use crate::animation::{FrameBuffer, icons, notch};

impl Controller {
    pub(crate) fn paint_bar_volume_feedback(
        &self,
        frame: &mut FrameBuffer,
        (pill_x, pill_y, pill_w, pill_h): (i32, i32, u32, u32),
    ) {
        let Some(metrics) = &self.bar_metrics_cache else {
            return;
        };
        let volume = metrics.volume;
        let (primary, secondary) = notch::ink_pair(&self.island.glass);
        let icon_size = pill_h.saturating_sub(4).min(18);
        let icon_x = pill_x + 10;
        icons::draw_icon(
            frame,
            if volume.muted {
                icons::VOLUME_MUTED
            } else {
                icons::VOLUME
            },
            icon_x,
            pill_y + ((pill_h - icon_size) / 2) as i32,
            icon_size,
            primary,
        );
        let label = if volume.muted {
            "Muted".to_string()
        } else {
            format!("{}%", volume.level)
        };
        let label_w = 44.min(pill_w);
        notch::draw_text_in_rect(
            frame,
            &label,
            (
                pill_x + pill_w as i32 - label_w as i32 - 8,
                pill_y,
                label_w,
                pill_h,
            ),
            12,
            true,
            primary,
            true,
        );
        let track_x = icon_x + icon_size as i32 + 10;
        let track_y = pill_y + (pill_h.saturating_sub(4) / 2) as i32;
        let track_w = pill_w.saturating_sub(10 + icon_size + 10 + label_w + 16);
        notch::draw_rounded_rect(frame, track_x, track_y, track_w, 4, 2, secondary, [0; 4]);
        let fill_w = if volume.muted {
            0
        } else {
            track_w * u32::from(volume.level.min(100)) / 100
        };
        if fill_w > 0 {
            notch::draw_rounded_rect(frame, track_x, track_y, fill_w, 4, 2, primary, [0; 4]);
        }
    }
}

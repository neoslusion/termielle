use super::super::{controller::Controller, types::HIT_NOTCH_CLOSE};
use crate::animation::{FrameBuffer, notch};

impl Controller {
    pub(crate) fn paint_notch_close(&mut self, frame: &mut FrameBuffer, width: u32) {
        let rect = (width as i32 - 44, 9, 28, 28);
        let hovered = self
            .hover_point
            .is_some_and(|(x, y)| x >= rect.0 && x < rect.0 + 28 && y >= rect.1 && y < rect.1 + 28);
        let palette = crate::theme::Palette::for_island(&self.island);
        let pressed = self.rect_pressed((rect.0, rect.1, 28, 28));
        notch::draw_rounded_rect(
            frame,
            rect.0,
            rect.1,
            28,
            28,
            14,
            if pressed {
                palette.pressed(palette.raised)
            } else if hovered {
                palette.raised
            } else {
                palette.surface
            },
            palette.border,
        );
        notch::draw_text_in_rect(frame, "×", rect, 16, false, palette.primary, true);
        self.icon_hits
            .push((HIT_NOTCH_CLOSE, rect.0, rect.1, rect.2, rect.3));
    }
}

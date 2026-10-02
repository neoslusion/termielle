use super::super::controller::Controller;
use super::super::paint::CardPaintCtx;
use super::super::types::{HIT_CARD_NOTIFICATIONS, HIT_NOTIFICATIONS_CLEAR};
use crate::animation::{FrameBuffer, notch};

const PAD: i32 = 16;
const FIRST_ROW_Y: i32 = 62;
const ROW_H: i32 = 72;
const ROW_GAP: i32 = 8;
const FOOTER_H: u32 = 34;
fn footer_y(count: usize) -> i32 {
    FIRST_ROW_Y + count as i32 * ROW_H + count.saturating_sub(1) as i32 * ROW_GAP + 12
}

pub(crate) fn panel_height(count: usize) -> u32 {
    (footer_y(count) + FOOTER_H as i32 + 12).max(180) as u32
}

impl Controller {
    pub(crate) fn paint_notification_panel(&mut self, frame: &mut FrameBuffer, ctx: &CardPaintCtx) {
        let (primary, secondary) = notch::ink_pair(&self.island.glass);
        let inner_width = ctx.width.saturating_sub(PAD as u32 * 2);
        notch::draw_text(
            frame,
            "Notifications",
            PAD,
            14,
            inner_width.saturating_sub(40),
            14,
            true,
            primary,
        );
        let close_x = ctx.width as i32 - PAD - 28;
        notch::draw_rounded_rect(frame, close_x, 10, 28, 28, 14, [255, 255, 255, 30], [0; 4]);
        notch::draw_text_in_rect(frame, "×", (close_x, 10, 28, 28), 15, false, primary, true);
        self.icon_hits
            .push((HIT_CARD_NOTIFICATIONS, close_x, 10, 28, 28));
        notch::draw_text(
            frame,
            "Recent in Termielle",
            PAD,
            37,
            inner_width,
            10,
            false,
            secondary,
        );
        if self.recent_notifications.is_empty() {
            notch::draw_text(
                frame,
                "You're all caught up",
                PAD + 12,
                112,
                inner_width.saturating_sub(24),
                13,
                false,
                secondary,
            );
            return;
        }
        for (index, entry) in self.recent_notifications.iter().enumerate() {
            let y = FIRST_ROW_Y + index as i32 * (ROW_H + ROW_GAP);
            notch::draw_rounded_rect(
                frame,
                PAD,
                y,
                inner_width,
                ROW_H as u32,
                12,
                [255, 255, 255, 20],
                [0; 4],
            );
            notch::draw_rounded_rect(frame, PAD + 11, y + 17, 5, 38, 3, entry.accent, [0; 4]);
            let text_width = inner_width.saturating_sub(42);
            notch::draw_text(
                frame,
                &entry.source,
                PAD + 27,
                y + 6,
                text_width.saturating_sub(48),
                9,
                true,
                secondary,
            );
            notch::draw_text(
                frame,
                &entry.received_at,
                ctx.width as i32 - PAD - 43,
                y + 6,
                38,
                9,
                false,
                secondary,
            );
            notch::draw_text(
                frame,
                &entry.title,
                PAD + 27,
                y + 25,
                text_width,
                12,
                true,
                primary,
            );
            let subtitle = if entry.subtitle == entry.source {
                entry.kind.label()
            } else {
                entry
                    .subtitle
                    .strip_prefix(entry.source.as_str())
                    .and_then(|body| body.strip_prefix(": "))
                    .unwrap_or(&entry.subtitle)
            };
            notch::draw_text(
                frame,
                subtitle,
                PAD + 27,
                y + 46,
                text_width,
                10,
                false,
                secondary,
            );
        }
        let clear_y = footer_y(self.recent_notifications.len());
        notch::draw_rounded_rect(
            frame,
            PAD,
            clear_y,
            inner_width,
            FOOTER_H,
            10,
            [255, 255, 255, 32],
            [0; 4],
        );
        notch::draw_text_in_rect(
            frame,
            "Clear all notifications",
            (PAD, clear_y, inner_width, FOOTER_H),
            11,
            true,
            primary,
            true,
        );
        self.icon_hits
            .push((HIT_NOTIFICATIONS_CLEAR, PAD, clear_y, inner_width, FOOTER_H));
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::types::MAX_RECENT_NOTIFICATIONS;
    use super::*;

    #[test]
    fn notification_rows_fit_above_clear_action() {
        for count in 1..=MAX_RECENT_NOTIFICATIONS {
            let bottom = FIRST_ROW_Y + (count as i32 - 1) * (ROW_H + ROW_GAP) + ROW_H;
            assert!(bottom < footer_y(count));
            assert!(footer_y(count) + (FOOTER_H as i32) < panel_height(count) as i32);
        }
        assert_eq!(panel_height(0), 180);
    }
}

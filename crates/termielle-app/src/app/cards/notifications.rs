use super::super::controller::Controller;
use super::super::paint::CardPaintCtx;
use super::super::types::{HIT_CARD_NOTIFICATIONS, HIT_NOTIFICATIONS_CLEAR};
use crate::animation::{FrameBuffer, notch};

const PAD: i32 = 16;
const FIRST_ROW_Y: i32 = 62;
const ROW_H: i32 = 96;
const ROW_GAP: i32 = 8;
const FOOTER_H: u32 = 34;
const VISIBLE_ROWS: usize = 3;

fn footer_y() -> i32 {
    FIRST_ROW_Y + VISIBLE_ROWS as i32 * ROW_H + (VISIBLE_ROWS - 1) as i32 * ROW_GAP + 12
}

fn scroll_limit(count: usize) -> usize {
    count.saturating_sub(VISIBLE_ROWS)
}

pub(crate) fn panel_height(count: usize) -> u32 {
    if count == 0 {
        180
    } else {
        (footer_y() + FOOTER_H as i32 + 12) as u32
    }
}

impl Controller {
    pub fn scroll_notifications(
        &mut self,
        physical_x: i32,
        physical_y: i32,
        delta: i16,
        now_ms: u64,
    ) -> Option<bool> {
        if !self.island.is_bar() || !self.is_notification_center_open() {
            return None;
        }
        let point = (self.to_logical(physical_x), self.to_logical(physical_y));
        let (width, height) = self.current_logical_size();
        let expanded =
            height.saturating_sub(self.island.bar.height + crate::app::bar::types::BAR_POPUP_GAP);
        let bar_y = if self.island.bar.position == termielle_core::BarPosition::Top {
            0
        } else {
            height.saturating_sub(self.island.bar.height) as i32
        };
        let rect = self.bar_panel_rect(
            width,
            expanded,
            bar_y,
            self.island.expanded_width.min(width.saturating_sub(32)),
        );
        if point.0 < rect.0
            || point.0 >= rect.0 + rect.2 as i32
            || point.1 < rect.1
            || point.1 >= rect.1 + rect.3 as i32
        {
            return None;
        }
        let previous = self.notification_scroll;
        self.notification_scroll = if delta < 0 {
            previous
                .saturating_add(1)
                .min(scroll_limit(self.recent_notifications.len()))
        } else if delta > 0 {
            previous.saturating_sub(1)
        } else {
            previous
        };
        let changed = previous != self.notification_scroll;
        if changed {
            self.current = self.render_island(self.state, width, height, now_ms);
        }
        Some(changed)
    }

    pub(crate) fn paint_notification_panel(&mut self, frame: &mut FrameBuffer, ctx: &CardPaintCtx) {
        let palette = crate::theme::Palette::for_island(&self.island);
        let (primary, secondary) = (palette.primary, palette.secondary);
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
        notch::draw_rounded_rect(
            frame,
            close_x,
            10,
            28,
            28,
            14,
            palette.raised,
            palette.border,
        );
        notch::draw_text_in_rect(frame, "×", (close_x, 10, 28, 28), 15, false, primary, true);
        self.icon_hits
            .push((HIT_CARD_NOTIFICATIONS, close_x, 10, 28, 28));
        self.notification_scroll = self
            .notification_scroll
            .min(scroll_limit(self.recent_notifications.len()));
        let subtitle = if self.recent_notifications.len() > VISIBLE_ROWS {
            format!(
                "{}–{} of {} recent · Scroll to browse",
                self.notification_scroll + 1,
                (self.notification_scroll + VISIBLE_ROWS).min(self.recent_notifications.len()),
                self.recent_notifications.len()
            )
        } else {
            format!("{} recent in Termielle", self.recent_notifications.len())
        };
        notch::draw_text(frame, &subtitle, PAD, 37, inner_width, 10, false, secondary);
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
        for (index, entry) in self
            .recent_notifications
            .iter()
            .skip(self.notification_scroll)
            .take(VISIBLE_ROWS)
            .enumerate()
        {
            let y = FIRST_ROW_Y + index as i32 * (ROW_H + ROW_GAP);
            notch::draw_rounded_rect(
                frame,
                PAD,
                y,
                inner_width,
                ROW_H as u32,
                12,
                palette.surface,
                palette.border,
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
            for (line_index, line) in crate::animation::text_layout::lines(
                &entry.title,
                text_width,
                13,
                true,
                frame.scale,
                2,
            )
            .iter()
            .enumerate()
            {
                notch::draw_text(
                    frame,
                    line,
                    PAD + 27,
                    y + 26 + line_index as i32 * 18,
                    text_width,
                    13,
                    true,
                    primary,
                );
            }
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
                y + 70,
                text_width,
                10,
                false,
                secondary,
            );
        }
        let clear_y = footer_y();
        let hovered = self.hover_point.is_some_and(|(hover_x, hover_y)| {
            hover_x >= PAD
                && hover_x < PAD + inner_width as i32
                && hover_y >= clear_y
                && hover_y < clear_y + FOOTER_H as i32
        });
        notch::draw_rounded_rect(
            frame,
            PAD,
            clear_y,
            inner_width,
            FOOTER_H,
            10,
            if hovered {
                palette.selection
            } else {
                palette.raised
            },
            if hovered {
                palette.accent
            } else {
                palette.border
            },
        );
        notch::draw_text_in_rect(
            frame,
            "Clear all notifications",
            (PAD, clear_y, inner_width, FOOTER_H),
            11,
            true,
            if hovered { [255; 4] } else { primary },
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

    fn notification_drawer(position: termielle_core::BarPosition, scale: f32) -> Controller {
        let mut island = termielle_core::IslandConfig {
            layout: termielle_core::IslandLayout::Bar,
            face_animated: false,
            ..Default::default()
        };
        island.bar.position = position;
        let mut controller = Controller::new_with_island(
            5_000,
            60_000,
            termielle_core::AssetCatalog::new(Vec::new()),
            true,
            None,
            island,
        );
        controller.set_bar_width(900);
        controller.set_dpi_scale(scale);
        for index in 0..MAX_RECENT_NOTIFICATIONS {
            controller
                .recent_notifications
                .push_front(crate::app::types::RecentNotification {
                    source: "Messages".into(),
                    received_at: "12:00".into(),
                    title: format!("Review message {index}"),
                    subtitle: "A bounded history row".into(),
                    accent: [200, 120, 80, 255],
                    kind: crate::app::types::AlertKind::System,
                    dedupe_key: format!("scroll:{index}"),
                });
        }
        controller.panel_open = true;
        controller.panel_kind = crate::app::types::RightPanel::Notifications;
        controller.morph_to_target(1_000);
        controller
    }

    #[test]
    fn notification_scrolling_is_bounded_without_moving_clear_all() {
        for position in [
            termielle_core::BarPosition::Top,
            termielle_core::BarPosition::Bottom,
        ] {
            for scale in [1.0, 1.5, 2.0] {
                let mut controller = notification_drawer(position, scale);
                let before = controller
                    .click_regions()
                    .iter()
                    .find(|hit| hit.0 == HIT_NOTIFICATIONS_CLEAR)
                    .copied()
                    .unwrap();
                let point = (
                    ((before.1 as f32 + before.3 as f32 / 2.0) * scale).round() as i32,
                    ((before.2 as f32 + before.4 as f32 / 2.0) * scale).round() as i32,
                );
                let pixels = controller.current_frame().pixels_pbgra.clone();
                assert_eq!(
                    controller.scroll_notifications(point.0, point.1, -120, 1_100),
                    Some(true)
                );
                assert_eq!(controller.notification_scroll, 1);
                assert_ne!(pixels, controller.current_frame().pixels_pbgra);
                let after = controller
                    .click_regions()
                    .iter()
                    .find(|hit| hit.0 == HIT_NOTIFICATIONS_CLEAR)
                    .copied()
                    .unwrap();
                assert_eq!(before, after);
                assert_eq!(
                    controller.scroll_notifications(point.0, point.1, -120, 1_110),
                    Some(false)
                );
                assert_eq!(
                    controller.scroll_notifications(point.0, point.1, 0, 1_120),
                    Some(false)
                );
                assert_eq!(controller.scroll_notifications(0, 0, -120, 1_130), None);
                assert_eq!(
                    controller.scroll_notifications(point.0, point.1, 120, 1_140),
                    Some(true)
                );
                assert_eq!(controller.notification_scroll, 0);
                controller.scroll_notifications(point.0, point.1, -120, 1_150);
                assert_eq!(
                    controller.handle_click(point.0, point.1, 1_160),
                    crate::app::types::ClickOutcome::NotificationsCleared
                );
                assert_eq!(controller.notification_scroll, 0);
                assert!(controller.recent_notifications.is_empty());
                controller.open_control_panel(1_170);
                assert_eq!(
                    controller.scroll_notifications(point.0, point.1, -120, 1_180),
                    None
                );
            }
        }
    }

    #[test]
    fn clear_all_hover_uses_panel_local_coordinates() {
        let mut controller = notification_drawer(termielle_core::BarPosition::Top, 1.5);
        let clear = controller
            .click_regions()
            .iter()
            .find(|hit| hit.0 == HIT_NOTIFICATIONS_CLEAR)
            .copied()
            .unwrap();
        let point = (
            ((clear.1 as f32 + clear.3 as f32 / 2.0) * 1.5).round() as i32,
            ((clear.2 as f32 + clear.4 as f32 / 2.0) * 1.5).round() as i32,
        );
        let before = controller.current_frame().pixels_pbgra.clone();
        assert!(controller.set_hover_point_at(Some(point), 1_010));
        assert_ne!(before, controller.current_frame().pixels_pbgra);
        let hovered = controller.current_frame().pixels_pbgra.clone();
        assert!(!controller.set_hover_point_at(Some((point.0 + 1, point.1)), 1_020));
        assert_eq!(hovered, controller.current_frame().pixels_pbgra);
    }

    #[test]
    fn notification_rows_fit_above_clear_action() {
        for count in 1..=MAX_RECENT_NOTIFICATIONS {
            let visible = count.min(VISIBLE_ROWS);
            let bottom = FIRST_ROW_Y + (visible as i32 - 1) * (ROW_H + ROW_GAP) + ROW_H;
            assert!(bottom < footer_y());
            assert!(footer_y() + (FOOTER_H as i32) < panel_height(count) as i32);
            assert_eq!(panel_height(count), panel_height(1));
        }
        assert_eq!(scroll_limit(MAX_RECENT_NOTIFICATIONS), 1);
        assert_eq!(panel_height(0), 180);
    }
}

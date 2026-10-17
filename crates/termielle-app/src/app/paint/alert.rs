//! Transient notification content for the island and bar.

use super::super::controller::Controller;
use super::super::types::{AlertBanner, AlertKind, HIT_ALERT_DISMISS};
use crate::animation::FrameBuffer;
use crate::animation::notch::{
    blit_rounded, draw_rounded_rect, draw_text, draw_text_in_rect, fill_rect_pub,
};
use termielle_core::IslandConfig;

fn alert_copy(alert: &AlertBanner, show_name: bool) -> (&str, &str, &str) {
    match alert.kind {
        AlertKind::Agent => {
            let source = if alert.subtitle.trim().is_empty() {
                if show_name { "Termielle" } else { "Agent" }
            } else {
                alert.subtitle.as_str()
            };
            let headline = match alert.title.as_str() {
                "Input" => "Needs your input",
                "Failed" => "Turn failed",
                _ => alert.title.as_str(),
            };
            (source, headline, "")
        }
        AlertKind::System => {
            let (source, body) =
                if !alert.source.trim().is_empty() && alert.source != alert.kind.label() {
                    let source = alert.source.as_str();
                    let body = if alert.subtitle == source {
                        ""
                    } else {
                        alert
                            .subtitle
                            .strip_prefix(source)
                            .and_then(|body| body.strip_prefix(": "))
                            .unwrap_or(&alert.subtitle)
                    };
                    (source, body)
                } else {
                    alert
                        .subtitle
                        .split_once(": ")
                        .unwrap_or((alert.subtitle.as_str(), ""))
                };
            let source = if source.trim().is_empty() {
                "Notification"
            } else {
                source
            };
            if alert.title.eq_ignore_ascii_case(source) && !body.is_empty() {
                (source, body, "")
            } else {
                (source, alert.title.as_str(), body)
            }
        }
    }
}

impl Controller {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_alert_banner(
        &mut self,
        frame: &mut FrameBuffer,
        island: &IslandConfig,
        width: u32,
        height: u32,
        now_ms: u64,
    ) -> bool {
        let Some(alert) = self.alerts.front() else {
            return false;
        };
        let (source, headline, detail) = alert_copy(alert, island.show_name);
        let accent = alert.accent;
        let palette = crate::theme::Palette::for_island(island);
        let close_x = width as i32 - 44;
        draw_rounded_rect(
            frame,
            close_x,
            9,
            28,
            28,
            14,
            palette.surface,
            palette.border,
        );
        draw_text_in_rect(
            frame,
            "×",
            (close_x, 9, 28, 28),
            16,
            false,
            palette.primary,
            true,
        );
        let avatar_x = 16;
        let avatar_y = 42;
        draw_rounded_rect(
            frame,
            avatar_x,
            avatar_y,
            40,
            40,
            12,
            [accent[0], accent[1], accent[2], 42],
            [accent[0], accent[1], accent[2], 65],
        );
        if alert.kind == AlertKind::Agent && island.has_widget("face") {
            blit_rounded(
                frame,
                &self.face_frame,
                avatar_x + 5,
                avatar_y + 5,
                30,
                30,
                8,
            );
        } else {
            let initial = source
                .chars()
                .next()
                .unwrap_or('N')
                .to_uppercase()
                .to_string();
            draw_text_in_rect(
                frame,
                &initial,
                (avatar_x, avatar_y, 40, 40),
                19,
                true,
                palette.primary,
                true,
            );
        }
        let text_x = 68;
        let text_w = width.saturating_sub(text_x as u32 + 18);
        draw_text(
            frame,
            source,
            text_x,
            19,
            text_w.saturating_sub(30),
            10,
            true,
            palette.secondary,
        );
        for (index, line) in
            crate::animation::text_layout::lines(headline, text_w, 16, true, frame.scale, 2)
                .iter()
                .enumerate()
        {
            draw_text(
                frame,
                line,
                text_x,
                42 + index as i32 * 21,
                text_w,
                16,
                true,
                palette.primary,
            );
        }
        for (index, line) in
            crate::animation::text_layout::lines(detail, text_w, 12, false, frame.scale, 2)
                .iter()
                .enumerate()
        {
            draw_text(
                frame,
                line,
                text_x,
                87 + index as i32 * 15,
                text_w,
                12,
                false,
                palette.secondary,
            );
        }
        let footer_y = height.saturating_sub(32) as i32;
        if island.is_bar() {
            let button_w = text_w.min(126);
            draw_rounded_rect(
                frame,
                text_x,
                footer_y,
                button_w,
                24,
                8,
                palette.surface,
                palette.border,
            );
            draw_text_in_rect(
                frame,
                "View history",
                (text_x, footer_y, button_w, 24),
                10,
                true,
                palette.primary,
                true,
            );
            self.icon_hits.push((
                super::super::types::HIT_CARD_NOTIFICATIONS,
                text_x,
                footer_y,
                button_w,
                24,
            ));
        } else {
            draw_text(
                frame,
                "Click to dismiss",
                text_x,
                footer_y + 6,
                text_w,
                10,
                false,
                palette.secondary,
            );
        }
        if self.alerts.len() > 1 {
            draw_text_in_rect(
                frame,
                &format!("+{} queued", self.alerts.len() - 1),
                (width as i32 - 100, footer_y, 82, 24),
                10,
                false,
                palette.secondary,
                true,
            );
        }
        let remaining = match (alert.duration_ms, alert.expires_at_ms) {
            (0, _) | (_, None) => 0.0,
            (duration, Some(expires_at)) => {
                expires_at.saturating_sub(now_ms) as f32 / duration as f32
            }
        }
        .clamp(0.0, 1.0);
        let track_y = height as i32 - 5;
        fill_rect_pub(
            frame,
            text_x,
            track_y,
            text_w,
            1,
            [
                palette.primary[0],
                palette.primary[1],
                palette.primary[2],
                18,
            ],
        );
        let fill_w = (text_w as f32 * remaining).round() as u32;
        if fill_w > 0 {
            fill_rect_pub(
                frame,
                text_x,
                track_y,
                fill_w,
                1,
                [accent[0], accent[1], accent[2], 140],
            );
        }
        self.icon_hits
            .push((HIT_ALERT_DISMISS, 0, 0, width, height));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banner(kind: AlertKind, title: &str, subtitle: &str) -> AlertBanner {
        AlertBanner {
            source: kind.label().into(),
            title: title.into(),
            subtitle: subtitle.into(),
            accent: [0; 4],
            kind,
            dedupe_key: String::new(),
            duration_ms: 0,
            expires_at_ms: None,
        }
    }

    #[test]
    fn agent_alert_uses_the_agent_as_source_and_a_clear_action() {
        let alert = banner(AlertKind::Agent, "Input", "claude");
        assert_eq!(alert_copy(&alert, true), ("claude", "Needs your input", ""));
        assert_eq!(
            alert_copy(&alert, false),
            ("claude", "Needs your input", "")
        );
        let unnamed = banner(AlertKind::Agent, "Input", "");
        assert_eq!(
            alert_copy(&unnamed, true),
            ("Termielle", "Needs your input", "")
        );
        assert_eq!(
            alert_copy(&unnamed, false),
            ("Agent", "Needs your input", "")
        );
    }

    #[test]
    fn explicit_source_is_not_mistaken_for_a_colon_in_the_body() {
        let mut alert = banner(AlertKind::System, "Review ready", "CI: All tests pass");
        alert.source = "Messages".into();
        assert_eq!(
            alert_copy(&alert, false),
            ("Messages", "Review ready", "CI: All tests pass")
        );
        alert.subtitle = "Messages: CI: All tests pass".into();
        assert_eq!(
            alert_copy(&alert, false),
            ("Messages", "Review ready", "CI: All tests pass")
        );
    }

    #[test]
    fn system_toast_separates_app_title_and_body() {
        let alert = banner(AlertKind::System, "Release ready", "Updates: Download now");
        assert_eq!(
            alert_copy(&alert, false),
            ("Updates", "Release ready", "Download now")
        );

        let alert = banner(AlertKind::System, "Updates", "Updates: Download now");
        assert_eq!(alert_copy(&alert, false), ("Updates", "Download now", ""));
    }
}

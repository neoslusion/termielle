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
            let (source, body) = alert
                .subtitle
                .split_once(": ")
                .unwrap_or((alert.subtitle.as_str(), ""));
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
        self.icon_hits
            .push((HIT_ALERT_DISMISS, 0, 0, width, height));

        let (source, headline, detail) = alert_copy(alert, island.show_name);
        let accent = alert.accent;
        let avatar_x = 16;
        let avatar_y = 27;
        draw_rounded_rect(
            frame,
            avatar_x,
            avatar_y,
            40,
            40,
            12,
            [accent[0], accent[1], accent[2], 78],
            [accent[0], accent[1], accent[2], 115],
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
                18,
                true,
                self.ink(),
                true,
            );
        }

        let text_x = 68;
        let text_w = width.saturating_sub(text_x as u32 + 18);
        let (source_y, headline_y) = if detail.is_empty() {
            (28, 47)
        } else {
            (17, 36)
        };
        let source_label = if alert.kind == AlertKind::Agent {
            let mut characters = source.chars();
            let first = characters.next().unwrap_or('T');
            first.to_uppercase().collect::<String>() + characters.as_str()
        } else {
            source.to_owned()
        };
        draw_text(
            frame,
            &source_label,
            text_x,
            source_y,
            text_w,
            11,
            true,
            accent,
        );
        draw_text(
            frame,
            headline,
            text_x,
            headline_y,
            text_w,
            16,
            true,
            self.ink(),
        );
        if !detail.is_empty() {
            draw_text(frame, detail, text_x, 58, text_w, 12, false, self.ink_dim());
        }

        let remaining = match (alert.duration_ms, alert.expires_at_ms) {
            (0, _) | (_, None) => 0.0,
            (duration, Some(expires_at)) => {
                expires_at.saturating_sub(now_ms) as f32 / duration as f32
            }
        }
        .clamp(0.0, 1.0);
        let track_w = width.saturating_sub(text_x as u32 + 18);
        let track_y = height as i32 - 8;
        fill_rect_pub(frame, text_x, track_y, track_w, 1, [255, 255, 255, 18]);
        let fill_w = (track_w as f32 * remaining).round() as u32;
        if fill_w > 0 {
            fill_rect_pub(
                frame,
                text_x,
                track_y,
                fill_w,
                1,
                [accent[0], accent[1], accent[2], 120],
            );
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banner(kind: AlertKind, title: &str, subtitle: &str) -> AlertBanner {
        AlertBanner {
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

//! Transient notification content for the island and bar.

use super::super::controller::Controller;
use super::super::types::{AlertBanner, AlertKind, HIT_ALERT_DISMISS};
use crate::animation::FrameBuffer;
use crate::animation::notch::{blit_rounded, draw_rounded_rect, draw_text, draw_text_in_rect};
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

/// Banner layout, in logical pixels. The card is `ALERT_HEIGHT` tall: a header
/// row, a headline, a detail, then a footer that clears the countdown line.
const PAD: i32 = 16;
const TILE: u32 = 36;
const TILE_RADIUS: u32 = 10;
const TEXT_X: i32 = PAD + TILE as i32 + 12;
const CLOSE: u32 = 24;
const HEADER_Y: i32 = 16;
const HEADLINE_Y: i32 = 33;
const HEADLINE_STEP: i32 = 20;
const DETAIL_STEP: i32 = 16;
const FOOTER_RISE: i32 = 42;
const FOOTER_H: u32 = 22;
const COUNTDOWN_RISE: i32 = 12;
const MAX_GROUP_DROP: i32 = 14;

/// The icon tile's fill: the accent, lighter at the top and deeper at the
/// bottom, so the tile has depth without a gloss. One column of pixels,
/// stretched over the tile by the rounded blit.
fn tile_gradient(accent: [u8; 4]) -> FrameBuffer {
    const ROWS: u32 = 16;
    let mut pixels = Vec::with_capacity(ROWS as usize * 4);
    for row in 0..ROWS {
        let t = row as f32 / (ROWS - 1) as f32;
        let (lift, sink) = (0.16 * (1.0 - t), 0.14 * t);
        let channel = |value: u8| {
            let value = f32::from(value);
            (value + (255.0 - value) * lift - value * sink)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        pixels.extend_from_slice(&[
            channel(accent[0]),
            channel(accent[1]),
            channel(accent[2]),
            255,
        ]);
    }
    FrameBuffer {
        width: 1,
        height: ROWS,
        pixels_pbgra: pixels,
        delay_ms: 0,
        loop_index: 0,
        scale: 1.0,
    }
}

/// The colour for a glyph on top of `accent`: dark on a light accent, white on
/// anything else, so the initial reads whatever colour the sender brought.
fn ink_on(accent: [u8; 4]) -> [u8; 4] {
    let luminance = (0.2126 * f32::from(accent[2])
        + 0.7152 * f32::from(accent[1])
        + 0.0722 * f32::from(accent[0]))
        / 255.0;
    if luminance > 0.62 {
        [24, 20, 20, 255]
    } else {
        [255; 4]
    }
}

fn over(point: Option<(i32, i32)>, (x, y, w, h): (i32, i32, u32, u32)) -> bool {
    point.is_some_and(|(px, py)| px >= x && px < x + w as i32 && py >= y && py < y + h as i32)
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
        let text_w = width.saturating_sub((TEXT_X + PAD) as u32);
        let headline_lines =
            crate::animation::text_layout::lines(headline, text_w, 16, true, frame.scale, 2);
        let detail_rows = if headline_lines.len() > 1 { 1 } else { 2 };
        let detail_lines = crate::animation::text_layout::lines(
            detail,
            text_w,
            12,
            false,
            frame.scale,
            detail_rows,
        );
        let footer_y = height as i32 - FOOTER_RISE;
        // The card is a fixed height, so short copy would leave a dead gap above
        // the footer. The icon and text move as one group to sit in the free
        // space, a little above centre, instead of hanging from the top edge.
        let block = (HEADLINE_Y - HEADER_Y)
            + headline_lines.len().max(1) as i32 * HEADLINE_STEP
            + 2
            + detail_lines.len() as i32 * DETAIL_STEP;
        let free = footer_y - 10 - HEADER_Y - block;
        let dy = (free / 2).clamp(0, MAX_GROUP_DROP);
        let header_y = HEADER_Y + dy;
        let headline_y = HEADLINE_Y + dy;

        // The dismiss circle: quiet at rest, lit under the pointer, lifted
        // while held. The whole card dismisses; this is the visible handle.
        let close = (width as i32 - PAD - CLOSE as i32, 12, CLOSE, CLOSE);
        let close_hovered = over(self.hover_point, close);
        draw_rounded_rect(
            frame,
            close.0,
            close.1,
            CLOSE,
            CLOSE,
            CLOSE / 2,
            if self.rect_pressed(close) {
                palette.pressed(palette.raised)
            } else if close_hovered {
                palette.raised
            } else {
                palette.surface
            },
            [0; 4],
        );
        draw_text_in_rect(
            frame,
            "×",
            close,
            15,
            false,
            if close_hovered {
                palette.primary
            } else {
                palette.secondary
            },
            true,
        );

        // The icon tile: a squircle in the sender's colour with a hairline
        // ring, and the sender's initial (or the agent's face) on it.
        let tile_y = header_y;
        blit_rounded(
            frame,
            &tile_gradient(accent),
            PAD,
            tile_y,
            TILE,
            TILE,
            TILE_RADIUS,
        );
        draw_rounded_rect(
            frame,
            PAD,
            tile_y,
            TILE,
            TILE,
            TILE_RADIUS,
            [0; 4],
            [255, 255, 255, 36],
        );
        if alert.kind == AlertKind::Agent && island.has_widget("face") {
            blit_rounded(frame, &self.face_frame, PAD + 4, tile_y + 4, 28, 28, 7);
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
                (PAD, tile_y, TILE, TILE),
                17,
                true,
                ink_on(accent),
                true,
            );
        }

        // Header, headline, detail. The detail sits right under the headline
        // however many lines it took, and gives up its second line when the
        // headline needed two, so the footer is never crowded.
        draw_text(
            frame,
            source,
            TEXT_X,
            header_y,
            text_w.saturating_sub(CLOSE + 8),
            11,
            true,
            palette.secondary,
        );
        for (index, line) in headline_lines.iter().enumerate() {
            draw_text(
                frame,
                line,
                TEXT_X,
                headline_y + index as i32 * HEADLINE_STEP,
                text_w,
                16,
                true,
                palette.primary,
            );
        }
        let detail_y = headline_y + headline_lines.len().max(1) as i32 * HEADLINE_STEP + 2;
        for (index, line) in detail_lines.iter().enumerate() {
            draw_text(
                frame,
                line,
                TEXT_X,
                detail_y + index as i32 * DETAIL_STEP,
                text_w,
                12,
                false,
                palette.secondary,
            );
        }

        if island.is_bar() {
            let button = (TEXT_X, footer_y, text_w.min(112), FOOTER_H);
            draw_rounded_rect(
                frame,
                button.0,
                button.1,
                button.2,
                button.3,
                FOOTER_H / 2,
                if self.rect_pressed(button) {
                    palette.pressed(palette.raised)
                } else if over(self.hover_point, button) {
                    palette.raised
                } else {
                    palette.surface
                },
                palette.border,
            );
            draw_text_in_rect(
                frame,
                "View history",
                button,
                10,
                true,
                palette.primary,
                true,
            );
            self.icon_hits.push((
                super::super::types::HIT_CARD_NOTIFICATIONS,
                button.0,
                button.1,
                button.2,
                button.3,
            ));
        } else {
            draw_text(
                frame,
                "Click to dismiss",
                TEXT_X,
                footer_y + 5,
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
                (width as i32 - PAD - 82, footer_y, 82, FOOTER_H),
                10,
                false,
                palette.secondary,
                true,
            );
        }

        // The countdown: a rounded track along the foot of the card that the
        // accent drains from the right, clear of the footer above it.
        let remaining = match (alert.duration_ms, alert.expires_at_ms) {
            (0, _) | (_, None) => 0.0,
            (duration, Some(expires_at)) => {
                expires_at.saturating_sub(now_ms) as f32 / duration as f32
            }
        }
        .clamp(0.0, 1.0);
        let track_y = height as i32 - COUNTDOWN_RISE;
        let track_w = width.saturating_sub(2 * PAD as u32);
        draw_rounded_rect(
            frame,
            PAD,
            track_y,
            track_w,
            2,
            1,
            [
                palette.primary[0],
                palette.primary[1],
                palette.primary[2],
                22,
            ],
            [0; 4],
        );
        let fill_w = (track_w as f32 * remaining).round() as u32;
        if fill_w > 1 {
            draw_rounded_rect(
                frame,
                PAD,
                track_y,
                fill_w,
                2,
                1,
                [accent[0], accent[1], accent[2], 200],
                [0; 4],
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
    fn the_initial_is_dark_on_a_light_accent_and_white_on_everything_else() {
        // Accents are BGRA.
        assert_eq!(ink_on([60, 200, 250, 255]), [24, 20, 20, 255], "yellow");
        assert_eq!(ink_on([240, 154, 92, 255]), [255; 4], "blue");
        assert_eq!(ink_on([0, 0, 0, 255]), [255; 4], "black");
        assert_eq!(ink_on([255; 4]), [24, 20, 20, 255], "white");
    }

    #[test]
    fn the_tile_gradient_is_lighter_at_the_top_and_deeper_at_the_bottom_and_opaque() {
        let tile = tile_gradient([240, 154, 92, 255]);
        let (top, bottom) = (
            &tile.pixels_pbgra[..4],
            &tile.pixels_pbgra[tile.pixels_pbgra.len() - 4..],
        );
        for channel in 0..3 {
            assert!(top[channel] > bottom[channel], "channel {channel}");
        }
        assert!(
            tile.pixels_pbgra
                .chunks_exact(4)
                .all(|pixel| pixel[3] == 255)
        );
        assert_eq!((tile.width, tile.height), (1, 16));
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

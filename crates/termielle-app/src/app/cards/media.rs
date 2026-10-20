use super::super::controller::Controller;
use super::super::paint::CardPaintCtx;
use super::super::types::{HIT_MEDIA_NEXT, HIT_MEDIA_PLAY_PAUSE, HIT_MEDIA_PREV};
use crate::animation::{FrameBuffer, icons, notch, text_layout};
use crate::tasks::MediaInfo;
use termielle_core::IslandConfig;

pub(crate) const MEDIA_HEIGHT: u32 = 244;

pub(crate) fn artwork_accent(media: Option<&MediaInfo>, fallback: [u8; 4]) -> [u8; 4] {
    let Some(art) = media.and_then(|media| media.thumbnail.as_ref()) else {
        return fallback;
    };
    let step = (art.pixels_pbgra.len() / 4 / 32).max(1);
    let mut total = [0u32; 3];
    let mut count = 0;
    for pixel in art.pixels_pbgra.chunks_exact(4).step_by(step).take(32) {
        let brightest = *pixel[..3].iter().max().unwrap();
        let darkest = *pixel[..3].iter().min().unwrap();
        if pixel[3] < 220 || brightest < 55 || brightest - darkest < 24 {
            continue;
        }
        for channel in 0..3 {
            total[channel] += u32::from(pixel[channel]);
        }
        count += 1;
    }
    if count == 0 {
        return fallback;
    }
    [
        ((total[0] / count + u32::from(fallback[0])) / 2) as u8,
        ((total[1] / count + u32::from(fallback[1])) / 2) as u8,
        ((total[2] / count + u32::from(fallback[2])) / 2) as u8,
        255,
    ]
}

pub(crate) fn paint_artwork(
    frame: &mut FrameBuffer,
    media: Option<&MediaInfo>,
    x: i32,
    y: i32,
    size: u32,
    radius: u32,
    accent: [u8; 4],
) {
    if let Some(art) = media.and_then(|media| media.thumbnail.as_ref()) {
        notch::blit_rounded_pixels(
            frame,
            &art.pixels_pbgra,
            art.width,
            art.height,
            x,
            y,
            size,
            size,
            radius,
        );
    } else {
        notch::draw_rounded_rect(
            frame,
            x,
            y,
            size,
            size,
            radius,
            [accent[0], accent[1], accent[2], 35],
            [accent[0], accent[1], accent[2], 45],
        );
        let icon_size = size * 3 / 5;
        let inset = ((size - icon_size) / 2) as i32;
        icons::draw_icon(frame, icons::MUSIC, x + inset, y + inset, icon_size, accent);
    }
}

pub(crate) fn paint_wave(
    frame: &mut FrameBuffer,
    x: i32,
    center_y: i32,
    now: u64,
    playing: bool,
    accent: [u8; 4],
) {
    for index in 0..4 {
        let phase = ((now % 1000) as f32 / 1000.0 * std::f32::consts::TAU) + index as f32 * 1.3;
        let height = if playing {
            (4.0 + phase.sin().abs() * 8.0).round() as u32
        } else {
            3
        };
        notch::draw_rounded_rect(
            frame,
            x + index * 4,
            center_y - height as i32 / 2,
            2,
            height,
            1,
            accent,
            [0; 4],
        );
    }
}

fn time_label(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

fn source_label(app: &str) -> String {
    let label = app
        .rsplit('!')
        .next()
        .unwrap_or(app)
        .trim_end_matches(".exe");
    if label.is_empty() {
        "Media session".into()
    } else {
        label.chars().take(48).collect()
    }
}

impl Controller {
    pub(crate) fn paint_bar_media(
        &self,
        frame: &mut FrameBuffer,
        rect: (i32, i32, u32, u32),
        now: u64,
    ) {
        let palette = crate::theme::Palette::for_island(&self.island);
        let accent = artwork_accent(self.media.as_ref(), palette.accent);
        let size = rect.3.saturating_sub(4).min(24);
        paint_artwork(
            frame,
            self.media.as_ref(),
            rect.0 + 3,
            rect.1 + (rect.3 - size) as i32 / 2,
            size,
            6,
            accent,
        );
        let text_x = rect.0 + size as i32 + 9;
        let label_width = rect.2.saturating_sub(size + 38);
        let title = self
            .media
            .as_ref()
            .map(|media| media.title.as_str())
            .filter(|title| !title.trim().is_empty())
            .unwrap_or("Media");
        notch::draw_text_in_rect(
            frame,
            title,
            (text_x, rect.1, label_width, rect.3),
            12,
            false,
            palette.primary,
            false,
        );
        paint_wave(
            frame,
            rect.0 + rect.2 as i32 - 23,
            rect.1 + rect.3 as i32 / 2,
            now,
            self.media_playing() && !self.reduced_motion,
            accent,
        );
    }

    pub(crate) fn paint_media_card(
        &mut self,
        frame: &mut FrameBuffer,
        island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        let palette = crate::theme::Palette::for_island(island);
        let accent = artwork_accent(self.media.as_ref(), palette.accent);
        let media = self.media.as_ref();
        let playing = media.is_some_and(|media| media.playing);
        let tag_x = ctx.pad + if island.has_widget("face") { 30 } else { 0 };
        notch::draw_text(
            frame,
            if playing { "NOW PLAYING" } else { "PAUSED" },
            tag_x,
            17,
            ctx.width.saturating_sub(tag_x as u32 + 100),
            10,
            true,
            palette.secondary,
        );
        paint_wave(
            frame,
            ctx.width as i32 - 76,
            23,
            ctx.eq_now,
            playing && !self.reduced_motion,
            accent,
        );
        notch::draw_rounded_rect(
            frame,
            ctx.pad,
            46,
            ctx.width.saturating_sub(ctx.pad as u32 * 2),
            94,
            16,
            [accent[0], accent[1], accent[2], 14],
            palette.border,
        );
        let art_size = (ctx.width.saturating_sub(ctx.pad as u32 * 2) / 3).clamp(28, 80);
        paint_artwork(frame, media, ctx.pad + 7, 53, art_size, 12, accent);
        let text_x = ctx.pad + art_size as i32 + 21;
        let text_w = ctx.width.saturating_sub(text_x as u32 + ctx.pad as u32 + 8);
        let title = media
            .map(|media| media.title.as_str())
            .filter(|title| !title.trim().is_empty())
            .unwrap_or("Untitled");
        for (index, line) in text_layout::lines(title, text_w, 16, true, frame.scale, 2)
            .iter()
            .enumerate()
        {
            notch::draw_text(
                frame,
                line,
                text_x,
                54 + index as i32 * 21,
                text_w,
                16,
                true,
                palette.primary,
            );
        }
        let artist = media
            .map(|media| media.artist.as_str())
            .filter(|artist| !artist.trim().is_empty())
            .unwrap_or("Unknown artist");
        notch::draw_text(
            frame,
            artist,
            text_x,
            99,
            text_w,
            12,
            false,
            palette.secondary,
        );
        let source = source_label(media.map(|media| media.app.as_str()).unwrap_or(""));
        notch::draw_text(
            frame,
            &source,
            text_x,
            119,
            text_w,
            10,
            false,
            palette.secondary,
        );
        let center_x = ctx.width as i32 / 2;
        let control_y = 168;
        for (hit, center, size) in [
            (HIT_MEDIA_PREV, center_x - 66, 42),
            (HIT_MEDIA_PLAY_PAUSE, center_x, 48),
            (HIT_MEDIA_NEXT, center_x + 66, 42),
        ] {
            if hit != HIT_MEDIA_PLAY_PAUSE && ctx.width < 220 {
                continue;
            }
            let rect = (
                center - size as i32 / 2,
                control_y - size as i32 / 2,
                size,
                size,
            );
            let hovered = self.hover_point.is_some_and(|(x, y)| {
                x >= rect.0 && x < rect.0 + size as i32 && y >= rect.1 && y < rect.1 + size as i32
            });
            let primary_control = hit == HIT_MEDIA_PLAY_PAUSE;
            let pressed = self.rect_pressed((rect.0, rect.1, size, size));
            notch::draw_rounded_rect(
                frame,
                rect.0,
                rect.1,
                size,
                size,
                size / 2,
                if primary_control {
                    // The solid control is already at the ink, so a press
                    // dims it instead of lifting it.
                    if pressed {
                        palette.secondary
                    } else {
                        palette.primary
                    }
                } else if hovered {
                    if pressed {
                        palette.pressed(palette.raised)
                    } else {
                        palette.raised
                    }
                } else {
                    palette.surface
                },
                if hovered {
                    [accent[0], accent[1], accent[2], 130]
                } else {
                    palette.border
                },
            );
            let ink = if primary_control {
                [
                    island.glass.tint[0],
                    island.glass.tint[1],
                    island.glass.tint[2],
                    255,
                ]
            } else {
                palette.primary
            };
            match hit {
                HIT_MEDIA_PREV => notch::draw_glyph_prev(frame, center, control_y, ink),
                HIT_MEDIA_NEXT => notch::draw_glyph_next(frame, center, control_y, ink),
                _ if playing => notch::draw_glyph_pause(frame, center, control_y, ink),
                _ => notch::draw_glyph_play(frame, center, control_y, ink),
            }
            if rect.1 + size as i32 <= ctx.height as i32 {
                self.icon_hits.push((hit, rect.0, rect.1, size, size));
            }
        }
        let track_w = ctx.width.saturating_sub(ctx.pad as u32 * 2);
        if let Some(timeline) = media
            .and_then(|media| media.timeline.as_ref())
            .filter(|timeline| timeline.duration_ms > 0)
        {
            notch::draw_rounded_rect(frame, ctx.pad, 205, track_w, 4, 2, palette.raised, [0; 4]);
            let fill = ((u128::from(timeline.position_ms.min(timeline.duration_ms))
                * u128::from(track_w))
                / u128::from(timeline.duration_ms)) as u32;
            if fill > 0 {
                notch::draw_rounded_rect(frame, ctx.pad, 205, fill, 4, 2, accent, [0; 4]);
            }
            notch::draw_text(
                frame,
                &time_label(timeline.position_ms),
                ctx.pad,
                218,
                track_w / 2,
                10,
                false,
                palette.secondary,
            );
            notch::draw_text_in_rect(
                frame,
                &time_label(timeline.duration_ms),
                (ctx.width as i32 - ctx.pad - 62, 216, 62, 18),
                10,
                false,
                palette.secondary,
                true,
            );
        } else {
            notch::draw_text_in_rect(
                frame,
                "Playback position unavailable",
                (ctx.pad, 212, track_w, 20),
                10,
                false,
                palette.secondary,
                true,
            );
        }
        self.paint_notch_close(frame, ctx.width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::ThumbBitmap;
    use termielle_core::AssetCatalog;

    #[test]
    fn media_labels_are_readable_and_artwork_tint_has_a_safe_default() {
        assert_eq!(time_label(83_000), "1:23");
        assert_eq!(time_label(3_661_000), "1:01:01");
        assert_eq!(source_label("Vendor.Player_123!Music"), "Music");
        assert_eq!(source_label("Spotify.exe"), "Spotify");
        assert_eq!(source_label(""), "Media session");
        let fallback = [255, 132, 10, 255];
        assert_eq!(artwork_accent(None, fallback), fallback);
        let mut media = MediaInfo {
            thumbnail: Some(ThumbBitmap {
                width: 1,
                height: 1,
                pixels_pbgra: vec![50, 50, 50, 255],
            }),
            ..Default::default()
        };
        assert_eq!(artwork_accent(Some(&media), fallback), fallback);
        media.thumbnail.as_mut().unwrap().pixels_pbgra = vec![180, 60, 80, 255];
        assert_eq!(artwork_accent(Some(&media), fallback), [217, 96, 45, 255]);
    }

    #[test]
    fn media_controls_fit_the_card_at_multiple_widths_and_scales() {
        for width in [180, 320, 420] {
            for scale in [1.0, 1.25, 2.0] {
                let island = IslandConfig::default();
                let mut controller = Controller::new_with_island(
                    5_000,
                    60_000,
                    AssetCatalog::new(Vec::new()),
                    true,
                    None,
                    island.clone(),
                );
                controller.dpi_scale = scale;
                controller.media = Some(MediaInfo {
                    title:
                        "A long title 日本語 that should wrap instead of overlapping the controls"
                            .into(),
                    playing: true,
                    ..Default::default()
                });
                let mut frame = controller.blank_frame(width, MEDIA_HEIGHT);
                controller.icon_hits.clear();
                controller.paint_media_card(
                    &mut frame,
                    &island,
                    &CardPaintCtx {
                        width,
                        height: MEDIA_HEIGHT,
                        cy: MEDIA_HEIGHT as i32 / 2,
                        accent: [255; 4],
                        now: 1_000,
                        eq_now: 1_000,
                        pad: 18,
                    },
                );
                for (_, x, y, w, h) in &controller.icon_hits {
                    assert!(
                        *x >= 0
                            && *y >= 0
                            && *x + *w as i32 <= width as i32
                            && *y + *h as i32 <= MEDIA_HEIGHT as i32
                    );
                }
                assert!(
                    controller
                        .icon_hits
                        .iter()
                        .any(|hit| hit.0 == HIT_MEDIA_PLAY_PAUSE && hit.3 == 48 && hit.4 == 48)
                );
                assert_eq!(controller.icon_hits.len(), if width < 220 { 2 } else { 4 });
                assert_eq!(frame.scale, scale);
            }
        }
    }

    #[test]
    fn paused_wave_is_static() {
        let mut first = FrameBuffer {
            width: 30,
            height: 20,
            pixels_pbgra: vec![0; 30 * 20 * 4],
            delay_ms: 0,
            loop_index: 0,
            scale: 1.0,
        };
        let mut second = first.clone();
        paint_wave(&mut first, 4, 10, 0, false, [255; 4]);
        paint_wave(&mut second, 4, 10, 443, false, [255; 4]);
        assert_eq!(first.pixels_pbgra, second.pixels_pbgra);
        paint_wave(&mut second, 4, 10, 443, true, [255; 4]);
        assert_ne!(first.pixels_pbgra, second.pixels_pbgra);
    }
}

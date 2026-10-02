use super::super::controller::Controller;
use super::super::paint::CardPaintCtx;
use super::super::types::{HIT_MEDIA_PLAY_PAUSE, HIT_PANEL_VOLUME_TRACK};
use crate::animation::{FrameBuffer, icons, notch};
use crate::bar::shell::ShellAction;
use termielle_core::IslandConfig;

const PANEL_PAD: i32 = 16;
const TILE_H: u32 = 52;
const TILE_GAP: i32 = 8;
const SOUND_Y: i32 = 160;
const MEDIA_Y: i32 = 242;
const STATS_Y: i32 = 316;
const FOOTER_Y: i32 = 350;

pub(crate) const fn panel_height() -> u32 {
    380
}

pub(crate) fn volume_track(width: u32) -> (i32, i32, u32, u32) {
    let x = PANEL_PAD + 12;
    (
        x,
        SOUND_Y + 51,
        width.saturating_sub((PANEL_PAD * 2 + 24) as u32),
        7,
    )
}

fn paint_tile(
    frame: &mut FrameBuffer,
    x: i32,
    y: i32,
    width: u32,
    label: &str,
    subtitle: &str,
    icon: icons::Icon,
    primary: [u8; 4],
    secondary: [u8; 4],
) {
    notch::draw_rounded_rect(frame, x, y, width, TILE_H, 12, [255, 255, 255, 20], [0; 4]);
    notch::draw_rounded_rect(frame, x + 8, y + 9, 34, 34, 17, [255, 255, 255, 25], [0; 4]);
    icons::draw_icon(frame, icon, x + 14, y + 15, 22, primary);
    let text_x = x + 48;
    let text_w = width.saturating_sub(54);
    notch::draw_text(frame, label, text_x, y + 10, text_w, 12, true, primary);
    notch::draw_text(
        frame,
        subtitle,
        text_x,
        y + 27,
        text_w,
        10,
        false,
        secondary,
    );
}

impl Controller {
    pub(crate) fn paint_control_panel(
        &mut self,
        frame: &mut FrameBuffer,
        _island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        let (primary, secondary) = notch::ink_pair(&self.island.glass);
        let dim = self.ink_dim();
        notch::draw_text(
            frame,
            "Control Center",
            PANEL_PAD,
            13,
            ctx.width.saturating_sub(2 * PANEL_PAD as u32),
            13,
            true,
            primary,
        );

        let inner_width = ctx.width.saturating_sub(2 * PANEL_PAD as u32);
        let tile_width = inner_width.saturating_sub(TILE_GAP as u32) / 2;
        let right_x = PANEL_PAD + tile_width as i32 + TILE_GAP;
        let connectivity = self
            .bar_metrics_cache
            .as_ref()
            .map(|metrics| metrics.connectivity)
            .unwrap_or_default();
        for (action, label, subtitle, icon, x, y) in [
            (
                ShellAction::Network,
                "Network",
                connectivity.network.label(),
                icons::WIFI,
                PANEL_PAD,
                40,
            ),
            (
                ShellAction::Bluetooth,
                "Bluetooth",
                connectivity.bluetooth.label(),
                icons::BLUETOOTH,
                right_x,
                40,
            ),
            (
                ShellAction::Focus,
                "Focus",
                "Settings",
                icons::MOON,
                PANEL_PAD,
                100,
            ),
            (
                ShellAction::Display,
                "Display",
                "Settings",
                icons::DEVICE_DESKTOP,
                right_x,
                100,
            ),
        ] {
            paint_tile(
                frame, x, y, tile_width, label, subtitle, icon, primary, secondary,
            );
            self.icon_hits
                .push((action.hit_id(), x, y, tile_width, TILE_H));
        }

        notch::draw_rounded_rect(
            frame,
            PANEL_PAD,
            SOUND_Y,
            inner_width,
            74,
            12,
            [255, 255, 255, 20],
            [0; 4],
        );
        let (level, muted) = self.panel_volume();
        let speaker_x = PANEL_PAD + 11;
        let speaker_y = SOUND_Y + 12;
        icons::draw_icon(
            frame,
            if muted {
                icons::VOLUME_MUTED
            } else {
                icons::VOLUME
            },
            speaker_x,
            speaker_y,
            22,
            primary,
        );
        self.icon_hits.push((
            crate::bar::HIT_BAR_VOLUME_TOGGLE,
            speaker_x,
            speaker_y,
            22,
            22,
        ));
        notch::draw_text(
            frame,
            "Sound",
            PANEL_PAD + 42,
            SOUND_Y + 15,
            inner_width.saturating_sub(98),
            12,
            true,
            primary,
        );
        notch::draw_text(
            frame,
            &format!("{level}%"),
            ctx.width as i32 - PANEL_PAD - 48,
            SOUND_Y + 15,
            36,
            12,
            false,
            secondary,
        );
        let (track_x, track_y, track_w, track_h) = volume_track(ctx.width);
        notch::draw_rounded_rect(
            frame,
            track_x,
            track_y,
            track_w,
            track_h,
            4,
            [255, 255, 255, 40],
            [0; 4],
        );
        let fill_w = track_w * u32::from(level) / 100;
        if fill_w > 0 {
            notch::draw_rounded_rect(
                frame,
                track_x,
                track_y,
                fill_w,
                track_h,
                4,
                if muted { dim } else { primary },
                [0; 4],
            );
        }
        if track_w > 0 {
            self.icon_hits
                .push((HIT_PANEL_VOLUME_TRACK, track_x, track_y - 5, track_w, 17));
        }

        notch::draw_rounded_rect(
            frame,
            PANEL_PAD,
            MEDIA_Y,
            inner_width,
            64,
            12,
            [255, 255, 255, 20],
            [0; 4],
        );
        notch::draw_rounded_rect(
            frame,
            PANEL_PAD + 9,
            MEDIA_Y + 9,
            46,
            46,
            9,
            [255, 255, 255, 25],
            [0; 4],
        );
        icons::draw_icon(
            frame,
            icons::MUSIC,
            PANEL_PAD + 20,
            MEDIA_Y + 20,
            24,
            primary,
        );
        let media = self.media.as_ref();
        let title = media
            .map(|item| item.title.as_str())
            .filter(|title| !title.is_empty())
            .unwrap_or("Nothing playing");
        let subtitle = media
            .map(|item| item.artist.as_str())
            .filter(|artist| !artist.is_empty())
            .unwrap_or("Media");
        let media_text_w = inner_width.saturating_sub(if media.is_some() { 128 } else { 70 });
        notch::draw_text(
            frame,
            title,
            PANEL_PAD + 65,
            MEDIA_Y + 12,
            media_text_w,
            12,
            true,
            primary,
        );
        notch::draw_text(
            frame,
            subtitle,
            PANEL_PAD + 65,
            MEDIA_Y + 34,
            media_text_w,
            10,
            false,
            secondary,
        );
        if let Some(media) = media {
            let button_x = ctx.width as i32 - PANEL_PAD - 43;
            let button_y = MEDIA_Y + 17;
            notch::draw_rounded_rect(
                frame,
                button_x,
                button_y,
                30,
                30,
                15,
                [255, 255, 255, 32],
                [0; 4],
            );
            icons::draw_icon(
                frame,
                if media.playing {
                    icons::PLAYER_PAUSE
                } else {
                    icons::PLAYER_PLAY
                },
                button_x + 5,
                button_y + 5,
                20,
                primary,
            );
            self.icon_hits
                .push((HIT_MEDIA_PLAY_PAUSE, button_x, button_y, 30, 30));
        }

        let (cpu, memory) = self
            .bar_metrics_cache
            .as_ref()
            .map(|metrics| (metrics.cpu_pct, metrics.memory_pct))
            .unwrap_or((0, 0));
        notch::draw_text(
            frame,
            &format!("CPU {cpu}%    Memory {memory}%"),
            PANEL_PAD + 4,
            STATS_Y,
            inner_width.saturating_sub(8),
            11,
            false,
            secondary,
        );
        notch::draw_text(
            frame,
            "Windows Settings",
            PANEL_PAD + 4,
            FOOTER_Y,
            inner_width.saturating_sub(8),
            11,
            false,
            secondary,
        );
        self.icon_hits.push((
            ShellAction::Settings.hit_id(),
            PANEL_PAD,
            FOOTER_Y - 2,
            inner_width,
            22,
        ));
    }

    pub(crate) fn panel_volume(&self) -> (u8, bool) {
        self.bar_metrics_cache
            .as_ref()
            .map(|metrics| (metrics.volume.level, metrics.volume.muted))
            .unwrap_or((0, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_fit_without_overlap() {
        assert!(100 + (TILE_H as i32) < SOUND_Y);
        assert!(SOUND_Y + 74 < MEDIA_Y);
        assert!(MEDIA_Y + 64 < STATS_Y);
        assert!(STATS_Y + 18 < FOOTER_Y);
        assert!(FOOTER_Y + 22 <= panel_height() as i32);
    }

    #[test]
    fn volume_track_stays_inside_card() {
        for width in [90, 320, 420] {
            let (x, _, track_width, _) = volume_track(width);
            assert!(x >= PANEL_PAD);
            assert!(x + track_width as i32 <= width as i32 - PANEL_PAD);
        }
    }
}

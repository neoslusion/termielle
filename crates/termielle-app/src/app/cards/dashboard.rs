//! Expanded-card section: standby dashboard (time, headline, telemetry cards).

use super::super::controller::Controller;
use super::super::paint::CardPaintCtx;
use crate::animation::FrameBuffer;
use termielle_core::IslandConfig;

impl Controller {
    /// Expanded card section: default standby and glanceables dashboard
    /// (time, headline, telemetry cards).
    pub(crate) fn paint_standby_dashboard(
        &mut self,
        frame: &mut FrameBuffer,
        island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        // Default Clean Standby & Glanceables Dashboard
        let tag_x = if island.has_widget("face") {
            ctx.pad + 30
        } else {
            ctx.pad
        };
        crate::animation::notch::draw_text(
            frame,
            "Standby • Ready",
            tag_x,
            16,
            ctx.width.saturating_sub((tag_x as u32) + 70),
            10,
            true,
            ctx.accent,
        );

        let stats = crate::system::collect();

        // Live time in top-right header
        crate::animation::notch::draw_text(
            frame,
            &stats.time,
            ctx.width as i32 - ctx.pad - 42,
            15,
            42,
            11,
            true,
            self.ink(),
        );

        // Trailing live beacon with halo next to time
        let beacon_x = ctx.width as i32 - ctx.pad - 52;
        crate::animation::notch::draw_disc(
            frame,
            beacon_x,
            22,
            5,
            [ctx.accent[0], ctx.accent[1], ctx.accent[2], 45],
        );
        crate::animation::notch::draw_disc(
            frame,
            beacon_x,
            22,
            2,
            [ctx.accent[0], ctx.accent[1], ctx.accent[2], 255],
        );

        // Headline & Subtitle
        let text_w = ctx.width.saturating_sub((ctx.pad as u32) * 2);
        crate::animation::notch::draw_text(
            frame,
            "Termielle is Ready",
            ctx.pad,
            42,
            text_w,
            13,
            true,
            self.ink(),
        );
        crate::animation::notch::draw_text(
            frame,
            "Standing by for agent instructions or media playback",
            ctx.pad,
            60,
            text_w,
            10,
            false,
            self.ink_dim(),
        );

        // System Telemetry Cards (CPU, RAM, Power/System)
        if ctx.height >= 125 {
            // 3 rich telemetry modules
            struct CardInfo {
                label: &'static str,
                dot_color: [u8; 4],
                value: String,
                pct: u8,
                fill_color: [u8; 4],
                caption: String,
            }

            // CPU card setup
            let cpu_heavy = stats.cpu_percent >= 75;
            let (cpu_dot, cpu_fill) = if cpu_heavy {
                ([30, 90, 245, 255], [30, 90, 245, 255])
            } else {
                (
                    [ctx.accent[0], ctx.accent[1], ctx.accent[2], 255],
                    [ctx.accent[0], ctx.accent[1], ctx.accent[2], 255],
                )
            };
            let cpu_caption = if stats.cpu_percent < 25 {
                "Calm".to_string()
            } else if stats.cpu_percent < 65 {
                "Active".to_string()
            } else {
                "High Load".to_string()
            };

            // RAM card setup
            let ram_dot = [225, 175, 20, 255]; // cyan/teal
            let ram_fill = [225, 175, 20, 255];
            let ram_caption = if stats.mem_total_gb > 0.0 {
                format!("{:.0}/{:.0} GB", stats.mem_used_gb, stats.mem_total_gb)
            } else {
                "System RAM".to_string()
            };

            // Power/Battery card setup
            let (bat_dot, bat_val, bat_fill, bat_pct, bat_caption) =
                if let Some(bat) = stats.battery {
                    let charging = stats.battery_charging;
                    let dot = if charging {
                        [60, 205, 80, 255]
                    } else if bat < 20 {
                        [20, 160, 245, 255]
                    } else {
                        [60, 205, 80, 255]
                    };
                    let val = if charging {
                        format!("{}% +", bat)
                    } else {
                        format!("{}%", bat)
                    };
                    let cap = if charging {
                        "Charging".to_string()
                    } else {
                        "On Battery".to_string()
                    };
                    (dot, val, dot, bat, cap)
                } else {
                    (
                        [60, 205, 80, 255],
                        "Online".to_string(),
                        [60, 205, 80, 255],
                        100,
                        "Desktop".to_string(),
                    )
                };

            let cards = [
                CardInfo {
                    label: "CPU",
                    dot_color: cpu_dot,
                    value: format!("{}%", stats.cpu_percent),
                    pct: stats.cpu_percent,
                    fill_color: cpu_fill,
                    caption: cpu_caption,
                },
                CardInfo {
                    label: "RAM",
                    dot_color: ram_dot,
                    value: format!("{}%", stats.mem_percent),
                    pct: stats.mem_percent,
                    fill_color: ram_fill,
                    caption: ram_caption,
                },
                CardInfo {
                    label: if stats.battery.is_some() {
                        "BATTERY"
                    } else {
                        "SYSTEM"
                    },
                    dot_color: bat_dot,
                    value: bat_val,
                    pct: bat_pct,
                    fill_color: bat_fill,
                    caption: bat_caption,
                },
            ];

            let card_count = cards.len() as i32;
            let gap = 8i32;
            let total_gap = (card_count - 1) * gap;
            let card_w = ((text_w as i32 - total_gap) / card_count).max(60);
            let card_y = 80i32;
            let card_h = 50u32;

            let card_bg = [
                (ctx.accent[0] as u32 * 20 / 255) as u8,
                (ctx.accent[1] as u32 * 20 / 255) as u8,
                (ctx.accent[2] as u32 * 20 / 255) as u8,
                26,
            ];
            let card_border = [255, 255, 255, 34];
            let track_color = [255, 255, 255, 20];

            for (i, card) in cards.iter().enumerate() {
                let cx = ctx.pad + i as i32 * (card_w + gap);

                // Smooth rounded frosted card
                crate::animation::notch::draw_rounded_rect(
                    frame,
                    cx,
                    card_y,
                    card_w as u32,
                    card_h,
                    7,
                    card_bg,
                    card_border,
                );

                // Top row: indicator dot + label + value
                crate::animation::notch::draw_disc(frame, cx + 9, card_y + 11, 3, card.dot_color);
                crate::animation::notch::draw_text(
                    frame,
                    card.label,
                    cx + 16,
                    card_y + 6,
                    (card_w - 48).max(20) as u32,
                    9,
                    true,
                    self.ink_dim(),
                );
                crate::animation::notch::draw_text(
                    frame,
                    &card.value,
                    cx + card_w - 38,
                    card_y + 6,
                    34,
                    9,
                    true,
                    self.ink(),
                );

                // Mini progress bar
                let bar_w = (card_w - 18).max(10) as u32;
                crate::animation::notch::draw_progress_bar(
                    frame,
                    cx + 9,
                    card_y + 23,
                    bar_w,
                    4,
                    card.pct,
                    track_color,
                    card.fill_color,
                );

                // Detail caption under progress bar
                crate::animation::notch::draw_text(
                    frame,
                    &card.caption,
                    cx + 9,
                    card_y + 32,
                    bar_w,
                    9,
                    false,
                    self.ink_dim(),
                );
            }
        }

        // Bottom ctx.accent pill bar
        if ctx.height >= 145 {
            let bar_w = 48u32;
            let bar_x = (ctx.width as i32 - bar_w as i32) / 2;
            crate::animation::notch::fill_rect_pub(
                frame,
                bar_x,
                ctx.height as i32 - 10,
                bar_w,
                3,
                [ctx.accent[0], ctx.accent[1], ctx.accent[2], 200],
            );
        }
    }
}

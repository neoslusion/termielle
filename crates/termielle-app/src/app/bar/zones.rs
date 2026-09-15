//! Bar zone painters: left (workspaces/title), right (clock/stats), center (island).

use super::super::controller::Controller;
use super::text::{ellipsize_middle, paint_metric_text};
use super::types::{BarCard, BarHit, BarMetricsCache};
use crate::animation::FrameBuffer;
use termielle_core::VisualState;

impl Controller {
    /// Bar zone 2 (left): workspace switcher plus the active-window title
    /// pill. Pure painter: draws into `frame` and returns its hit targets
    /// for the orchestrator to install.
    pub(crate) fn paint_bar_left(
        &self,
        frame: &mut FrameBuffer,
        metrics: &BarMetricsCache,
        accent: [u8; 4],
        bar_x: i32,
        pill_y: i32,
        pill_h: u32,
    ) -> Vec<BarHit> {
        let mut hits = Vec::new();
        // 2. Modules Left: Workspaces + Window Title
        let mut cur_x = bar_x + 12;

        if self.bar_module("left", "workspaces") {
            // Workspaces
            let ws = metrics.workspaces;
            let ws_w = 28u32;
            for i in 1..=ws.total.min(10) {
                let active = i == ws.active;
                let (bg, border, text_col) = if active {
                    (
                        [accent[0], accent[1], accent[2], 190],
                        [255, 255, 255, 140],
                        [255, 255, 255, 255],
                    )
                } else {
                    (
                        [255, 255, 255, 24],
                        [255, 255, 255, 36],
                        [190, 190, 190, 220],
                    )
                };
                crate::animation::notch::draw_rounded_rect(
                    frame,
                    cur_x,
                    pill_y,
                    ws_w,
                    pill_h,
                    pill_h / 2,
                    bg,
                    border,
                );
                let num_str = format!("{}", i);
                let text_x = cur_x + ((ws_w - 8) / 2) as i32;
                let text_y = pill_y + ((pill_h - 12) / 2) as i32;
                crate::animation::notch::draw_text(
                    frame, &num_str, text_x, text_y, 16, 11, true, text_col,
                );

                // Register hit target for workspace switching
                hits.push((
                    crate::bar::HIT_BAR_WORKSPACE_BASE - i as isize,
                    cur_x,
                    pill_y,
                    ws_w,
                    pill_h,
                ));

                cur_x += ws_w as i32 + 6;
            }
        }

        // Window Title
        if self.bar_module("left", "window") && !metrics.window_title.is_empty() {
            cur_x += 6;
            let app_name = crate::media::app_name_from_title(&metrics.window_title);
            let display_text = if app_name.chars().count() < metrics.window_title.chars().count() {
                format!("{} — {}", app_name, metrics.window_title)
            } else {
                metrics.window_title.clone()
            };
            // Taskbar style: keep head and tail so a long
            // `HOST: session` title keeps its distinctive tail.
            let truncated = ellipsize_middle(&display_text, 36);
            let title_w = (truncated.chars().count() as u32 * 8 + 24).clamp(60, 260);

            crate::animation::notch::draw_rounded_rect(
                frame,
                cur_x,
                pill_y,
                title_w,
                pill_h,
                pill_h / 2,
                [255, 255, 255, 18],
                [255, 255, 255, 30],
            );
            crate::animation::notch::draw_text(
                frame,
                &truncated,
                cur_x + 10,
                pill_y + ((pill_h - 12) / 2) as i32,
                title_w - 16,
                11,
                false,
                [230, 230, 230, 230],
            );
        }

        hits
    }

    /// Bar zone 3 (right): clock, battery, volume, RAM, and CPU pills.
    /// Pure painter: draws into `frame` and returns its hit targets
    /// for the orchestrator to install.
    pub(crate) fn paint_bar_right(
        &self,
        frame: &mut FrameBuffer,
        metrics: &BarMetricsCache,
        width: u32,
        bar_x: i32,
        pill_y: i32,
        pill_h: u32,
    ) -> Vec<BarHit> {
        let mut hits: Vec<BarHit> = Vec::new();
        let mut cur_right = (width as i32) - bar_x - 12;

        if self.bar_module("right", "clock") {
            // Clock
            let clock_w = 84u32;
            cur_right -= clock_w as i32;
            crate::animation::notch::draw_rounded_rect(
                frame,
                cur_right,
                pill_y,
                clock_w,
                pill_h,
                pill_h / 2,
                [255, 255, 255, 24],
                [255, 255, 255, 36],
            );
            crate::animation::notch::draw_text(
                frame,
                &metrics.time_str,
                cur_right + 8,
                pill_y + ((pill_h - 12) / 2) as i32,
                clock_w - 12,
                11,
                true,
                [240, 240, 240, 255],
            );
        }

        if self.bar_module("right", "battery") {
            // Battery
            let (bat_opt, is_charging) = metrics.battery;
            if let Some(bat_pct) = bat_opt {
                cur_right -= 8;
                let bat_w = 78u32;
                cur_right -= bat_w as i32;
                let (bat_label, bat_label_w, bat_value) = if is_charging {
                    ("⚡ ", 20, format!("{}%", bat_pct))
                } else {
                    ("BAT ", 32, format!("{}%", bat_pct))
                };
                crate::animation::notch::draw_rounded_rect(
                    frame,
                    cur_right,
                    pill_y,
                    bat_w,
                    pill_h,
                    pill_h / 2,
                    [255, 255, 255, 20],
                    [255, 255, 255, 32],
                );
                paint_metric_text(
                    frame,
                    cur_right + 8,
                    pill_y + ((pill_h - 12) / 2) as i32,
                    bat_w - 12,
                    bat_label,
                    bat_label_w,
                    &bat_value,
                );
            }
        }

        if self.bar_module("right", "volume") {
            // Volume
            cur_right -= 8;
            let vol = metrics.volume;
            let vol_w = 78u32;
            cur_right -= vol_w as i32;
            let vol_muted = vol.muted;
            let vol_value = format!("{}%", vol.level);
            let vol_bg = if vol.muted {
                [160, 40, 40, 60]
            } else {
                [255, 255, 255, 20]
            };
            crate::animation::notch::draw_rounded_rect(
                frame,
                cur_right,
                pill_y,
                vol_w,
                pill_h,
                pill_h / 2,
                vol_bg,
                [255, 255, 255, 32],
            );
            if vol_muted {
                crate::animation::notch::draw_text(
                    frame,
                    "MUTED",
                    cur_right + 8,
                    pill_y + ((pill_h - 12) / 2) as i32,
                    vol_w - 12,
                    11,
                    false,
                    [220, 220, 220, 230],
                );
            } else {
                paint_metric_text(
                    frame,
                    cur_right + 8,
                    pill_y + ((pill_h - 12) / 2) as i32,
                    vol_w - 12,
                    "VOL ",
                    32,
                    &vol_value,
                );
            }
            hits.push((
                crate::bar::HIT_BAR_VOLUME_TOGGLE,
                cur_right,
                pill_y,
                vol_w,
                pill_h,
            ));
        }

        if self.bar_module("right", "memory") {
            // RAM
            cur_right -= 8;
            let mem_pct = metrics.memory_pct;
            let mem_w = 78u32;
            cur_right -= mem_w as i32;
            let mem_value = format!("{}%", mem_pct);
            crate::animation::notch::draw_rounded_rect(
                frame,
                cur_right,
                pill_y,
                mem_w,
                pill_h,
                pill_h / 2,
                [255, 255, 255, 20],
                [255, 255, 255, 32],
            );
            paint_metric_text(
                frame,
                cur_right + 8,
                pill_y + ((pill_h - 12) / 2) as i32,
                mem_w - 12,
                "RAM ",
                32,
                &mem_value,
            );
        }

        if self.bar_module("right", "cpu") {
            // CPU
            cur_right -= 8;
            let cpu_pct = metrics.cpu_pct;
            let cpu_w = 78u32;
            cur_right -= cpu_w as i32;
            let cpu_value = format!("{}%", cpu_pct);
            crate::animation::notch::draw_rounded_rect(
                frame,
                cur_right,
                pill_y,
                cpu_w,
                pill_h,
                pill_h / 2,
                [255, 255, 255, 20],
                [255, 255, 255, 32],
            );
            paint_metric_text(
                frame,
                cur_right + 8,
                pill_y + ((pill_h - 12) / 2) as i32,
                cpu_w - 12,
                "CPU ",
                32,
                &cpu_value,
            );
        }

        hits
    }

    /// Bar zone 4 (center): the resting island pill, or the expanded card
    /// composited below the bar. Owns [`Controller::icon_hits`]: it installs
    /// the passed-in module hits alongside the island's own.
    pub(crate) fn paint_bar_center(
        &mut self,
        frame: &mut FrameBuffer,
        state: VisualState,
        now_ms: u64,
        width: u32,
        card: BarCard,
        mut hits: Vec<BarHit>,
    ) {
        // 4. Center Module: Dynamic Island
        if !card.expanded {
            if self.bar_module("center", "island") {
                // Resting Dynamic Island pill flush in the center of the bar.
                // Shared with the hover sensor via bar_pill_rect: same rect here,
                // in the hit target below, and in set_hover.
                let (pill_cx, pill_y, pill_w, pill_h) = self.bar_pill_rect(width);

                crate::animation::notch::draw_rounded_rect(
                    frame,
                    pill_cx,
                    pill_y,
                    pill_w,
                    pill_h,
                    pill_h / 2,
                    [10, 10, 10, 190],
                    [255, 255, 255, 45],
                );

                // Mini face on the left of the pill
                let face_sz = (pill_h - 4).min(22);
                let face_x = pill_cx + 4;
                let face_y = pill_y + ((pill_h - face_sz) / 2) as i32;
                crate::animation::notch::blit_rounded(
                    frame,
                    &self.face_frame,
                    face_x,
                    face_y,
                    face_sz,
                    face_sz,
                    face_sz / 2,
                );

                // Status label or media wave inside the pill
                let label_x = face_x + face_sz as i32 + 6;
                let label_max_w = pill_w.saturating_sub((label_x - pill_cx) as u32 + 6);
                if self.media_playing() && self.island.has_widget("music") {
                    if let Some(media) = &self.media {
                        let track = format!("{} — {}", media.title, media.artist);
                        crate::animation::notch::draw_text(
                            frame,
                            &track,
                            label_x,
                            pill_y + ((pill_h - 12) / 2) as i32,
                            label_max_w,
                            11,
                            false,
                            [220, 220, 220, 240],
                        );
                    } else {
                        crate::animation::notch::draw_text(
                            frame,
                            "Now Playing",
                            label_x,
                            pill_y + ((pill_h - 12) / 2) as i32,
                            label_max_w,
                            11,
                            false,
                            [220, 220, 220, 240],
                        );
                    }
                } else if state != VisualState::Idle {
                    let (sc, _) = crate::animation::notch::accent_colors(state);
                    let dot_r = 3u32;
                    crate::animation::notch::draw_disc(
                        frame,
                        label_x + 4,
                        pill_y + (pill_h / 2) as i32,
                        dot_r,
                        sc,
                    );
                    let state_name = format!("{:?}", state);
                    crate::animation::notch::draw_text(
                        frame,
                        &state_name,
                        label_x + 12,
                        pill_y + ((pill_h - 12) / 2) as i32,
                        label_max_w.saturating_sub(14),
                        11,
                        false,
                        [220, 220, 220, 240],
                    );
                } else {
                    crate::animation::notch::draw_text(
                        frame,
                        "Termielle",
                        label_x,
                        pill_y + ((pill_h - 12) / 2) as i32,
                        label_max_w,
                        11,
                        true,
                        [220, 220, 220, 220],
                    );
                }

                // Register hit target for clicking the Dynamic Island pill
                hits.push((
                    crate::bar::HIT_BAR_ISLAND_PILL,
                    pill_cx,
                    pill_y,
                    pill_w,
                    pill_h,
                ));
            }
            self.icon_hits = hits;
        } else {
            // Expanded Dynamic Island card dropping organically below the bar!
            // Island content only renders when the center module is listed;
            // the glass card and module hits below stay unconditional.
            let mut content = self.blank_frame(card.island_w, card.exp_h);
            if self.bar_module("center", "island") {
                let sub_blobs = [crate::animation::notch::BlobRect {
                    x: 0,
                    y: 0,
                    w: card.island_w,
                    h: card.exp_h,
                    r: 18,
                    attached: true,
                }];

                let island_cfg = self.island.clone();
                self.render_content(
                    &mut content,
                    state,
                    &island_cfg,
                    crate::animation::notch::Presentation::Expanded,
                    card.island_w,
                    card.exp_h,
                    &sub_blobs,
                    now_ms,
                );
            }

            // Offset hit targets recorded in sub-frame by (island_x, island_y)
            for hit in &mut self.icon_hits {
                hit.1 += card.island_x;
                hit.2 += card.island_y;
            }
            self.icon_hits.extend(hits);

            if self.bar_module("center", "island") {
                let age_ms = now_ms.saturating_sub(self.state_since_ms);
                let (alpha, dx, dy) = Self::content_motion(
                    self.spring.as_ref(),
                    crate::animation::notch::Presentation::Expanded,
                    state,
                    age_ms,
                );
                crate::animation::notch::blend_frame_over(
                    frame,
                    &content,
                    card.island_x + dx,
                    card.island_y + dy,
                    alpha,
                );
            }
        }
    }
}

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
        _accent: [u8; 4],
        bar_x: i32,
        pill_y: i32,
        pill_h: u32,
    ) -> Vec<BarHit> {
        let mut hits = Vec::new();
        // 2. Modules Left: Workspaces + Window Title
        let mut cur_x = bar_x + 12;
        let (primary, secondary) = crate::animation::notch::ink_pair(&self.island.glass);

        if self.bar_module("left", "workspaces") {
            // Workspaces
            let ws = metrics.workspaces;
            let ws_w = 28u32;
            for i in 1..=ws.total.min(10) {
                let active = i == ws.active;
                let (bg, border, text_col) = if active {
                    (
                        [primary[0], primary[1], primary[2], 28],
                        [0, 0, 0, 0],
                        primary,
                    )
                } else {
                    ([0, 0, 0, 0], [0, 0, 0, 0], secondary)
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
                crate::animation::notch::draw_text_in_rect(
                    frame,
                    &num_str,
                    (cur_x, pill_y, ws_w, pill_h),
                    12,
                    active,
                    text_col,
                    true,
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
                [0, 0, 0, 0],
                [0, 0, 0, 0],
            );
            crate::animation::notch::draw_text_in_rect(
                frame,
                &truncated,
                (cur_x + 10, pill_y, title_w - 16, pill_h),
                12,
                false,
                primary,
                false,
            );
        }

        hits
    }

    /// Status items use a shared baseline without permanent button chrome.
    pub(crate) fn paint_bar_right(
        &self,
        frame: &mut FrameBuffer,
        metrics: &BarMetricsCache,
        width: u32,
        bar_x: i32,
        pill_y: i32,
        pill_h: u32,
    ) -> Vec<BarHit> {
        use crate::animation::notch::{draw_rounded_rect, draw_text_in_rect, ink_pair};
        let mut hits = Vec::new();
        let (primary, secondary) = ink_pair(&self.island.glass);
        let mut right = width as i32 - bar_x - 12;
        if self.bar_module("right", "clock") {
            right -= 84;
            draw_text_in_rect(
                frame,
                &metrics.time_str,
                (right, pill_y, 84, pill_h),
                12,
                false,
                primary,
                true,
            );
            right -= 16;
        }
        if self.bar_module("right", "battery") {
            if let Some(percent) = metrics.battery.0 {
                right -= 72;
                draw_text_in_rect(
                    frame,
                    &format!("{percent}%"),
                    (right, pill_y, 38, pill_h),
                    12,
                    false,
                    primary,
                    true,
                );
                let x = right + 44;
                let y = pill_y + (pill_h as i32 - 10) / 2;
                let color = if percent <= 20 && !metrics.battery.1 {
                    [85, 85, 240, 255]
                } else {
                    primary
                };
                draw_rounded_rect(frame, x, y, 22, 10, 2, [0; 4], color);
                draw_rounded_rect(frame, x + 23, y + 3, 2, 4, 1, color, [0; 4]);
                if percent > 0 {
                    let fill = (18 * u32::from(percent.min(100)) / 100).max(1);
                    draw_rounded_rect(frame, x + 2, y + 2, fill, 6, 1, color, [0; 4]);
                }
                if metrics.battery.1 {
                    draw_text_in_rect(
                        frame,
                        "ϟ",
                        (x + 5, y - 3, 12, 16),
                        14,
                        true,
                        self.island.glass.tint,
                        true,
                    );
                }
                right -= 16;
            }
        }
        if self.bar_module("right", "volume") {
            right -= 24;
            super::text::paint_speaker(
                frame,
                right,
                pill_y + (pill_h as i32 - 16) / 2,
                metrics.volume.muted,
                primary,
            );
            hits.push((
                crate::bar::HIT_BAR_VOLUME_TOGGLE,
                right - 4,
                pill_y,
                32,
                pill_h,
            ));
            right -= 16;
        }
        for (module, label, value) in [
            ("memory", "RAM", metrics.memory_pct),
            ("cpu", "CPU", metrics.cpu_pct),
        ] {
            if self.bar_module("right", module) {
                right -= 74;
                paint_metric_text(
                    frame,
                    (right, pill_y, 74, pill_h),
                    label,
                    &format!("{value}%"),
                    primary,
                    secondary,
                );
                right -= 16;
            }
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
        if card.progress < 0.35 {
            let (primary, _) = crate::animation::notch::ink_pair(&self.island.glass);
            let mut compact = self.blank_frame(width, self.island.bar.height);
            let compact_frame = &mut compact;
            if self.bar_module("center", "termielle") {
                // Resting Dynamic Island pill flush in the center of the bar.
                // Shared with the hover sensor via bar_pill_rect: same rect here,
                // in the hit target below, and in set_hover.
                let (pill_cx, local_pill_y, pill_w, pill_h) = self.bar_pill_rect(width);
                let pill_y = card.bar_y + local_pill_y;

                // The face is an optional Termielle widget. Bar mode follows
                // the same widget contract as standalone Island mode.
                let face_sz = if self.island.has_widget("face") {
                    let face_sz = (pill_h - 4).min(22);
                    let face_x = pill_cx + 4;
                    let face_y = pill_y + ((pill_h - face_sz) / 2) as i32;
                    crate::animation::notch::blit_rounded(
                        compact_frame,
                        &self.face_frame,
                        face_x,
                        face_y,
                        face_sz,
                        face_sz,
                        face_sz / 2,
                    );
                    face_sz
                } else {
                    0
                };

                // Status label or media wave inside the pill
                let label_x = if face_sz == 0 {
                    pill_cx + 10
                } else {
                    pill_cx + 4 + face_sz as i32 + 6
                };
                let label_max_w = pill_w.saturating_sub((label_x - pill_cx) as u32 + 6);
                if self.media_available() && self.island.has_widget("music") {
                    if let Some(media) = &self.media {
                        let track = media.title.as_str();
                        crate::animation::notch::draw_text_in_rect(
                            compact_frame,
                            track,
                            (label_x, pill_y, label_max_w, pill_h),
                            12,
                            false,
                            primary,
                            false,
                        );
                    } else {
                        crate::animation::notch::draw_text_in_rect(
                            compact_frame,
                            "Media",
                            (label_x, pill_y, label_max_w, pill_h),
                            12,
                            false,
                            primary,
                            false,
                        );
                    }
                } else if state != VisualState::Idle {
                    let (sc, _) = crate::animation::notch::accent_colors(state);
                    crate::animation::notch::draw_disc(
                        compact_frame,
                        label_x + 4,
                        pill_y + (pill_h / 2) as i32,
                        3,
                        sc,
                    );
                    crate::animation::notch::draw_text_in_rect(
                        compact_frame,
                        state.display_name(),
                        (label_x + 12, pill_y, label_max_w.saturating_sub(14), pill_h),
                        12,
                        false,
                        primary,
                        false,
                    );
                } else {
                    crate::animation::notch::draw_text_in_rect(
                        compact_frame,
                        "Termielle",
                        (label_x, pill_y, label_max_w, pill_h),
                        12,
                        true,
                        primary,
                        false,
                    );
                }

                // Register hit target for clicking the Dynamic Island pill
                hits.push((
                    crate::bar::HIT_BAR_TERMIELLE_MODULE,
                    pill_cx,
                    pill_y,
                    pill_w,
                    pill_h,
                ));
            }
            let alpha = (255.0
                * (1.0 - crate::animation::notch::smoothstep(0.0, 0.35, card.progress)))
            .round() as u8;
            let bar_y = card.bar_y;
            crate::animation::notch::blend_frame_over(frame, &compact, 0, bar_y, alpha);
        }
        if card.expanded && self.bar_module("center", "termielle") {
            let (pill_cx, local_pill_y, pill_w, pill_h) = self.bar_pill_rect(width);
            let pill_y = card.bar_y + local_pill_y;
            hits.push((
                crate::bar::HIT_BAR_TERMIELLE_MODULE,
                pill_cx,
                pill_y,
                pill_w,
                pill_h,
            ));
        }
        if !card.expanded {
            self.icon_hits = hits;
        } else {
            // Expanded Dynamic Island card dropping organically below the bar!
            // Island content only renders when the center module is listed;
            // the glass card and module hits below stay unconditional.
            let mut content = self.blank_frame(card.content_w, card.content_h);
            if self.bar_module("center", "termielle") {
                let sub_blobs = [crate::animation::notch::BlobRect {
                    x: 0,
                    y: 0,
                    w: card.content_w,
                    h: card.content_h,
                    r: 18,
                    attached: true,
                }];

                let island_cfg = self.island.clone();
                let saved_hover = self.hover_point;
                let content_x = (width.saturating_sub(card.content_w) / 2) as i32;
                self.hover_point = saved_hover.map(|(x, y)| (x - content_x, y - card.island_y));
                self.render_content(
                    &mut content,
                    state,
                    &island_cfg,
                    crate::animation::notch::Presentation::Expanded,
                    card.content_w,
                    card.content_h,
                    &sub_blobs,
                    now_ms,
                );
                self.hover_point = saved_hover;
            }

            // Offset hit targets recorded in sub-frame by (island_x, island_y)
            let content_x = (width.saturating_sub(card.content_w) / 2) as i32;
            for hit in &mut self.icon_hits {
                hit.1 += content_x;
                hit.2 += card.island_y;
            }
            // A fading or clipped control must not intercept clicks before it is visible.
            if card.progress < 0.98 {
                self.icon_hits.clear();
            }
            self.icon_hits.extend(hits);

            if self.bar_module("center", "termielle") {
                let alpha = (255.0 * crate::animation::notch::smoothstep(0.65, 0.98, card.progress))
                    .round() as u8;
                let mut clipped = self.blank_frame(card.island_w, card.exp_h);
                crate::animation::notch::blend_frame_over(
                    &mut clipped,
                    &content,
                    content_x - card.island_x,
                    0,
                    255,
                );
                crate::animation::notch::blend_frame_over(
                    frame,
                    &clipped,
                    card.island_x,
                    card.island_y,
                    alpha,
                );
            }
        }
    }
}

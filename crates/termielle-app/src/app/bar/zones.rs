//! Bar zone painters: left (workspaces/title), right (clock/stats), center (island).

use super::super::controller::Controller;
use super::text::ellipsize_middle;
use super::types::{
    BATTERY_W, BarCard, BarHit, BarMetricsCache, CLOCK_ICON, CLOCK_TEXT_W, CLOCK_W,
    CONTROL_CENTER_W, ICON, ICON_GAP, METRIC_W, MODULE_GAP, VALUE_W, VOLUME_W,
};
use crate::animation::FrameBuffer;
use termielle_core::VisualState;

const MACCHIATO_SURFACE: [u8; 4] = [79, 58, 54, 232];
const MACCHIATO_ACTIVE_SURFACE: [u8; 4] = [100, 77, 73, 255];
const MACCHIATO_BORDER: [u8; 4] = [246, 160, 198, 44];
const MACCHIATO_TEXT: [u8; 4] = [245, 211, 202, 255];
const MACCHIATO_MAUVE: [u8; 4] = [246, 160, 198, 255];
const MACCHIATO_PEACH: [u8; 4] = [127, 169, 245, 255];
const MACCHIATO_ICON_SIZE: i32 = 18;

fn paint_macchiato_chip(frame: &mut FrameBuffer, x: i32, y: i32, width: u32, height: u32) {
    crate::animation::notch::draw_rounded_rect(
        frame,
        x,
        y,
        width,
        height,
        8,
        MACCHIATO_SURFACE,
        MACCHIATO_BORDER,
    );
}

impl Controller {
    /// Bar zone 2 (left): workspace switcher plus the active-window title
    /// pill. Pure painter: draws into `frame` and returns its hit targets
    /// for the orchestrator to install.
    pub(crate) fn paint_bar_left(
        &self,
        frame: &mut FrameBuffer,
        metrics: &BarMetricsCache,
        bar_x: i32,
        pill_y: i32,
        pill_h: u32,
        left_limit: i32,
    ) -> Vec<BarHit> {
        let mut hits = Vec::new();
        // 2. Modules Left: Workspaces + Window Title
        let mut cur_x = bar_x + 12;
        let macchiato = self.island.theme == "catppuccin-macchiato";
        let (primary, secondary) = if macchiato {
            (MACCHIATO_TEXT, MACCHIATO_MAUVE)
        } else {
            crate::animation::notch::ink_pair(&self.island.glass)
        };
        // Everything the left zone paints is laid out from the same anchor and
        // dropped, lowest priority first, when the center pill runs out of
        // room. Hidden controls must not keep hit targets either.
        let room = |x: i32, w: u32| x + w as i32 <= left_limit;
        let apps_enabled = self.bar_module("left", "apps");

        if self.island.is_bar() && self.island.bar.replace_taskbar {
            use crate::bar::shell::ShellAction;
            for action in ShellAction::ALL {
                let control_w = action.control_width();
                if !room(cur_x, control_w) {
                    break;
                }
                crate::animation::notch::draw_rounded_rect(
                    frame,
                    cur_x,
                    pill_y,
                    control_w,
                    pill_h,
                    8,
                    // A scrim, not a highlight: the bar is translucent, so a
                    // light wash would fight the 10 px label instead of
                    // helping it read over whatever is behind the strip.
                    [0, 0, 0, 56],
                    [255, 255, 255, 32],
                );
                crate::animation::notch::draw_text_in_rect(
                    frame,
                    action.label_of(),
                    (cur_x, pill_y, control_w, pill_h),
                    10,
                    false,
                    primary,
                    true,
                );
                hits.push((action.hit_id(), cur_x, pill_y, control_w, pill_h));
                cur_x += control_w as i32 + 4;
            }
            cur_x += 6;

            // Native replacement mode keeps real application buttons in the
            // bar. The worker already filters to visible, non-cloaked windows.
            if !apps_enabled {
                let icon = (pill_h.saturating_sub(8)).min(24);
                for task in self.tasks.iter().take(4) {
                    if !room(cur_x, icon) {
                        break;
                    }
                    crate::animation::notch::blit_rounded_pixels(
                        frame,
                        &task.pixels_pbgra,
                        task.width,
                        task.height,
                        cur_x + 2,
                        pill_y + (pill_h as i32 - icon as i32) / 2,
                        icon,
                        icon,
                        6,
                    );
                    hits.push((task.hwnd, cur_x, pill_y, icon, pill_h));
                    cur_x += icon as i32 + 6;
                }
            }
        }

        let mut grouped_items = false;
        if apps_enabled {
            let mut next_x = cur_x + 4;
            let mut apps = Vec::new();
            for task in self.tasks.iter().take(6) {
                if !room(next_x, 32) {
                    break;
                }
                apps.push((task, next_x));
                next_x += 32;
            }
            if !apps.is_empty() {
                crate::animation::notch::draw_rounded_rect(
                    frame,
                    cur_x,
                    pill_y,
                    (next_x - cur_x) as u32,
                    pill_h,
                    8,
                    if macchiato {
                        MACCHIATO_SURFACE
                    } else {
                        [0, 0, 0, 96]
                    },
                    if macchiato {
                        MACCHIATO_BORDER
                    } else {
                        [255, 255, 255, 32]
                    },
                );
                grouped_items = true;
            }
            for (task, slot_x) in apps {
                if task.hwnd == metrics.foreground_hwnd {
                    crate::animation::notch::draw_rounded_rect(
                        frame,
                        slot_x,
                        pill_y,
                        28,
                        pill_h,
                        7,
                        if macchiato {
                            MACCHIATO_ACTIVE_SURFACE
                        } else {
                            [primary[0], primary[1], primary[2], 64]
                        },
                        [0; 4],
                    );
                }
                crate::animation::notch::blit_rounded_pixels(
                    frame,
                    &task.pixels_pbgra,
                    task.width,
                    task.height,
                    slot_x + 5,
                    pill_y + (pill_h as i32 - 18) / 2,
                    18,
                    18,
                    5,
                );
                hits.push((task.hwnd, slot_x, pill_y, 28, pill_h));
            }
            if grouped_items {
                cur_x = next_x;
            }
        } else if self.bar_module("left", "workspaces") {
            let ws = metrics.workspaces;
            let mut next_x = cur_x + if macchiato { 4 } else { 0 };
            let mut slots = Vec::new();
            for index in 1..=ws.total.min(10) {
                let slot_w = if macchiato && index == ws.active {
                    46
                } else {
                    28
                };
                if !room(next_x, slot_w + if macchiato { 4 } else { 0 }) {
                    break;
                }
                slots.push((index, next_x, slot_w));
                next_x += slot_w as i32 + if macchiato { 4 } else { 6 };
            }
            if macchiato && !slots.is_empty() {
                crate::animation::notch::draw_rounded_rect(
                    frame,
                    cur_x,
                    pill_y,
                    (next_x - cur_x) as u32,
                    pill_h,
                    8,
                    MACCHIATO_SURFACE,
                    MACCHIATO_BORDER,
                );
                grouped_items = true;
            }
            for (index, slot_x, slot_w) in &slots {
                let active = *index == ws.active;
                let (bg, border, text_col) = if macchiato && active {
                    (MACCHIATO_ACTIVE_SURFACE, [0; 4], MACCHIATO_PEACH)
                } else if macchiato {
                    ([0; 4], [0; 4], secondary)
                } else if active {
                    ([primary[0], primary[1], primary[2], 28], [0; 4], primary)
                } else {
                    ([0; 4], [0; 4], secondary)
                };
                crate::animation::notch::draw_rounded_rect(
                    frame,
                    *slot_x,
                    pill_y,
                    *slot_w,
                    pill_h,
                    if macchiato {
                        7
                    } else {
                        (*slot_w).min(pill_h) / 2
                    },
                    bg,
                    border,
                );
                let icon = if macchiato && active {
                    metrics.foreground_icon.as_ref()
                } else {
                    None
                };
                crate::animation::notch::draw_text_in_rect(
                    frame,
                    &index.to_string(),
                    (
                        *slot_x,
                        pill_y,
                        if icon.is_some() { 20 } else { *slot_w },
                        pill_h,
                    ),
                    12,
                    active,
                    text_col,
                    true,
                );
                if let Some(icon) = icon {
                    crate::animation::notch::blit_rounded_pixels(
                        frame,
                        &icon.pixels_pbgra,
                        icon.width,
                        icon.height,
                        *slot_x + 24,
                        pill_y + (pill_h as i32 - 16) / 2,
                        16,
                        16,
                        4,
                    );
                }
                hits.push((
                    crate::bar::HIT_BAR_WORKSPACE_BASE - *index as isize,
                    *slot_x,
                    pill_y,
                    *slot_w,
                    pill_h,
                ));
            }
            if !slots.is_empty() {
                cur_x = next_x;
            }
        }

        // Window Title
        if self.bar_module("left", "window") && !metrics.window_title.is_empty() {
            let title_x = cur_x + if grouped_items { 24 } else { 6 };
            let app_name = crate::media::app_name_from_title(&metrics.window_title);
            let display_text = if macchiato && !app_name.is_empty() {
                app_name
            } else if app_name.chars().count() < metrics.window_title.chars().count() {
                format!("{} — {}", app_name, metrics.window_title)
            } else {
                metrics.window_title.clone()
            };
            // Taskbar style: keep head and tail so a long
            // `HOST: session` title keeps its distinctive tail.
            let truncated = ellipsize_middle(&display_text, if macchiato { 20 } else { 36 });
            let title_w = (truncated.chars().count() as u32 * 8 + 24)
                .clamp(60, if macchiato { 184 } else { 260 });
            // Nothing legible fits: the title yields to the fixed zones.
            let title_w = if room(title_x, title_w) {
                title_w
            } else {
                let available = (left_limit - title_x).max(0) as u32;
                if available < 60 { 0 } else { available }
            };
            if title_w == 0 {
                return hits;
            }
            if grouped_items {
                crate::animation::notch::draw_text_in_rect(
                    frame,
                    "›",
                    (cur_x + 7, pill_y, 12, pill_h),
                    20,
                    true,
                    MACCHIATO_PEACH,
                    true,
                );
            }

            crate::animation::notch::draw_rounded_rect(
                frame,
                title_x,
                pill_y,
                title_w,
                pill_h,
                if macchiato { 8 } else { pill_h / 2 },
                if macchiato { MACCHIATO_SURFACE } else { [0; 4] },
                if macchiato { MACCHIATO_BORDER } else { [0; 4] },
            );
            crate::animation::notch::draw_text_in_rect(
                frame,
                &truncated,
                (title_x + 10, pill_y, title_w - 16, pill_h),
                12,
                false,
                primary,
                false,
            );
        }

        hits
    }

    /// Status items use a shared baseline without permanent button chrome.
    ///
    /// The zone is a set of readouts, so a module is an icon plus its number
    /// and no label word: the glyph names the metric and the number is the
    /// thing being read. Dropping "CPU" and "RAM" frees about 60 px and lets
    /// the numbers carry bold primary ink against a dimmer icon.
    pub(crate) fn paint_bar_right(
        &self,
        frame: &mut FrameBuffer,
        metrics: &BarMetricsCache,
        width: u32,
        bar_x: i32,
        pill_y: i32,
        pill_h: u32,
    ) -> Vec<BarHit> {
        use crate::animation::icons;
        use crate::animation::notch::{draw_text_in_rect, ink_pair};
        let mut hits = Vec::new();
        let macchiato = self.island.theme == "catppuccin-macchiato";
        let (primary, secondary) = if macchiato {
            (MACCHIATO_TEXT, MACCHIATO_MAUVE)
        } else {
            ink_pair(&self.island.glass)
        };
        let capsule_padding = if macchiato { 6 } else { 0 };
        let icon_size = if macchiato { MACCHIATO_ICON_SIZE } else { ICON };
        let clock_icon_size = if macchiato {
            MACCHIATO_ICON_SIZE
        } else {
            CLOCK_ICON
        };
        let control_center_icon_size = if macchiato {
            MACCHIATO_ICON_SIZE
        } else {
            CONTROL_CENTER_W
        };
        let volume_icon_size = if macchiato {
            MACCHIATO_ICON_SIZE
        } else {
            VOLUME_W
        };
        let mut right = width as i32 - bar_x - 12;
        if self.bar_module("right", "clock") {
            let capsule_width = CLOCK_W + (clock_icon_size - CLOCK_ICON) + capsule_padding * 2;
            right -= capsule_width;
            if macchiato {
                paint_macchiato_chip(frame, right, pill_y, capsule_width as u32, pill_h);
            }
            let content_x = right + capsule_padding;
            icons::draw_icon(
                frame,
                icons::CLOCK,
                content_x,
                pill_y + (pill_h as i32 - clock_icon_size) / 2,
                clock_icon_size as u32,
                secondary,
            );
            draw_text_in_rect(
                frame,
                &metrics.time_str,
                (
                    content_x + clock_icon_size + ICON_GAP,
                    pill_y,
                    CLOCK_TEXT_W,
                    pill_h,
                ),
                12,
                true,
                primary,
                true,
            );
            hits.push((
                crate::app::types::HIT_CARD_NOTIFICATIONS,
                right,
                pill_y,
                capsule_width as u32,
                pill_h,
            ));
            if self.unread_notifications > 0 {
                crate::animation::notch::draw_rounded_rect(
                    frame,
                    right + capsule_width - 8,
                    pill_y + 3,
                    5,
                    5,
                    3,
                    secondary,
                    [0; 4],
                );
            }
            right -= MODULE_GAP;
        }
        if self.bar_module("right", "control_center") {
            // Where macOS puts it: a menu-bar item between the status icons
            // and the clock, live whether or not a card is open. It used to
            // live as a glyph inside the pill, which made it unreachable the
            // moment hovering the pill opened that card.
            let capsule_width = control_center_icon_size + capsule_padding * 2;
            right -= capsule_width;
            if macchiato {
                paint_macchiato_chip(frame, right, pill_y, capsule_width as u32, pill_h);
            }
            icons::draw_icon(
                frame,
                icons::ADJUSTMENTS,
                right + capsule_padding,
                pill_y + (pill_h as i32 - control_center_icon_size) / 2,
                control_center_icon_size as u32,
                primary,
            );
            hits.push((
                crate::app::types::HIT_CARD_PANEL,
                right - if macchiato { 0 } else { 4 },
                pill_y,
                (capsule_width + if macchiato { 0 } else { 8 }) as u32,
                pill_h,
            ));
            right -= MODULE_GAP;
        }
        if self.bar_module("right", "battery") {
            if let Some(percent) = metrics.battery.0 {
                let capsule_width = BATTERY_W + (icon_size - ICON) + capsule_padding * 2;
                right -= capsule_width;
                if macchiato {
                    paint_macchiato_chip(frame, right, pill_y, capsule_width as u32, pill_h);
                }
                let content_x = right + capsule_padding;
                // Low battery is the one case that outranks the theme's ink,
                // and charging is the one state worth a different glyph.
                let ink = if percent <= 20 && !metrics.battery.1 {
                    [85, 85, 240, 255]
                } else {
                    primary
                };
                draw_text_in_rect(
                    frame,
                    &format!("{percent}%"),
                    (content_x, pill_y, VALUE_W, pill_h),
                    13,
                    true,
                    ink,
                    true,
                );
                icons::draw_icon(
                    frame,
                    if metrics.battery.1 {
                        icons::BATTERY_CHARGING
                    } else {
                        icons::BATTERY
                    },
                    content_x + VALUE_W as i32 + ICON_GAP,
                    pill_y + (pill_h as i32 - icon_size) / 2,
                    icon_size as u32,
                    ink,
                );
                right -= MODULE_GAP;
            }
        }
        if self.bar_module("right", "volume") {
            let capsule_width = volume_icon_size + capsule_padding * 2;
            right -= capsule_width;
            if macchiato {
                paint_macchiato_chip(frame, right, pill_y, capsule_width as u32, pill_h);
            }
            icons::draw_icon(
                frame,
                if metrics.volume.muted {
                    icons::VOLUME_MUTED
                } else {
                    icons::VOLUME
                },
                right + capsule_padding,
                pill_y + (pill_h as i32 - volume_icon_size) / 2,
                volume_icon_size as u32,
                primary,
            );
            hits.push((
                crate::bar::HIT_BAR_VOLUME_TOGGLE,
                right - if macchiato { 0 } else { 4 },
                pill_y,
                (capsule_width + if macchiato { 0 } else { 8 }) as u32,
                pill_h,
            ));
            right -= MODULE_GAP;
        }
        if self.bar_module("right", "network") {
            let capsule_width = icon_size + capsule_padding * 2;
            right -= capsule_width;
            if macchiato {
                paint_macchiato_chip(frame, right, pill_y, capsule_width as u32, pill_h);
            }
            let ink = match metrics.connectivity.network {
                crate::bar::connectivity::NetworkState::Online => primary,
                crate::bar::connectivity::NetworkState::Limited => [120, 185, 245, 255],
                _ => secondary,
            };
            icons::draw_icon(
                frame,
                icons::WIFI,
                right + capsule_padding,
                pill_y + (pill_h as i32 - icon_size) / 2,
                icon_size as u32,
                ink,
            );
            hits.push((
                crate::bar::shell::ShellAction::Network.hit_id(),
                right,
                pill_y,
                capsule_width as u32,
                pill_h,
            ));
            right -= MODULE_GAP;
        }
        for (module, icon, value) in [
            ("memory", icons::MEMORY, metrics.memory_pct),
            ("cpu", icons::CPU, metrics.cpu_pct),
        ] {
            if self.bar_module("right", module) {
                let capsule_width = METRIC_W + (icon_size - ICON) + capsule_padding * 2;
                right -= capsule_width;
                if macchiato {
                    paint_macchiato_chip(frame, right, pill_y, capsule_width as u32, pill_h);
                }
                let content_x = right + capsule_padding;
                icons::draw_icon(
                    frame,
                    icon,
                    content_x,
                    pill_y + (pill_h as i32 - icon_size) / 2,
                    icon_size as u32,
                    secondary,
                );
                draw_text_in_rect(
                    frame,
                    &format!("{value}%"),
                    (content_x + icon_size + ICON_GAP, pill_y, VALUE_W, pill_h),
                    13,
                    true,
                    primary,
                    true,
                );
                right -= MODULE_GAP;
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
        //
        // The pill lives in the strip; island cards and alerts morph it away.
        // Control Center does not. Window height cannot decide pill opacity
        // because either popover may keep that same window tall.
        if !card.pill_morphing || card.progress < 0.35 {
            let (primary, _) = crate::animation::notch::ink_pair(&self.island.glass);
            let mut compact = self.blank_frame(width, self.island.bar.height);
            let compact_frame = &mut compact;
            if self.bar_module("center", "termielle") {
                // Resting Dynamic Island pill flush in the center of the bar.
                // Shared with the hover sensor via bar_pill_rect: same rect here,
                // in the hit target below, and in set_hover.
                let (pill_cx, local_pill_y, pill_w, pill_h) = self.bar_pill_rect(width);
                let pill_y = card.bar_y + local_pill_y;

                // The pill is its own piece of glass, not a hole in the strip:
                // the content below rides on top of a rounded chip with the
                // theme's tint, border, highlight and shadow. Without it the
                // center reads as whatever happens to be behind the
                // translucent strip, and a dark window behind the bar looks
                // like a mis-painted rectangle rather than an island.
                //
                // It is painted into the compact frame rather than the bar
                // frame, so it inherits that frame's fade and dissolves with
                // the pill as the card takes over.
                let pill_r = pill_h / 2;
                let pill_blobs = [crate::animation::notch::BlobRect {
                    x: 0,
                    y: 0,
                    w: pill_w,
                    h: pill_h,
                    r: pill_r,
                    attached: false,
                }];
                let chip =
                    self.glass_layer_blobs(pill_w, pill_h, pill_r, false, &pill_blobs, false);
                crate::animation::notch::blend_frame_over(
                    compact_frame,
                    &chip,
                    pill_cx,
                    local_pill_y,
                    255,
                );

                // The face is an optional Termielle widget. Bar mode follows
                // the same widget contract as standalone Island mode.
                let volume_feedback = !card.pill_morphing
                    && !self.island_card_open()
                    && self.alerts.is_empty()
                    && self.volume_feedback_deadline.is_some_and(|at| now_ms < at);
                let face_sz = if volume_feedback {
                    0
                } else if self.island.has_widget("face") {
                    let face_sz = (pill_h - 4).min(22);
                    let face_x = pill_cx + 4;
                    let face_y = local_pill_y + ((pill_h - face_sz) / 2) as i32;
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
                if volume_feedback {
                    self.paint_bar_volume_feedback(
                        compact_frame,
                        (pill_cx, local_pill_y, pill_w, pill_h),
                    );
                } else if self.media_available() && self.island.has_widget("music") {
                    if let Some(media) = &self.media {
                        let track = media.title.as_str();
                        crate::animation::notch::draw_text_in_rect(
                            compact_frame,
                            track,
                            (label_x, local_pill_y, label_max_w, pill_h),
                            12,
                            false,
                            primary,
                            false,
                        );
                    } else {
                        crate::animation::notch::draw_text_in_rect(
                            compact_frame,
                            "Media",
                            (label_x, local_pill_y, label_max_w, pill_h),
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
                        local_pill_y + (pill_h / 2) as i32,
                        3,
                        sc,
                    );
                    crate::animation::notch::draw_text_in_rect(
                        compact_frame,
                        state.display_name(),
                        (
                            label_x + 12,
                            local_pill_y,
                            label_max_w.saturating_sub(14),
                            pill_h,
                        ),
                        12,
                        false,
                        primary,
                        false,
                    );
                } else {
                    crate::animation::notch::draw_text_in_rect(
                        compact_frame,
                        "Termielle",
                        (label_x, local_pill_y, label_max_w, pill_h),
                        12,
                        true,
                        primary,
                        false,
                    );
                }
                // The Control Center moved to the strip, so the pill's label
                // runs the full width again.
                // Register hit target for clicking the Dynamic Island pill
                hits.push((
                    crate::bar::HIT_BAR_TERMIELLE_MODULE,
                    pill_cx,
                    pill_y,
                    pill_w,
                    pill_h,
                ));
            }
            // The pill fades only for its own card or an alert morph.
            let alpha = if card.pill_morphing {
                (255.0 * (1.0 - crate::animation::notch::smoothstep(0.0, 0.35, card.progress)))
                    .round() as u8
            } else {
                255
            };
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
                let content_x = card.content_x;
                self.hover_point = saved_hover.map(|(x, y)| (x - content_x, y - card.island_y));
                if !self.alert_pill_morphing || !self.alerts.is_empty() || self.island_card_open() {
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
                }
                self.hover_point = saved_hover;
            }

            // Offset hit targets recorded in sub-frame by (content_x, island_y)
            let content_x = card.content_x;
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
                let (fade_start, fade_end) = if self.alerts.is_empty() {
                    (0.65, 0.98)
                } else {
                    (0.35, 0.85)
                };
                let alpha = (255.0
                    * crate::animation::notch::smoothstep(fade_start, fade_end, card.progress))
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

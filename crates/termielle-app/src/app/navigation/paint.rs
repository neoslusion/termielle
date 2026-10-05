use super::model::{Action, Group, Mode, PAGE_SIZE, SLOT};
use crate::animation::{FrameBuffer, notch};
use crate::app::bar::types::BarHit;
use crate::app::controller::Controller;

fn contains(point: Option<(i32, i32)>, rect: (i32, i32, u32, u32)) -> bool {
    point.is_some_and(|(x, y)| {
        x >= rect.0 && x < rect.0 + rect.2 as i32 && y >= rect.1 && y < rect.1 + rect.3 as i32
    })
}
fn group_icon(frame: &mut FrameBuffer, group: &Group, x: i32, y: i32, size: u32, ink: [u8; 4]) {
    if let Some(icon) = &group.icon {
        notch::blit_rounded_pixels(
            frame,
            &icon.pixels_pbgra,
            icon.width,
            icon.height,
            x,
            y,
            size,
            size,
            4,
        );
    } else {
        notch::draw_rounded_rect(
            frame,
            x,
            y,
            size,
            size,
            5,
            [ink[0], ink[1], ink[2], 35],
            [0; 4],
        );
        let letter = group
            .name
            .chars()
            .find(|c| c.is_alphanumeric())
            .unwrap_or('?')
            .to_uppercase()
            .to_string();
        notch::draw_text_in_rect(frame, &letter, (x, y, size, size), 12, true, ink, true);
    }
}

impl Controller {
    pub(crate) fn paint_navigation_rail(
        &self,
        frame: &mut FrameBuffer,
        mut x: i32,
        y: i32,
        h: u32,
        limit: i32,
        foreground: isize,
    ) -> (Vec<BarHit>, i32) {
        let mut hits = Vec::new();
        self.navigation.rail_actions.borrow_mut().clear();
        let (primary, secondary) = notch::ink_pair(&self.island.glass);
        let room = (limit - x).max(0) as u32;
        if room < 32 {
            return (hits, x);
        }
        let narrow = room < 104;
        let launcher_w = if narrow { 32 } else { 64 };
        let action = if narrow {
            Action::Overflow
        } else {
            Action::Launcher
        };
        notch::draw_rounded_rect(
            frame,
            x,
            y,
            launcher_w,
            h,
            7,
            [primary[0], primary[1], primary[2], 25],
            [primary[0], primary[1], primary[2], 35],
        );
        crate::animation::icons::draw_icon(
            frame,
            crate::animation::icons::SEARCH,
            x + 8,
            y + (h as i32 - 16) / 2,
            16,
            primary,
        );
        if !narrow {
            notch::draw_text_in_rect(frame, "Apps", (x + 28, y, 32, h), 11, true, primary, true);
        }
        hits.push((self.navigation.rail_action(action), x, y, launcher_w, h));
        x += launcher_w as i32 + 6;
        if narrow {
            return (hits, x);
        }
        let capacity = (limit - x - 32).max(0) as usize / SLOT as usize;
        for group in self.navigation.groups.iter().take(capacity) {
            let active = group.windows.iter().any(|w| w.hwnd == foreground);
            let rect = (x, y, SLOT - 2, h);
            let hover = contains(self.hover_point, rect);
            notch::draw_rounded_rect(
                frame,
                x,
                y,
                SLOT - 2,
                h,
                7,
                if self.navigation.rail_pressed.as_ref() == Some(&Action::Group(group.key.clone()))
                {
                    [primary[0], primary[1], primary[2], 95]
                } else if active {
                    [primary[0], primary[1], primary[2], 60]
                } else if hover {
                    [primary[0], primary[1], primary[2], 30]
                } else {
                    [0; 4]
                },
                [0; 4],
            );
            if let Some((_, after)) = self
                .navigation
                .drag
                .as_ref()
                .filter(|drag| drag.active)
                .and_then(|drag| drag.target.as_ref())
                .filter(|(key, _)| *key == group.key)
            {
                notch::draw_rounded_rect(
                    frame,
                    x + if *after { SLOT as i32 - 1 } else { -1 },
                    y,
                    2,
                    h,
                    1,
                    primary,
                    [0; 4],
                );
            }
            let size = 20.min(h.saturating_sub(6));
            group_icon(
                frame,
                group,
                x + 7,
                y + (h as i32 - size as i32) / 2 - 1,
                size,
                primary,
            );
            if self.navigation.pending.contains_key(&group.key) {
                notch::draw_text_in_rect(frame, "…", (x + 19, y, 16, 12), 10, true, primary, true);
            }
            if !group.windows.is_empty() {
                let mark_w = if active { 14 } else { 4 };
                notch::draw_rounded_rect(
                    frame,
                    x + (SLOT as i32 - mark_w as i32) / 2,
                    y + h as i32 - 3,
                    mark_w,
                    2,
                    1,
                    if active { primary } else { secondary },
                    [0; 4],
                );
            }
            if group.windows.len() > 1 {
                let count = if group.windows.len() > 9 {
                    "9+".into()
                } else {
                    group.windows.len().to_string()
                };
                let mut badge = self.island.glass.tint;
                badge[3] = 255;
                notch::draw_rounded_rect(frame, x + 23, y, 12, 11, 3, badge, [0; 4]);
                notch::draw_text_in_rect(
                    frame,
                    &count,
                    (x + 23, y, 12, 11),
                    9,
                    true,
                    primary,
                    true,
                );
            }
            hits.push((
                self.navigation
                    .rail_action(Action::Group(group.key.clone())),
                x,
                y,
                SLOT,
                h,
            ));
            x += SLOT as i32;
        }
        notch::draw_rounded_rect(
            frame,
            x,
            y,
            32,
            h,
            7,
            [primary[0], primary[1], primary[2], 20],
            [0; 4],
        );
        notch::draw_text_in_rect(frame, "···", (x, y, 32, h), 16, true, primary, true);
        hits.push((self.navigation.rail_action(Action::Overflow), x, y, 32, h));
        (hits, x + 32)
    }

    fn navigation_button(
        &mut self,
        frame: &mut FrameBuffer,
        label: &str,
        rect: (i32, i32, u32, u32),
        action: Option<Action>,
        hits: &mut Vec<BarHit>,
    ) {
        let (primary, secondary) = notch::ink_pair(&self.island.glass);
        let selected = action
            .as_ref()
            .is_some_and(|a| self.navigation.selected.as_ref() == Some(a));
        notch::draw_rounded_rect(
            frame,
            rect.0,
            rect.1,
            rect.2,
            rect.3,
            6,
            if selected {
                [primary[0], primary[1], primary[2], 55]
            } else {
                [primary[0], primary[1], primary[2], 20]
            },
            if selected { primary } else { [0; 4] },
        );
        notch::draw_text_in_rect(
            frame,
            label,
            rect,
            11,
            false,
            if action.is_some() { primary } else { secondary },
            true,
        );
        if let Some(action) = action {
            hits.push((
                self.navigation.popup_action(action),
                rect.0,
                rect.1,
                rect.2,
                rect.3,
            ));
        }
    }
    pub(crate) fn paint_navigation_popup(
        &mut self,
        frame: &mut FrameBuffer,
        width: u32,
        exp_h: u32,
        bar_y: i32,
    ) {
        let Some(popup) = self.navigation.popup.clone() else {
            return;
        };
        let (x, y, w, h) = self.navigation_rect(width, exp_h, bar_y);
        if w < 128 || h == 0 {
            self.navigation.popup_actions.clear();
            return;
        }
        let full_h = self.navigation.height();
        let mut content = self.blank_frame(w, full_h);
        let mut tint = self.island.glass.tint;
        tint[3] = 245;
        notch::draw_rounded_rect(&mut content, 0, 0, w, full_h, 12, tint, [255, 255, 255, 30]);
        let (primary, secondary) = notch::ink_pair(&self.island.glass);
        self.navigation.popup_actions.clear();
        let mut hits = Vec::new();
        let group = match &popup.mode {
            Mode::App(key) => self.navigation.group(key).cloned(),
            _ => None,
        };
        let title = match &popup.mode {
            Mode::App(_) => group.as_ref().map_or("Application", |g| g.name.as_str()),
            Mode::Overflow => "Applications",
            Mode::Notice => "Couldn't complete that action",
        };
        let title_x = if group.is_some() { 42 } else { 16 };
        if group.is_some() {
            self.navigation_button(
                &mut content,
                "‹",
                (12, 12, 24, 26),
                Some(Action::Back),
                &mut hits,
            );
        }
        notch::draw_text_in_rect(
            &mut content,
            title,
            (title_x, 12, w.saturating_sub(title_x as u32 + 48), 22),
            14,
            true,
            primary,
            false,
        );
        self.navigation_button(
            &mut content,
            "×",
            (w as i32 - 38, 12, 24, 26),
            Some(Action::Dismiss),
            &mut hits,
        );
        let subtitle = if let Some(g) = &group {
            format!(
                "{} · {} window{}",
                if g.pinned { "Pinned" } else { "Running" },
                g.windows.len(),
                if g.windows.len() == 1 { "" } else { "s" }
            )
        } else {
            format!("{} apps · Arrow keys / Enter", self.navigation.groups.len())
        };
        if popup.mode != Mode::Notice {
            notch::draw_text_in_rect(
                &mut content,
                self.navigation.notice.as_deref().unwrap_or(&subtitle),
                (16, 38, w - 32, 16),
                10,
                false,
                secondary,
                false,
            );
        }
        let start = popup.page * PAGE_SIZE;
        let row_actions = if let Some(g) = &group {
            g.windows
                .iter()
                .skip(start)
                .take(PAGE_SIZE)
                .map(|win| Action::Focus(win.hwnd, win.process_id))
                .collect::<Vec<_>>()
        } else if popup.mode == Mode::Overflow {
            self.navigation
                .groups
                .iter()
                .skip(start)
                .take(PAGE_SIZE)
                .map(|g| Action::Group(g.key.clone()))
                .collect()
        } else {
            Vec::new()
        };
        // Keep keyboard selection attached to identity, not a mutable row index.
        if self.navigation.selected.as_ref().is_none_or(|a| {
            !row_actions.contains(a)
                && !matches!(
                    a,
                    Action::Pin(_)
                        | Action::Unpin(_)
                        | Action::Launch(_)
                        | Action::Page(_)
                        | Action::Back
                        | Action::Launcher
                        | Action::Settings
                )
        }) {
            self.navigation.selected = row_actions.first().cloned();
        }
        let groups = self
            .navigation
            .groups
            .iter()
            .skip(start)
            .take(PAGE_SIZE)
            .cloned()
            .collect::<Vec<_>>();
        for (i, action) in row_actions.into_iter().enumerate() {
            let row_y = 60 + i as i32 * 46;
            let selected = self.navigation.selected.as_ref() == Some(&action);
            let hover = contains(self.hover_point, (x + 12, y + row_y, w - 24, 42));
            notch::draw_rounded_rect(
                &mut content,
                12,
                row_y,
                w - 24,
                42,
                7,
                if selected {
                    [primary[0], primary[1], primary[2], 45]
                } else if hover {
                    [primary[0], primary[1], primary[2], 25]
                } else {
                    [0; 4]
                },
                if selected {
                    [primary[0], primary[1], primary[2], 90]
                } else {
                    [0; 4]
                },
            );
            let (label, detail, icon_group, close) = match &action {
                Action::Focus(hwnd, pid) => {
                    let Some(win) = group.as_ref().and_then(|g| {
                        g.windows
                            .iter()
                            .find(|win| win.hwnd == *hwnd && win.process_id == *pid)
                    }) else {
                        continue;
                    };
                    let active = self
                        .bar_metrics_cache
                        .as_ref()
                        .is_some_and(|m| m.foreground_hwnd == *hwnd);
                    (
                        win.title.clone(),
                        if win.minimized {
                            "Minimized · click to restore".into()
                        } else if active {
                            "Active window".into()
                        } else {
                            "Click to switch".into()
                        },
                        group.as_ref(),
                        Some((*hwnd, *pid)),
                    )
                }
                Action::Group(key) => {
                    let Some(g) = groups.iter().find(|g| g.key == *key) else {
                        continue;
                    };
                    (
                        g.name.clone(),
                        if g.windows.is_empty() {
                            "Pinned · click to launch".into()
                        } else {
                            format!(
                                "{} window{}{}",
                                g.windows.len(),
                                if g.windows.len() == 1 { "" } else { "s" },
                                if g.pinned { " · pinned" } else { "" }
                            )
                        },
                        Some(g),
                        None,
                    )
                }
                _ => continue,
            };
            if let Some(g) = icon_group {
                group_icon(&mut content, g, 22, row_y + 9, 24, primary);
            }
            let label_w = w.saturating_sub(if close.is_some() { 114 } else { 70 });
            notch::draw_text_in_rect(
                &mut content,
                &label,
                (56, row_y + 4, label_w, 20),
                12,
                false,
                primary,
                false,
            );
            notch::draw_text_in_rect(
                &mut content,
                &detail,
                (56, row_y + 23, label_w, 15),
                10,
                false,
                secondary,
                false,
            );
            hits.push((
                self.navigation.popup_action(action),
                12,
                row_y,
                w - 24 - if close.is_some() { 34 } else { 0 },
                42,
            ));
            if let Some((hwnd, pid)) = close {
                self.navigation_button(
                    &mut content,
                    "×",
                    (w as i32 - 44, row_y + 8, 24, 26),
                    Some(Action::Close(hwnd, pid)),
                    &mut hits,
                );
            }
        }
        let rows = self.navigation.row_count().clamp(1, PAGE_SIZE);
        if self.navigation.row_count() == 0 {
            let text = if popup.mode == Mode::Notice {
                self.navigation.notice.as_deref().unwrap_or("Try again.")
            } else if group.is_some() {
                "This app isn't running. Open a new window below."
            } else {
                "No open apps. Use Apps to launch one."
            };
            notch::draw_text_in_rect(
                &mut content,
                text,
                (18, 64, w - 36, 38),
                11,
                false,
                secondary,
                false,
            );
        }
        let footer = 60 + rows as i32 * 46 + 8;
        let last = self.navigation.row_count().saturating_sub(1) / PAGE_SIZE;
        if last > 0 {
            self.navigation_button(
                &mut content,
                "Prev",
                (16, footer, 50, 26),
                (popup.page > 0).then(|| Action::Page(popup.page - 1)),
                &mut hits,
            );
            notch::draw_text_in_rect(
                &mut content,
                &format!("{} / {}", popup.page + 1, last + 1),
                (72, footer, w.saturating_sub(144), 26),
                11,
                false,
                secondary,
                true,
            );
            self.navigation_button(
                &mut content,
                "Next",
                (w as i32 - 66, footer, 50, 26),
                (popup.page < last).then_some(Action::Page(popup.page + 1)),
                &mut hits,
            );
        }
        let button_y = footer + if last > 0 { 34 } else { 0 };
        if let Some(g) = group {
            let bw = w.saturating_sub(44) / 3;
            let pin_action = if g.pinned {
                Some(Action::Unpin(g.key.clone()))
            } else {
                g.target
                    .clone()
                    .filter(|_| self.island.bar.pinned_apps.len() < termielle_core::MAX_PINNED_APPS)
                    .map(Action::Pin)
            };
            self.navigation_button(
                &mut content,
                if g.pinned {
                    "Unpin"
                } else if g.target.is_none() {
                    "Not pinnable"
                } else if self.island.bar.pinned_apps.len() >= termielle_core::MAX_PINNED_APPS {
                    "Pin limit"
                } else {
                    "Pin"
                },
                (16, button_y, bw, 28),
                pin_action,
                &mut hits,
            );
            let launch = g
                .target
                .filter(|_| {
                    !self.navigation.launching && !self.navigation.pending.contains_key(&g.key)
                })
                .map(Action::Launch);
            self.navigation_button(
                &mut content,
                if self.navigation.launching || self.navigation.pending.contains_key(&g.key) {
                    "Opening…"
                } else {
                    "New window"
                },
                (22 + bw as i32, button_y, bw, 28),
                launch,
                &mut hits,
            );
            self.navigation_button(
                &mut content,
                "Bar settings",
                (28 + 2 * bw as i32, button_y, bw, 28),
                Some(Action::Settings),
                &mut hits,
            );
        } else {
            let bw = w.saturating_sub(38) / 2;
            self.navigation_button(
                &mut content,
                "Open launcher",
                (16, button_y, bw, 28),
                Some(Action::Launcher),
                &mut hits,
            );
            self.navigation_button(
                &mut content,
                "Termielle settings",
                (22 + bw as i32, button_y, bw, 28),
                Some(Action::Settings),
                &mut hits,
            );
        }
        // No late-morph input gate: only currently painted pixels are clickable.
        hits.retain_mut(|hit| {
            hit.4 = (hit.2 + hit.4 as i32)
                .min(h as i32)
                .saturating_sub(hit.2)
                .max(0) as u32;
            hit.1 += x;
            hit.2 += y;
            hit.4 >= 24
        });
        hits.push((self.navigation.popup_action(Action::Shield), x, y, w, h));
        self.icon_hits.splice(0..0, hits);
        let mut clipped = self.blank_frame(w, h);
        notch::blend_frame_over(&mut clipped, &content, 0, 0, 255);
        notch::blend_frame_over(frame, &clipped, x, y, 255);
    }
}

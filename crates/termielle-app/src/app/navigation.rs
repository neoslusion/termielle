//! App-navigation state, separate from agent activity and system popovers.
mod model;
mod paint;
#[cfg(test)]
mod tests;
use super::{controller::Controller, types::ClickOutcome};
pub(crate) use model::Navigation;
use model::{Action, Mode};
use termielle_core::PinnedApp;

impl Controller {
    pub fn navigation_pointer_down(&mut self, x: i32, y: i32, now: u64) {
        let point = (self.to_logical(x), self.to_logical(y));
        self.navigation.drag = None;
        self.navigation.cancelled_click = false;
        self.navigation.rail_pressed = self
            .icon_hits
            .iter()
            .find(|hit| {
                model::Navigation::is_rail_hit(hit.0)
                    && point.0 >= hit.1
                    && point.0 < hit.1 + hit.3 as i32
                    && point.1 >= hit.2
                    && point.1 < hit.2 + hit.4 as i32
            })
            .and_then(|hit| self.navigation.action(hit.0));
        self.bar_left_cache = None;
        self.morph_to_target(now);
        if self.navigation.popup.is_some() {
            return;
        }
        if let Some(key) = self
            .icon_hits
            .iter()
            .filter(|hit| model::Navigation::is_rail_hit(hit.0))
            .find(|hit| {
                point.0 >= hit.1
                    && point.0 < hit.1 + hit.3 as i32
                    && point.1 >= hit.2
                    && point.1 < hit.2 + hit.4 as i32
            })
            .and_then(|hit| self.navigation.action(hit.0))
            .and_then(|action| match action {
                Action::Group(key) => Some(key),
                _ => None,
            })
            .filter(|key| self.navigation.group(key).is_some_and(|group| group.pinned))
        {
            self.navigation.drag = Some(model::PinDrag {
                key,
                origin: point,
                baseline: self.pinned_apps().iter().map(PinnedApp::key).collect(),
                target: None,
                active: false,
            });
        }
    }
    pub fn navigation_pointer_motion(&mut self, x: i32, y: i32, now: u64) -> bool {
        let point = (self.to_logical(x), self.to_logical(y));
        let Some(drag) = self.navigation.drag.as_ref() else {
            return false;
        };
        let active = drag.active
            || point
                .0
                .abs_diff(drag.origin.0)
                .max(point.1.abs_diff(drag.origin.1))
                >= 6;
        if !active {
            return false;
        }
        let target = self
            .icon_hits
            .iter()
            .filter(|hit| {
                model::Navigation::is_rail_hit(hit.0)
                    && point.1 >= hit.2
                    && point.1 < hit.2 + hit.4 as i32
                    && point.0 >= hit.1 - 12
                    && point.0 < hit.1 + hit.3 as i32 + 12
            })
            .filter_map(|hit| match self.navigation.action(hit.0) {
                Some(Action::Group(key))
                    if self.navigation.group(&key).is_some_and(|g| g.pinned) =>
                {
                    Some((key, point.0 >= hit.1 + hit.3 as i32 / 2))
                }
                _ => None,
            })
            .next();
        let drag = self.navigation.drag.as_mut().unwrap();
        let changed = !drag.active || drag.target != target;
        drag.active = true;
        drag.target = target;
        if changed {
            self.bar_left_cache = None;
            self.morph_to_target(now);
        }
        changed
    }
    pub fn cancel_navigation_drag(&mut self, now: u64) -> bool {
        let pressed = self.navigation.rail_pressed.take().is_some();
        if pressed || self.navigation.drag.is_some() {
            self.navigation.cancelled_click = true;
        }
        if self.navigation.drag.take().is_none() && !pressed {
            return false;
        }
        self.bar_left_cache = None;
        self.morph_to_target(now);
        true
    }
    pub fn finish_navigation_pointer(&mut self, x: i32, y: i32, now: u64) -> Option<ClickOutcome> {
        self.navigation.rail_pressed = None;
        self.bar_left_cache = None;
        if std::mem::take(&mut self.navigation.cancelled_click) {
            return Some(ClickOutcome::NavigationChanged);
        }
        if !self.navigation.drag.as_ref().is_some_and(|d| d.active) {
            self.navigation.drag = None;
            self.morph_to_target(now);
            return None;
        }
        self.navigation_pointer_motion(x, y, now);
        let drag = self.navigation.drag.take().unwrap();
        let unchanged = drag.baseline
            == self
                .pinned_apps()
                .iter()
                .map(PinnedApp::key)
                .collect::<Vec<_>>();
        let moved = unchanged
            && drag.target.is_some_and(|(target, after)| {
                model::reorder(&mut self.island.bar.pinned_apps, &drag.key, &target, after)
            });
        self.sync_navigation();
        self.bar_left_cache = None;
        self.morph_to_target(now);
        Some(if moved {
            ClickOutcome::NavigationPinsChanged
        } else {
            ClickOutcome::NavigationChanged
        })
    }
    pub fn navigation_tooltips(&self) -> Vec<crate::window::NavigationHint> {
        if self.navigation.popup.is_some()
            || self.navigation.drag.is_some()
            || self.navigation.rail_pressed.is_some()
        {
            return Vec::new();
        }
        self.icon_hits
            .iter()
            .filter_map(|hit| {
                let text = match self.navigation.action(hit.0)? {
                    Action::Group(key) => {
                        let g = self.navigation.group(&key)?;
                        format!(
                            "{} — {} window{} · {}",
                            g.name,
                            g.windows.len(),
                            if g.windows.len() == 1 { "" } else { "s" },
                            if g.windows.is_empty() {
                                "click to launch"
                            } else if g.windows.len() == 1 {
                                "click to switch"
                            } else {
                                "click to choose"
                            }
                        )
                    }
                    Action::Launcher => "Apps — open the launcher (Alt+Space)".into(),
                    Action::Overflow => "More applications and Bar settings".into(),
                    Action::Settings => "Termielle preferences".into(),
                    _ => return None,
                };
                let s = self.render_scale();
                Some(crate::window::NavigationHint {
                    id: hit.0,
                    rect: (
                        (hit.1 as f32 * s).round() as i32,
                        (hit.2 as f32 * s).round() as i32,
                        (hit.3 as f32 * s).round() as u32,
                        (hit.4 as f32 * s).round() as u32,
                    ),
                    text,
                })
            })
            .collect()
    }
    pub fn is_navigation_open(&self) -> bool {
        self.island.is_bar() && self.navigation.popup.is_some()
    }
    pub fn wants_navigation_focus(&self) -> bool {
        self.navigation.drag.as_ref().is_some_and(|d| d.active)
            || self.is_navigation_open()
                && self
                    .navigation
                    .popup
                    .as_ref()
                    .is_some_and(|p| p.mode != Mode::Notice)
    }
    pub fn pinned_apps(&self) -> &[PinnedApp] {
        &self.island.bar.pinned_apps
    }
    pub(crate) fn sync_navigation(&mut self) {
        self.navigation
            .sync(&self.windows, &self.tasks, &self.island.bar.pinned_apps);
    }
    pub fn navigation_notice(&mut self, text: &str, now: u64) {
        self.navigation.notice = Some(text.into());
        self.navigation.open(Mode::Notice, 120);
        self.interaction_deadline = Some(now.saturating_add(100));
        self.morph_to_target(now);
    }
    pub fn refresh_pending_launches(&mut self, now: u64) -> bool {
        let old = self.navigation.pending.len();
        let complete = self
            .navigation
            .pending
            .iter()
            .filter(|(key, (count, deadline))| {
                now >= *deadline
                    || self
                        .navigation
                        .group(key)
                        .is_some_and(|g| g.windows.len() > *count)
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in complete {
            self.navigation.pending.remove(&key);
        }
        if self.navigation.pending.len() != old {
            self.bar_left_cache = None;
            self.morph_to_target(now);
            return true;
        }
        false
    }
    pub fn app_launch_finished(&mut self, code: i32, now: u64) {
        self.navigation.launching = false;
        if let Some(key) = self.navigation.dispatch_key.take() {
            if code != 0 {
                self.navigation.pending.remove(&key);
            }
        }
        self.bar_left_cache = None;
        if code != 0 {
            if self.is_navigation_open() {
                self.navigation.notice =
                    Some("Couldn't open the app. Check that it is still installed.".into());
                self.morph_to_target(now);
            } else {
                self.navigation_notice(
                    "Couldn't open this app. Check that it is still installed.",
                    now,
                );
            }
        } else {
            self.morph_to_target(now);
        }
    }
    pub fn close_navigation(&mut self, now: u64) -> bool {
        if self.navigation.popup.is_none() {
            return false;
        }
        self.navigation.dismiss();
        self.morph_to_target(now);
        true
    }
    pub fn restore_navigation_pins(&mut self, pins: Vec<PinnedApp>, now: u64) {
        self.island.bar.pinned_apps = pins;
        self.sync_navigation();
        self.bar_left_cache = None;
        self.morph_to_target(now);
    }
    pub(crate) fn handle_navigation_hit(&mut self, id: isize, now: u64) -> ClickOutcome {
        let Some(action) = self.navigation.action(id) else {
            return ClickOutcome::None;
        };
        let anchor = self
            .icon_hits
            .iter()
            .find(|h| h.0 == id)
            .map_or(120, |h| h.1 + h.3 as i32 / 2);
        self.navigation_action(action, anchor, now)
    }
    pub fn handle_context_click(&mut self, x: i32, y: i32, now: u64) -> ClickOutcome {
        let (x, y) = (self.to_logical(x), self.to_logical(y));
        let Some((id, hx, _, hw, _)) = self
            .icon_hits
            .iter()
            .copied()
            .find(|h| x >= h.1 && x < h.1 + h.3 as i32 && y >= h.2 && y < h.2 + h.4 as i32)
        else {
            return ClickOutcome::None;
        };
        match self.navigation.action(id) {
            Some(Action::Group(key)) => {
                self.navigation_action(Action::Context(key), hx + hw as i32 / 2, now)
            }
            Some(Action::Launcher | Action::Overflow) => {
                self.navigation_action(Action::Overflow, hx + hw as i32 / 2, now)
            }
            _ => ClickOutcome::None,
        }
    }
    pub fn handle_navigation_key(&mut self, key: u32, now: u64) -> ClickOutcome {
        if key == 27 && self.cancel_navigation_drag(now) {
            return ClickOutcome::NavigationChanged;
        }
        if !self.is_navigation_open() {
            return ClickOutcome::None;
        }
        let reverse = key == 265;
        let key = if reverse { 9 } else { key };
        if key == 27 {
            self.close_navigation(now);
            return ClickOutcome::NavigationChanged;
        }
        let visible = self
            .navigation
            .popup_actions
            .iter()
            .enumerate()
            .filter(|(i, a)| {
                a.keyboard_accessible()
                    && self
                        .icon_hits
                        .iter()
                        .any(|h| h.0 == model::POPUP_BASE - *i as isize)
            })
            .map(|(_, a)| a.clone())
            .collect::<Vec<_>>();
        if key == 13 {
            if let Some(action) = self
                .navigation
                .selected
                .clone()
                .filter(|a| visible.contains(a))
            {
                let anchor = self.navigation.popup.as_ref().map_or(120, |p| p.anchor);
                return self.navigation_action(action, anchor, now);
            }
        } else if matches!(key, 9 | 38 | 40 | 36 | 35) {
            let choices = visible
                .into_iter()
                .filter(|a| key == 9 || a.is_row())
                .collect::<Vec<_>>();
            if !choices.is_empty() {
                let old = self
                    .navigation
                    .selected
                    .as_ref()
                    .and_then(|a| choices.iter().position(|c| c == a));
                let index = match key {
                    36 => 0,
                    35 => choices.len() - 1,
                    38 | 9 if key == 38 || reverse => old.map_or(choices.len() - 1, |i| {
                        (i + choices.len() - 1) % choices.len()
                    }),
                    _ => old.map_or(0, |i| (i + 1) % choices.len()),
                };
                self.navigation.selected = Some(choices[index].clone());
            }
            self.morph_to_target(now);
        } else if matches!(key, 33 | 34) {
            if let Some(popup) = self.navigation.popup.as_ref() {
                let page = if key == 33 {
                    popup.page.saturating_sub(1)
                } else {
                    popup.page + 1
                };
                return self.navigation_action(Action::Page(page), 120, now);
            }
        }
        ClickOutcome::NavigationChanged
    }
    fn navigation_action(&mut self, action: Action, anchor: i32, now: u64) -> ClickOutcome {
        let mut outcome = ClickOutcome::NavigationChanged;
        let context = matches!(&action, Action::Context(_));
        let close = matches!(&action, Action::Close(..));
        match action {
            Action::Launcher => {
                self.navigation.dismiss();
                outcome = ClickOutcome::Shell(crate::bar::shell::ShellAction::Search);
            }
            Action::Settings => {
                self.navigation.dismiss();
                outcome = ClickOutcome::OpenSettings;
            }
            Action::Dismiss => self.navigation.dismiss(),
            Action::Shield => return ClickOutcome::None,
            Action::Overflow => {
                if self
                    .navigation
                    .popup
                    .as_ref()
                    .is_some_and(|p| p.mode == Mode::Overflow)
                {
                    self.navigation.dismiss();
                } else {
                    self.navigation.open(Mode::Overflow, anchor);
                }
            }
            Action::Group(key) | Action::Context(key) => {
                let Some(group) = self.navigation.group(&key).cloned() else {
                    return ClickOutcome::None;
                };
                if !context && group.windows.len() == 1 {
                    let window = &group.windows[0];
                    self.navigation.dismiss();
                    outcome =
                        ClickOutcome::ActivateAssociatedWindow(window.hwnd, window.process_id);
                } else if !context && group.windows.is_empty() {
                    if let Some(pin) = group.target {
                        return self.navigation_action(Action::Launch(pin), anchor, now);
                    }
                } else {
                    self.navigation.open(Mode::App(key), anchor);
                }
            }
            Action::Focus(hwnd, pid) | Action::Close(hwnd, pid) => {
                if self
                    .windows
                    .iter()
                    .any(|w| w.hwnd == hwnd && w.process_id == pid)
                {
                    self.navigation.dismiss();
                    outcome = if close {
                        ClickOutcome::CloseAppWindow(hwnd, pid)
                    } else {
                        ClickOutcome::ActivateAssociatedWindow(hwnd, pid)
                    };
                }
            }
            Action::Pin(pin) => {
                if pin.is_valid()
                    && self.island.bar.pinned_apps.len() < termielle_core::MAX_PINNED_APPS
                    && !self
                        .island
                        .bar
                        .pinned_apps
                        .iter()
                        .any(|p| p.key() == pin.key())
                {
                    self.island.bar.pinned_apps.push(pin);
                    self.navigation.selected = None;
                    self.sync_navigation();
                    self.bar_left_cache = None;
                    outcome = ClickOutcome::NavigationPinsChanged;
                }
            }
            Action::Unpin(key) => {
                self.island.bar.pinned_apps.retain(|p| p.key() != key);
                self.navigation.selected = None;
                self.sync_navigation();
                self.bar_left_cache = None;
                outcome = ClickOutcome::NavigationPinsChanged;
            }
            Action::Launch(pin) => {
                self.refresh_pending_launches(now);
                if pin.is_valid()
                    && !self.navigation.launching
                    && !self.navigation.pending.contains_key(&pin.key())
                {
                    let key = pin.key();
                    let count = self.navigation.group(&key).map_or(0, |g| g.windows.len());
                    self.navigation
                        .pending
                        .insert(key.clone(), (count, now.saturating_add(10_000)));
                    self.navigation.dispatch_key = Some(key);
                    self.bar_left_cache = None;
                    self.navigation.launching = true;
                    self.navigation.dismiss();
                    outcome = ClickOutcome::LaunchApp(pin);
                }
            }
            Action::Page(page) => {
                if let Some(popup) = self.navigation.popup.as_mut() {
                    popup.page = page;
                }
                self.navigation.clamp_page();
                self.navigation.selected = None;
                if let Some(popup) = self
                    .navigation
                    .popup
                    .as_ref()
                    .filter(|p| p.mode == Mode::Overflow)
                {
                    self.navigation.overflow_page = popup.page;
                }
            }
            Action::Back => {
                let anchor = self.navigation.popup.as_ref().map_or(anchor, |p| p.anchor);
                self.navigation.open(Mode::Overflow, anchor);
            }
        }
        self.interaction_deadline = self
            .navigation
            .popup
            .as_ref()
            .map(|_| now.saturating_add(100));
        self.morph_to_target(now);
        outcome
    }
    pub(crate) fn navigation_rect(
        &self,
        width: u32,
        exp_h: u32,
        bar_y: i32,
    ) -> (i32, i32, u32, u32) {
        let popup_w = 340.min(width.saturating_sub(16));
        let h = exp_h.min(self.navigation.height());
        let anchor = self.navigation.popup.as_ref().map_or(120, |p| p.anchor);
        let x = (anchor - popup_w as i32 / 2).clamp(0, (width - popup_w) as i32);
        let y = if self.island.bar.position == termielle_core::BarPosition::Top {
            (self.island.bar.height + super::bar::types::BAR_POPUP_GAP) as i32
        } else {
            bar_y - super::bar::types::BAR_POPUP_GAP as i32 - h as i32
        };
        (x, y, popup_w, h)
    }
}

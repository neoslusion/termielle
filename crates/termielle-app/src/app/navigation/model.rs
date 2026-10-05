use crate::tasks::{TaskIcon, WindowInfo};
use std::cell::RefCell;
use std::collections::HashMap;
use termielle_core::PinnedApp;

pub(super) const PAGE_SIZE: usize = 4;
pub(super) const RAIL_BASE: isize = -1000;
pub(super) const POPUP_BASE: isize = -2000;
pub(super) const SLOT: u32 = 36;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Action {
    Launcher,
    Settings,
    Dismiss,
    Overflow,
    Shield,
    Group(String),
    Context(String),
    Focus(isize, u32),
    Close(isize, u32),
    Pin(PinnedApp),
    Unpin(String),
    Launch(PinnedApp),
    Page(usize),
    Back,
}
impl Action {
    pub fn is_row(&self) -> bool {
        matches!(self, Self::Group(_) | Self::Focus(..))
    }
    pub fn keyboard_accessible(&self) -> bool {
        !matches!(self, Self::Shield)
    }
}

#[derive(Clone)]
pub(super) struct Group {
    pub key: String,
    pub name: String,
    pub target: Option<PinnedApp>,
    pub pinned: bool,
    pub windows: Vec<WindowInfo>,
    pub icon: Option<TaskIcon>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Mode {
    Overflow,
    App(String),
    Notice,
}
#[derive(Clone)]
pub(super) struct Popup {
    pub mode: Mode,
    pub anchor: i32,
    pub page: usize,
}

#[derive(Clone)]
pub(super) struct PinDrag {
    pub key: String,
    pub origin: (i32, i32),
    pub baseline: Vec<String>,
    pub target: Option<(String, bool)>,
    pub active: bool,
}

/// Move a pin by explicit identity. Return false for stale/no-op drops.
pub(super) fn reorder(pins: &mut Vec<PinnedApp>, key: &str, target: &str, after: bool) -> bool {
    let Some(from) = pins.iter().position(|p| p.key() == key) else {
        return false;
    };
    let Some(to) = pins.iter().position(|p| p.key() == target) else {
        return false;
    };
    let destination = to + usize::from(after);
    let destination = destination.saturating_sub(usize::from(from < destination));
    if from == destination {
        return false;
    }
    let pin = pins.remove(from);
    pins.insert(destination.min(pins.len()), pin);
    true
}

#[derive(Default)]
pub(crate) struct Navigation {
    pub(super) groups: Vec<Group>,
    pub(super) popup: Option<Popup>,
    pub(super) rail_actions: RefCell<Vec<Action>>,
    pub(super) popup_actions: Vec<Action>,
    pub(super) selected: Option<Action>,
    pub(super) notice: Option<String>,
    pub(super) launching: bool,
    pub(super) dispatch_key: Option<String>,
    pub(super) pending: HashMap<String, (usize, u64)>,
    pub(super) drag: Option<PinDrag>,
    pub(super) rail_pressed: Option<Action>,
    pub(super) cancelled_click: bool,
    order: Vec<String>,
    icons: HashMap<String, TaskIcon>,
    window_order: HashMap<String, Vec<(isize, u32)>>,
    pub(super) overflow_page: usize,
}

impl Navigation {
    pub fn is_rail_hit(id: isize) -> bool {
        id <= RAIL_BASE && id > POPUP_BASE
    }
    pub fn is_popup_hit(id: isize) -> bool {
        id <= POPUP_BASE && id > POPUP_BASE - 1000
    }
    pub(super) fn action(&self, id: isize) -> Option<Action> {
        if Self::is_rail_hit(id) {
            self.rail_actions
                .borrow()
                .get((RAIL_BASE - id) as usize)
                .cloned()
        } else if Self::is_popup_hit(id) {
            self.popup_actions.get((POPUP_BASE - id) as usize).cloned()
        } else {
            None
        }
    }
    pub(super) fn rail_action(&self, action: Action) -> isize {
        let mut actions = self.rail_actions.borrow_mut();
        let id = RAIL_BASE - actions.len() as isize;
        actions.push(action);
        id
    }
    pub(super) fn popup_action(&mut self, action: Action) -> isize {
        let id = POPUP_BASE - self.popup_actions.len() as isize;
        self.popup_actions.push(action);
        id
    }
    pub(super) fn group(&self, key: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.key == key)
    }

    pub fn sync(&mut self, windows: &[WindowInfo], tasks: &[TaskIcon], pins: &[PinnedApp]) {
        let mut groups: HashMap<String, Group> = HashMap::new();
        for pin in pins
            .iter()
            .filter(|p| p.is_valid())
            .take(termielle_core::MAX_PINNED_APPS)
        {
            groups.entry(pin.key()).or_insert_with(|| Group {
                key: pin.key(),
                name: pin.name.clone(),
                target: Some(pin.clone()),
                pinned: true,
                windows: Vec::new(),
                icon: None,
            });
        }
        for window in windows {
            let key = window
                .application
                .as_ref()
                .map(PinnedApp::key)
                .unwrap_or_else(|| format!("window:{}:{}", window.hwnd, window.process_id));
            let group = groups.entry(key.clone()).or_insert_with(|| Group {
                key,
                name: window
                    .application
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| window.title.clone()),
                target: window.application.clone(),
                pinned: false,
                windows: Vec::new(),
                icon: None,
            });
            if let Some(icon) = tasks.iter().find(|t| t.hwnd == window.hwnd) {
                self.icons.insert(group.key.clone(), icon.clone());
            }
            group.windows.push(window.clone());
        }
        self.order.retain(|key| groups.contains_key(key));
        let mut added: Vec<_> = groups
            .values()
            .filter(|g| !self.order.contains(&g.key))
            .map(|g| (g.name.to_lowercase(), g.key.clone()))
            .collect();
        added.sort();
        self.order.extend(added.into_iter().map(|(_, key)| key));
        self.icons.retain(|key, _| groups.contains_key(key));
        if self.icons.len() > 96 {
            let mut keys = self.icons.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys.into_iter().skip(96) {
                self.icons.remove(&key);
            }
        }
        self.window_order.retain(|key, _| groups.contains_key(key));
        let mut keys: Vec<_> = pins
            .iter()
            .filter(|p| p.is_valid())
            .map(PinnedApp::key)
            .collect();
        keys.extend(
            self.order
                .iter()
                .filter(|key| !keys.contains(key))
                .cloned()
                .collect::<Vec<_>>(),
        );
        self.groups = keys
            .into_iter()
            .filter_map(|key| groups.remove(&key))
            .map(|mut group| {
                let order = self.window_order.entry(group.key.clone()).or_default();
                let identities = group
                    .windows
                    .iter()
                    .map(|w| (w.hwnd, w.process_id))
                    .collect::<std::collections::HashSet<_>>();
                order.retain(|id| identities.contains(id));
                let known = order
                    .iter()
                    .copied()
                    .collect::<std::collections::HashSet<_>>();
                let mut added = group
                    .windows
                    .iter()
                    .filter(|w| !known.contains(&(w.hwnd, w.process_id)))
                    .map(|w| (w.title.to_lowercase(), w.hwnd, w.process_id))
                    .collect::<Vec<_>>();
                added.sort();
                order.extend(added.into_iter().map(|(_, hwnd, pid)| (hwnd, pid)));
                let positions = order
                    .iter()
                    .enumerate()
                    .map(|(i, id)| (*id, i))
                    .collect::<HashMap<_, _>>();
                group
                    .windows
                    .sort_by_key(|w| positions[&(w.hwnd, w.process_id)]);
                group.icon = self.icons.get(&group.key).cloned();
                group
            })
            .collect();
        if let Some(popup) = self.popup.as_ref() {
            if let Mode::App(key) = &popup.mode {
                if self.group(key).is_none() {
                    self.popup = None;
                    self.popup_actions.clear();
                    self.selected = None;
                }
            }
        }
        if self
            .drag
            .as_ref()
            .is_some_and(|drag| !self.groups.iter().any(|g| g.key == drag.key && g.pinned))
        {
            self.drag = None;
        }
        self.clamp_page();
    }
    pub fn pending_deadline(&self) -> Option<u64> {
        self.pending.values().map(|(_, at)| *at).min()
    }
    pub fn row_count(&self) -> usize {
        match self.popup.as_ref().map(|p| &p.mode) {
            Some(Mode::Overflow) => self.groups.len(),
            Some(Mode::App(key)) => self.group(key).map_or(0, |g| g.windows.len()),
            _ => 0,
        }
    }
    pub fn clamp_page(&mut self) {
        let last = self.row_count().saturating_sub(1) / PAGE_SIZE;
        if let Some(popup) = self.popup.as_mut() {
            popup.page = popup.page.min(last);
        }
    }
    pub fn height(&self) -> u32 {
        60 + self.row_count().clamp(1, PAGE_SIZE) as u32 * 46
            + if self.row_count() > PAGE_SIZE { 84 } else { 50 }
    }
    pub(super) fn open(&mut self, mode: Mode, anchor: i32) {
        self.drag = None;
        if mode != Mode::Notice {
            self.notice = None;
        }
        let page = if mode == Mode::Overflow {
            self.overflow_page
        } else {
            0
        };
        self.popup = Some(Popup { mode, anchor, page });
        self.selected = None;
        self.clamp_page();
    }
    pub fn dismiss(&mut self) {
        self.popup = None;
        self.popup_actions.clear();
        self.selected = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use termielle_core::AppLaunchTarget;
    fn app(name: &str) -> PinnedApp {
        PinnedApp {
            name: name.into(),
            target: AppLaunchTarget::Executable(format!("C:\\Apps\\{name}.exe")),
        }
    }
    fn window(hwnd: isize, app: PinnedApp) -> WindowInfo {
        WindowInfo {
            hwnd,
            process_id: hwnd as u32,
            title: format!("Window {hwnd}"),
            application: Some(app),
            minimized: false,
        }
    }
    #[test]
    fn pins_stay_first_closed_pins_remain_and_focus_cannot_shuffle_apps() {
        let mut nav = Navigation::default();
        let a = app("A");
        let b = app("B");
        let c = app("C");
        nav.sync(
            &[window(2, b.clone()), window(1, a.clone())],
            &[],
            std::slice::from_ref(&c),
        );
        let keys = nav.groups.iter().map(|g| g.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys, [c.key(), a.key(), b.key()]);
        assert!(nav.groups[0].windows.is_empty());
        nav.sync(
            &[window(1, a.clone()), window(2, b)],
            &[],
            std::slice::from_ref(&c),
        );
        assert_eq!(
            keys,
            nav.groups.iter().map(|g| g.key.clone()).collect::<Vec<_>>()
        );
        nav.sync(&[], &[], &[c]);
        assert_eq!(nav.groups.len(), 1);
    }
    #[test]
    fn grouping_uses_identity_not_names_and_iconless_windows_are_not_dropped() {
        let mut nav = Navigation::default();
        let a = app("A");
        let mut b = app("B");
        b.name = "A".into();
        nav.sync(
            &[window(1, a.clone()), window(2, a), window(3, b)],
            &[],
            &[],
        );
        assert_eq!(nav.groups.len(), 2);
        assert_eq!(nav.groups[0].windows.len(), 2);
        assert!(nav.groups.iter().all(|g| g.icon.is_none()));
    }
    #[test]
    fn painted_actions_keep_identity_after_live_order_changes() {
        let mut nav = Navigation::default();
        let id = nav.rail_action(Action::Group(app("A").key()));
        nav.sync(&[window(9, app("B"))], &[], &[]);
        assert_eq!(nav.action(id), Some(Action::Group(app("A").key())));
    }
    #[test]
    fn pagination_is_bounded_and_reconciles_a_removed_app() {
        let mut nav = Navigation::default();
        let a = app("A");
        nav.sync(
            &(1..10).map(|i| window(i, a.clone())).collect::<Vec<_>>(),
            &[],
            &[],
        );
        nav.open(Mode::App(a.key()), 100);
        nav.popup.as_mut().unwrap().page = 2;
        assert_eq!(nav.height(), 328);
        nav.sync(&[window(1, a)], &[], &[]);
        assert_eq!(nav.popup.as_ref().unwrap().page, 0);
        nav.sync(&[], &[], &[]);
        assert!(nav.popup.is_none());
    }
}

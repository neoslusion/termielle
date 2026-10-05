//! Local, explicit session-to-window links. No hook metadata or guessed targets.

use std::collections::HashMap;
use termielle_core::{SessionSummary, Source};

pub(crate) const PAGE_SIZE: usize = 4;
pub(crate) const ROW_HEIGHT: u32 = 52;
pub(crate) const MAX_CARD_HEIGHT: u32 = 52 + PAGE_SIZE as u32 * ROW_HEIGHT + 36;
const HIT_BASE: isize = -600;
const HIT_LIMIT: isize = -650;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SessionKey {
    pub source: Source,
    pub id: String,
}

impl From<&SessionSummary> for SessionKey {
    fn from(session: &SessionSummary) -> Self {
        Self {
            source: session.source.clone(),
            id: session.session_id.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WindowLink {
    pub hwnd: isize,
    pub process_id: u32,
}

impl WindowLink {
    pub fn matches(&self, task: &crate::tasks::WindowInfo) -> bool {
        self.hwnd == task.hwnd && self.process_id == task.process_id && self.process_id != 0
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ActivityAction {
    Select(SessionKey),
    Focus(SessionKey),
    Bind(SessionKey, WindowLink),
    Unlink(SessionKey),
    Back,
    Page(bool),
}

#[derive(Default)]
pub(crate) struct SessionActivity {
    pub links: HashMap<SessionKey, WindowLink>,
    pub picking: Option<SessionKey>,
    pub session_page: usize,
    pub window_page: usize,
    /// Actions refer to the identities painted, never a freshly sorted index.
    pub painted_actions: Vec<ActivityAction>,
    pub revision: u64,
    pub had_sessions: bool,
    pub deadline: Option<u64>,
}

impl SessionActivity {
    pub fn reconcile(&mut self, sessions: &[SessionSummary], tasks: &[crate::tasks::WindowInfo]) {
        let exists = |key: &SessionKey| sessions.iter().any(|s| SessionKey::from(s) == *key);
        self.links
            .retain(|key, link| exists(key) && tasks.iter().any(|t| link.matches(t)));
        if self.picking.as_ref().is_some_and(|key| !exists(key)) {
            self.picking = None;
            self.window_page = 0;
        }
        self.session_page = self.session_page.min(last_page(sessions.len()));
        self.window_page = self.window_page.min(last_page(tasks.len()));
    }

    pub fn height(&self, sessions: usize, windows: usize) -> u32 {
        let count = if self.picking.is_some() {
            windows
        } else {
            sessions
        };
        let page = if self.picking.is_some() {
            self.window_page
        } else {
            self.session_page
        };
        52 + (count.saturating_sub(page * PAGE_SIZE).clamp(1, PAGE_SIZE) as u32) * ROW_HEIGHT + 36
    }

    pub fn register(&mut self, action: ActivityAction) -> isize {
        let id = HIT_BASE - self.painted_actions.len() as isize;
        assert!(
            id > HIT_LIMIT,
            "activity actions must fit the reserved hit range"
        );
        self.painted_actions.push(action);
        id
    }

    pub fn action(&self, id: isize) -> Option<ActivityAction> {
        if !Self::is_hit(id) {
            return None;
        }
        self.painted_actions.get((HIT_BASE - id) as usize).cloned()
    }

    pub fn is_hit(id: isize) -> bool {
        id <= HIT_BASE && id > HIT_LIMIT
    }
}

impl super::controller::Controller {
    pub(crate) fn activity_available(&self) -> bool {
        self.island.has_widget("agents") && self.reducer.session_count() > 0
    }

    pub(crate) fn activity_visible(&self) -> bool {
        self.island.is_enabled()
            && self.activity_available()
            && self.alerts.is_empty()
            && self.island_card_open()
            && (self.island.is_bar() || !self.panel_open)
    }

    pub(crate) fn activity_height(&self) -> u32 {
        self.activity
            .height(self.reducer.session_count(), self.windows.len())
    }

    /// Reconcile links and repaint non-primary changes without changing the
    /// primary face or borrowing the right popover's state.
    pub(crate) fn sync_activity(&mut self, now_ms: u64) -> bool {
        if self.activity.revision == self.reducer.revision() {
            return false;
        }
        self.activity.revision = self.reducer.revision();
        let sessions = self.reducer.session_summaries();
        let last_session_left = self.activity.had_sessions && sessions.is_empty();
        self.activity.had_sessions = !sessions.is_empty();
        self.activity.reconcile(&sessions, &self.windows);
        if last_session_left && self.island.has_widget("agents") && !self.media_available() {
            self.manually_expanded = false;
            self.hover_expanded = false;
            self.hover_deadline = None;
            self.activity.deadline = None;
            return self.morph_to_target(now_ms);
        }
        if self.activity_visible() {
            // A reordered row must never inherit an old row's visible label
            // through a crossfade while already accepting the new action.
            self.content_transition = None;
            self.morph_to_target(now_ms)
        } else {
            false
        }
    }

    pub(crate) fn handle_activity_hit(
        &mut self,
        id: isize,
        now_ms: u64,
    ) -> super::types::ClickOutcome {
        use super::types::ClickOutcome;
        let Some(action) = self.activity.action(id) else {
            return ClickOutcome::None;
        };
        let sessions = self.reducer.session_summaries();
        let exists = |key: &SessionKey| sessions.iter().any(|s| SessionKey::from(s) == *key);
        match action {
            ActivityAction::Focus(key) if exists(&key) => {
                if let Some(link) = self
                    .activity
                    .links
                    .get(&key)
                    .filter(|link| self.windows.iter().any(|t| link.matches(t)))
                {
                    return ClickOutcome::ActivateAssociatedWindow(link.hwnd, link.process_id);
                }
                self.activity.picking = Some(key);
                self.activity.window_page = 0;
            }
            ActivityAction::Select(key) if exists(&key) => {
                self.activity.picking = Some(key);
                self.activity.window_page = 0;
            }
            ActivityAction::Bind(key, link)
                if exists(&key)
                    && self.activity.picking.as_ref() == Some(&key)
                    && self.windows.iter().any(|t| link.matches(t)) =>
            {
                self.activity.links.insert(key, link);
                self.activity.picking = None;
            }
            ActivityAction::Unlink(key) if exists(&key) => {
                self.activity.links.remove(&key);
                self.activity.picking = None;
            }
            ActivityAction::Back => {
                self.activity.picking = None;
            }
            ActivityAction::Page(next) => {
                let (page, count) = if self.activity.picking.is_some() {
                    (&mut self.activity.window_page, self.windows.len())
                } else {
                    (&mut self.activity.session_page, sessions.len())
                };
                *page = if next {
                    page.saturating_add(1).min(last_page(count))
                } else {
                    page.saturating_sub(1)
                };
            }
            _ => return ClickOutcome::None,
        }
        // A click in a hover-opened card pins the picker so it does not close
        // while the user chooses a target. Control Center stays independent.
        self.manually_expanded = true;
        self.hover_deadline = None;
        self.interaction_deadline = Some(now_ms.saturating_add(100));
        self.morph_to_target(now_ms);
        ClickOutcome::ActivityChanged
    }
}

pub(crate) fn last_page(count: usize) -> usize {
    count.saturating_sub(1) / PAGE_SIZE
}

pub(crate) fn elapsed_label(age_ms: u64) -> String {
    let seconds = age_ms / 1_000;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", seconds / 60)
    } else {
        format!("{}h {}m", seconds / 3_600, seconds % 3_600 / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{ClickOutcome, Controller};
    use crate::tasks::{WindowInfo, WorkerUpdate};
    use termielle_core::{
        AssetCatalog, BarPosition, EventKind, EventMessage, IslandConfig, IslandLayout,
        PROTOCOL_VERSION,
    };

    fn controller(layout: IslandLayout, position: BarPosition) -> Controller {
        let mut config = IslandConfig {
            layout,
            widgets: vec!["face".into(), "agents".into(), "music".into()],
            expanded_width: 360,
            ..IslandConfig::default()
        };
        config.bar.position = position;
        let mut c = Controller::new_with_island(
            5_000,
            600_000,
            AssetCatalog::new(Vec::new()),
            true,
            None,
            config,
        );
        c.set_bar_width(800);
        c
    }

    fn event(c: &mut Controller, source: &str, id: &str, kind: EventKind, at: u64, now: u64) {
        c.handle_event(
            EventMessage {
                version: PROTOCOL_VERSION,
                source: Source::parse(source).unwrap(),
                session_id: id.into(),
                event: kind,
                timestamp_ms: at,
            },
            now,
        );
    }

    fn tasks(c: &mut Controller, count: usize, pid: u32) {
        c.set_task_update_at(
            WorkerUpdate {
                windows: (1..=count)
                    .map(|i| WindowInfo {
                        hwnd: i as isize,
                        process_id: pid,
                        title: format!("Terminal {i}"),
                        application: None,
                        minimized: false,
                    })
                    .collect(),
                ..Default::default()
            },
            100_000,
        );
    }

    fn click(c: &mut Controller, predicate: impl Fn(&ActivityAction) -> bool) -> ClickOutcome {
        let index = c
            .activity
            .painted_actions
            .iter()
            .position(predicate)
            .expect("painted action");
        let id = HIT_BASE - index as isize;
        let &(_, x, y, w, h) = c
            .icon_hits
            .iter()
            .find(|hit| hit.0 == id)
            .expect("visible hit");
        let scale = c.render_scale();
        c.handle_click(
            ((x + w as i32 / 2) as f32 * scale).round() as i32,
            ((y + h as i32 / 2) as f32 * scale).round() as i32,
            100_001,
        )
    }

    fn working(c: &mut Controller, id: &str) {
        event(c, "claude", id, EventKind::ThinkingEnded, 99_000, 100_000);
    }

    #[test]
    fn list_links_and_focuses_only_the_explicitly_selected_window_in_each_layout() {
        for (layout, position) in [
            (IslandLayout::Bar, BarPosition::Top),
            (IslandLayout::Bar, BarPosition::Bottom),
            (IslandLayout::Island, BarPosition::Top),
            (IslandLayout::Notch, BarPosition::Top),
        ] {
            let mut c = controller(layout, position);
            working(&mut c, "one");
            working(&mut c, "two");
            tasks(&mut c, 2, 17);
            c.toggle_expand(100_000);
            assert!(
                c.activity.links.is_empty(),
                "never auto-associate a foreground window"
            );
            assert_eq!(
                click(
                    &mut c,
                    |a| matches!(a, ActivityAction::Select(k) if k.id == "two")
                ),
                ClickOutcome::ActivityChanged
            );
            assert_eq!(
                click(
                    &mut c,
                    |a| matches!(a, ActivityAction::Bind(_, w) if w.hwnd == 2)
                ),
                ClickOutcome::ActivityChanged
            );
            assert_eq!(
                click(
                    &mut c,
                    |a| matches!(a, ActivityAction::Focus(k) if k.id == "two")
                ),
                ClickOutcome::ActivateAssociatedWindow(2, 17)
            );
            assert_eq!(c.activity.links.len(), 1);
            assert_eq!(
                click(
                    &mut c,
                    |a| matches!(a, ActivityAction::Select(k) if k.id == "one")
                ),
                ClickOutcome::ActivityChanged
            );
            assert_eq!(
                click(&mut c, |a| matches!(a, ActivityAction::Back)),
                ClickOutcome::ActivityChanged
            );
        }
    }

    #[test]
    fn session_and_window_pages_reach_every_entry_and_shrink_safely() {
        let mut c = controller(IslandLayout::Bar, BarPosition::Top);
        for i in 0..9 {
            working(&mut c, &format!("session-{i}"));
        }
        tasks(&mut c, 9, 7);
        c.toggle_expand(100_000);
        assert_eq!(c.activity_height(), MAX_CARD_HEIGHT);
        assert_eq!(
            click(&mut c, |a| matches!(a, ActivityAction::Page(true))),
            ClickOutcome::ActivityChanged
        );
        assert_eq!(c.activity.session_page, 1);
        click(&mut c, |a| matches!(a, ActivityAction::Page(true)));
        assert_eq!(c.activity.session_page, 2);
        click(
            &mut c,
            |a| matches!(a, ActivityAction::Select(k) if k.id == "session-8"),
        );
        click(&mut c, |a| matches!(a, ActivityAction::Page(true)));
        click(&mut c, |a| matches!(a, ActivityAction::Page(true)));
        click(
            &mut c,
            |a| matches!(a, ActivityAction::Bind(_, w) if w.hwnd == 9),
        );
        assert_eq!(
            click(&mut c, |a| matches!(a, ActivityAction::Focus(_))),
            ClickOutcome::ActivateAssociatedWindow(9, 7)
        );
        for i in 0..9 {
            event(
                &mut c,
                "claude",
                &format!("session-{i}"),
                EventKind::SessionEnded,
                100_002,
                100_002,
            );
        }
        assert_eq!(c.activity.session_page, 0);
        assert!(c.activity.links.is_empty());
        assert!(c.activity.picking.is_none());
    }

    #[test]
    fn a_secondary_event_repaints_even_when_primary_and_geometry_do_not_change() {
        for layout in [IslandLayout::Bar, IslandLayout::Island] {
            let mut c = controller(layout, BarPosition::Top);
            // Old enough not to enqueue an alert, but within the acceptance window.
            event(
                &mut c,
                "claude",
                "ask",
                EventKind::NeedsInput,
                10_000,
                100_000,
            );
            event(
                &mut c,
                "codex",
                "run",
                EventKind::ThinkingEnded,
                99_000,
                100_000,
            );
            c.toggle_expand(100_000);
            let before = c.current_frame().pixels_pbgra.clone();
            let result = c.handle_event(
                EventMessage {
                    version: PROTOCOL_VERSION,
                    source: Source::parse("codex").unwrap(),
                    session_id: "run".into(),
                    event: EventKind::TurnCompleted,
                    timestamp_ms: 100_100,
                },
                100_100,
            );
            assert!(result.present_frame);
            assert_eq!(c.state, termielle_core::VisualState::NeedsInput);
            assert_ne!(before, c.current_frame().pixels_pbgra);
            let before = c.current_frame().pixels_pbgra.clone();
            let result = c.on_timer(105_100); // Secondary Ready -> Idle.
            assert!(result.present_frame);
            assert_eq!(
                c.reducer.session_summaries()[1].state,
                termielle_core::VisualState::Idle
            );
            assert_ne!(before, c.current_frame().pixels_pbgra);
            assert!(
                c.content_transition.is_none(),
                "rows cannot show an old label with a new action"
            );
        }
    }

    #[test]
    fn links_survive_title_changes_but_not_closed_windows_or_process_reuse() {
        let mut c = controller(IslandLayout::Bar, BarPosition::Top);
        working(&mut c, "one");
        tasks(&mut c, 1, 17);
        c.toggle_expand(100_000);
        click(&mut c, |a| matches!(a, ActivityAction::Select(_)));
        click(&mut c, |a| matches!(a, ActivityAction::Bind(..)));
        let mut update = WorkerUpdate {
            windows: c.windows.clone(),
            ..Default::default()
        };
        update.windows[0].title = "Renamed terminal tab".into();
        c.set_task_update_at(update, 100_001);
        assert_eq!(
            click(&mut c, |a| matches!(a, ActivityAction::Focus(_))),
            ClickOutcome::ActivateAssociatedWindow(1, 17)
        );
        tasks(&mut c, 1, 18);
        assert!(c.activity.links.is_empty());
        click(&mut c, |a| matches!(a, ActivityAction::Select(_)));
        click(&mut c, |a| matches!(a, ActivityAction::Bind(..)));
        tasks(&mut c, 0, 18);
        assert!(c.activity.links.is_empty());
    }

    #[test]
    fn links_are_source_scoped_and_unlink_is_explicit() {
        let mut c = controller(IslandLayout::Bar, BarPosition::Top);
        working(&mut c, "same");
        event(
            &mut c,
            "codex",
            "same",
            EventKind::ThinkingEnded,
            99_000,
            100_000,
        );
        tasks(&mut c, 1, 17);
        c.toggle_expand(100_000);
        click(
            &mut c,
            |a| matches!(a, ActivityAction::Select(k) if k.source.as_str() == "claude"),
        );
        click(&mut c, |a| matches!(a, ActivityAction::Bind(..)));
        assert_eq!(c.activity.links.len(), 1);
        assert!(
            c.activity
                .painted_actions
                .iter()
                .any(|a| matches!(a, ActivityAction::Select(k) if k.source.as_str() == "codex"))
        );
        click(
            &mut c,
            |a| matches!(a, ActivityAction::Select(k) if k.source.as_str() == "claude"),
        );
        click(&mut c, |a| matches!(a, ActivityAction::Unlink(_)));
        assert!(c.activity.links.is_empty());
    }

    #[test]
    fn painted_identities_do_not_retarget_to_a_new_sort_order_or_stale_window() {
        let mut c = controller(IslandLayout::Bar, BarPosition::Top);
        working(&mut c, "one");
        tasks(&mut c, 1, 17);
        c.toggle_expand(100_000);
        click(&mut c, |a| matches!(a, ActivityAction::Select(_)));
        let bind = c
            .activity
            .painted_actions
            .iter()
            .position(|a| matches!(a, ActivityAction::Bind(..)))
            .unwrap();
        c.windows.clear(); // Change the snapshot without re-rendering the old hit.
        assert_eq!(
            c.handle_activity_hit(HIT_BASE - bind as isize, 100_001),
            ClickOutcome::None
        );
        assert!(c.activity.links.is_empty());
        c.reducer.apply(EventMessage {
            version: PROTOCOL_VERSION,
            source: Source::parse("claude").unwrap(),
            session_id: "one".into(),
            event: EventKind::SessionEnded,
            timestamp_ms: 100_002,
        });
        assert_eq!(
            c.handle_activity_hit(HIT_BASE - bind as isize, 100_002),
            ClickOutcome::None
        );
    }

    #[test]
    fn activity_keeps_finished_sessions_visible_and_does_not_steal_control_center() {
        let mut c = controller(IslandLayout::Bar, BarPosition::Bottom);
        event(
            &mut c,
            "claude",
            "done",
            EventKind::TurnCompleted,
            99_000,
            100_000,
        );
        c.toggle_expand(100_000);
        c.open_control_panel(100_000);
        let result = c.on_timer(104_000);
        assert!(result.present_frame);
        assert!(c.is_manually_expanded());
        assert!(c.is_panel_open());
        assert!(
            c.activity
                .painted_actions
                .iter()
                .any(|a| matches!(a, ActivityAction::Select(_)))
        );
        assert!(c.activity.deadline.is_some());
        c.collapse_if_expanded(104_000); // Right popover closes first.
        assert!(c.is_manually_expanded());
        c.collapse_if_expanded(104_000);
        c.on_timer(106_000);
        assert!(
            c.activity.deadline.is_none(),
            "no activity repaint cadence while closed"
        );
    }

    #[test]
    fn active_agents_remain_visible_when_media_is_also_available() {
        let mut c = controller(IslandLayout::Bar, BarPosition::Top);
        working(&mut c, "one");
        c.set_task_update_at(
            WorkerUpdate {
                media: Some(crate::tasks::MediaInfo {
                    title: "A song".into(),
                    playing: true,
                    ..Default::default()
                }),
                ..Default::default()
            },
            100_000,
        );
        c.toggle_expand(100_000);
        assert!(
            c.activity
                .painted_actions
                .iter()
                .any(|a| matches!(a, ActivityAction::Select(_)))
        );
        assert!(c.media_available());
        let hit = c
            .icon_hits
            .iter()
            .find(|hit| hit.0 == crate::app::HIT_MEDIA_PLAY_PAUSE)
            .copied()
            .unwrap();
        assert_eq!(
            c.handle_click(hit.1 + 5, hit.2 + 5, 100_001),
            ClickOutcome::MediaToggle
        );
    }

    #[test]
    fn activity_targets_follow_physical_dpi_and_do_not_require_window_artwork() {
        let mut c = controller(IslandLayout::Bar, BarPosition::Bottom);
        c.set_dpi_scale(1.5);
        working(&mut c, "one");
        tasks(&mut c, 9, 17);
        assert!(
            c.tasks.is_empty(),
            "picker does not depend on decoded icons"
        );
        c.toggle_expand(100_000);
        click(&mut c, |a| matches!(a, ActivityAction::Select(_)));
        click(&mut c, |a| matches!(a, ActivityAction::Page(true)));
        click(&mut c, |a| matches!(a, ActivityAction::Page(true)));
        click(
            &mut c,
            |a| matches!(a, ActivityAction::Bind(_, w) if w.hwnd == 9),
        );
        assert_eq!(
            click(&mut c, |a| matches!(a, ActivityAction::Focus(_))),
            ClickOutcome::ActivateAssociatedWindow(9, 17)
        );
    }

    #[test]
    fn visible_activity_buttons_are_clipped_and_usable_before_the_morph_finishes() {
        for position in [BarPosition::Top, BarPosition::Bottom] {
            let mut c = controller(IslandLayout::Bar, position);
            for i in 0..4 {
                working(&mut c, &format!("session-{i}"));
            }
            c.toggle_expand(100_000);
            let card_height = c.activity_height();
            let base = c.island.bar.height + crate::app::bar::types::BAR_POPUP_GAP;
            let hidden_height = base + card_height * 65 / 100;
            c.current = c.render_bar(c.state, 800, hidden_height, 100_000);
            assert!(!c.icon_hits.iter().any(|hit| SessionActivity::is_hit(hit.0)));
            let visible_height = base + card_height * 90 / 100;
            c.current = c.render_bar(c.state, 800, visible_height, 100_000);
            assert!(c.icon_hits.iter().any(|hit| SessionActivity::is_hit(hit.0)));
            assert!(
                c.icon_hits
                    .iter()
                    .filter(|hit| SessionActivity::is_hit(hit.0))
                    .all(|hit| hit.2 >= 0 && hit.2 + hit.4 as i32 <= visible_height as i32)
            );
            assert_eq!(
                click(
                    &mut c,
                    |a| matches!(a, ActivityAction::Select(k) if k.id == "session-0")
                ),
                ClickOutcome::ActivityChanged
            );
        }
    }

    #[test]
    fn elapsed_labels_are_bounded_and_use_whole_seconds() {
        assert_eq!(elapsed_label(0), "0s");
        assert_eq!(elapsed_label(59_999), "59s");
        assert_eq!(elapsed_label(60_000), "1m");
        assert_eq!(elapsed_label(3_660_000), "1h 1m");
    }
}

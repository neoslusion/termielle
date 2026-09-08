//! Windows toast forwarding: app notifications ride the island.
//!
//! A dedicated STA thread polls the notification listener (the listener
//! rejects MTA callers, so this cannot live on the media worker thread),
//! diffs Action Center contents against a bounded seen-set, and posts only
//! fresh arrivals. The first poll seeds the set so pre-existing toasts never
//! flood the island on startup. Everything fails open: denied access,
//! transient WinRT errors, or a dropped receiver just ends the thread.

use std::collections::{HashSet, VecDeque};
use std::sync::mpsc::Sender;
use std::time::Duration;

use windows::UI::Notifications::Management::{
    UserNotificationListener, UserNotificationListenerAccessStatus,
};
use windows::UI::Notifications::{NotificationKinds, UserNotification};

/// Poll cadence: Action Center is low-frequency; 3 s keeps idle CPU nil.
pub const TOAST_POLL_MS: u64 = 3000;

/// Budget for the consent-backed access call (once, at startup, off thread).
const ACCESS_POLL_MS: u64 = 100;
const ACCESS_BUDGET_ROUNDS: u32 = 100;

/// How many notification ids the seen-set retains; older ones may re-fire
/// after heavy churn, which is preferable to unbounded growth.
const MAX_SEEN: usize = 128;

/// Cap banners per poll so waking from sleep to 50 queued toasts shows the
/// five freshest instead of strobing the island.
const MAX_EVENTS_PER_POLL: usize = 5;

/// Title/body character budgets for the alert card layout.
const MAX_TITLE_CHARS: usize = 48;
const MAX_BODY_CHARS: usize = 96;

/// One fresh Windows toast, ready for an alert banner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToastEvent {
    pub id: u32,
    pub app: String,
    pub title: String,
    pub body: String,
}

impl ToastEvent {
    /// Card headline: the toast's own title, falling back to the app name.
    pub fn display_title(&self) -> &str {
        if self.title.trim().is_empty() {
            &self.app
        } else {
            &self.title
        }
    }

    /// Card subline: app plus body, trimmed to layout budget.
    pub fn display_subtitle(&self) -> String {
        if self.body.trim().is_empty() {
            trim_chars(&self.app, MAX_BODY_CHARS)
        } else {
            trim_chars(&format!("{}: {}", self.app, self.body), MAX_BODY_CHARS)
        }
    }
}

/// One watcher round: fresh toasts. Access denial ends the thread silently
/// (fail-open); there is no unavailable signal to handle.
#[derive(Clone, Debug, Default)]
pub struct ToastBatch {
    pub events: Vec<ToastEvent>,
}

impl ToastBatch {
    pub fn events(events: Vec<ToastEvent>) -> Self {
        Self { events }
    }
}

/// Bounded seen-set of notification ids.
#[derive(Debug, Default)]
struct SeenIds {
    set: HashSet<u32>,
    order: VecDeque<u32>,
}

impl SeenIds {
    /// Marks every id seen, returning the fresh subset in arrival order
    /// (capped). The cap keeps a wake-from-sleep flood to a few banners.
    fn take_new(&mut self, ids: &[u32]) -> Vec<u32> {
        let mut fresh = Vec::new();
        for &id in ids {
            if self.set.insert(id) {
                self.order.push_back(id);
                if fresh.len() < MAX_EVENTS_PER_POLL {
                    fresh.push(id);
                }
            }
        }
        while self.order.len() > MAX_SEEN {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        fresh
    }
}

/// Truncates to a character budget on a char boundary.
fn trim_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// Blocks a WinRT async operation by polling status (Startup/access path;
/// never the GUI thread).
fn block_async<T, Op>(
    op: &Op,
    status: impl Fn(&Op) -> windows::core::Result<windows_future::AsyncStatus>,
    results: impl FnOnce() -> windows::core::Result<T>,
    rounds: u32,
    wait_ms: u64,
) -> windows::core::Result<T> {
    use windows_future::AsyncStatus;
    for _ in 0..rounds {
        match status(op)? {
            AsyncStatus::Completed => return results(),
            AsyncStatus::Canceled | AsyncStatus::Error => return Err(windows::core::Error::empty()),
            _ => std::thread::sleep(Duration::from_millis(wait_ms)),
        }
    }
    Err(windows::core::Error::empty())
}

/// Starts the toast watcher. It exits when the receiver is dropped, or
/// silently after one access denial (fail-open: the island is unaffected).
pub fn spawn_toast_watcher(
    sender: Sender<ToastBatch>,
    wake: crate::window::WakeHandle,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        use windows::Win32::System::WinRT::{RO_INIT_SINGLETHREADED, RoInitialize};
        // STA: the listener rejects MTA callers with RPC_E_WRONG_THREAD.
        let _ = unsafe { RoInitialize(RO_INIT_SINGLETHREADED) };
        let listener = match UserNotificationListener::Current() {
            Ok(l) => l,
            Err(_) => return,
        };
        let access = listener.RequestAccessAsync().ok().and_then(|op| {
            block_async(
                &op,
                |o| o.Status(),
                || op.GetResults(),
                ACCESS_BUDGET_ROUNDS,
                ACCESS_POLL_MS,
            )
            .ok()
        });
        if access != Some(UserNotificationListenerAccessStatus::Allowed) {
            return;
        }
        let mut seen = SeenIds::default();
        let mut seeded = false;
        loop {
            if let Some(ids) = current_toast_ids(&listener) {
                if !seeded {
                    // Baseline: pre-existing toasts are not news.
                    for id in ids {
                        seen.take_new(&[id]);
                    }
                    seeded = true;
                } else {
                    let fresh = seen.take_new(&ids);
                    if !fresh.is_empty() {
                        let events: Vec<ToastEvent> = fresh
                            .iter()
                            .filter_map(|id| read_toast(&listener, *id))
                            .collect();
                        if !events.is_empty() {
                            if sender.send(ToastBatch::events(events)).is_err() {
                                break;
                            }
                            // Wake the GUI loop so the banner paints promptly.
                            let _ = wake.post();
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(TOAST_POLL_MS));
        }
    })
}

/// Ids currently in Action Center; `None` on transient failure (keeps the
/// old seen-set and retries next poll).
fn current_toast_ids(listener: &UserNotificationListener) -> Option<Vec<u32>> {
    let op = listener
        .GetNotificationsAsync(NotificationKinds::Toast)
        .ok()?;
    let list = block_async(&op, |o| o.Status(), || op.GetResults(), 50, 50).ok()?;
    Some(list.into_iter().filter_map(|n| n.Id().ok()).collect())
}

/// Reads one toast's app name plus title/body text. Anything unreadable
/// yields `None` (a removed toast); the id stays marked seen.
fn read_toast(listener: &UserNotificationListener, id: u32) -> Option<ToastEvent> {
    let n = listener.GetNotification(id).ok()?;
    let app = n
        .AppInfo()
        .and_then(|a| a.DisplayInfo())
        .and_then(|d| d.DisplayName())
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "Notification".to_string());
    let lines = toast_lines(&n);
    let mut lines = lines.into_iter();
    let title = trim_chars(&lines.next().unwrap_or_default(), MAX_TITLE_CHARS);
    let body = trim_chars(&lines.collect::<Vec<_>>().join(" "), MAX_BODY_CHARS);
    Some(ToastEvent {
        id,
        app,
        title,
        body,
    })
}

/// Toast text lines in layout order, skipping blanks.
fn toast_lines(n: &UserNotification) -> Vec<String> {
    let template = windows::core::HSTRING::from("ToastGeneric");
    n.Notification()
        .and_then(|n| n.Visual())
        .and_then(|v| v.GetBinding(&template))
        .and_then(|b| b.GetTextElements())
        .map(|elements| {
            elements
                .into_iter()
                .filter_map(|t| t.Text().ok().map(|s| s.to_string()))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seen_set_returns_only_fresh_ids_in_order() {
        let mut seen = SeenIds::default();
        assert_eq!(seen.take_new(&[1, 2, 3]), vec![1, 2, 3]);
        assert!(seen.take_new(&[1, 2, 3]).is_empty());
        assert_eq!(seen.take_new(&[3, 4]), vec![4]);
    }

    #[test]
    fn seen_set_caps_events_and_evics_oldest() {
        let mut seen = SeenIds::default();
        let ids: Vec<u32> = (0..MAX_SEEN as u32 + 10).collect();
        let fresh = seen.take_new(&ids);
        assert_eq!(fresh.len(), MAX_EVENTS_PER_POLL);
        assert_eq!(&fresh[..], &ids[..MAX_EVENTS_PER_POLL]);
        // Evicted ids may re-fire; the set stays bounded.
        assert!(seen.order.len() <= MAX_SEEN);
        let _ = seen.take_new(&[0]);
        assert!(seen.order.len() <= MAX_SEEN);
    }

    #[test]
    fn trim_keeps_char_boundaries() {
        assert_eq!(trim_chars("abc", 5), "abc");
        assert_eq!(trim_chars("héllo wörld", 7), "héllo w");
        assert_eq!(trim_chars("héllo wörld", 7).chars().count(), 7);
    }

    #[test]
    fn display_title_falls_back_to_app() {
        let e = ToastEvent {
            id: 1,
            app: "Mail".to_string(),
            title: "  ".to_string(),
            body: "hi".to_string(),
        };
        assert_eq!(e.display_title(), "Mail");
        assert_eq!(e.display_subtitle(), "Mail: hi");
    }
}

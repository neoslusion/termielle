use std::cmp::Ordering;
use std::collections::HashMap;

use crate::protocol::{EventKind, EventMessage, Source};

/// Milliseconds a submitted prompt stays [`VisualState::Thinking`] before it
/// becomes [`VisualState::Working`].
const THINKING_HOLD_MS: u64 = 1_000;

/// Milliseconds without an accepted event after which a session is dropped.
const STALE_SESSION_MS: u64 = 14_400_000;

/// Maximum accepted difference between an emitter clock and the overlay
/// clock. Local hooks normally share the system clock; a small allowance
/// absorbs scheduler jitter without letting a far-future event park a
/// session or its deadlines years away.
pub const MAX_FUTURE_SKEW_MS: u64 = 60_000;

/// What the overlay should render for the busiest tracked session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualState {
    Idle,
    Thinking,
    Working,
    NeedsInput,
    Ready,
    Failed,
}

impl VisualState {
    /// Higher wins when several sessions are active at once.
    fn priority(self) -> u8 {
        match self {
            Self::NeedsInput => 5,
            Self::Failed => 4,
            Self::Ready => 3,
            Self::Thinking | Self::Working => 2,
            Self::Idle => 1,
        }
    }

    /// Stable user-facing label for tray, bar, and detail surfaces.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Thinking => "Thinking",
            Self::Working => "Working",
            Self::NeedsInput => "Input needed",
            Self::Ready => "Ready",
            Self::Failed => "Failed",
        }
    }

    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Thinking => "thinking",
            Self::Working => "working",
            Self::NeedsInput => "needs_input",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

/// Result of feeding one event into [`SessionReducer::apply`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplyOutcome {
    /// Accepted, and the visible state or the next deadline moved.
    Changed,
    /// Accepted, but nothing observable moved.
    Unchanged,
    /// Same timestamp and kind as the last accepted event for the session.
    Duplicate,
    /// Older than the last accepted event for the session.
    Stale,
    /// Outside the wall-clock acceptance window. The event never enters the
    /// fold, so an ancient journal line cannot recreate an expired session.
    Rejected,
}

type SessionKey = (Source, String);

/// A pending automatic transition for a single session.
#[derive(Clone, Copy, Debug)]
struct Deadline {
    at_ms: u64,
    next: VisualState,
}

#[derive(Debug)]
struct Session {
    state: VisualState,
    /// Kind of the last accepted event, paired with `last_activity_ms` to spot
    /// duplicate deliveries.
    last_kind: EventKind,
    /// Timestamp of the last accepted event. Because the reducer never reads a
    /// clock, this doubles as the session's last activity time.
    last_activity_ms: u64,
    deadline: Option<Deadline>,
    /// A Thinking or Working session that hears nothing until this moment
    /// decays back to Idle; cleared when the session leaves a busy state.
    busy_stall_at_ms: Option<u64>,
}

impl Session {
    fn stale_at_ms(&self) -> u64 {
        self.last_activity_ms.saturating_add(STALE_SESSION_MS)
    }

    fn next_deadline_ms(&self) -> u64 {
        let mut earliest = self.stale_at_ms();
        if let Some(deadline) = self.deadline {
            earliest = earliest.min(deadline.at_ms);
        }
        if let Some(stall) = self.busy_stall_at_ms {
            earliest = earliest.min(stall);
        }
        earliest
    }
}

/// Folds lifecycle events from every tracked agent session into one visual state.
///
/// The reducer never reads a clock: time only enters through event timestamps
/// and the `now_ms` argument of [`SessionReducer::advance`].
#[derive(Debug)]
pub struct SessionReducer {
    sessions: HashMap<SessionKey, Session>,
    ready_hold_ms: u64,
    busy_stall_ms: u64,
}

impl SessionReducer {
    /// Creates a reducer that holds [`VisualState::Ready`] for `ready_hold_ms`
    /// after a completed turn, and decays a [`VisualState::Thinking`] or
    /// [`VisualState::Working`] session back to [`VisualState::Idle`] after
    /// `busy_stall_ms` without an event.
    pub fn new(ready_hold_ms: u64, busy_stall_ms: u64) -> Self {
        Self {
            sessions: HashMap::new(),
            ready_hold_ms,
            busy_stall_ms,
        }
    }

    /// Folds one event in, ignoring stale and duplicate deliveries.
    pub fn apply(&mut self, event: EventMessage) -> ApplyOutcome {
        let before = self.observable();
        let timestamp_ms = event.timestamp_ms;
        let kind = event.event;
        let key: SessionKey = (event.source, event.session_id);

        if let Some(session) = self.sessions.get(&key) {
            if timestamp_ms < session.last_activity_ms {
                return ApplyOutcome::Stale;
            }
            if timestamp_ms == session.last_activity_ms && kind == session.last_kind {
                return ApplyOutcome::Duplicate;
            }
        }

        match self.transition(kind, timestamp_ms) {
            None => {
                self.sessions.remove(&key);
            }
            Some((state, deadline)) => {
                self.sessions.insert(
                    key,
                    Session {
                        state,
                        last_kind: kind,
                        last_activity_ms: timestamp_ms,
                        deadline,
                        busy_stall_at_ms: busy_stall_at(state, timestamp_ms, self.busy_stall_ms),
                    },
                );
            }
        }

        if self.observable() == before {
            ApplyOutcome::Unchanged
        } else {
            ApplyOutcome::Changed
        }
    }

    /// Applies an event only when its timestamp is plausible relative to the
    /// consuming overlay's wall clock.
    ///
    /// Journal replay needs this boundary in addition to per-session ordering:
    /// once `advance` removes a stale session, ordering alone no longer has a
    /// tombstone to compare against and would accept the next old line as a
    /// brand-new session.
    pub fn apply_at(&mut self, event: EventMessage, now_ms: u64) -> ApplyOutcome {
        let oldest = now_ms.saturating_sub(STALE_SESSION_MS);
        let newest = now_ms.saturating_add(MAX_FUTURE_SKEW_MS);
        if event.timestamp_ms <= oldest || event.timestamp_ms > newest {
            return ApplyOutcome::Rejected;
        }
        self.apply(event)
    }

    /// Fires every deadline due at `now_ms`, decays busy sessions that have
    /// been silent for the configured stall, and drops stale sessions.
    ///
    /// `now_ms` must be a Unix epoch millisecond timestamp, the same time base as [`EventMessage::timestamp_ms`].
    ///
    /// Returns `true` when the visible state changed as a result.
    pub fn advance(&mut self, now_ms: u64) -> bool {
        let before = self.visible_state();

        self.sessions
            .retain(|_, session| now_ms < session.stale_at_ms());

        for session in self.sessions.values_mut() {
            if let Some(deadline) = session.deadline {
                if now_ms >= deadline.at_ms {
                    session.state = deadline.next;
                    session.deadline = None;
                }
            }
        }

        // Busy sessions that have heard nothing decay back to Idle. The record
        // itself survives until the stale window drops it, so a later prompt
        // resumes where the session left off.
        for session in self.sessions.values_mut() {
            if matches!(session.state, VisualState::Thinking | VisualState::Working)
                && session.busy_stall_at_ms.is_some_and(|at| now_ms >= at)
            {
                session.state = VisualState::Idle;
                session.deadline = None;
                session.busy_stall_at_ms = None;
            }
        }

        self.visible_state() != before
    }

    /// The state of the highest-priority session, or [`VisualState::Idle`] when
    /// nothing is tracked.
    pub fn visible_state(&self) -> VisualState {
        self.sessions
            .iter()
            .max_by(|(left_key, left), (right_key, right)| {
                left.state
                    .priority()
                    .cmp(&right.state.priority())
                    .then(left.last_activity_ms.cmp(&right.last_activity_ms))
                    .then_with(|| stable_cmp(right_key, left_key))
            })
            .map_or(VisualState::Idle, |(_, session)| session.state)
    }

    /// The primary active session (source, session_id, state), or `None` when
    /// no session is active.
    pub fn primary_session(&self) -> Option<(Source, String, VisualState)> {
        self.sessions
            .iter()
            .max_by(|(left_key, left), (right_key, right)| {
                left.state
                    .priority()
                    .cmp(&right.state.priority())
                    .then(left.last_activity_ms.cmp(&right.last_activity_ms))
                    .then_with(|| stable_cmp(right_key, left_key))
            })
            .map(|(key, session)| (key.0.clone(), key.1.clone(), session.state))
    }

    /// The earliest moment [`SessionReducer::advance`] can change anything.
    pub fn next_deadline_ms(&self) -> Option<u64> {
        self.sessions.values().map(Session::next_deadline_ms).min()
    }

    /// How many sessions are currently tracked.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// The state and pending deadline an event kind produces, or `None` when the
    /// event removes the session.
    fn transition(
        &self,
        kind: EventKind,
        timestamp_ms: u64,
    ) -> Option<(VisualState, Option<Deadline>)> {
        let transition = match kind {
            EventKind::SessionStarted => (VisualState::Idle, None),
            EventKind::PromptSubmitted => (
                VisualState::Thinking,
                Some(Deadline {
                    at_ms: timestamp_ms.saturating_add(THINKING_HOLD_MS),
                    next: VisualState::Working,
                }),
            ),
            // A real reasoning signal overrides the one-second heuristic: the
            // overlay holds Thinking until the agent reports reasoning done.
            EventKind::ThinkingStarted => (VisualState::Thinking, None),
            EventKind::ThinkingEnded => (VisualState::Working, None),
            EventKind::NeedsInput => (VisualState::NeedsInput, None),
            EventKind::TurnCompleted => (
                VisualState::Ready,
                Some(Deadline {
                    at_ms: timestamp_ms.saturating_add(self.ready_hold_ms),
                    next: VisualState::Idle,
                }),
            ),
            EventKind::TurnFailed => (VisualState::Failed, None),
            EventKind::SessionEnded => return None,
        };

        Some(transition)
    }

    fn observable(&self) -> (VisualState, Option<u64>) {
        (self.visible_state(), self.next_deadline_ms())
    }
}

/// When the busy-stall decay fires for a freshly transitioned session, if it
/// is in a busy state at all.
fn busy_stall_at(state: VisualState, timestamp_ms: u64, busy_stall_ms: u64) -> Option<u64> {
    match state {
        VisualState::Thinking | VisualState::Working => {
            Some(timestamp_ms.saturating_add(busy_stall_ms))
        }
        _ => None,
    }
}

/// Orders session keys deterministically so results never depend on hash
/// order. Sources compare as plain words, which keeps the historical
/// `claude < codex < opencode` order and extends it to any future agent.
fn stable_cmp(left: &SessionKey, right: &SessionKey) -> Ordering {
    left.0
        .as_str()
        .cmp(right.0.as_str())
        .then_with(|| left.1.cmp(&right.1))
}

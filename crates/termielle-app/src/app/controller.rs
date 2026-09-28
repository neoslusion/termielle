//! [`Controller`]: the session-reducer fold, the classic animation path, event/timer entry points.

use super::bar::modules::BarDamage;
use super::bar::types::{BarMetricsCache, BarZoneCache};
use super::types::ALERT_COUNTDOWN_TICK_MS;
use super::types::ALERT_FRESHNESS_MS;
use super::types::AlertBanner;
use super::types::BAR_REFRESH_MS;
use super::types::ControllerActions;
use super::types::FACE_SIZE;
use super::types::FALLBACK_FRAME_SIZE;
use super::types::GlassCacheKey;
use super::types::MAX_QUEUED_ALERTS;
use crate::animation::spring::{Spring1, Spring2D};
use crate::animation::{
    AnimationError, AnimationSource, FrameBuffer, GifAnimation, fallback_frame,
};
use crate::tasks::{MediaInfo, WorkerUpdate};
use std::collections::VecDeque;
use std::rc::Rc;

/// Destination-independent content swap used when geometry does not change.
/// The previous content is shared rather than copied, so state/activity swaps
/// do not duplicate a full-resolution frame.
pub(crate) struct ContentTransition {
    pub(crate) previous: Rc<crate::animation::FrameBuffer>,
    pub(crate) started_ms: u64,
    pub(crate) duration_ms: u64,
}
use termielle_core::{
    ApplyOutcome, AssetCatalog, EventMessage, IslandConfig, SessionReducer, VisualState,
};

/// Ties the session reducer, the animation pipeline, and one deadline together.
///
/// All times are Unix epoch milliseconds, the same base the emitter stamps
/// events with. All animation objects stay on the thread that constructed the
/// controller, because [`GifAnimation`] is deliberately not `Send`.
pub struct Controller {
    pub(crate) reducer: SessionReducer,
    pub(crate) assets: AssetCatalog,
    pub(crate) reduced_motion: bool,
    pub(crate) state: VisualState,
    pub(crate) animation: AnimationSource,
    /// Frame just decoded, cloned out of the reused decoder canvas.
    pub(crate) current: FrameBuffer,
    /// When the next GIF frame is due; `None` for stills and reduced motion.
    pub(crate) frame_deadline: Option<u64>,
    /// Next procedural-motion repaint; `None` when nothing moves.
    pub(crate) motion_deadline: Option<u64>,
    /// Fixed interval between animation frames in milliseconds; when set, it
    /// overrides each frame's GIF delay so the animation plays at a constant
    /// frame rate (e.g. 60 fps) instead of the file's own timing.
    pub(crate) frame_interval_ms: Option<u64>,
    pub(crate) motion_interval_ms: u64,
    /// Smoothed cost of presenting one frame, in milliseconds.
    pub(crate) present_cost_ms: u64,
    pub(crate) island: IslandConfig,
    /// Active 2D spring (island mode); `None` when settled or classic.
    pub(crate) spring: Option<Spring2D>,
    /// Timestamp of the last spring integration (for real elapsed dt).
    pub(crate) spring_last_ms: u64,
    /// Current animated corner radius; the source of truth the spring
    /// integrates toward [`Self::target_radius`].
    pub(crate) radius: f32,
    /// Active blob-separation spring; `None` when the island is merged.
    pub(crate) separation: Option<Spring1>,
    /// True while the pointer is held down on the pill: the island swells
    /// slightly, like the Dynamic Island under the fingertip.
    pub(crate) pressed: bool,
    pub(crate) manually_expanded: bool,
    /// Set while the cursor hovers the island (when `expand_on_hover`).
    pub(crate) hover_expanded: bool,
    /// Which body the bar popup is showing: `None` is the default card (media,
    /// activity, switcher, dashboard by priority), `Some` is the control
    /// panel. The panel is a deliberate choice, so it never steals the card
    /// from an activity: the pill's own click closes it first.
    pub(crate) panel_open: bool,
    pub(crate) hover_deadline: Option<(bool, u64)>,
    pub(crate) hover_suppressed: bool,
    /// Pre-decoded termielle face frames (downscaled to [`FACE_SIZE`]) plus
    /// per-frame delays in ms. Empty means the static procedural fallback.
    pub(crate) face_frames: Vec<FrameBuffer>,
    pub(crate) face_delays: Vec<u32>,
    pub(crate) face_idx: usize,
    /// Streaming GIF decoder kept while the face loop is still filling (see
    /// `refresh_face`). `None` once the loop is complete or the face is
    /// static.
    pub(crate) face_decoder: Option<GifAnimation>,
    /// Next face-animation tick; `None` for stills, reduced motion, classic
    /// mode, or `face_animated: false`.
    pub(crate) face_deadline: Option<u64>,
    /// The face frame currently composited into the notch.
    pub(crate) face_frame: FrameBuffer,
    /// Live running-task icons for the expanded pill. Filled by the
    /// background worker via [`Controller::set_task_update`]; the GUI thread
    /// never captures, so hovering and morphing never stall.
    /// Hit rects for the task icons of the last rendered frame:
    /// (hwnd, x, y, w, h) in frame coordinates. Used for click activation.
    pub(crate) icon_hits: Vec<(isize, i32, i32, u32, u32)>,
    /// Cursor position in frame coordinates (from the hover poll), so the
    /// icon under the cursor draws its accent border.
    pub(crate) hover_point: Option<(i32, i32)>,
    /// Whether the last painted frame showed a notification banner. The
    /// timeout countdown only ticks while this holds, so a pending alert
    /// that is not on screen costs nothing.
    pub(crate) alert_visible: bool,
    /// Next timeout-countdown repaint while a banner is visible.
    pub(crate) alert_deadline: Option<u64>,
    /// Cached frosted-glass layers keyed by (w, h, radius, attached, black,
    /// blob count, media-blob x, media-blob w, scale, first-blob w,
    /// separation). The single-pass paint is ~1ms; caching makes face ticks
    /// ~free. A bar frame needs two live layers at once (the strip and the
    /// center pill), so this holds a few rather than one: with a single slot
    /// the two would evict each other every frame and repaint both.
    pub(crate) glass_caches: Vec<(GlassCacheKey, FrameBuffer)>,
    /// Last now-playing media state from the background worker.
    pub(crate) media: Option<MediaInfo>,
    /// Open window task icons from the background worker.
    pub(crate) tasks: Vec<crate::tasks::TaskIcon>,
    /// Queued notification alert banners; the front one shows.
    pub(crate) alerts: VecDeque<AlertBanner>,
    /// Last controller-clock time, for renders without their own timestamp.
    pub(crate) clock_ms: u64,
    /// When the current visual state began, for one-shot state effects.
    pub(crate) state_since_ms: u64,
    /// Monitor DPI scale (physical px per logical px), refreshed from the
    /// window on every present. 1.0 until the first present.
    pub(crate) dpi_scale: f32,
    /// User zoom from `AppConfig.scale`, applied on top of DPI.
    pub(crate) user_scale: f32,
    /// Monitor logical width for Waybar layout.
    pub(crate) bar_width: u32,
    /// Next bar periodic refresh deadline; None when not in bar mode.
    pub(crate) bar_deadline: Option<u64>,
    /// Cached bar metrics to prevent heavy COM/Registry/CPU queries on every render.
    /// Most recently rendered transparent content layer for same-geometry
    /// crossfades.
    pub(crate) content_layer: Option<Rc<crate::animation::FrameBuffer>>,
    pub(crate) content_transition: Option<ContentTransition>,
    /// Short poll cadence while a manual card is open, so outside click and
    /// Escape dismissal do not depend on unrelated worker/face wakeups.
    pub(crate) interaction_deadline: Option<u64>,
    pub(crate) bar_metrics_cache: Option<BarMetricsCache>,
    pub(crate) bar_left_cache: Option<BarZoneCache>,
    pub(crate) bar_right_cache: Option<BarZoneCache>,
}
impl Controller {
    /// Runtime animation wakes are gated by the monitor clock, not a fixed FPS.
    /// GIF frame durations remain independent of procedural animation cadence.
    pub fn enable_display_pacing(&mut self) {
        self.motion_interval_ms = 0;
    }

    /// Builds a controller showing the Idle state, ready to present.
    /// `frame_interval_ms` overrides the GIF's per-frame delays when set.
    pub fn new(
        ready_hold_ms: u64,
        busy_stall_ms: u64,
        assets: AssetCatalog,
        reduced_motion: bool,
        frame_interval_ms: Option<u64>,
    ) -> Self {
        Self::new_with_island(
            ready_hold_ms,
            busy_stall_ms,
            assets,
            reduced_motion,
            frame_interval_ms,
            IslandConfig {
                layout: termielle_core::IslandLayout::Classic,
                ..IslandConfig::default()
            },
        )
    }

    /// Builds a controller with explicit island config.
    pub fn new_with_island(
        ready_hold_ms: u64,
        busy_stall_ms: u64,
        assets: AssetCatalog,
        reduced_motion: bool,
        frame_interval_ms: Option<u64>,
        island: IslandConfig,
    ) -> Self {
        let state = VisualState::Idle;
        let face_frame = fallback_frame(state, FACE_SIZE, 1.0);
        // Overwritten below for both island and classic modes.
        let still = fallback_frame(state, FALLBACK_FRAME_SIZE, 1.0);
        let (current, animation, frame_deadline) =
            (still.clone(), AnimationSource::Still(still), None);
        let radius = island.height as f32 / 2.0;
        let bar_deadline = island.is_bar().then_some(BAR_REFRESH_MS);
        let mut controller = Self {
            reducer: SessionReducer::new(ready_hold_ms, busy_stall_ms),
            assets,
            reduced_motion,
            state,
            animation,
            current,
            frame_deadline,
            motion_deadline: None,
            frame_interval_ms,
            present_cost_ms: 0,
            motion_interval_ms: 16,
            island,
            spring: None,
            spring_last_ms: 0,
            radius,
            separation: None,
            pressed: false,
            manually_expanded: false,
            panel_open: false,
            hover_expanded: false,
            hover_deadline: None,
            hover_suppressed: false,
            face_frames: Vec::new(),
            face_delays: Vec::new(),
            face_idx: 0,
            face_decoder: None,
            face_deadline: None,
            face_frame,
            icon_hits: Vec::new(),
            alert_visible: false,
            alert_deadline: None,
            hover_point: None,
            media: None,
            tasks: Vec::new(),
            glass_caches: Vec::new(),
            clock_ms: 0,
            state_since_ms: 0,
            alerts: VecDeque::new(),
            dpi_scale: 1.0,
            user_scale: 1.0,
            bar_width: 1920,
            bar_deadline,
            content_layer: None,
            content_transition: None,
            interaction_deadline: None,
            bar_metrics_cache: None,
            bar_left_cache: None,
            bar_right_cache: None,
        };
        if controller.island.is_enabled() {
            controller.refresh_face(state);
            let (w, h) = controller.target_size(state);
            controller.current = controller.render_island(state, w, h, 0);
            controller.arm_face_deadline(0);
        } else {
            let _ = controller.load_animation_classic(VisualState::Idle, 0);
        }
        controller
    }

    /// Stores a background worker round (media state + backdrop) at the
    /// controller's current clock. Compatibility wrapper for render examples;
    /// production drains call [`Self::set_task_update_at`] with a fresh clock.
    /// Returns true when the visible frame should repaint.
    pub fn set_task_update(&mut self, update: WorkerUpdate) -> bool {
        self.set_task_update_at(update, self.clock_ms)
    }

    /// Stores one worker snapshot and reconciles any content-driven geometry
    /// change immediately. If a morph is already active, the new target and
    /// velocity replace the old ones instead of being discarded until a later
    /// reducer transition happens to repair the layout.
    pub fn set_task_update_at(&mut self, update: WorkerUpdate, now_ms: u64) -> bool {
        self.clock_ms = now_ms;
        let from = self.current_logical_size();
        let media_changed = self
            .media
            .as_ref()
            .map(|m| (&m.title, &m.artist, &m.app, m.playing, &m.thumbnail))
            != update
                .media
                .as_ref()
                .map(|m| (&m.title, &m.artist, &m.app, m.playing, &m.thumbnail));
        let tasks_changed = self.tasks.len() != update.tasks.len()
            || self.tasks.iter().zip(&update.tasks).any(|(a, b)| {
                a.hwnd != b.hwnd
                    || a.title != b.title
                    || a.width != b.width
                    || a.height != b.height
                    || a.pixels_pbgra != b.pixels_pbgra
            });
        self.media = update.media;
        self.tasks = update.tasks;
        if !self.island.is_enabled() || (!media_changed && !tasks_changed) {
            return false;
        }

        let target = self.target_size(self.state);
        if target != from {
            self.morph_to_target(now_ms);
            return true;
        }
        if tasks_changed && self.island.is_bar() && self.island.bar.replace_taskbar {
            self.bar_left_cache = None;
            self.current =
                self.render_bar_with_damage(self.state, from.0, from.1, now_ms, BarDamage::LEFT);
            return true;
        }
        if !self.dashboard_expanded() && !self.media_available() {
            return false;
        }

        if !self.island.is_bar() && (media_changed || tasks_changed) {
            self.begin_content_transition(now_ms);
        }
        self.current = self.render_island(self.state, from.0, from.1, now_ms);
        true
    }

    /// Queues a transient alert. Repeated deliveries with the same stable key
    /// update the existing banner instead of multiplying transitions. A
    /// user-action agent alert may temporarily preempt a passive system
    /// toast; the toast returns to the queue with a fresh display lifetime.
    #[allow(clippy::too_many_arguments)] // alert content and ownership stay explicit at the queue boundary.
    pub fn trigger_alert(
        &mut self,
        kind: super::types::AlertKind,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        accent: [u8; 4],
        duration_ms: u64,
        now_ms: u64,
        dedupe_key: impl Into<String>,
    ) -> bool {
        if !self.island.is_enabled() {
            return false;
        }
        let previous_geometry = self.current_logical_size();
        let dedupe_key = dedupe_key.into();
        if let Some(index) = self
            .alerts
            .iter()
            .position(|alert| alert.dedupe_key == dedupe_key)
        {
            let is_front = index == 0;
            let alert = &mut self.alerts[index];
            alert.title = title.into();
            alert.subtitle = subtitle.into();
            alert.accent = accent;
            alert.duration_ms = duration_ms;
            alert.expires_at_ms = is_front.then(|| now_ms.saturating_add(duration_ms));
            if is_front {
                let (width, height) = self.current_logical_size();
                self.current = self.render_island(self.state, width, height, now_ms);
            }
            return is_front;
        }

        let alert = AlertBanner {
            title: title.into(),
            subtitle: subtitle.into(),
            accent,
            kind,
            dedupe_key,
            expires_at_ms: None,
            duration_ms,
        };
        let was_empty = self.alerts.is_empty();
        let preempts_system = kind == super::types::AlertKind::Agent
            && self
                .alerts
                .front()
                .is_some_and(|front| front.kind == super::types::AlertKind::System);
        if preempts_system {
            if let Some(mut deferred) = self.alerts.pop_front() {
                deferred.expires_at_ms = None;
                self.alerts.push_front(alert);
                self.alerts.insert(1, deferred);
            }
        } else {
            self.alerts.push_back(alert);
        }
        while self.alerts.len() > MAX_QUEUED_ALERTS + 1 {
            // The front owns the screen; discard the oldest unseen duplicate
            // when a burst exceeds the bounded deferred queue.
            self.alerts.remove(1);
        }
        if was_empty || preempts_system {
            self.arm_front_alert(now_ms);
        } else {
            // Unseen activities have no visual ownership and no timer yet.
            return false;
        }
        if preempts_system
            && !self.island.is_bar()
            && previous_geometry == self.target_size(self.state)
        {
            self.begin_content_transition(now_ms);
        }
        self.morph_to_target(now_ms)
    }

    /// Starts the visible banner's timeout only when it reaches the front.
    pub(crate) fn arm_front_alert(&mut self, now_ms: u64) {
        if let Some(alert) = self.alerts.front_mut() {
            alert.expires_at_ms = Some(now_ms.saturating_add(alert.duration_ms));
        }
    }

    /// Update island config live (e.g. theme switch).
    pub fn set_island_config(&mut self, island: IslandConfig, now_ms: u64) {
        let was_enabled = self.island.is_enabled();
        let is_enabled = island.is_enabled();
        let mode_changed = self.island.layout != island.layout;
        self.island = island;
        self.glass_caches.clear();
        self.bar_left_cache = None;
        self.bar_right_cache = None;
        self.hover_expanded = false;
        self.hover_deadline = None;
        self.hover_suppressed = false;
        if mode_changed {
            self.pressed = false;
            self.manually_expanded = false;
            self.interaction_deadline = None;
            self.hover_point = None;
            self.separation = None;
        }

        if was_enabled != is_enabled || mode_changed {
            // A surface cutover has no meaningful shared geometry. Snap once,
            // but never carry pointer or split state into the new layout.
            self.spring = None;
            if is_enabled {
                let (width, height) = self.target_size(self.state);
                self.current = self.render_island(self.state, width, height, now_ms);
                self.frame_deadline = None;
                self.arm_face_deadline(now_ms);
            } else {
                self.separation = None;
                let _ = self.load_animation_classic(self.state, now_ms);
            }
        } else if is_enabled {
            // Theme and geometry changes are interruptions, not restarts:
            // retarget the live spring and retain its physical velocity.
            self.morph_to_target(now_ms);
            self.arm_face_deadline(now_ms);
        }
    }

    /// Begins a short source/destination blend when the container itself is
    /// staying at one geometry. Geometry morphs keep their existing spatial
    /// sequencing and therefore do not need a second opacity system.
    pub(crate) fn begin_content_transition(&mut self, now_ms: u64) {
        if self.reduced_motion || !self.island.is_enabled() {
            self.content_transition = None;
            return;
        }
        if let Some(previous) = self.content_layer.clone() {
            self.content_transition = Some(ContentTransition {
                previous,
                started_ms: now_ms,
                duration_ms: 140,
            });
            self.motion_deadline = Some(now_ms.saturating_add(self.motion_interval_ms));
        }
    }

    /// Feeds the measured cost of the last present back into frame pacing.
    /// Opens the control panel body. The pill's glyph is the user-facing
    /// door; this is the same door for the review harness and for a host that
    /// wants to show the panel directly.
    pub fn open_control_panel(&mut self, now_ms: u64) -> bool {
        self.panel_open = true;
        self.morph_to_target(now_ms)
    }

    pub fn set_present_cost(&mut self, elapsed_ms: u64) {
        self.present_cost_ms = (self.present_cost_ms + elapsed_ms) / 2;
    }

    /// Folds one pipe event in and returns what the window must do.
    pub fn handle_event(&mut self, event: EventMessage, now_ms: u64) -> ControllerActions {
        self.clock_ms = now_ms;
        let kind = event.event;
        let source = event.source.as_str().to_owned();
        let session = event.session_id.clone();
        let alert_key = format!("agent:{source}:{session}:{kind:?}");
        let fresh = now_ms.saturating_sub(event.timestamp_ms) <= ALERT_FRESHNESS_MS;
        self.reducer.advance(now_ms);
        let outcome = self.reducer.apply_at(event, now_ms);
        let accepted = !matches!(
            outcome,
            ApplyOutcome::Stale | ApplyOutcome::Duplicate | ApplyOutcome::Rejected
        );
        let mut actions = self.sync_state(now_ms);

        if accepted && self.island.is_enabled() {
            match kind {
                termielle_core::EventKind::NeedsInput if fresh => {
                    self.trigger_alert(
                        super::types::AlertKind::Agent,
                        "Input",
                        source.clone(),
                        crate::animation::notch::accent_colors(VisualState::NeedsInput).0,
                        3500,
                        now_ms,
                        alert_key.clone(),
                    );
                    actions.present_frame = true;
                }
                termielle_core::EventKind::TurnFailed if fresh => {
                    self.trigger_alert(
                        super::types::AlertKind::Agent,
                        "Failed",
                        source.clone(),
                        crate::animation::notch::accent_colors(VisualState::Failed).0,
                        3500,
                        now_ms,
                        alert_key.clone(),
                    );
                    actions.present_frame = true;
                }
                _ => {}
            }
        }

        actions.next_deadline_ms = self.next_deadline_ms();
        actions
    }

    /// Fires every reducer and animation deadline due at `now_ms`.
    pub fn on_timer(&mut self, now_ms: u64) -> ControllerActions {
        self.clock_ms = now_ms;
        self.reducer.advance(now_ms);
        let mut actions = self.sync_state(now_ms);
        if let Some((expanded, at)) = self.hover_deadline {
            if now_ms >= at {
                self.hover_deadline = None;
                self.hover_expanded = expanded;
                actions.present_frame |= self.morph_to_target(now_ms);
            }
        }

        // NOTE: dashboard content arrives from the background worker via
        // `set_task_update`; nothing here may block on window capture or
        // media queries, otherwise hovering and morphing visibly stall.

        // Check if the showing transient notification alert expired; the
        // next queued banner (if any) takes its place via the re-morph.
        let expired = self.alerts.front().is_some_and(|alert| {
            alert
                .expires_at_ms
                .is_some_and(|expires_at| now_ms >= expires_at)
        });
        if expired {
            self.alerts.pop_front();
            self.arm_front_alert(now_ms);
            let previous_geometry = self.current_logical_size();
            if !self.island.is_bar() && previous_geometry == self.target_size(self.state) {
                self.begin_content_transition(now_ms);
            }
            self.morph_to_target(now_ms);
            actions.present_frame = true;
        }

        // Timeout countdown tick: the visible banner's hairline has to
        // advance every frame, otherwise the remaining life reads as a
        // stutter. Only runs while a banner is on screen; the bar's 2 s
        // metrics refresh is far too coarse to carry it.
        if self.alert_visible
            && self
                .alerts
                .front()
                .is_some_and(|alert| alert.expires_at_ms.is_some())
        {
            if self.alert_deadline.is_none_or(|due| now_ms >= due) {
                self.repaint_alert_countdown(now_ms);
                actions.present_frame = true;
            }
            self.alert_deadline = Some(now_ms.saturating_add(ALERT_COUNTDOWN_TICK_MS));
        } else {
            self.alert_deadline = None;
        }

        // Spring morph tick: integrate (width, height, radius) toward the
        // target with the iOS spring model, alongside the blob-separation
        // spring.
        if self.island.is_enabled() {
            if let Some(spring) = self.spring.as_mut() {
                let mut elapsed = now_ms.saturating_sub(self.spring_last_ms).min(200);
                self.spring_last_ms = now_ms;
                let omega = spring.stiffness.sqrt().max(1.0);
                let sub_ms = ((0.35 / omega) * 1000.0).ceil().max(2.0) as u64;
                let mut current_size = (spring.x.round() as u32, spring.y.round() as u32);
                let mut settled = false;
                while elapsed > 0 && !settled {
                    let dt = (elapsed.min(sub_ms) as f32 / 1000.0).max(0.0005);
                    elapsed = elapsed.saturating_sub(sub_ms);
                    if let Some(sep) = self.separation.as_mut() {
                        sep.step(dt);
                    }
                    let ((nw, nh), ns) = {
                        let (size, s) = spring.step(dt);
                        ((size.0, size.1), s)
                    };
                    current_size = (nw, nh);
                    settled = ns;
                }
                self.radius = spring.z;
                let target_w = spring.target_x.round() as u32;
                let target_h = spring.target_y.round() as u32;
                let separation_settled = self.separation.as_ref().is_none_or(|s| s.settled());
                let target_reached = settled && separation_settled;
                if target_reached {
                    self.spring = None;
                    self.separation = None;
                    self.current = self.render_island(self.state, target_w, target_h, now_ms);
                    self.frame_deadline = None;
                    self.arm_face_deadline(now_ms);
                } else {
                    let (w, h) = current_size;
                    self.current = self.render_island(self.state, w, h, now_ms);
                    let interval = self.motion_interval_ms;
                    let next = now_ms
                        .saturating_add(interval)
                        .saturating_sub(self.present_cost_ms.min(interval.saturating_sub(1)));
                    self.frame_deadline = Some(next);
                }
                actions.present_frame = true;
            } else if self.separation.is_some() {
                // The size settled but the blobs are still travelling.
                let mut sep = self.separation.take().unwrap_or_default();
                let elapsed = now_ms.saturating_sub(self.spring_last_ms).min(100);
                self.spring_last_ms = now_ms;
                for _ in 0..elapsed {
                    sep.step(0.001);
                }
                let settled = sep.settled();
                // `separation_now` already supplies the settled split gap;
                // retaining a zero-velocity Some keeps procedural motion
                // permanently gated.
                if !settled {
                    self.separation = Some(sep);
                }
                if !settled || self.split_active() {
                    let (w, h) = self.target_size(self.state);
                    self.current = self.render_island(self.state, w, h, now_ms);
                    actions.present_frame = true;
                }
                if !settled {
                    let interval = self.motion_interval_ms;
                    self.frame_deadline = Some(now_ms.saturating_add(interval));
                }
            }
        }

        // Face-animation tick: cheap (pre-decoded frames + cached icons).
        // It keeps ticking during morphs — on the real island the activity
        // content never freezes while the container is moving.
        if self.island.is_enabled() {
            if let Some(face_at) = self.face_deadline {
                if now_ms >= face_at {
                    self.advance_face(now_ms);
                    actions.present_frame = true;
                }
            }
        }

        let content_due = self.content_transition.as_ref().is_some_and(|transition| {
            now_ms >= transition.started_ms.saturating_add(transition.duration_ms)
        });

        // Procedural-motion tick: repaints the pill so bounce, orbit, pulse,
        // sparkles, shake, and the equalizer move even with no other
        // deadline pending. Skipped while a morph tick already repainted
        // this call — the poses ride the morph renders instead.
        if (self.motion_deadline.is_some_and(|d| now_ms >= d) || content_due)
            && self.spring.is_none()
            && self.separation.is_none()
        {
            self.motion_deadline = None;
            let (w, h) = self.current_logical_size();
            self.current = self.render_island(self.state, w, h, now_ms);
            actions.present_frame = true;
        }
        if self.motion_active(now_ms) {
            let due = now_ms.saturating_add(self.motion_interval_ms);
            self.motion_deadline = Some(match self.motion_deadline {
                Some(d) => d.min(due),
                None => due,
            });
        } else {
            self.motion_deadline = None;
        }

        // Bar mode periodic update for clock and passive system metrics.
        if self.island.is_bar() {
            if self.bar_deadline.is_some_and(|d| now_ms >= d) {
                self.bar_deadline = Some(now_ms.saturating_add(BAR_REFRESH_MS));
                let (w, h) = self.current_logical_size();
                self.current = self.render_island(self.state, w, h, now_ms);
                actions.present_frame = true;
            } else if self.bar_deadline.is_none() {
                self.bar_deadline = Some(now_ms.saturating_add(BAR_REFRESH_MS));
            }
        }

        if let Some(deadline) = self.frame_deadline {
            if self.spring.is_none() && now_ms >= deadline {
                actions.present_frame = true;
                if let Some(code) = self.advance_animation(now_ms) {
                    actions.error_code = Some(code);
                }
            }
        }

        self.interaction_deadline = self.manually_expanded.then(|| now_ms.saturating_add(100));
        actions.next_deadline_ms = self.next_deadline_ms();
        actions
    }

    /// Repaints only what a running timeout changes: the card carrying the
    /// hairline. The bar's side zones keep their cached layers, so a live
    /// countdown costs a card redraw rather than a full bar rebuild.
    fn repaint_alert_countdown(&mut self, now_ms: u64) {
        let (width, height) = self.current_logical_size();
        self.current = if self.island.is_bar() {
            self.render_bar_with_damage(self.state, width, height, now_ms, BarDamage::CENTER)
        } else {
            self.render_island(self.state, width, height, now_ms)
        };
    }

    /// Whether any procedural motion is live: thinking bounce, worker orbit,
    /// input pulse, celebration/shake one-shots, or a playing equalizer.
    /// Gated on island mode and full motion — reduced motion stills
    /// everything procedural (the face keeps its own deadline). The media
    /// equalizer only moves while actually playing; a paused pose is static.
    pub(crate) fn motion_active(&self, now_ms: u64) -> bool {
        if !self.island.is_enabled() || self.reduced_motion {
            return false;
        }
        if self.content_transition.is_some() {
            return true;
        }
        if self.media_playing() {
            return true;
        }
        match self.state {
            VisualState::Thinking | VisualState::Working | VisualState::NeedsInput => true,
            VisualState::Ready => now_ms.saturating_sub(self.state_since_ms) < 600,
            VisualState::Failed => now_ms.saturating_sub(self.state_since_ms) < 300,
            VisualState::Idle => false,
        }
    }

    /// The earliest moment [`Controller::handle_event`] or [`Controller::on_timer`]
    /// can change anything, or `None` when no deadline is pending.
    pub fn next_deadline_ms(&self) -> Option<u64> {
        let earliest = match (self.reducer.next_deadline_ms(), self.frame_deadline) {
            (Some(reducer_at), Some(frame_at)) => Some(reducer_at.min(frame_at)),
            (reducer_at, frame_at) => reducer_at.or(frame_at),
        };
        let mut deadline = match (earliest, self.face_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if let Some(expires_at) = self.alerts.front().and_then(|alert| alert.expires_at_ms) {
            deadline = Some(deadline.map_or(expires_at, |at| at.min(expires_at)));
        }
        if let Some(countdown_at) = self.alert_deadline {
            deadline = Some(deadline.map_or(countdown_at, |at| at.min(countdown_at)));
        }
        if let Some(transition) = &self.content_transition {
            let ends_at = transition.started_ms.saturating_add(transition.duration_ms);
            deadline = Some(deadline.map_or(ends_at, |at| at.min(ends_at)));
        }
        deadline = match (deadline, self.motion_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if let Some(interaction_at) = self.interaction_deadline {
            deadline = Some(deadline.map_or(interaction_at, |at| at.min(interaction_at)));
        }
        if self.island.is_bar() {
            if let Some(bar_at) = self.bar_deadline {
                deadline = match deadline {
                    Some(d) => Some(d.min(bar_at)),
                    None => Some(bar_at),
                };
            }
        }
        if let Some((_, hover_at)) = self.hover_deadline {
            deadline = Some(deadline.map_or(hover_at, |at| at.min(hover_at)));
        }
        deadline
    }

    /// The frame that should currently be on screen.
    pub fn current_frame(&self) -> &FrameBuffer {
        &self.current
    }

    /// The click regions the current frame installed, as
    /// `(id, x, y, w, h)` in frame coordinates. Read-only: it lets callers
    /// that cannot see `icon_hits` - tests, the review renderer - address a
    /// module by asking where it is instead of recomputing its layout.
    pub fn click_regions(&self) -> &[(isize, i32, i32, u32, u32)] {
        &self.icon_hits
    }

    /// The state currently being presented; drives the acknowledgement file.
    pub fn visible_state(&self) -> VisualState {
        self.state
    }

    /// Whether island mode is active.
    pub fn is_island(&self) -> bool {
        self.island.is_enabled()
    }

    /// Replaces the active frame with the still for the current state.
    pub fn fallback_to_still(&mut self) {
        if self.island.is_enabled() {
            let (w, h) = self.target_size(self.state);
            self.spring = None;
            self.current = self.render_island(self.state, w, h, self.clock_ms);
            self.separation = None;
            self.frame_deadline = None;
            self.arm_face_deadline(0);
        } else {
            let still = fallback_frame(self.state, FALLBACK_FRAME_SIZE, self.render_scale());
            self.frame_deadline = None;
            self.current = still.clone();
            self.animation = AnimationSource::Still(still);
        }
    }

    /// Reloads the animation when the visible state moved.
    pub(crate) fn sync_state(&mut self, now_ms: u64) -> ControllerActions {
        let mut actions = ControllerActions::default();
        let state = self.reducer.visible_state();
        let previous_geometry = self.current_logical_size();
        if state != self.state {
            self.state = state;
            self.state_since_ms = now_ms;
            actions.visible_state = Some(state);
            actions.present_frame = true;
            // A manual expansion is the user's explicit open: mid-turn flips
            // (thinking -> working) must not collapse it from under them.
            // Only the turn ending retires the card.
            if state == VisualState::Idle {
                self.manually_expanded = false;
                self.interaction_deadline = None;
            }
            if self.island.is_enabled() {
                self.refresh_face(state);
                if !self.island.is_bar() && previous_geometry == self.target_size(state) {
                    self.begin_content_transition(now_ms);
                }
                self.morph_to_target(now_ms);
                self.arm_face_deadline(now_ms);
            } else {
                actions.error_code = self.load_animation_classic(state, now_ms);
            }
        } else if self.island.is_enabled() {
            // Visual state didn't move, but active content (e.g. sessions or media) may
            // dynamically change the target pill size.
            let (target_w, target_h) = self.target_size(self.state);
            let (current_w, current_h) = self.current_logical_size();
            let target_changed = self
                .spring
                .as_ref()
                .map_or((current_w, current_h) != (target_w, target_h), |spring| {
                    spring.target_x != target_w as f32 || spring.target_y != target_h as f32
                });
            if target_changed && !self.pressed {
                actions.present_frame |= self.morph_to_target(now_ms);
            }
        }
        actions
    }

    /// Loads the animation for `state` (classic pet path only).
    pub(crate) fn load_animation_classic(
        &mut self,
        state: VisualState,
        now_ms: u64,
    ) -> Option<i32> {
        self.frame_deadline = None;
        self.current = Self::to_render_size(
            self.render_scale(),
            fallback_frame(state, FALLBACK_FRAME_SIZE, 1.0),
        );

        let path = self.assets.resolve(state)?;
        let mut gif = match GifAnimation::open(&path) {
            Ok(gif) => gif,
            Err(error) => {
                self.animation = AnimationSource::Still(Self::to_render_size(
                    self.render_scale(),
                    fallback_frame(state, FALLBACK_FRAME_SIZE, 1.0),
                ));
                return Some(error_code(&error));
            }
        };

        match gif.next_frame() {
            Ok(frame) => {
                self.current = Self::to_render_size(self.render_scale(), frame.clone());
                if self.reduced_motion {
                    self.animation = AnimationSource::Still(self.current.clone());
                } else {
                    let interval = self.frame_interval_ms;
                    let present_cost = self.present_cost_ms;
                    self.frame_deadline =
                        Some(deadline_for(interval, present_cost, now_ms, frame.delay_ms));
                    self.animation = AnimationSource::Gif(gif);
                }
                None
            }
            Err(error) => {
                self.animation = AnimationSource::Still(Self::to_render_size(
                    self.render_scale(),
                    fallback_frame(state, FALLBACK_FRAME_SIZE, 1.0),
                ));
                Some(error_code(&error))
            }
        }
    }

    /// Advances the active GIF by one frame and re-arms its deadline.
    pub(crate) fn advance_animation(&mut self, now_ms: u64) -> Option<i32> {
        if self.island.is_enabled() {
            return None;
        }
        let interval = self.frame_interval_ms;
        let present_cost = self.present_cost_ms;
        let render_scale = self.render_scale();
        let gif = match &mut self.animation {
            AnimationSource::Gif(gif) => gif,
            AnimationSource::Still(_) => {
                self.frame_deadline = None;
                return None;
            }
        };

        match gif.next_frame() {
            Ok(frame) => {
                self.current = Self::to_render_size(render_scale, frame.clone());
                self.frame_deadline =
                    Some(deadline_for(interval, present_cost, now_ms, frame.delay_ms));
                None
            }
            Err(error) => {
                let code = error_code(&error);
                self.fallback_to_still();
                Some(code)
            }
        }
    }
}
/// The deadline for the next animation frame: the fixed interval when
/// configured (minus the smoothed present cost), else the frame's own GIF
/// delay.
pub(crate) fn deadline_for(
    interval: Option<u64>,
    present_cost_ms: u64,
    now_ms: u64,
    gif_delay_ms: u32,
) -> u64 {
    match interval {
        Some(interval) => now_ms
            .saturating_add(interval)
            .saturating_sub(present_cost_ms.min(interval.saturating_sub(1))),
        None => now_ms.saturating_add(u64::from(gif_delay_ms.max(1))),
    }
}
/// Stable numeric identity for an animation failure, safe for bounded logs.
fn error_code(error: &AnimationError) -> i32 {
    match error {
        AnimationError::Win32(code) => *code as i32,
        AnimationError::ReadFailed => 1,
        AnimationError::Empty => 2,
        AnimationError::Unsupported => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use termielle_core::{EventKind, EventMessage, IslandLayout, Source};

    fn quiet_island() -> Controller {
        Controller::new_with_island(
            5_000,
            60_000,
            AssetCatalog::new(Vec::new()),
            false,
            None,
            IslandConfig {
                layout: IslandLayout::Island,
                face_animated: false,
                forward_toasts: false,
                ..IslandConfig::default()
            },
        )
    }

    #[test]
    fn retarget_preserves_fractional_position_and_duplicate_target_clock() {
        let mut c = quiet_island();
        c.toggle_expand(1_000);
        c.on_timer(1_017);
        let before = c.spring.unwrap();
        let last = c.spring_last_ms;
        c.morph_to_target(1_020);
        assert_eq!(c.spring_last_ms, last);
        assert_eq!(c.spring.unwrap().x, before.x);
        c.collapse_if_expanded(1_023);
        let after = c.spring.unwrap();
        assert_eq!(after.x, before.x);
        assert_eq!(after.y, before.y);
        assert_eq!(after.vx, before.vx);
        assert_eq!(after.vy, before.vy);
    }

    #[test]
    fn passive_content_crossfade_schedules_intermediate_frames_then_sleeps() {
        let mut c = quiet_island();
        c.begin_content_transition(1_000);
        assert!(c.next_deadline_ms().unwrap() < 1_140);
        assert!(c.on_timer(1_020).present_frame);
        assert!(c.content_transition.is_some());
        assert!(c.next_deadline_ms().unwrap() < 1_140);
        c.on_timer(1_140);
        assert!(c.content_transition.is_none());
        assert!(c.next_deadline_ms().is_none());
    }

    #[test]
    fn queued_alert_does_not_restart_visible_morph() {
        use super::super::types::AlertKind;
        let mut c = quiet_island();
        c.trigger_alert(AlertKind::System, "one", "", [255; 4], 2_000, 1_000, "one");
        c.on_timer(1_017);
        let before = c.spring.unwrap();
        assert!(!c.trigger_alert(AlertKind::System, "two", "", [255; 4], 2_000, 1_020, "two"));
        assert_eq!(c.spring_last_ms, 1_017);
        assert_eq!(c.spring.unwrap().vx, before.vx);
        assert_eq!(c.alerts[0].expires_at_ms, Some(3_000));
        assert_eq!(c.alerts[1].expires_at_ms, None);
    }

    #[test]
    fn identical_worker_snapshot_is_not_a_frame_request() {
        let mut c = quiet_island();
        let update = WorkerUpdate {
            media: Some(crate::tasks::MediaInfo {
                title: "Paused song".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(c.set_task_update_at(update.clone(), 1_000));
        assert!(!c.set_task_update_at(update, 1_010));
    }

    #[test]
    fn single_frame_and_hidden_faces_do_not_schedule_repaints() {
        let mut c = quiet_island();
        c.island.face_animated = true;
        c.face_frames.push(c.face_frame.clone());
        c.face_delays.push(20);
        c.advance_face(1_000);
        assert!(c.face_deadline.is_none());
        c.face_frames.push(c.face_frame.clone());
        c.face_delays.push(20);
        c.island.auto_hide = true;
        c.arm_face_deadline(1_100);
        assert!(c.face_deadline.is_none());
    }

    /// A resting bar face must still advance. Suppressing its deadline pinned
    /// the pill to the GIF's opening frame, which is nearly blank, so the face
    /// looked like it had vanished rather than sitting still.
    #[test]
    fn a_resting_bar_face_still_schedules_its_next_frame() {
        let mut c = Controller::new_with_island(
            5_000,
            60_000,
            AssetCatalog::new(Vec::new()),
            false,
            None,
            IslandConfig {
                layout: IslandLayout::Bar,
                face_animated: true,
                ..IslandConfig::default()
            },
        );
        c.set_bar_width(1536);
        c.face_frames.push(c.face_frame.clone());
        c.face_delays.push(40);
        c.face_frames.push(c.face_frame.clone());
        c.face_delays.push(40);

        c.arm_face_deadline(1_000);
        let armed = c
            .face_deadline
            .expect("a resting bar face must arm its next frame");
        // The asset's own 40 ms cadence, floored at 20: not the island's
        // 10 fps resting throttle, which is what froze it.
        assert_eq!(armed, 1_040);

        let before = c.face_idx;
        c.advance_face(1_040);
        assert_ne!(c.face_idx, before, "the face must advance a frame");
        assert!(c.face_deadline.is_some());
    }

    /// The bar pill and the short card blit `face_frame`, not `face_idx`.
    /// Cycling the index without publishing the new frame left both showing
    /// the GIF's first frame while the tick ran on schedule, which is exactly
    /// "the animation is playing but nothing moves".
    #[test]
    fn advancing_the_face_publishes_the_frame_the_pill_blits() {
        let mut c = Controller::new_with_island(
            5_000,
            60_000,
            AssetCatalog::new(Vec::new()),
            false,
            None,
            IslandConfig {
                layout: IslandLayout::Bar,
                face_animated: true,
                ..IslandConfig::default()
            },
        );
        c.set_bar_width(1536);
        let mut frames = Vec::new();
        for shade in [10u8, 90, 170, 240] {
            let mut frame = c.face_frame.clone();
            frame.pixels_pbgra.fill(shade);
            frames.push(frame);
        }
        c.face_frames = frames;
        c.face_delays = vec![40; 4];
        c.face_frame = c.face_frames[0].clone();

        c.advance_face(1_000);
        assert_eq!(c.face_idx, 1);
        assert_eq!(
            c.face_frame.pixels_pbgra[0], 90,
            "the blitted face must be the advanced frame, not frame 0"
        );
        c.advance_face(1_040);
        c.advance_face(1_080);
        assert_eq!(c.face_idx, 3);
        assert_eq!(c.face_frame.pixels_pbgra[0], 240);
    }

    /// A bar with the hover setting in the given state. Reduced motion so the
    /// morph lands on its target in one tick and the tests can assert geometry
    /// rather than an intermediate frame of the spring.
    fn bar_with_hover(expand_on_hover: bool) -> Controller {
        let mut c = Controller::new_with_island(
            5_000,
            60_000,
            AssetCatalog::new(Vec::new()),
            true,
            None,
            IslandConfig {
                layout: IslandLayout::Bar,
                face_animated: true,
                expand_on_hover,
                ..IslandConfig::default()
            },
        );
        c.set_bar_width(1536);
        c
    }

    /// A bar's pill is the Dynamic Island: with `expand_on_hover` on, dwelling
    /// over it opens the same card a click opens. This used to be refused
    /// outright for bar layout while the control panel and the tray menu both
    /// offered the toggle.
    #[test]
    fn hovering_a_bar_pill_opens_its_card_after_the_dwell() {
        let mut c = bar_with_hover(true);
        c.set_hover(true, 1_000);
        // Dwell deferred: nothing on screen has changed yet.
        assert!(!c.hover_expanded, "hover must not open on the first move");
        assert_eq!(
            c.hover_deadline.map(|(_, at)| at),
            Some(1_000 + Controller::HOVER_DWELL_MS)
        );

        c.on_timer(1_000 + Controller::HOVER_DWELL_MS);
        assert!(c.hover_expanded);
        // A hovered bar pill reports Expanded, so the window grows the same
        // way a click does.
        assert!(matches!(
            c.presentation(),
            crate::animation::notch::Presentation::Expanded
        ));
    }

    /// A pointer resting on a pill is never perfectly still: a pixel of
    /// jitter reads as leaving. Without cancelling the armed close the card
    /// collapsed, re-armed open, and the two chased each other - nine opens
    /// and fourteen closes in two seconds.
    #[test]
    fn jittering_on_the_pill_does_not_oscillate_an_open_card() {
        let mut c = bar_with_hover(true);
        let mut now;

        // Settle first: the dwell is continuous time on the pill.
        for step in 0..25 {
            now = 1_000 + step * 20;
            c.set_hover(true, now);
            c.on_timer(now);
        }
        assert!(c.hover_expanded, "a settled pointer must open the card");

        // Then jitter: a single poll off the pill every twenty.
        let mut opens = 0;
        let mut closes = 0;
        for step in 0..60 {
            now = 1_500 + step * 20;
            let inside = step % 20 != 7;
            c.set_hover(inside, now);
            let before = c.hover_expanded;
            c.on_timer(now);
            if c.hover_expanded && !before {
                opens += 1;
            }
            if before && !c.hover_expanded {
                closes += 1;
            }
        }
        assert_eq!(closes, 0, "a jittering pointer must not close the card");
        assert_eq!(opens, 0, "an open card must not re-open on jitter");
        assert!(c.hover_expanded, "the card must still be open at the end");
    }

    /// Leaving waits a grace period so the pointer can reach the card it just
    /// opened instead of dismissing it on the way down.
    #[test]
    fn leaving_a_bar_pill_closes_after_the_grace() {
        let mut c = bar_with_hover(true);
        c.set_hover(true, 1_000);
        c.on_timer(1_000 + Controller::HOVER_DWELL_MS);
        assert!(c.hover_expanded);

        c.set_hover(false, 2_000);
        assert!(
            c.hover_expanded,
            "the card must survive the pointer leaving"
        );
        assert_eq!(
            c.hover_deadline.map(|(_, at)| at),
            Some(2_000 + Controller::HOVER_GRACE_MS)
        );

        c.on_timer(2_000 + Controller::HOVER_GRACE_MS);
        assert!(!c.hover_expanded);
    }

    /// A click that dismissed the card must not be undone by the pointer still
    /// resting on the pill; leaving and returning re-arms it.
    #[test]
    fn a_click_close_is_not_undone_by_a_still_pointer() {
        let mut c = bar_with_hover(true);
        c.manually_expanded = true;
        c.collapse_if_expanded(1_000);
        assert!(c.hover_suppressed);

        c.set_hover(true, 1_100);
        c.on_timer(1_100 + Controller::HOVER_DWELL_MS);
        assert!(
            !c.hover_expanded,
            "a dismissed card must stay dismissed while the pointer rests"
        );

        c.set_hover(false, 1_200);
        c.set_hover(true, 1_300);
        c.on_timer(1_300 + Controller::HOVER_DWELL_MS);
        assert!(c.hover_expanded, "a fresh hover must be able to reopen it");
    }

    /// Reaching a switch means travelling from the pill down a card that is
    /// taller than the strip. If "left the pill" closed the card, the pointer
    /// arrived at the switch after the card had gone and the click landed on
    /// the desktop - the control panel's switches were unreachable by hover.
    #[test]
    fn a_hover_opened_card_survives_the_trip_from_the_pill_to_a_switch() {
        let mut c = bar_with_hover(true);
        c.set_bar_width(1536);
        let (width, height) = c.current_logical_size();
        let (pill_cx, pill_off, pill_w, pill_h) = c.bar_pill_rect(c.bar_width);

        // The pointer lands on the pill and dwells.
        let pill_mid_x = pill_cx + (pill_w / 2) as i32;
        let pill_mid_y = pill_off + (pill_h / 2) as i32;
        let on_pill = (pill_mid_x, pill_mid_y);
        c.set_hover(true, 1_000);
        c.on_timer(1_000 + Controller::HOVER_DWELL_MS);
        assert!(c.hover_expanded, "the card must be open");

        let (_, open_height) = c.current_logical_size();
        assert!(open_height > height, "the frame must have grown");
        let on_switch = (pill_mid_x, pill_mid_y + 80);

        // Mid-trip, well past the old 500 ms grace, the surface still counts.
        assert!(
            c.point_over_bar_surface(on_switch),
            "a hover-opened card must stay open while the pointer is on it"
        );
        assert!(c.point_over_bar_surface(on_pill));
        // The bar strip spans the full width, so staying over the window is
        // still "on the surface"; leaving means leaving the window.
        assert!(
            c.point_over_bar_surface((10, 10)),
            "the strip itself stays on the surface while the card is open"
        );
        assert!(
            !c.point_over_bar_surface((pill_mid_x, open_height as i32 + 40)),
            "leaving the whole surface must end the hover"
        );

        // A card opened by *click* covers the window too, and it has to:
        // click-outside dismissal asks the same question, and testing the pill
        // alone made every click inside the panel look like a click outside,
        // so the card dismissed itself instead of pressing the switch.
        let mut clicked = bar_with_hover(true);
        clicked.set_hover(true, 1_000);
        clicked.on_timer(1_000 + Controller::HOVER_DWELL_MS);
        clicked.manually_expanded = true;
        clicked.hover_expanded = false;
        clicked.on_timer(2_000);
        let (open_w, open_h) = clicked.current_logical_size();
        assert!(
            open_h > 36,
            "the card must be open for this to mean anything"
        );
        let inside = (pill_cx + (pill_w / 2) as i32, open_h as i32 - 10);
        assert!(
            clicked.point_over_bar_surface(inside),
            "a click inside a click-opened card is not a click outside it"
        );
        assert!(
            !clicked.point_over_bar_surface((5, open_h as i32 + 20)),
            "a click past the card still dismisses"
        );
        let _ = (on_switch, open_w);
        let _ = (width, pill_w, pill_h);
    }

    /// The toggle has to be able to close what it opened, otherwise turning it
    /// off mid-hover leaves the card stuck open.
    #[test]
    fn turning_expand_on_hover_off_closes_a_hover_opened_card() {
        let mut c = bar_with_hover(true);
        c.set_hover(true, 1_000);
        c.on_timer(1_000 + Controller::HOVER_DWELL_MS);
        assert!(c.hover_expanded);

        c.island.expand_on_hover = false;
        c.set_hover(true, 2_000);
        assert!(!c.hover_expanded);
        assert!(c.hover_deadline.is_none());
    }

    /// `frame_rate` claims to override each GIF's own delays. It only ever
    /// reached the classic surface, because `advance_animation` returns early
    /// once the island is enabled - so setting 60 did nothing for a bar face,
    /// which stayed on the asset's 40 ms cadence.
    #[test]
    fn a_configured_frame_rate_overrides_the_face_asset_delay() {
        let mut c = Controller::new_with_island(
            5_000,
            60_000,
            AssetCatalog::new(Vec::new()),
            false,
            // 60 Hz, as main.rs derives it from a 60 fps setting.
            Some(17),
            IslandConfig {
                layout: IslandLayout::Bar,
                face_animated: true,
                ..IslandConfig::default()
            },
        );
        c.set_bar_width(1536);
        c.face_frames.push(c.face_frame.clone());
        c.face_delays.push(200);
        c.face_frames.push(c.face_frame.clone());
        c.face_delays.push(200);

        c.arm_face_deadline(1_000);
        // 200 ms of authored delay, overridden by the 17 ms cadence.
        assert_eq!(c.face_deadline, Some(1_017));

        c.frame_interval_ms = None;
        c.arm_face_deadline(1_000);
        assert_eq!(c.face_deadline, Some(1_200), "the asset delay takes over");
    }

    #[test]
    fn session_count_retargets_an_active_morph_without_waiting_for_settlement() {
        let mut c = quiet_island();
        c.island.minimal_width = 48;
        let event = |session: &str, at| EventMessage {
            version: 1,
            source: Source::parse("codex").unwrap(),
            session_id: session.into(),
            event: EventKind::PromptSubmitted,
            timestamp_ms: at,
        };
        c.handle_event(event("one", 1_000), 1_000);
        c.on_timer(1_017);
        let before = c.spring.unwrap();
        c.handle_event(event("two", 1_020), 1_020);
        let after = c.spring.unwrap();
        assert_ne!(before.target_x, after.target_x);
        assert_eq!(before.x, after.x);
        assert_eq!(before.vx, after.vx);
        assert_eq!(after.target_x, c.target_size(c.state).0 as f32);
    }

    #[test]
    fn settled_bar_keeps_only_its_periodic_metrics_deadline() {
        let mut island = IslandConfig {
            layout: IslandLayout::Bar,
            face_animated: false,
            forward_toasts: false,
            ..IslandConfig::default()
        };
        island.bar.modules_left.clear();
        island.bar.modules_right.clear();
        let mut controller = Controller::new_with_island(
            5_000,
            60_000,
            AssetCatalog::new(Vec::new()),
            false,
            None,
            island,
        );
        controller.enable_display_pacing();
        controller.set_bar_width(1920);
        let event = |kind, at| EventMessage {
            version: 1,
            source: Source::parse("codex").unwrap(),
            session_id: "bench".into(),
            event: kind,
            timestamp_ms: at,
        };
        controller.handle_event(event(EventKind::NeedsInput, 10_000), 10_000);
        controller.handle_event(event(EventKind::TurnCompleted, 11_000), 11_000);
        for now in (11_016..=20_000).step_by(16) {
            controller.on_timer(now);
        }

        assert_eq!(controller.visible_state(), VisualState::Idle);
        assert!(controller.spring.is_none());
        assert!(controller.separation.is_none());
        assert!(controller.frame_deadline.is_none());
        assert!(controller.face_deadline.is_none());
        assert!(controller.motion_deadline.is_none());
        assert!(controller.content_transition.is_none());
        assert!(controller.interaction_deadline.is_none());
        assert!(controller.next_deadline_ms().is_some_and(|at| at > 20_000));
    }
}

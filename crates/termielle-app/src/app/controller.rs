//! [`Controller`]: the session-reducer fold, the classic animation path, event/timer entry points.

use super::bar::types::{BarMetricsCache, BarZoneCache};
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
    /// Cached frosted-glass layer keyed by (w, h, radius, attached). The
    /// single-pass paint is ~1ms; caching makes face ticks ~free.
    /// Cached frosted-glass layer keyed by (w, h, radius, attached, black,
    /// blob count, media-blob x, media-blob w). The single-pass paint is
    /// ~1ms; caching makes face ticks ~free.
    pub(crate) glass_cache: Option<(GlassCacheKey, FrameBuffer)>,
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
            hover_point: None,
            media: None,
            tasks: Vec::new(),
            glass_cache: None,
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
        self.glass_cache = None;
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
fn deadline_for(
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

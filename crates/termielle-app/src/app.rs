//! The controller: folds pipe events and timer ticks into window actions.
//!
//! [`Controller`] owns the reducer, the active animation, and the single
//! deadline the window's timer is armed for. It never touches Win32 itself;
//! [`main`](crate::main) maps its actions onto the window, the pipe, and the
//! log.

use crate::animation::{
    AnimationError, AnimationSource, FrameBuffer, GifAnimation, fallback_frame,
};
use termielle_core::{AssetCatalog, EventMessage, SessionReducer, VisualState};

/// Square edge length of procedural fallback frames.
pub const FALLBACK_FRAME_SIZE: u32 = 360;

/// What the GUI thread must do after one controller step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ControllerActions {
    /// Set when the visible state changed and its animation was reloaded.
    pub visible_state: Option<VisualState>,
    /// True when the frame changed and the window should repaint.
    pub present_frame: bool,
    /// Nearest reducer or animation deadline, for the single Win32 timer.
    pub next_deadline_ms: Option<u64>,
    /// Numeric code of an asset that failed to decode this step, if any.
    pub error_code: Option<i32>,
}

/// Ties the session reducer, the animation pipeline, and one deadline together.
///
/// All times are Unix epoch milliseconds, the same base the emitter stamps
/// events with. All animation objects stay on the thread that constructed the
/// controller, because [`GifAnimation`] is deliberately not `Send`.
pub struct Controller {
    reducer: SessionReducer,
    assets: AssetCatalog,
    reduced_motion: bool,
    state: VisualState,
    animation: AnimationSource,
    /// Frame just decoded, cloned out of the reused decoder canvas.
    current: FrameBuffer,
    /// When the next GIF frame is due; `None` for stills and reduced motion.
    frame_deadline: Option<u64>,
}

impl Controller {
    /// Builds a controller showing the Idle state, ready to present.
    pub fn new(
        ready_hold_ms: u64,
        busy_stall_ms: u64,
        assets: AssetCatalog,
        reduced_motion: bool,
    ) -> Self {
        let mut controller = Self {
            reducer: SessionReducer::new(ready_hold_ms, busy_stall_ms),
            assets,
            reduced_motion,
            state: VisualState::Idle,
            animation: AnimationSource::Still(fallback_frame(
                VisualState::Idle,
                FALLBACK_FRAME_SIZE,
            )),
            current: fallback_frame(VisualState::Idle, FALLBACK_FRAME_SIZE),
            frame_deadline: None,
        };
        let _ = controller.load_animation(VisualState::Idle, 0);
        controller
    }

    /// Folds one pipe event in and returns what the window must do.
    ///
    /// Time passes while an event travels, so due transitions fire before the
    /// event is applied: a late event sees the state machine it actually is in.
    pub fn handle_event(&mut self, event: EventMessage, now_ms: u64) -> ControllerActions {
        self.reducer.advance(now_ms);
        self.reducer.apply(event);
        let mut actions = self.sync_state(now_ms);
        actions.next_deadline_ms = self.next_deadline_ms();
        actions
    }

    /// Fires every reducer and animation deadline due at `now_ms`.
    pub fn on_timer(&mut self, now_ms: u64) -> ControllerActions {
        self.reducer.advance(now_ms);
        let mut actions = self.sync_state(now_ms);

        if let Some(deadline) = self.frame_deadline {
            if now_ms >= deadline {
                actions.present_frame = true;
                if let Some(code) = self.advance_animation(now_ms) {
                    actions.error_code = Some(code);
                }
            }
        }

        actions.next_deadline_ms = self.next_deadline_ms();
        actions
    }

    /// The earliest moment [`Controller::handle_event`] or [`Controller::on_timer`]
    /// can change anything, or `None` when no deadline is pending.
    pub fn next_deadline_ms(&self) -> Option<u64> {
        match (self.reducer.next_deadline_ms(), self.frame_deadline) {
            (Some(reducer_at), Some(frame_at)) => Some(reducer_at.min(frame_at)),
            (reducer_at, frame_at) => reducer_at.or(frame_at),
        }
    }

    /// The frame that should currently be on screen.
    pub fn current_frame(&self) -> &FrameBuffer {
        &self.current
    }

    /// The state currently being presented; drives the acknowledgement file.
    pub fn visible_state(&self) -> VisualState {
        self.state
    }

    /// Replaces the active animation with the procedural still for the current
    /// state, cancelling any pending frame deadline.
    pub fn fallback_to_still(&mut self) {
        let still = fallback_frame(self.state, FALLBACK_FRAME_SIZE);
        self.frame_deadline = None;
        self.current = still.clone();
        self.animation = AnimationSource::Still(still);
    }

    /// Reloads the animation when the visible state moved; the new frame is
    /// marked for presentation.
    fn sync_state(&mut self, now_ms: u64) -> ControllerActions {
        let mut actions = ControllerActions::default();
        let state = self.reducer.visible_state();
        if state != self.state {
            self.state = state;
            actions.visible_state = Some(state);
            actions.present_frame = true;
            actions.error_code = self.load_animation(state, now_ms);
        }
        actions
    }

    /// Loads the animation for `state`. Returns the numeric error code when
    /// the asset cannot be decoded; the procedural still is shown instead.
    fn load_animation(&mut self, state: VisualState, now_ms: u64) -> Option<i32> {
        self.frame_deadline = None;
        self.current = fallback_frame(state, FALLBACK_FRAME_SIZE);

        let path = self.assets.resolve(state)?;
        let mut gif = match GifAnimation::open(&path) {
            Ok(gif) => gif,
            Err(error) => {
                self.animation = AnimationSource::Still(fallback_frame(state, FALLBACK_FRAME_SIZE));
                return Some(error_code(&error));
            }
        };

        match gif.next_frame() {
            Ok(frame) => {
                self.current = frame.clone();
                if self.reduced_motion {
                    // First frame only: a still with no frame deadline.
                    self.animation = AnimationSource::Still(frame.clone());
                } else {
                    self.frame_deadline =
                        Some(now_ms.saturating_add(u64::from(frame.delay_ms.max(1))));
                    self.animation = AnimationSource::Gif(gif);
                }
                None
            }
            Err(error) => {
                self.animation = AnimationSource::Still(fallback_frame(state, FALLBACK_FRAME_SIZE));
                Some(error_code(&error))
            }
        }
    }

    /// Advances the active GIF by one frame and re-arms its deadline. Returns
    /// the numeric error code when the frame fails to decode, in which case
    /// the procedural still replaces the animation.
    fn advance_animation(&mut self, now_ms: u64) -> Option<i32> {
        let gif = match &mut self.animation {
            AnimationSource::Gif(gif) => gif,
            AnimationSource::Still(_) => {
                self.frame_deadline = None;
                return None;
            }
        };

        match gif.next_frame() {
            Ok(frame) => {
                self.current = frame.clone();
                self.frame_deadline = Some(now_ms.saturating_add(u64::from(frame.delay_ms.max(1))));
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

/// Stable numeric identity for an animation failure, safe for bounded logs.
fn error_code(error: &AnimationError) -> i32 {
    match error {
        AnimationError::Win32(code) => *code as i32,
        AnimationError::ReadFailed => 1,
        AnimationError::Empty => 2,
        AnimationError::Unsupported => 3,
    }
}

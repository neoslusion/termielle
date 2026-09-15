//! Motion signatures: shake, think-bob, worker orbit, ready sparkles, morph content-motion.

use super::super::controller::Controller;
use crate::animation::spring::Spring2D;
use termielle_core::VisualState;

impl Controller {
    /// Content motion during a morph: a slight opacity dip while the
    /// container is at its fastest (mid-spring) and a small rise into
    /// place when the target is the expanded card. Settled frames get
    /// full opacity.
    pub(crate) fn content_motion(
        spring: Option<&Spring2D>,
        presentation: crate::animation::notch::Presentation,
        state: VisualState,
        state_age_ms: u64,
    ) -> (u8, i32, i32) {
        use crate::animation::notch::Presentation;
        let dx = Self::shake_dx(state, state_age_ms);
        let Some(spring) = spring else {
            return (255, dx, 0);
        };
        let p = spring.progress();
        let dip = (1.0 - 0.45 * (p * std::f32::consts::PI).sin()).clamp(0.0, 1.0);
        let alpha = (255.0 * dip).round() as u8;
        let dy = match presentation {
            Presentation::Expanded => ((1.0 - p) * 6.0).round() as i32,
            _ => 0,
        };
        (alpha, dx, dy)
    }

    /// Damped horizontal shake for failed turns: ±4 px decaying over 300 ms,
    /// then exactly zero so settled frames stay pixel-stable.
    pub(crate) fn shake_dx(state: VisualState, age_ms: u64) -> i32 {
        if state != VisualState::Failed || age_ms >= 300 {
            return 0;
        }
        let age = age_ms as f32;
        (4.0 * (-age / 90.0).exp() * (age * 0.22).sin()).round() as i32
    }

    /// Vertical bounce for thinking dots: staggered sine, ±2 px. Zero for
    /// every other state so settled frames stay pixel-stable.
    pub(crate) fn think_bob(state: VisualState, index: usize, now_ms: u64) -> i32 {
        if state != VisualState::Thinking {
            return 0;
        }
        (2.0 * ((now_ms as f32 / 240.0) + index as f32 * 2.1).sin()).round() as i32
    }

    /// Worker orbit position: dots circle the face while tools run.
    /// Positions only; the caller gates on Working and paints.
    pub(crate) fn orbit_dot(cx: i32, cy: i32, radius: i32, index: u32, now_ms: u64) -> (i32, i32) {
        let a = now_ms as f32 / 600.0 * std::f32::consts::TAU + index as f32 * 2.094;
        (
            cx + (radius as f32 * a.cos()).round() as i32,
            cy + (radius as f32 * a.sin()).round() as i32,
        )
    }

    /// Celebration sparkle after a turn completes: one of 8 dots flying out
    /// from (`cx`, `cy`) over 600 ms with fading alpha, then `None` forever.
    /// Deterministic in age: no particle state to keep.
    pub(crate) fn sparkle_dot(cx: i32, cy: i32, index: u32, age_ms: u64) -> Option<(i32, i32, u8)> {
        if age_ms >= 600 {
            return None;
        }
        let t = age_ms as f32 / 600.0;
        let a = index as f32 * std::f32::consts::TAU / 8.0;
        let r = 6.0 + 20.0 * t;
        Some((
            cx + (r * a.cos()).round() as i32,
            cy + (r * a.sin()).round() as i32,
            ((1.0 - t) * 255.0).round() as u8,
        ))
    }
}

#[test]
fn think_bob_bounces_only_while_thinking() {
    use termielle_core::VisualState;
    assert_eq!(Controller::think_bob(VisualState::Thinking, 0, 0), 0);
    assert_ne!(
        Controller::think_bob(VisualState::Thinking, 0, 0),
        Controller::think_bob(VisualState::Thinking, 0, 120)
    );
    assert_eq!(Controller::think_bob(VisualState::Working, 0, 120), 0);
}

#[test]
fn shake_fires_briefly_then_locks_to_zero() {
    use termielle_core::VisualState;
    assert_eq!(Controller::shake_dx(VisualState::Failed, 0), 0);
    assert_ne!(Controller::shake_dx(VisualState::Failed, 50), 0);
    assert_eq!(Controller::shake_dx(VisualState::Failed, 300), 0);
    assert_eq!(Controller::shake_dx(VisualState::Failed, 5000), 0);
    assert_eq!(Controller::shake_dx(VisualState::Working, 50), 0);
}

#[test]
fn sparkle_flies_out_then_vanishes() {
    let start = Controller::sparkle_dot(100, 100, 0, 0).unwrap();
    let mid = Controller::sparkle_dot(100, 100, 0, 300).unwrap();
    assert!(mid.0 - 100 > start.0 - 100, "radius must grow with age");
    assert!(mid.2 < start.2, "alpha must fade with age");
    assert_eq!(Controller::sparkle_dot(100, 100, 0, 600), None);
    assert_eq!(Controller::sparkle_dot(100, 100, 0, 5000), None);
}

#[test]
fn orbit_circles_with_time() {
    let a = Controller::orbit_dot(50, 50, 10, 0, 0);
    let b = Controller::orbit_dot(50, 50, 10, 0, 150);
    assert_ne!(a, b);
    for t in [0, 100, 250, 599] {
        let (x, y) = Controller::orbit_dot(50, 50, 10, 1, t);
        let d = (((x - 50) * (x - 50) + (y - 50) * (y - 50)) as f32).sqrt();
        assert!((d - 10.0).abs() <= 1.5, "off circle: {d}");
    }
}

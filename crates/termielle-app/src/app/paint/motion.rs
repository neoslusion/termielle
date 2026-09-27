//! Motion signatures: shake, think-bob, ready sparkles, and morph content-motion.

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
        // Gate expanded copy on available space rather than elapsed progress:
        // interrupted springs and the small press swell must not restart a fade.
        let opacity = if presentation == Presentation::Expanded {
            let fit =
                (spring.x / spring.target_x.max(1.0)).min(spring.y / spring.target_y.max(1.0));
            crate::animation::notch::smoothstep(0.80, 0.98, fit)
        } else {
            dip
        };
        let alpha = (255.0 * opacity).round() as u8;
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
        // Epoch milliseconds lose minutes of precision in f32. Reduce the
        // phase in f64 before handing a small angle to the rasterizer.
        let phase = (now_ms as f64 / 240.0).rem_euclid(std::f64::consts::TAU) as f32;
        (2.0 * (phase + index as f32 * 2.1).sin()).round() as i32
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
fn thinking_motion_advances_at_real_epoch_timestamps() {
    let epoch = 1_790_000_000_000;
    let positions: Vec<_> = (0..60)
        .map(|frame| Controller::think_bob(VisualState::Thinking, 0, epoch + frame * 16))
        .collect();
    assert!(positions.iter().any(|position| *position != positions[0]));
}

#[test]
fn expanded_copy_waits_for_space_but_press_feedback_keeps_it_visible() {
    use crate::animation::notch::Presentation;
    let params = termielle_core::spring_params(350, 0.18);
    let mut spring = Spring2D::new(72, 36, 18.0, 320, 154, 24.0, params);
    let opacity = |s: &Spring2D| {
        Controller::content_motion(Some(s), Presentation::Expanded, VisualState::Working, 0).0
    };
    assert_eq!(opacity(&spring), 0);
    spring.x = 300.0;
    spring.y = 145.0;
    assert!(opacity(&spring) > 0 && opacity(&spring) < 255);
    spring.x = 320.0;
    spring.y = 154.0;
    assert_eq!(opacity(&spring), 255);
    let pressed = Spring2D::new(320, 154, 24.0, 330, 159, 25.0, params);
    assert!(opacity(&pressed) > 240);
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

//! The shared motion vocabulary: how long each kind of motion takes, and the
//! curve that shapes it.
//!
//! These values come from the design-engineering skills under
//! `.opencode/skills/` (`animate`, `review-animations/STANDARDS.md`). They live
//! in one place so that surfaces which should feel alike cannot drift apart by
//! one hand-typed number. The island morph is not here: it is a spring, and its
//! timing is the user's `animation_ms` and `spring_bounce` settings.

/// A status chip taking or losing its hover fill. Skills: hover and colour
/// changes, 100-160 ms.
pub(crate) const HOVER_MS: u64 = 120;

/// A pressed status chip letting go of its press fill. The press itself is
/// instant; only the release eases. Skills: press feedback, 100-160 ms.
pub(crate) const PRESS_RELEASE_MS: u64 = 140;

/// The launcher's arrow-key highlight travelling one row. Keyboard navigation
/// is repeated hundreds of times a day, so this is near-imperceptible by
/// design: fast enough to read as continuity rather than as a delay.
pub(crate) const SELECTION_GLIDE_MS: u64 = 60;

/// The launcher growing out of the bar's pill. Alt+Space is a keyboard action
/// and the skills' frequency gate says such actions should not animate at all,
/// so this is the shortest a spatial connection to the pill can be drawn in: a
/// strong ease-out that does most of its work in the first third, interruptible
/// by typing (the window is live from the first frame). With no bar to grow
/// from, or reduced motion, there is no reveal.
pub(crate) const LAUNCHER_REVEAL_MS: u64 = 160;

/// The launcher collapsing back into the pill. The reverse of the opening, and
/// shorter: a deliberate action takes the longer beat, and putting the window
/// away is the system's answer to it.
pub(crate) const LAUNCHER_COLLAPSE_MS: u64 = 120;

/// The dot on the pill that stands for notifications waiting behind the
/// launcher fading in and out. Skills: small UI, 100-160 ms.
pub(crate) const NOTIFICATION_DOT_FADE_MS: u64 = 120;

/// Same-geometry island content swapping for new content (state, media, or
/// alert). Skills: small UI, under 300 ms.
pub(crate) const CONTENT_SWAP_MS: u64 = 140;

/// The strong ease-out the skills ask for on anything entering or answering
/// the pointer: `cubic-bezier(0.23, 1, 0.32, 1)`. It starts fast, so the
/// moment the user is watching is the moment most of the change happens; the
/// built-in cubic ease-out it replaces covers about 49 % of the distance by
/// 20 % of the time, this covers about 68 %.
pub(crate) fn ease_out(progress: f32) -> f32 {
    cubic_bezier(0.23, 1.0, 0.32, 1.0, progress)
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` evaluated at `progress` (0..=1).
///
/// The curve is parametric in an internal variable, so the x coordinate has to
/// be inverted to find where `progress` falls on it. x is monotonic whenever
/// `x1` and `x2` are in 0..=1, which makes bisection exact and unconditionally
/// stable.
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, progress: f32) -> f32 {
    // The endpoints are exact: a transition must start precisely at its origin
    // and land precisely on its target, not within bisection error of them.
    if progress <= 0.0 {
        return 0.0;
    }
    if progress >= 1.0 {
        return 1.0;
    }
    let at = |a: f32, b: f32, s: f32| {
        let inverse = 1.0 - s;
        3.0 * a * inverse * inverse * s + 3.0 * b * inverse * s * s + s * s * s
    };
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    for _ in 0..24 {
        let middle = (low + high) / 2.0;
        if at(x1, x2, middle) < progress {
            low = middle;
        } else {
            high = middle;
        }
    }
    at(y1, y2, (low + high) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_out_runs_from_zero_to_one_and_never_steps_backwards() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        let mut previous = 0.0;
        for step in 1..=100 {
            let value = ease_out(step as f32 / 100.0);
            assert!(value >= previous, "step {step}: {value} < {previous}");
            previous = value;
        }
    }

    #[test]
    fn ease_out_is_front_loaded_more_than_the_cubic_it_replaces() {
        let cubic = 1.0 - (1.0_f32 - 0.2).powi(3);
        assert!(ease_out(0.2) > cubic + 0.1);
        assert!(ease_out(0.2) > 0.55);
    }

    #[test]
    fn progress_outside_the_unit_range_is_clamped() {
        assert_eq!(ease_out(-1.0), 0.0);
        assert_eq!(ease_out(2.0), 1.0);
    }
}

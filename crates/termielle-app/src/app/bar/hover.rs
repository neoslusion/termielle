use crate::animation::motion::{HOVER_MS, PRESS_RELEASE_MS};
use crate::animation::transition::Transition;

pub(crate) type Capsule = (i32, i32, u32, u32);

#[derive(Default)]
pub(crate) struct CapsuleHover {
    entries: Vec<(Capsule, Transition)>,
    pressed: Option<(Capsule, Transition)>,
    pub(crate) deadline: Option<u64>,
}

impl CapsuleHover {
    pub(crate) fn set_target(
        &mut self,
        target: Option<Capsule>,
        now_ms: u64,
        reduced: bool,
    ) -> bool {
        self.entries
            .retain(|(_, transition)| transition.target > 0.0 || transition.value(now_ms) > 0.0);
        let mut changed = false;
        for (capsule, transition) in &mut self.entries {
            changed |= reduced && transition.active(now_ms);
            let value = if Some(*capsule) == target { 1.0 } else { 0.0 };
            changed |= transition.retarget(value, now_ms, HOVER_MS);
            if reduced {
                transition.snap(value);
            }
        }
        if let Some(capsule) = target {
            if !self
                .entries
                .iter()
                .any(|(existing, _)| *existing == capsule)
            {
                let mut transition = Transition::default();
                transition.retarget(1.0, now_ms, HOVER_MS);
                if reduced {
                    transition.snap(1.0);
                }
                self.entries.push((capsule, transition));
                changed = true;
            }
        }
        if changed {
            self.schedule(now_ms, 1);
        }
        changed
    }

    pub(crate) fn amount(&self, capsule: Capsule, now_ms: u64) -> f32 {
        self.entries
            .iter()
            .find(|(existing, _)| *existing == capsule)
            .map_or(0.0, |(_, transition)| transition.value(now_ms))
    }

    /// Presses `target` (or releases the held chip when `None`). A press is
    /// instant: feedback that waits for the release is feedback that arrives
    /// after the action. Returns whether anything changed.
    pub(crate) fn set_pressed(
        &mut self,
        target: Option<Capsule>,
        now_ms: u64,
        reduced: bool,
    ) -> bool {
        let changed = match (target, self.pressed.as_mut()) {
            (Some(capsule), Some((held, transition))) if *held == capsule => {
                transition.retarget(1.0, now_ms, 0)
            }
            (Some(capsule), _) => {
                let mut transition = Transition::default();
                transition.retarget(1.0, now_ms, 0);
                self.pressed = Some((capsule, transition));
                true
            }
            (None, Some((_, transition))) => {
                transition.retarget(0.0, now_ms, if reduced { 0 } else { PRESS_RELEASE_MS })
            }
            (None, None) => false,
        };
        if changed {
            self.schedule(now_ms, 1);
        }
        changed
    }

    pub(crate) fn press_amount(&self, capsule: Capsule, now_ms: u64) -> f32 {
        self.pressed
            .as_ref()
            .filter(|(held, _)| *held == capsule)
            .map_or(0.0, |(_, transition)| transition.value(now_ms))
    }

    pub(crate) fn schedule(&mut self, now_ms: u64, interval_ms: u64) {
        let pressed_active = self
            .pressed
            .as_ref()
            .is_some_and(|(_, transition)| transition.active(now_ms));
        self.deadline = (pressed_active
            || self
                .entries
                .iter()
                .any(|(_, transition)| transition.active(now_ms)))
        .then(|| now_ms.saturating_add(interval_ms.max(1)));
        self.entries
            .retain(|(_, transition)| transition.target > 0.0 || transition.active(now_ms));
        if self
            .pressed
            .as_ref()
            .is_some_and(|(_, transition)| transition.target == 0.0 && !transition.active(now_ms))
        {
            self.pressed = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_lands_instantly_then_eases_out_on_release() {
        let chip = (10, 5, 40, 26);
        let mut hover = CapsuleHover::default();
        assert!(hover.set_pressed(Some(chip), 100, false));
        assert_eq!(hover.press_amount(chip, 100), 1.0);
        assert_eq!(hover.deadline, None);
        assert!(!hover.set_pressed(Some(chip), 110, false));
        assert!(hover.set_pressed(None, 120, false));
        let mid = hover.press_amount(chip, 190);
        assert!(mid > 0.0 && mid < 1.0);
        assert_eq!(hover.deadline, Some(121));
        hover.schedule(300, 16);
        assert_eq!(hover.deadline, None);
        assert_eq!(hover.press_amount(chip, 300), 0.0);
        assert!(!hover.set_pressed(None, 300, false));
        assert!(hover.set_pressed(Some(chip), 400, true));
        assert!(hover.set_pressed(None, 410, true));
        assert_eq!(hover.press_amount(chip, 410), 0.0);
        assert_eq!(hover.deadline, None);
    }

    #[test]
    fn hover_crossfades_then_parks_and_reduced_motion_is_immediate() {
        let first = (10, 5, 40, 26);
        let second = (60, 5, 40, 26);
        let mut hover = CapsuleHover::default();
        assert!(hover.set_target(Some(first), 0, false));
        assert!(hover.amount(first, 60) > 0.0);
        assert!(!hover.set_target(Some(first), 60, false));
        hover.set_target(Some(second), 60, false);
        assert!(hover.amount(first, 90) > 0.0);
        assert!(hover.amount(second, 90) > 0.0);
        hover.schedule(200, 16);
        assert_eq!(hover.deadline, None);
        assert_eq!(hover.amount(first, 200), 0.0);
        assert_eq!(hover.amount(second, 200), 1.0);
        hover.set_target(None, 210, true);
        assert_eq!(hover.deadline, None);
        assert_eq!(hover.amount(second, 210), 0.0);
    }
}

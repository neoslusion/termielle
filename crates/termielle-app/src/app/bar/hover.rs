use crate::animation::transition::Transition;

pub(crate) type Capsule = (i32, i32, u32, u32);

#[derive(Default)]
pub(crate) struct CapsuleHover {
    entries: Vec<(Capsule, Transition)>,
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
            changed |= transition.retarget(value, now_ms, 120);
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
                transition.retarget(1.0, now_ms, 120);
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

    pub(crate) fn schedule(&mut self, now_ms: u64, interval_ms: u64) {
        self.deadline = self
            .entries
            .iter()
            .any(|(_, transition)| transition.active(now_ms))
            .then(|| now_ms.saturating_add(interval_ms.max(1)));
        self.entries
            .retain(|(_, transition)| transition.target > 0.0 || transition.active(now_ms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

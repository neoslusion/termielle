#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Transition {
    from: f32,
    pub(crate) target: f32,
    started_ms: u64,
    duration_ms: u64,
}

impl Transition {
    pub(crate) fn is_constant(self) -> bool {
        self.from == self.target
    }

    pub(crate) fn value(self, now_ms: u64) -> f32 {
        if self.duration_ms == 0 {
            return self.target;
        }
        let progress = (now_ms.saturating_sub(self.started_ms) as f32 / self.duration_ms as f32)
            .clamp(0.0, 1.0);
        self.from + (self.target - self.from) * super::motion::ease_out(progress)
    }

    pub(crate) fn active(self, now_ms: u64) -> bool {
        self.from != self.target && now_ms < self.started_ms.saturating_add(self.duration_ms)
    }

    pub(crate) fn retarget(&mut self, target: f32, now_ms: u64, duration_ms: u64) -> bool {
        if self.target == target {
            return false;
        }
        self.from = self.value(now_ms);
        self.target = target;
        self.started_ms = now_ms;
        self.duration_ms = duration_ms;
        true
    }

    pub(crate) fn snap(&mut self, target: f32) {
        self.from = target;
        self.target = target;
        self.duration_ms = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_end_and_retarget_without_jumping_or_restarting() {
        let mut transition = Transition::default();
        assert!(transition.retarget(1.0, 100, 160));
        assert_eq!(transition.value(100), 0.0);
        let middle = transition.value(180);
        assert!(middle > 0.0 && middle < 1.0);
        assert!(!transition.retarget(1.0, 180, 160));
        assert!(transition.retarget(0.0, 180, 110));
        assert_eq!(transition.value(180), middle);
        assert!(!transition.active(290));
        assert_eq!(transition.value(290), 0.0);
        transition.snap(1.0);
        assert!(!transition.active(291));
        assert_eq!(transition.value(291), 1.0);
    }
}

//! Spring integrators for the Dynamic Island morphs.
//!
//! The island morphs between presentations with a spring, not a timing
//! curve — interrupted morphs keep their velocity and settle naturally.

/// The 2D morph integrator: tracks pill width, height, and corner radius
/// in px and velocity (px/s). The radius rides the same spring so the
/// silhouette morphs continuously instead of snapping between the pill and
/// the expanded card.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Spring2D {
    /// Current animated width.
    pub(crate) x: f32,
    /// Current animated height.
    pub(crate) y: f32,
    /// Current animated corner radius.
    pub(crate) z: f32,
    /// Current width velocity in px/s.
    pub(crate) vx: f32,
    /// Current height velocity in px/s.
    pub(crate) vy: f32,
    /// Current radius velocity in px/s.
    pub(crate) vz: f32,
    /// Target width.
    pub(crate) target_x: f32,
    /// Target height.
    pub(crate) target_y: f32,
    /// Target corner radius.
    target_z: f32,
    /// Where this morph started, so interrupted morphs still report progress.
    start_x: f32,
    start_y: f32,
    start_z: f32,
    /// Params from `spring_params(animation_ms, spring_bounce)`.
    pub(crate) stiffness: f32,
    damping: f32,
    /// High-water mark for content sequencing. Physical geometry may overshoot,
    /// but opacity/offset must never rewind when a morph is interrupted.
    max_progress: f32,
}

/// Below these the spring is considered settled and snaps to target.
const SETTLE_PX: f32 = 0.5;
const SETTLE_V: f32 = 2.0;

/// Exact solution for the unit-mass damped oscillator. Unlike Euler steps,
/// this preserves the same trajectory at every display refresh rate.
fn integrate(x: &mut f32, v: &mut f32, target: f32, stiffness: f32, damping: f32, dt: f32) {
    if !dt.is_finite() || dt <= 0.0 {
        return;
    }
    let dt = f64::from(dt.min(0.2));
    let a = f64::from(damping) / 2.0;
    let k = f64::from(stiffness);
    let offset = f64::from(*x - target);
    let velocity = f64::from(*v);
    let discriminant = k - a * a;
    let (c, s) = if discriminant.abs() < k.max(1.0) * 1e-6 {
        let decay = (-a * dt).exp();
        (decay, decay * dt)
    } else if discriminant > 0.0 {
        let w = discriminant.sqrt();
        let decay = (-a * dt).exp();
        (decay * (w * dt).cos(), decay * (w * dt).sin() / w)
    } else {
        let w = (-discriminant).sqrt();
        let slow = ((-a + w) * dt).exp();
        let fast = ((-a - w) * dt).exp();
        ((slow + fast) / 2.0, (slow - fast) / (2.0 * w))
    };
    *x = (f64::from(target) + offset * c + (velocity + a * offset) * s) as f32;
    *v = (velocity * c - (a * velocity + k * offset) * s) as f32;
}

impl Spring2D {
    pub(crate) fn new(
        from_w: u32,
        from_h: u32,
        from_r: f32,
        target_w: u32,
        target_h: u32,
        target_r: f32,
        params: termielle_core::SpringParams,
    ) -> Self {
        Self {
            x: from_w as f32,
            y: from_h as f32,
            z: from_r,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            target_x: target_w as f32,
            target_y: target_h as f32,
            target_z: target_r,
            start_x: from_w as f32,
            start_y: from_h as f32,
            start_z: from_r,
            stiffness: params.stiffness,
            damping: params.damping,
            max_progress: 0.0,
        }
    }

    /// Morph progress toward the target, 0-1. `step` records the high-water
    /// mark before `progress` reads it, keeping this method shareable with
    /// immutable renderers.
    pub(crate) fn progress(&self) -> f32 {
        self.max_progress
    }

    pub(crate) fn target_radius(&self) -> f32 {
        self.target_z
    }

    pub(crate) fn preserve_position(&mut self, x: f32, y: f32) {
        self.x = x;
        self.y = y;
        self.start_x = x;
        self.start_y = y;
    }

    fn measured_progress(&self) -> f32 {
        let total = (self.start_x - self.target_x)
            .abs()
            .max((self.start_y - self.target_y).abs())
            .max((self.start_z - self.target_z).abs());
        if total < 1.0 {
            return 1.0;
        }
        let remaining = (self.x - self.target_x)
            .abs()
            .max((self.y - self.target_y).abs())
            .max((self.z - self.target_z).abs());
        (1.0 - remaining / total).clamp(0.0, 1.0)
    }

    /// Continues content sequencing when geometry is retargeted without
    /// discarding the current physical position or velocity.
    pub(crate) fn preserve_progress(&mut self, progress: f32) {
        self.max_progress = self.max_progress.max(progress.clamp(0.0, 1.0));
    }

    /// Integrates one step of `dt` seconds. Returns ((width, height, radius), settled).
    pub(crate) fn step(&mut self, dt: f32) -> ((u32, u32, u32), bool) {
        integrate(
            &mut self.x,
            &mut self.vx,
            self.target_x,
            self.stiffness,
            self.damping,
            dt,
        );
        integrate(
            &mut self.y,
            &mut self.vy,
            self.target_y,
            self.stiffness,
            self.damping,
            dt,
        );
        integrate(
            &mut self.z,
            &mut self.vz,
            self.target_z,
            self.stiffness,
            self.damping,
            dt,
        );
        self.max_progress = self.max_progress.max(self.measured_progress());

        let settled_x = (self.x - self.target_x).abs() < SETTLE_PX && self.vx.abs() < SETTLE_V;
        let settled_y = (self.y - self.target_y).abs() < SETTLE_PX && self.vy.abs() < SETTLE_V;
        let settled_z = (self.z - self.target_z).abs() < SETTLE_PX && self.vz.abs() < SETTLE_V;

        if settled_x {
            self.x = self.target_x;
            self.vx = 0.0;
        }
        if settled_y {
            self.y = self.target_y;
            self.vy = 0.0;
        }
        if settled_z {
            self.z = self.target_z;
            self.vz = 0.0;
        }

        let settled = settled_x && settled_y && settled_z;
        (
            (
                self.x.round().max(1.0) as u32,
                self.y.round().max(1.0) as u32,
                self.z.round().max(0.0) as u32,
            ),
            settled,
        )
    }
}

/// 1-D spring for the blob separation in pixels: 0 is the merged single
/// pill, [`BLOB_GAP_PX`] is the fully split island. It runs alongside the
/// size spring so the split and the stretch feel like one motion.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Spring1 {
    pub(crate) x: f32,
    pub(crate) v: f32,
    pub(crate) target: f32,
    stiffness: f32,
    damping: f32,
}

impl Spring1 {
    pub(crate) fn new(from: f32, target: f32, params: termielle_core::SpringParams) -> Self {
        Self {
            x: from,
            v: 0.0,
            target,
            stiffness: params.stiffness,
            damping: params.damping,
        }
    }

    pub(crate) fn step(&mut self, dt: f32) {
        integrate(
            &mut self.x,
            &mut self.v,
            self.target,
            self.stiffness,
            self.damping,
            dt,
        );
        if (self.x - self.target).abs() < SETTLE_PX && self.v.abs() < SETTLE_V {
            self.x = self.target;
            self.v = 0.0;
        }
    }

    pub(crate) fn settled(&self) -> bool {
        (self.x - self.target).abs() < SETTLE_PX && self.v.abs() < SETTLE_V
    }
}

/// Gap between the two blobs of a split island, in pixels — the Dynamic
/// Island keeps its two live activities close, joined earlier by the
/// liquid bridge.
pub(crate) const BLOB_GAP_PX: f32 = 9.0;

#[cfg(test)]
mod tests {
    use super::*;
    use termielle_core::spring_params;

    #[test]
    fn trajectory_is_independent_of_refresh_rate() {
        for bounce in [0.0, 0.18, 0.5] {
            let params = spring_params(350, bounce);
            let mut reference = Spring1::new(72.0, 320.0, params);
            reference.step(0.2);
            for hz in [60, 75, 120, 144, 165, 240] {
                let mut sampled = Spring1::new(72.0, 320.0, params);
                let dt = 1.0 / hz as f32;
                let steps = (0.2 / dt).floor() as usize;
                for _ in 0..steps {
                    sampled.step(dt);
                }
                sampled.step(0.2 - steps as f32 * dt);
                assert!(
                    (reference.x - sampled.x).abs() < 0.002,
                    "{hz} Hz, bounce {bounce}"
                );
                assert!((reference.v - sampled.v).abs() < 0.02, "{hz} Hz velocity");
            }
        }
    }

    #[test]
    fn pathological_deltas_cannot_explode_fast_springs() {
        let mut spring = Spring1::new(72.0, 320.0, spring_params(100, 0.5));
        for dt in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
            spring.step(dt);
            assert_eq!(spring.x, 72.0);
        }
        spring.step(60.0);
        assert!(spring.x.is_finite() && spring.v.is_finite());
        assert!((spring.x - 320.0).abs() < 2.0);
    }

    #[test]
    fn radius_rides_the_same_spring_to_its_target() {
        let mut spring = Spring2D::new(140, 36, 18.0, 320, 154, 28.0, spring_params(500, 0.2));
        for _ in 0..2000 {
            let (_, settled) = spring.step(1.0 / 120.0);
            if settled {
                break;
            }
        }
        assert_eq!(spring.x, 320.0);
        assert_eq!(spring.y, 154.0);
        assert_eq!(spring.z, 28.0, "corner radius must settle at its target");
    }

    #[test]
    fn progress_advances_monotonically_and_preserves_velocity_on_retarget() {
        let mut trajectory = Spring2D::new(140, 36, 18.0, 320, 36, 18.0, spring_params(350, 0.18));
        let mut last = 0.0;
        for _ in 0..2_000 {
            let (_, settled) = trajectory.step(1.0 / 240.0);
            let progress = trajectory.progress();
            assert!(progress >= last - 1e-6, "progress must not regress");
            last = progress;
            if settled {
                break;
            }
        }
        assert!(last > 0.99, "full trajectory must settle perceptually");

        let mut spring = Spring2D::new(140, 36, 18.0, 320, 36, 18.0, spring_params(350, 0.18));
        for _ in 0..20 {
            spring.step(1.0 / 60.0);
        }
        assert!(spring.vx != 0.0, "spring must be moving mid-flight");

        // Interrupted morph: the new spring inherits the velocity.
        let inherited = spring.vx;
        let mut retargeted = Spring2D::new(
            spring.x.round() as u32,
            36,
            18.0,
            200,
            36,
            18.0,
            spring_params(350, 0.18),
        );
        retargeted.vx = inherited;
        retargeted.preserve_progress(spring.progress());
        let dt = 1.0 / 60.0;
        retargeted.step(dt);
        assert!(retargeted.progress() >= spring.progress());
        let coasted = retargeted.x;
        let mut from_rest = Spring2D::new(
            spring.x.round() as u32,
            36,
            18.0,
            200,
            36,
            18.0,
            spring_params(350, 0.18),
        );
        from_rest.step(dt);
        assert!(
            (coasted - from_rest.x).abs() > 0.01,
            "inherited velocity must carry the morph forward"
        );
    }
}

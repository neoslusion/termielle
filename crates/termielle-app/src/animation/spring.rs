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
}

/// Below these the spring is considered settled and snaps to target.
const SETTLE_PX: f32 = 0.5;
const SETTLE_V: f32 = 2.0;

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
        }
    }

    /// Morph progress toward the target, 0-1, from the remaining
    /// displacement fraction. Content fade/slide derives from this.
    pub(crate) fn progress(&self) -> f32 {
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

    /// Integrates one step of `dt` seconds. Returns ((width, height, radius), settled).
    pub(crate) fn step(&mut self, dt: f32) -> ((u32, u32, u32), bool) {
        let accel_x = -self.stiffness * (self.x - self.target_x) - self.damping * self.vx;
        self.vx += accel_x * dt;
        self.x += self.vx * dt;

        let accel_y = -self.stiffness * (self.y - self.target_y) - self.damping * self.vy;
        self.vy += accel_y * dt;
        self.y += self.vy * dt;

        let accel_z = -self.stiffness * (self.z - self.target_z) - self.damping * self.vz;
        self.vz += accel_z * dt;
        self.z += self.vz * dt;

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
                self.z.round().max(1.0) as u32,
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
        let accel = -self.stiffness * (self.x - self.target) - self.damping * self.v;
        self.v += accel * dt;
        self.x += self.v * dt;
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
        let mut spring = Spring2D::new(140, 36, 18.0, 320, 36, 18.0, spring_params(350, 0.18));
        let mut last = 0.0;
        for _ in 0..20 {
            spring.step(1.0 / 60.0);
            let p = spring.progress();
            assert!(p >= last - 1e-6, "progress must not regress");
            last = p;
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
        let dt = 1.0 / 60.0;
        retargeted.step(dt);
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

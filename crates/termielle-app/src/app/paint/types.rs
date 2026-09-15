//! Content-painter shared inputs: one expanded-card section paint pass.

/// Shared inputs for one expanded-card section paint pass: frame geometry
/// plus the precomputed clock/accent values, so six section painters do
/// not each re-query the registry or recompute the pose clock.
pub(crate) struct CardPaintCtx {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) cy: i32,
    pub(crate) accent: [u8; 4],
    pub(crate) now: u64,
    pub(crate) eq_now: u64,
    pub(crate) pad: i32,
}

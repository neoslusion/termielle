//! Frosted-glass layer cache: geometry/material/blob-layout keyed, theme changes clear it.

use super::super::controller::Controller;
use super::super::types::GlassCacheKey;
use crate::animation::FrameBuffer;
use crate::animation::notch::BRIDGE_K_MAX;
use crate::animation::spring::BLOB_GAP_PX;

impl Controller {
    /// How many frosted-glass layers stay cached at once. A bar frame paints
    /// the strip and the center pill in the same pass, so one slot would make
    /// them evict each other and repaint both every frame.
    const GLASS_CACHE_SLOTS: usize = 4;

    /// The cache key for one glass layer: its geometry, its material, and the
    /// blob layout it was authored for.
    fn glass_cache_key(
        &self,
        w: u32,
        h: u32,
        r: u32,
        attached: bool,
        blobs: &[crate::animation::notch::BlobRect],
        black: bool,
    ) -> GlassCacheKey {
        (
            w,
            h,
            r,
            attached,
            black,
            blobs.len() as u32,
            blobs.get(1).map_or(0, |b| b.x),
            blobs.get(1).map_or(0, |b| b.w),
            self.render_scale().to_bits(),
            blobs.first().map_or(0, |blob| blob.w),
            self.separation_now().to_bits(),
        )
    }

    /// Renders the frosted-glass material into `out`.
    ///
    /// The cache holds the pure material. A hit copies it into `out` instead
    /// of handing back a clone of its own, so the caller can composite
    /// content over the result without taking a fresh allocation on every
    /// frame of a morph.
    #[allow(clippy::too_many_arguments)] // material renderer: every input is a distinct material or geometry term.
    pub(crate) fn glass_layer_blobs_into(
        &mut self,
        out: &mut FrameBuffer,
        w: u32,
        h: u32,
        r: u32,
        attached: bool,
        blobs: &[crate::animation::notch::BlobRect],
        black: bool,
    ) {
        let key = self.glass_cache_key(w, h, r, attached, blobs, black);
        if let Some(index) = self
            .glass_caches
            .iter()
            .position(|(cached, _)| *cached == key)
        {
            let (_, buf) = &self.glass_caches[index];
            out.width = buf.width;
            out.height = buf.height;
            out.scale = buf.scale;
            out.delay_ms = 0;
            out.loop_index = 0;
            out.pixels_pbgra.clear();
            out.pixels_pbgra.extend_from_slice(&buf.pixels_pbgra);
            return;
        }
        // The liquid bridge between blobs thins and snaps as the
        // separation extends.
        let t = self.separation_now().clamp(0.0, BLOB_GAP_PX) / BLOB_GAP_PX;
        let bridge_k = BRIDGE_K_MAX * (1.0 - t);
        crate::animation::notch::glass_layer_blobs_into(
            out,
            w,
            h,
            blobs,
            &self.island.glass,
            black,
            bridge_k,
            self.render_scale(),
        );
        // Bounded so a run of distinct geometries cannot grow the cache
        // without limit. Four covers every live layer a frame paints: the
        // bar strip, the center pill, and the expanded card.
        if self.glass_caches.len() >= Self::GLASS_CACHE_SLOTS {
            self.glass_caches.remove(0);
        }
        // The cache keeps its own copy: the caller composites content over
        // `out`, so the pristine material has to survive.
        self.glass_caches.push((key, out.clone()));
    }

    /// Renders the frosted-glass material into a buffer of its own.
    ///
    /// Prefer [`Self::glass_layer_blobs_into`] on a hot path: this one
    /// allocates, because the material has to outlive the call.
    pub(crate) fn glass_layer_blobs(
        &mut self,
        w: u32,
        h: u32,
        r: u32,
        attached: bool,
        blobs: &[crate::animation::notch::BlobRect],
        black: bool,
    ) -> FrameBuffer {
        let mut frame = FrameBuffer::default();
        self.glass_layer_blobs_into(&mut frame, w, h, r, attached, blobs, black);
        frame
    }
}

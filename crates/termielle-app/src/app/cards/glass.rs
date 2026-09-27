//! Frosted-glass layer cache: geometry/material/blob-layout keyed, theme changes clear it.

use super::super::controller::Controller;
use crate::animation::FrameBuffer;
use crate::animation::notch::BRIDGE_K_MAX;
use crate::animation::spring::BLOB_GAP_PX;

impl Controller {
    /// Theme/material changes clear it via [`Controller::set_island_config`].
    pub(crate) fn glass_layer_blobs(
        &mut self,
        w: u32,
        h: u32,
        r: u32,
        attached: bool,
        blobs: &[crate::animation::notch::BlobRect],
        black: bool,
    ) -> FrameBuffer {
        let blob_count = blobs.len() as u32;
        let right_x = blobs.get(1).map_or(0, |b| b.x);
        let right_w = blobs.get(1).map_or(0, |b| b.w);
        let key = (
            w,
            h,
            r,
            attached,
            black,
            blob_count,
            right_x,
            right_w,
            self.render_scale().to_bits(),
            blobs.first().map_or(0, |blob| blob.w),
            self.separation_now().to_bits(),
        );
        if let Some((cached_key, buf)) = &self.glass_cache {
            if *cached_key == key {
                return buf.clone();
            }
        }
        // The liquid bridge between blobs thins and snaps as the
        // separation extends.
        let t = self.separation_now().clamp(0.0, BLOB_GAP_PX) / BLOB_GAP_PX;
        let bridge_k = BRIDGE_K_MAX * (1.0 - t);
        let buf = crate::animation::notch::glass_layer_blobs(
            w,
            h,
            blobs,
            &self.island.glass,
            black,
            bridge_k,
            self.render_scale(),
        );
        self.glass_cache = Some((key, buf.clone()));
        buf
    }
}

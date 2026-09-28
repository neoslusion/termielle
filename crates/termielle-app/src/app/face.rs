//! Termielle face cache: pre-decoded GIF frames plus the downscale helper.

use super::controller::Controller;
use super::types::FACE_SIZE;
use crate::animation::{FrameBuffer, GifAnimation, fallback_frame};
use termielle_core::VisualState;

impl Controller {
    /// Reloads the termielle face for `state`: decodes the first animation
    /// frame synchronously (fast startup — the pipe must be listening within
    /// milliseconds) and keeps the streaming decoder so the remaining loop
    /// frames fill in one per face tick (see `advance_face`). Falls back to
    /// the procedural glyph when the asset is missing or undecodable.
    pub(crate) fn refresh_face(&mut self, state: VisualState) {
        self.face_frames.clear();
        self.face_delays.clear();
        self.face_idx = 0;
        self.face_deadline = None;
        self.face_decoder = None;
        self.face_frame = fallback_frame(state, FACE_SIZE, 1.0);
        if !self.island.has_widget("face") {
            return;
        }
        let Some(path) = self.assets.resolve(state) else {
            return;
        };
        let Ok(mut gif) = GifAnimation::open(&path) else {
            return;
        };
        let Ok(first) = gif.next_frame() else {
            return;
        };
        if first.loop_index > 0 {
            return;
        }
        self.face_delays.push(first.delay_ms.clamp(20, 1000));
        self.face_frames.push(downscale_frame(first, FACE_SIZE));
        self.face_frame = self.face_frames[0].clone();
        // Animated faces keep streaming in the background of face ticks;
        // stills drop the decoder immediately.
        if self.island.face_animated && !self.reduced_motion {
            self.face_decoder = Some(gif);
        }
    }

    /// Idle faces remain alive but at a restrained 10 fps; active/playing
    /// surfaces retain the asset cadence because another animation already
    /// keeps the compositor active there.
    ///
    /// The bar is the exception even when idle. Its tick repaints only the
    /// center surface off the cached side zones, and holding a resting bar
    /// face to 10 fps froze it on the GIF's opening frame, which is nearly
    /// blank - it read as a missing face rather than a still one.
    fn face_min_delay_ms(&self) -> u64 {
        let resting = self.state == termielle_core::VisualState::Idle && !self.media_playing();
        if resting && !self.island.is_bar() {
            100
        } else {
            20
        }
    }

    /// Arms the next face-animation tick when the face can advance: island
    /// mode, animation allowed, and either frames are filling or 2+ are cached.
    pub(crate) fn arm_face_deadline(&mut self, now_ms: u64) {
        self.face_deadline = None;
        if !self.island.is_enabled() || self.reduced_motion || !self.island.face_animated {
            return;
        }
        if self.presentation() == crate::animation::notch::Presentation::Hidden {
            return;
        }
        // A bar face tick is a cached-zone repaint of the center surface, not
        // a cold rebuild, so it can animate even while the bar rests.
        if self.face_decoder.is_none() && self.face_frames.len() < 2 {
            return;
        }
        self.face_deadline = Some(self.face_deadline_for(now_ms));
    }

    /// When the user configured a `frame_rate`, that cadence wins over the
    /// asset's own delay. This is what the option has always claimed to do,
    /// but only the classic surface read it: `advance_animation` returns
    /// early once the island is enabled, so island and bar faces kept the
    /// GIF's timing no matter what was set. Subtracting the smoothed present
    /// cost keeps a 60 Hz face on a 60 Hz clock instead of drifting late.
    fn face_deadline_for(&self, now_ms: u64) -> u64 {
        if self.frame_interval_ms.is_some() {
            return super::controller::deadline_for(
                self.frame_interval_ms,
                self.present_cost_ms,
                now_ms,
                0,
            );
        }
        let delay = self
            .face_delays
            .get(self.face_idx)
            .copied()
            .unwrap_or(40)
            .max(self.face_min_delay_ms() as u32);
        now_ms.saturating_add(u64::from(delay))
    }

    /// Advances the face animation one step and re-renders the current pill:
    /// decodes one more loop frame while filling, then cycles the cache.
    pub(crate) fn advance_face(&mut self, now_ms: u64) {
        if self.presentation() == crate::animation::notch::Presentation::Hidden {
            // Invisible top-edge sensor: keep the tick cadence (and the
            // background loop fill above), skip the pixels nobody sees.
            self.arm_face_deadline(now_ms);
            return;
        }
        if let Some(gif) = self.face_decoder.as_mut() {
            let done = match gif.next_frame() {
                Ok(frame) if frame.loop_index == 0 && self.face_frames.len() < 120 => {
                    self.face_delays.push(frame.delay_ms.clamp(20, 1000));
                    self.face_frames.push(downscale_frame(frame, FACE_SIZE));
                    false
                }
                _ => true,
            };
            if done {
                self.face_decoder = None;
            }
        }
        if self.face_frames.len() < 2 {
            self.arm_face_deadline(now_ms);
            return;
        }
        self.face_idx = (self.face_idx + 1) % self.face_frames.len();
        // `face_frame` is what the bar pill and the short card actually blit,
        // so advancing the index alone left both showing the GIF's first
        // frame forever: the animation ticked and nothing moved.
        self.face_frame = self.face_frames[self.face_idx].clone();
        self.face_deadline = Some(self.face_deadline_for(now_ms));
        let (w, h) = self.current_logical_size();
        // A bar face tick redraws only the center surface. The side zones are
        // unchanged by a face frame, so taking the cached-zone path here is
        // what keeps a resting bar animating without a full strip rebuild
        // every tick.
        self.current = if self.island.is_bar() {
            self.render_bar_with_damage(
                self.state,
                w,
                h,
                now_ms,
                crate::app::bar::modules::BarDamage::CENTER,
            )
        } else {
            self.render_island(self.state, w, h, now_ms)
        };
    }
}
/// Box-downscales a premultiplied BGRA frame to `size x size`.
fn downscale_frame(src: &FrameBuffer, size: u32) -> FrameBuffer {
    let mut out = vec![0u8; (size * size * 4) as usize];
    if src.width == 0 || src.height == 0 {
        return FrameBuffer {
            width: size,
            height: size,
            pixels_pbgra: out,
            delay_ms: 0,
            loop_index: 0,
            scale: 1.0,
        };
    }
    for ty in 0..size {
        let sy0 = ty as u64 * src.height as u64 / size as u64;
        let sy1 = ((ty + 1) as u64 * src.height as u64 / size as u64).max(sy0 + 1);
        for tx in 0..size {
            let sx0 = tx as u64 * src.width as u64 / size as u64;
            let sx1 = ((tx + 1) as u64 * src.width as u64 / size as u64).max(sx0 + 1);
            let mut acc = [0u64; 4];
            let mut count = 0u64;
            for sy in sy0..sy1.min(src.height as u64) {
                for sx in sx0..sx1.min(src.width as u64) {
                    let i = ((sy * u64::from(src.width) + sx) * 4) as usize;
                    for (c, slot) in acc.iter_mut().enumerate() {
                        *slot += src.pixels_pbgra[i + c] as u64;
                    }
                    count += 1;
                }
            }
            let o = ((ty * size + tx) * 4) as usize;
            let count = count.max(1);
            for (c, slot) in out[o..o + 4].iter_mut().enumerate() {
                *slot = (acc[c] / count) as u8;
            }
        }
    }
    FrameBuffer {
        width: size,
        height: size,
        pixels_pbgra: out,
        delay_ms: 0,
        loop_index: 0,
        scale: 1.0,
    }
}

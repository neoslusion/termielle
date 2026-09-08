//! GIF streaming and procedural fallback rendering.
//!
//! [`FrameBuffer`] is the shared composited-canvas type: [`GifAnimation`]
//! streams frames into one reused canvas, and [`fallback_frame`] synthesizes a
//! single still frame for any visual state so the overlay works even when no
//! verified artwork is installed.

mod fallback;
mod gif;
pub mod notch;

pub use fallback::fallback_frame;
pub use gif::{AnimationError, AnimationSource, FrameBuffer, GifAnimation};
pub use notch::{
    BlobRect, NotchContent, Presentation, blend_frame_over, blit_rounded, blit_scaled,
    draw_accent_strip, draw_button_circle, draw_disc, draw_glyph_next, draw_glyph_pause,
    draw_glyph_play, draw_glyph_prev, draw_text, fill_rect_pub, glass_layer, glass_layer_blobs,
    island_frame, smoothstep,
};

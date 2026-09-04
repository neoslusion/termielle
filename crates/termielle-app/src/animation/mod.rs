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
    NotchContent, Presentation, blit_icon, blit_scaled, draw_accent_strip, draw_disc,
    fill_rect_pub, glass_layer, island_frame,
};

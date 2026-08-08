//! GIF streaming and procedural fallback rendering.
//!
//! [`FrameBuffer`] is the shared composited-canvas type: [`GifAnimation`]
//! streams frames into one reused canvas, and [`fallback_frame`] synthesizes a
//! single still frame for any visual state so the overlay works even when no
//! verified artwork is installed.

mod fallback;
mod gif;

pub use fallback::fallback_frame;
pub use gif::{AnimationError, AnimationSource, FrameBuffer, GifAnimation};

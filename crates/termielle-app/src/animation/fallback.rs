//! Procedural still frames for every visual state.
//!
//! Until artwork provenance is confirmed (see `THIRD_PARTY_NOTICES.md`), the
//! overlay renders these generated frames: transparent corners, a rounded body,
//! and a per-state ring, fill, and glyph so the states stay distinguishable at
//! a glance. Rendering is plain arithmetic over a BGRA buffer — no assets, no
//! disk reads.

use termielle_core::VisualState;

use crate::animation::FrameBuffer;

/// The two body colors for a state: the outer ring and the inner fill, as
/// BGRA bytes (blue, green, red, alpha).
fn body_colors(state: VisualState) -> ([u8; 4], [u8; 4]) {
    match state {
        // Waiting for a session: muted steel blue.
        VisualState::Idle => ([140, 123, 107, 255], [176, 163, 143, 255]),
        // Private reasoning: amber.
        VisualState::Thinking => ([61, 163, 232, 255], [107, 201, 242, 255]),
        // Answer generation: green.
        VisualState::Working => ([106, 158, 61, 255], [141, 192, 108, 255]),
        // Waiting for the human: blue with a marker row.
        VisualState::NeedsInput => ([212, 127, 74, 255], [224, 163, 123, 255]),
        // Turn finished: teal.
        VisualState::Ready => ([160, 168, 47, 255], [194, 201, 92, 255]),
        // Turn failed: a red body with a dark red ring.
        VisualState::Failed => ([0, 0, 138, 255], [0, 0, 255, 255]),
    }
}

/// Builds the procedural still frame for one state.
pub fn fallback_frame(state: VisualState, size: u32) -> FrameBuffer {
    let mut frame = FrameBuffer {
        width: size,
        height: size,
        pixels_pbgra: vec![0u8; (size * size * 4) as usize],
        delay_ms: 0,
        loop_index: 0,
    };

    let (ring, fill) = body_colors(state);
    let margin = size / 8;
    let thickness = size / 8;

    // Rounded body: a ring around a fill, with transparent corners outside the
    // margin, so the pet's corners never block clicks into the application.
    for y in margin..size - margin {
        for x in margin..size - margin {
            let on_ring = x - margin < thickness
                || size - margin - x <= thickness
                || y - margin < thickness
                || size - margin - y <= thickness;
            let color = if on_ring { ring } else { fill };
            set_pixel(&mut frame, x as i32, y as i32, color);
        }
    }

    draw_glyph(&mut frame, state, size);

    frame
}

/// A small per-state marker inside the body so states stay distinct even for
/// viewers who cannot tell the palette apart.
fn draw_glyph(frame: &mut FrameBuffer, state: VisualState, size: u32) {
    let center = (size / 2) as i32;
    let inner = (size / 4) as i32;
    match state {
        // A single centered dot: one idle session is open.
        VisualState::Idle => fill_rect(frame, center - 1, center - 1, 3, 3, [255, 255, 255, 255]),
        // A hollow circle: private reasoning.
        VisualState::Thinking => {
            draw_circle(frame, center, center, inner as u32, [255, 255, 255, 255])
        }
        // A solid square: generation in progress.
        VisualState::Working => fill_rect(
            frame,
            center - inner / 2,
            center - inner / 2,
            inner as u32,
            inner as u32,
            [255, 255, 255, 255],
        ),
        // Three dots: the agent is asking the human.
        VisualState::NeedsInput => {
            let radius = (size / 24) as i32;
            for offset in [-(inner / 2), 0, inner / 2] {
                fill_rect(
                    frame,
                    center + offset - radius,
                    center - radius,
                    (radius * 2 + 1) as u32,
                    (radius * 2 + 1) as u32,
                    [255, 255, 255, 255],
                );
            }
        }
        // A ring with a dot: answer is ready to read.
        VisualState::Ready => {
            draw_circle(frame, center, center, inner as u32, [255, 255, 255, 255]);
            fill_rect(frame, center - 1, center - 1, 3, 3, [255, 255, 255, 255]);
        }
        // A diagonal cross: the turn failed.
        VisualState::Failed => {
            let half = inner / 2;
            for step in -half..=half {
                let depth = if step.abs() <= 1 { 3 } else { 1 };
                fill_rect(
                    frame,
                    center + step,
                    center - step - 1,
                    depth,
                    depth + 2,
                    [255, 255, 255, 255],
                );
                fill_rect(
                    frame,
                    center - step - 1,
                    center - step - 1,
                    depth,
                    depth + 2,
                    [255, 255, 255, 255],
                );
            }
        }
    }
}

/// Draws a square annulus approximating a circle at `(center_x, center_y)`.
fn draw_circle(frame: &mut FrameBuffer, center_x: i32, center_y: i32, radius: u32, color: [u8; 4]) {
    let radius = radius as i32;
    for y in -radius..=radius {
        for x in -radius..=radius {
            let distance_sq = x * x + y * y;
            let inner_sq = (radius - 1) * (radius - 1);
            if distance_sq <= radius * radius && distance_sq >= inner_sq {
                set_pixel(frame, center_x + x, center_y + y, color);
            }
        }
    }
}

/// Draws a filled rectangle, clamped to the canvas.
fn fill_rect(
    frame: &mut FrameBuffer,
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    color: [u8; 4],
) {
    let left = left.clamp(0, frame.width as i32 - 1);
    let top = top.clamp(0, frame.height as i32 - 1);
    let right = (left + width as i32).min(frame.width as i32);
    let bottom = (top + height as i32).min(frame.height as i32);
    for y in top..bottom {
        for x in left..right {
            set_pixel(frame, x, y, color);
        }
    }
}

/// Writes one BGRA pixel, ignoring out-of-bounds coordinates.
fn set_pixel(frame: &mut FrameBuffer, x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x >= frame.width as i32 || y >= frame.height as i32 {
        return;
    }
    let index = ((y as u32 * frame.width + x as u32) * 4) as usize;
    frame.pixels_pbgra[index..index + 4].copy_from_slice(&color);
}

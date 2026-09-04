//! Notch / island procedural renderer — custom layered glass.
//!
//! Produces a `FrameBuffer` for the top-center notch or floating island.
//! The glass material is baked into premultiplied BGRA so `UpdateLayeredWindow`
//! composites it with per-pixel alpha — no DWM acrylic dependency.
//!
//! Geometry is logical pixels before `scale`; `window.rs` scales the buffer
//! with nearest-neighbor if needed.

use termielle_core::{GlassConfig, IslandGeometry, VisualState};

use crate::animation::FrameBuffer;

/// Per-state accent colors (BGRA) drawn as a subtle centered glyph so the
/// pill is distinguishable without reading text. Same palette as the legacy
/// fallback but dimmed for glass.
pub fn accent_colors(state: VisualState) -> ([u8; 4], [u8; 4]) {
    match state {
        VisualState::Idle => ([140, 123, 107, 220], [176, 163, 143, 180]),
        VisualState::Thinking => ([61, 163, 232, 220], [107, 201, 242, 180]),
        VisualState::Working => ([106, 158, 61, 220], [141, 192, 108, 180]),
        VisualState::NeedsInput => ([212, 127, 74, 240], [224, 163, 123, 190]),
        VisualState::Ready => ([160, 168, 47, 220], [194, 201, 92, 180]),
        VisualState::Failed => ([0, 0, 138, 220], [0, 0, 255, 190]),
    }
}

/// Builds one glass notch/island frame.
///
/// `progress` is 0.0 (collapsed) to 1.0 (expanded) — caller interpolates
/// `geometry` already, so `progress` only drives inner content (glyph scale).
/// The pill is purely visual: no text is ever rendered.
pub fn island_frame(
    state: VisualState,
    geom: IslandGeometry,
    glass: &GlassConfig,
    progress: f32,
) -> FrameBuffer {
    let width = geom.width.max(1);
    let height = geom.height.max(1);
    let radius = geom.radius.min(height / 2);
    let mut frame = glass_layer(width, height, radius, geom.attached, glass);

    // State glyph, centered, scales slightly with progress.
    draw_island_glyph(&mut frame, state, width, height, progress);

    frame
}

/// Accent color adapted to the glass luminance: light on dark glass, dark
/// on light glass. Feeds the session dots and accent strip.
pub fn text_color_for(glass: &GlassConfig) -> [u8; 4] {
    let lum = (0.299 * f64::from(glass.tint[2])
        + 0.587 * f64::from(glass.tint[1])
        + 0.114 * f64::from(glass.tint[0])) as u8;
    if lum > 140 {
        [0, 0, 0, 230]
    } else {
        [255, 255, 255, 230]
    }
}

/// Which iOS presentation the pill is currently in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Presentation {
    /// Tiny resting dot: nothing live (iOS "minimal").
    Minimal,
    /// Resting pill while an agent session or media is live (iOS "compact").
    Compact,
    /// Hovered or pinned dashboard (iOS "expanded").
    Expanded,
}

/// Content shown inside the notch: the termielle face, one app icon per
/// running task, session dots, and the media accent strip. Purely visual —
/// no text anywhere. Composed by the controller from cached layers via the
/// public helpers below.
pub struct NotchContent<'a> {
    /// The termielle character face (premultiplied BGRA frame), optional.
    pub face: Option<&'a FrameBuffer>,
    /// Running-task app icons, premultiplied BGRA, drawn left-to-right
    /// after the face.
    pub icons: &'a [crate::tasks::TaskIcon],
    /// True while media plays: paints the accent strip teal when idle.
    pub media_playing: bool,
    /// Live agent sessions, drawn as accent dots (capped by the caller).
    pub session_dots: usize,
    /// The current presentation. `Minimal` hides icons and dots (iOS
    /// "minimal" shows only the highest-priority activity indicator).
    pub presentation: Presentation,
}

/// Pixel size of one app icon in the dashboard row.
pub const ICON_PX: u32 = 24;

/// Filled disc for session dots.
pub fn draw_disc(frame: &mut FrameBuffer, cx: i32, cy: i32, radius: u32, color: [u8; 4]) {
    let r = radius as i32;
    for y in -r..=r {
        for x in -r..=r {
            if x * x + y * y <= r * r {
                blend_pixel(frame, (cx + x) as u32, (cy + y) as u32, color);
            }
        }
    }
}

/// Renders the frosted-glass material (shadow, body, border, highlight) in
/// ONE pass: the rounded-rect signed distance is evaluated once per pixel
/// and all layers are composed in registers. ~1ms for a 720x56 pill, and
/// the result is cached by the controller per geometry.
pub fn glass_layer(
    width: u32,
    height: u32,
    radius: u32,
    attached: bool,
    glass: &GlassConfig,
) -> FrameBuffer {
    let radius = radius.min(height / 2);
    let mut frame = FrameBuffer {
        width,
        height,
        pixels_pbgra: vec![0u8; (width * height * 4) as usize],
        delay_ms: 0,
        loop_index: 0,
    };
    let tint = glass.tint;
    // Border color: tint lightened toward white.
    let mix: u32 = 80;
    let bb = tint[0].saturating_add(((255 - tint[0] as u32) * mix / 255) as u8);
    let bg = tint[1].saturating_add(((255 - tint[1] as u32) * mix / 255) as u8);
    let br = tint[2].saturating_add(((255 - tint[2] as u32) * mix / 255) as u8);

    for y in 0..height {
        let fy = y as f32 + 0.5;
        for x in 0..width {
            let fx = x as f32 + 0.5;
            let sd = signed_distance_rounded_rect(
                fx,
                fy,
                width as f32,
                height as f32,
                radius as f32,
                attached,
            );
            let cov = ((0.5 - sd).clamp(0.0, 1.0) * 255.0) as u8;
            if cov == 0 {
                continue;
            }
            let cov_u = cov as u32;

            // Layer stack, composed in registers with premultiplied-over.
            let mut acc = [0u8; 4];

            // 1. Shadow: the same shape shifted 2px down, visible only
            //    outside the body (alpha composed below under the tint).
            if glass.shadow_alpha > 0 {
                let sd_sh = signed_distance_rounded_rect(
                    fx,
                    fy - 2.0,
                    width as f32,
                    height as f32,
                    radius as f32,
                    attached,
                );
                let cov_sh = ((0.5 - sd_sh).clamp(0.0, 1.0) * 255.0) as u32;
                // Only where the body does not already cover.
                let sa = u32::from(glass.shadow_alpha) * cov_sh / 255 * (255 - cov_u) / 255;
                if sa > 0 {
                    // Opaque black shadow.
                    acc[3] = sa as u8;
                }
            }

            // 2. Body: premultiplied tint.
            let ba = tint[3] as u32 * cov_u / 255;
            let body = [
                (tint[0] as u32 * ba / 255) as u8,
                (tint[1] as u32 * ba / 255) as u8,
                (tint[2] as u32 * ba / 255) as u8,
                ba as u8,
            ];
            let ia = 255 - body[3] as u32;
            acc[0] = body[0] + (acc[0] as u32 * ia / 255) as u8;
            acc[1] = body[1] + (acc[1] as u32 * ia / 255) as u8;
            acc[2] = body[2] + (acc[2] as u32 * ia / 255) as u8;
            acc[3] = (body[3] as u32 + acc[3] as u32 * ia / 255) as u8;

            // 3. Border: 1px inner stroke.
            if glass.border_alpha > 0 && -sd < 1.2 {
                let a = (glass.border_alpha as u32 * cov_u / 255 / 2) as u8;
                let p = [
                    (bb as u32 * a as u32 / 255) as u8,
                    (bg as u32 * a as u32 / 255) as u8,
                    (br as u32 * a as u32 / 255) as u8,
                    a,
                ];
                let ia = 255 - p[3] as u32;
                acc[0] = p[0] + (acc[0] as u32 * ia / 255) as u8;
                acc[1] = p[1] + (acc[1] as u32 * ia / 255) as u8;
                acc[2] = p[2] + (acc[2] as u32 * ia / 255) as u8;
                acc[3] = (p[3] as u32 + acc[3] as u32 * ia / 255) as u8;
            }

            // 4. Highlight: top 1px inner edge.
            if glass.highlight_alpha > 0 && y < 3 && -sd < 1.5 {
                let a = (glass.highlight_alpha as u32 * if y == 0 { 1 } else { 0 } * cov_u
                    / 255
                    / 3) as u8;
                if a > 0 {
                    let p = [a, a, a, a];
                    let ia = 255 - a as u32;
                    acc[0] = p[0] + (acc[0] as u32 * ia / 255) as u8;
                    acc[1] = p[1] + (acc[1] as u32 * ia / 255) as u8;
                    acc[2] = p[2] + (acc[2] as u32 * ia / 255) as u8;
                    acc[3] = (p[3] as u32 + acc[3] as u32 * ia / 255) as u8;
                }
            }

            let idx = ((y * width + x) * 4) as usize;
            frame.pixels_pbgra[idx..idx + 4].copy_from_slice(&acc);
        }
    }
    frame
}

/// Blits a premultiplied BGRA frame (the termielle face) scaled to
/// (tw, th) at (x, y), nearest-sampled.
pub fn blit_scaled(frame: &mut FrameBuffer, src: &FrameBuffer, x: i32, y: i32, tw: u32, th: u32) {
    if src.width == 0 || src.height == 0 || tw == 0 || th == 0 {
        return;
    }
    for ty in 0..th as i32 {
        for tx in 0..tw as i32 {
            let sx = (tx as u64 * src.width as u64 / tw as u64) as u32;
            let sy = (ty as u64 * src.height as u64 / th as u64) as u32;
            let si = ((sy * src.width + sx) * 4) as usize;
            let a = src.pixels_pbgra[si + 3];
            if a == 0 {
                continue;
            }
            blend_pixel(
                frame,
                (x + tx) as u32,
                (y + ty) as u32,
                [
                    src.pixels_pbgra[si],
                    src.pixels_pbgra[si + 1],
                    src.pixels_pbgra[si + 2],
                    a,
                ],
            );
        }
    }
}

/// Blits one premultiplied-BGRA app icon, clipped to small rounded corners.
/// `hovered` draws the Windows-style accent border for the interactive cue.
pub fn blit_icon(
    frame: &mut FrameBuffer,
    icon: &crate::tasks::TaskIcon,
    x: i32,
    y: i32,
    hovered: bool,
    accent: [u8; 4],
) {
    if icon.width == 0 || icon.height == 0 {
        return;
    }
    let clip_radius = 5.min(icon.width / 2).min(icon.height / 2);
    // Hover highlight: a 1px accent ring just outside the icon.
    if hovered {
        let r = clip_radius + 1;
        for ty in -1..=icon.height as i32 {
            for tx in -1..=icon.width as i32 {
                let cov =
                    rounded_rect_coverage_aa(tx, ty, icon.width + 2, icon.height + 2, r, false);
                if cov == 0 {
                    continue;
                }
                let inner =
                    rounded_rect_coverage_aa(tx, ty, icon.width, icon.height, clip_radius, false);
                let a = cov.saturating_sub(inner);
                if a == 0 {
                    continue;
                }
                blend_pixel(
                    frame,
                    (x + tx) as u32,
                    (y + ty) as u32,
                    [accent[0], accent[1], accent[2], a],
                );
            }
        }
    }
    for ty in 0..icon.height as i32 {
        for tx in 0..icon.width as i32 {
            let cov = rounded_rect_coverage_aa(tx, ty, icon.width, icon.height, clip_radius, false);
            if cov == 0 {
                continue;
            }
            let si = ((ty as u32 * icon.width + tx as u32) * 4) as usize;
            let (b, g, r, a) = (
                icon.pixels_pbgra[si],
                icon.pixels_pbgra[si + 1],
                icon.pixels_pbgra[si + 2],
                icon.pixels_pbgra[si + 3],
            );
            if a == 0 {
                continue;
            }
            // Fold the rounded-clip coverage into the icon alpha.
            let a = ((a as u32 * cov as u32) / 255) as u8;
            if a == 0 {
                continue;
            }
            let b = ((b as u32 * cov as u32) / 255) as u8;
            let g = ((g as u32 * cov as u32) / 255) as u8;
            let r = ((r as u32 * cov as u32) / 255) as u8;
            blend_pixel(frame, (x + tx) as u32, (y + ty) as u32, [b, g, r, a]);
        }
    }
}

/// Draws the 2px state accent strip along the pill's top edge (flush for
/// attached notches, inset for floating islands).
pub fn draw_accent_strip(frame: &mut FrameBuffer, attached: bool, radius: u32, color: [u8; 4]) {
    let (width, height) = (frame.width, frame.height);
    let y0 = if attached { 0 } else { 2 };
    for y in y0..(y0 + 2).min(height as i32) {
        for x in 8..(width as i32 - 8) {
            let cov = rounded_rect_coverage_aa(x, y, frame.width, frame.height, radius, attached);
            if cov == 0 {
                continue;
            }
            let a = ((color[3] as u32) * cov as u32 / 255) as u8;
            if a == 0 {
                continue;
            }
            let b = ((color[0] as u32 * a as u32) / 255) as u8;
            let g = ((color[1] as u32 * a as u32) / 255) as u8;
            let r = ((color[2] as u32 * a as u32) / 255) as u8;
            blend_pixel(frame, x as u32, y as u32, [b, g, r, a]);
        }
    }
}

fn draw_island_glyph(
    frame: &mut FrameBuffer,
    state: VisualState,
    width: u32,
    height: u32,
    progress: f32,
) {
    let (_ring, fill) = accent_colors(state);
    let cx = (width / 2) as i32;
    let cy = (height / 2) as i32;
    let base = (height.min(width) / 3) as i32;
    // Scale with progress so expand feels alive
    let scale = 1.0 + progress.clamp(0.0, 1.0) * 0.15;
    let inner = (base as f32 * scale).round() as i32;

    match state {
        VisualState::Idle => {
            // Single dot
            fill_rect(frame, cx - 2, cy - 2, 4, 4, [255, 255, 255, 220]);
        }
        VisualState::Thinking => {
            draw_circle(frame, cx, cy, inner as u32, [255, 255, 255, 200]);
        }
        VisualState::Working => {
            let s = inner;
            fill_rect(
                frame,
                cx - s / 2,
                cy - s / 2,
                s as u32,
                s as u32,
                [255, 255, 255, 220],
            );
        }
        VisualState::NeedsInput => {
            let r = (height / 8) as i32;
            let gap = inner / 2;
            for off in [-gap, 0, gap] {
                fill_rect(
                    frame,
                    cx + off - r,
                    cy - r,
                    (r * 2 + 1) as u32,
                    (r * 2 + 1) as u32,
                    [255, 255, 255, 240],
                );
            }
        }
        VisualState::Ready => {
            draw_circle(frame, cx, cy, inner as u32, [255, 255, 255, 200]);
            fill_rect(frame, cx - 1, cy - 1, 3, 3, [255, 255, 255, 255]);
        }
        VisualState::Failed => {
            let half = inner / 2;
            for s in -half..=half {
                let d: u32 = if s.abs() <= 1 { 3 } else { 1 };
                fill_rect(frame, cx + s, cy - s - 1, d, d + 1, [255, 255, 255, 255]);
                fill_rect(
                    frame,
                    cx - s - 1,
                    cy - s - 1,
                    d,
                    d + 1,
                    [255, 255, 255, 255],
                );
            }
        }
    }
    // Accent fill behind glyph for richer glass
    let _ = fill;
}

// ---- geometry helpers -----------------------------------------------------

/// AA coverage for rounded rect (0-255). Notch has flat top.
fn rounded_rect_coverage_aa(x: i32, y: i32, w: u32, h: u32, r: u32, attached: bool) -> u8 {
    // Supersample 2x2 for AA
    let mut inside = 0u32;
    let samples = [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)];
    for (ox, oy) in samples {
        let fx = x as f32 + ox;
        let fy = y as f32 + oy;
        if point_inside_rounded_rect(fx, fy, w as f32, h as f32, r as f32, attached) {
            inside += 1;
        }
    }
    (inside * 255 / 4) as u8
}

/// Distance to edge (0 at border, >0 inside, negative outside). Used for stroke.
fn point_inside_rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32, attached: bool) -> bool {
    signed_distance_rounded_rect(x + 0.5, y + 0.5, w, h, r, attached) <= 0.0
}

fn signed_distance_rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32, attached: bool) -> f32 {
    // Transform to centered coordinates with origin at rect center
    // For notch attached, top edge is flat y=0, so rect is [0,w] x [0,h] with bottom rounding only.
    // For island, full rounding.
    if attached {
        // Flat top, rounded bottom: shape = rect minus top corners
        // Inside if x in [0,w] and y in [0,h] and (not in top corner outside? top is square so no rounding)
        // Bottom corners: distance to (r, h-r) and (w-r, h-r)
        if x < 0.0 || x >= w || y < 0.0 || y >= h {
            // Outside bounding box — compute distance to closest edge/corner
            let dx = (if x < 0.0 {
                -x
            } else if x >= w {
                x - w + 1.0
            } else {
                0.0
            })
            .max(0.0);
            let dy = (if y < 0.0 {
                -y
            } else if y >= h {
                y - h + 1.0
            } else {
                0.0
            })
            .max(0.0);
            if dx == 0.0 && dy == 0.0 {
                // Actually inside but near edge
                -1.0
            } else if y >= h - r && (x < r || x >= w - r) {
                let cx = if x < r { r } else { w - r };
                let cy = h - r;
                ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r
            } else {
                (dx * dx + dy * dy).sqrt()
            }
        } else if y < h - r {
            // Upper part (square)
            -f32::min(
                f32::min(x + 0.5, w - x - 0.5),
                f32::min(y + 0.5, h - y - 0.5),
            )
        } else if x >= r && x < w - r {
            // Center bottom strip
            -f32::min(x + 0.5, w - x - 0.5).min(h - y - 0.5)
        } else {
            // Corner arcs
            let cx = if x < r { r } else { w - r };
            let cy = h - r;
            ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r
        }
    } else {
        // Full pill: standard rounded rect SDF
        let half_w = w / 2.0;
        let half_h = h / 2.0;
        let cx = half_w;
        let cy = half_h;
        let px = (x - cx).abs() - (half_w - r);
        let py = (y - cy).abs() - (half_h - r);
        let ax = px.max(0.0);
        let ay = py.max(0.0);
        let outside = (ax * ax + ay * ay).sqrt();
        let inside = px.max(py).min(0.0);
        outside + inside - r
    }
}

// ---- pixel helpers --------------------------------------------------------

fn blend_pixel(frame: &mut FrameBuffer, x: u32, y: u32, src: [u8; 4]) {
    if x >= frame.width || y >= frame.height {
        return;
    }
    let idx = ((y * frame.width + x) * 4) as usize;
    let dst = &mut frame.pixels_pbgra[idx..idx + 4];
    // src is premultiplied, blend over dst (also premultiplied)
    let sa = src[3] as u32;
    let ia = 255 - sa;
    dst[0] = (src[0] as u32 + dst[0] as u32 * ia / 255).min(255) as u8;
    dst[1] = (src[1] as u32 + dst[1] as u32 * ia / 255).min(255) as u8;
    dst[2] = (src[2] as u32 + dst[2] as u32 * ia / 255).min(255) as u8;
    dst[3] = (sa + dst[3] as u32 * ia / 255) as u8;
}

fn fill_rect(frame: &mut FrameBuffer, left: i32, top: i32, w: u32, h: u32, color: [u8; 4]) {
    let left = left.clamp(0, frame.width as i32 - 1);
    let top = top.clamp(0, frame.height as i32 - 1);
    let right = (left + w as i32).min(frame.width as i32);
    let bottom = (top + h as i32).min(frame.height as i32);
    for y in top..bottom {
        for x in left..right {
            blend_pixel(frame, x as u32, y as u32, color);
        }
    }
}

fn draw_circle(frame: &mut FrameBuffer, cx: i32, cy: i32, radius: u32, color: [u8; 4]) {
    let r = radius as i32;
    for y in -r..=r {
        for x in -r..=r {
            let d2 = x * x + y * y;
            let ir = (r - 1) * (r - 1);
            if d2 <= r * r && d2 >= ir {
                blend_pixel(frame, (cx + x) as u32, (cy + y) as u32, color);
            }
        }
    }
}

// ---- 5x7 bitmap font for idle clock ---------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use termielle_core::{GlassConfig, IslandGeometry, VisualState};

    #[test]
    fn notch_and_island_frames_have_correct_size() {
        let glass = GlassConfig::default();
        let geom = IslandGeometry {
            width: 140,
            height: 36,
            radius: 18,
            attached: true,
            y_offset: 0,
        };
        let f = island_frame(VisualState::Idle, geom, &glass, 0.0);
        assert_eq!(f.width, 140);
        assert_eq!(f.height, 36);
        assert_eq!(f.pixels_pbgra.len(), 140 * 36 * 4);
    }

    #[test]
    fn floating_island_corners_are_transparent() {
        let glass = GlassConfig::default();
        let geom = IslandGeometry {
            width: 120,
            height: 36,
            radius: 18,
            attached: false,
            y_offset: 8,
        };
        let f = island_frame(VisualState::Idle, geom, &glass, 0.0);
        // Corners should be transparent (alpha ~0)
        assert_eq!(f.alpha_at(0, 0), 0);
        assert_eq!(f.alpha_at(119, 0), 0);
        assert_eq!(f.alpha_at(0, 35), 0);
        assert_eq!(f.alpha_at(119, 35), 0);
        // Center should be opaque
        assert!(f.alpha_at(60, 18) > 100);
    }

    #[test]
    fn notch_top_is_square_bottom_rounded() {
        let glass = GlassConfig::default();
        let geom = IslandGeometry {
            width: 200,
            height: 36,
            radius: 18,
            attached: true,
            y_offset: 0,
        };
        let f = island_frame(VisualState::Working, geom, &glass, 0.0);
        // Top row should be solid (no rounding)
        assert!(f.alpha_at(1, 0) > 100);
        assert!(f.alpha_at(198, 0) > 100);
        // Bottom corners transparent
        assert_eq!(f.alpha_at(0, 35), 0);
        assert_eq!(f.alpha_at(199, 35), 0);
    }

    #[test]
    fn coverage_aa_gives_mid_alpha_at_edge() {
        let c = rounded_rect_coverage_aa(0, 0, 120, 36, 18, false);
        // Corner of floating pill should be partial or zero
        assert!(c < 255);
    }

    #[test]
    fn glass_layer_plus_helpers_render_dashboard() {
        use crate::animation::fallback_frame;
        let glass = GlassConfig::default();
        let mut f = glass_layer(300, 56, 20, true, &glass);
        let face = fallback_frame(VisualState::Idle, 44);
        let icon = crate::tasks::TaskIcon {
            hwnd: 0,
            title: "demo".into(),
            width: 24,
            height: 24,
            pixels_pbgra: vec![200u8; 24 * 24 * 4],
        };
        blit_scaled(&mut f, &face, 14, 6, 44, 44);
        blit_icon(&mut f, &icon, 70, 16, false, [215, 120, 0, 255]);
        draw_disc(&mut f, 110, 28, 3, [107, 201, 242, 255]);
        draw_accent_strip(&mut f, true, 20, [61, 163, 232, 220]);
        // Face fill, icon block, dots and the accent strip must leave
        // clearly visible pixels — all without a single glyph.
        let bright = f
            .pixels_pbgra
            .chunks_exact(4)
            .filter(|px| px[3] > 128 && (px[0] as u32 + px[1] as u32 + px[2] as u32) / 3 > 150)
            .count();
        assert!(bright > 200, "expected visible dashboard pixels");
    }

    #[test]
    fn glass_layer_minimal_material_has_no_content() {
        let f = glass_layer(140, 36, 18, false, &GlassConfig::default());
        assert_eq!((f.width, f.height), (140, 36));
        // Tinted glass only: no bright content pixels.
        let bright = f
            .pixels_pbgra
            .chunks_exact(4)
            .filter(|px| px[3] > 128 && (px[0] as u32 + px[1] as u32 + px[2] as u32) / 3 > 150)
            .count();
        assert_eq!(bright, 0);
    }

    #[test]
    fn glass_layer_covers_pill_with_alpha() {
        let f = glass_layer(200, 36, 18, false, &GlassConfig::default());
        // Interior opaque-ish, corners transparent.
        assert!(f.alpha_at(100, 18) > 100);
        assert_eq!(f.alpha_at(0, 0), 0);
        assert_eq!(f.alpha_at(199, 0), 0);
    }
}

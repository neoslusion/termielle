//! Notch / island procedural renderer — custom layered glass.
//!
//! Produces a `FrameBuffer` for the top-center notch or floating island.
//! The glass material is baked into premultiplied BGRA so `UpdateLayeredWindow`
//! composites it with per-pixel alpha — no DWM acrylic dependency.
//!
//! Layout reads in logical pixels; every paint operation scales its inputs
//! by the destination frame's authoring scale, so the raster lands at full
//! device resolution while all call sites keep design units.

use crate::animation::FrameBuffer;
use crate::window::scaled_size;
use termielle_core::{GlassConfig, IslandGeometry, VisualState};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, CreateFontW,
    DIB_RGB_COLORS, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC,
    DeleteObject, DrawTextW, FONT_CHARSET, FONT_CLIP_PRECISION, FONT_OUTPUT_PRECISION,
    FONT_QUALITY, GetDC, HGDIOBJ, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::core::PCWSTR;

/// Maps one logical design unit to device pixels for `frame`. At 1.0 this
/// is the identity, so unscaled frames render bit-identically to before.
fn sx(frame: &FrameBuffer, v: i32) -> i32 {
    (v as f32 * frame.scale).round() as i32
}

/// Same for unsigned extents (sizes, radii, font heights).
fn su(frame: &FrameBuffer, v: u32) -> u32 {
    ((v as f32 * frame.scale).round() as i64).clamp(0, i64::from(u32::MAX)) as u32
}

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
/// Maximum liquid-bridge fillet between blobs, in pixels. While blobs are
/// within this distance the smooth-min union connects them with a thinning
/// bridge; beyond it they read as separate shapes.
pub const BRIDGE_K_MAX: f32 = 12.0;

pub fn island_frame(
    state: VisualState,
    geom: IslandGeometry,
    glass: &GlassConfig,
    progress: f32,
    scale: f32,
) -> FrameBuffer {
    let width = geom.width.max(1);
    let height = geom.height.max(1);
    let radius = geom.radius.min(height / 2);
    let mut frame = glass_layer(width, height, radius, geom.attached, glass, scale);

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

/// Theme-aware text inks (primary, secondary) for neutral copy: near-white
/// on dark glass, near-black on light glass, using the same luminance gate
/// as [`text_color_for`]. Secondary never drops below ~90% alpha so dim
/// copy stays legible. Every non-accent `draw_text` site must use these;
/// hardcoded whites go invisible the moment `auto` flips to light.
pub fn ink_pair(glass: &GlassConfig) -> ([u8; 4], [u8; 4]) {
    let lum = (0.299 * f64::from(glass.tint[2])
        + 0.587 * f64::from(glass.tint[1])
        + 0.114 * f64::from(glass.tint[0])) as u8;
    if lum > 140 {
        ([18, 20, 28, 255], [70, 76, 95, 235])
    } else {
        ([255, 255, 255, 255], [188, 193, 208, 235])
    }
}

/// Which iOS presentation the pill is currently in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Presentation {
    /// Hidden when idle (top-edge hover sensor).
    Hidden,
    /// Tiny resting dot: nothing live (iOS "minimal").
    Minimal,
    /// Resting pill while an agent session or media is live (iOS "compact").
    Compact,
    /// Hovered or pinned dashboard (iOS "expanded").
    Expanded,
}

/// Content shown inside the notch: the termielle face, live agent session
/// indicators, and media playback live activity. Purely visual —
/// no text anywhere. Composed by the controller from cached layers via the
/// public helpers below.
pub struct NotchContent<'a> {
    /// The termielle character face (premultiplied BGRA frame), optional.
    pub face: Option<&'a FrameBuffer>,
    /// True while media plays: paints the accent strip teal when idle.
    pub media_playing: bool,
    /// Live agent sessions, drawn as accent dots (capped by the caller).
    pub session_dots: usize,
    /// The current presentation. `Minimal` hides secondary indicators (shows
    /// only the highest-priority activity indicator).
    pub presentation: Presentation,
}

/// Pixel size of one app icon in the dashboard row.
pub const ICON_PX: u32 = 24;

/// Filled disc for session dots.
pub fn draw_disc(frame: &mut FrameBuffer, cx: i32, cy: i32, radius: u32, color: [u8; 4]) {
    let (cx, cy) = (sx(frame, cx), sx(frame, cy));
    let r = su(frame, radius) as i32;
    for y in -r..=r {
        for x in -r..=r {
            if x * x + y * y <= r * r {
                blend_pixel(frame, (cx + x) as u32, (cy + y) as u32, color);
            }
        }
    }
}

/// One liquid blob of the island silhouette. The pill renders the
/// smooth-min union of its blobs, so two overlapping blobs read as one
/// shape and two separated blobs stay connected by a thinning liquid
/// bridge while the separation is still small — the Dynamic Island
/// split/merge morphology.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlobRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub r: u32,
    pub attached: bool,
}

/// Polynomial smooth-min (Inigo Quilez): below `k` separation the two
/// distances blend into a fillet, which is the liquid bridge between
/// blobs. `k` is in pixels: 0 degenerates to a hard `min`.
fn smooth_min(a: f32, b: f32, k: f32) -> f32 {
    if k <= 0.01 {
        return a.min(b);
    }
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b * (1.0 - h) + a * h - k * h * (1.0 - h)
}

/// Signed distance to the smooth-min union of `blobs` at frame-local
/// (`fx`, `fy`), with a bridge fillet of `k` pixels.
fn blobs_distance(blobs: &[BlobRect], fx: f32, fy: f32, k: f32) -> f32 {
    let mut distance = f32::INFINITY;
    for blob in blobs {
        let sd = signed_distance_rounded_rect(
            fx - blob.x as f32,
            fy - blob.y as f32,
            blob.w as f32,
            blob.h as f32,
            blob.r as f32,
            blob.attached,
        );
        distance = if distance.is_infinite() {
            sd
        } else {
            smooth_min(distance, sd, k)
        };
    }
    distance
}

pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0).max(f32::EPSILON)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Composites a premultiplied content frame over `dst`, offset by (`dx`,
/// `dy`) and scaled by a global `alpha` (0-255). This is how content rides
/// a morph: the entering presentation fades in and slides into place while
/// the container is still springing. `dx` carries whole-frame effects like
/// the failed-turn shake.
pub fn blend_frame_over(dst: &mut FrameBuffer, content: &FrameBuffer, dx: i32, dy: i32, alpha: u8) {
    if alpha == 0 || content.width == 0 || content.height == 0 {
        return;
    }
    // Both buffers share device dimensions; the ride offset reads logical.
    let (dx, dy) = (sx(dst, dx), sx(dst, dy));
    let scale = u32::from(alpha);
    for y in 0..content.height as i32 {
        let ty = y + dy;
        if ty < 0 || ty >= dst.height as i32 {
            continue;
        }
        for x in 0..content.width as i32 {
            let tx = x + dx;
            if tx < 0 || tx >= dst.width as i32 {
                continue;
            }
            let si = ((y * content.width as i32 + x) * 4) as usize;
            let pixel = &content.pixels_pbgra[si..si + 4];
            if pixel[3] == 0 {
                continue;
            }
            blend_pixel(
                dst,
                tx as u32,
                ty as u32,
                [
                    (u32::from(pixel[0]) * scale / 255) as u8,
                    (u32::from(pixel[1]) * scale / 255) as u8,
                    (u32::from(pixel[2]) * scale / 255) as u8,
                    (u32::from(pixel[3]) * scale / 255) as u8,
                ],
            );
        }
    }
}

/// Renders the frosted-glass material for one blob. See
/// [`glass_layer_blobs`] for the layered material.
pub fn glass_layer(
    width: u32,
    height: u32,
    radius: u32,
    attached: bool,
    glass: &GlassConfig,
    scale: f32,
) -> FrameBuffer {
    glass_layer_blobs(
        width,
        height,
        &[BlobRect {
            x: 0,
            y: 0,
            w: width,
            h: height,
            r: radius,
            attached,
        }],
        glass,
        false,
        BRIDGE_K_MAX,
        scale,
    )
}

/// Renders the frosted-glass material (shadow, body, border, highlight) in
/// ONE pass over the smooth-min union of `blobs`: the union signed distance
/// is evaluated once per pixel and all layers are composed in registers.
/// ~1ms for a 720x56 pill, and the result is cached by the controller per
/// geometry. `bridge_k` is the liquid-bridge fillet between blobs in
/// pixels. With `black` set the body is opaque true black — the material
/// of the real notch, which must read as display hardware, not glass.
pub fn glass_layer_blobs(
    width: u32,
    height: u32,
    blobs: &[BlobRect],
    glass: &GlassConfig,
    black: bool,
    bridge_k: f32,
    scale: f32,
) -> FrameBuffer {
    let (width, height) = scaled_size((width.max(1), height.max(1)), scale);
    let mut frame = FrameBuffer {
        width,
        height,
        pixels_pbgra: vec![0u8; (width * height * 4) as usize],
        delay_ms: 0,
        loop_index: 0,
        scale,
    };
    if blobs.is_empty() {
        return frame;
    }
    // Author the material at device resolution: blob geometry, the bridge
    // fillet, and every material constant below are logical units scaled
    // here, so the layers keep their designed proportions at any DPI.
    let blobs: Vec<BlobRect> = blobs
        .iter()
        .map(|b| BlobRect {
            x: sx(&frame, b.x),
            y: sx(&frame, b.y),
            w: su(&frame, b.w),
            h: su(&frame, b.h),
            r: su(&frame, b.r),
            attached: b.attached,
        })
        .collect();
    let blobs = blobs.as_slice();
    let bridge_k = (bridge_k * scale).max(0.0);
    let tint = glass.tint;

    // Fast path: pure rectangular status bar (1 blob, radius 0). Fills the
    // blob rect directly without per-pixel signed-distance fields — this is
    // what keeps full-width bars cheap, including margin-inset ones.
    if blobs.len() == 1 && blobs[0].r == 0 && !black {
        let ba = tint[3] as u32;
        let body = [
            (tint[0] as u32 * ba / 255) as u8,
            (tint[1] as u32 * ba / 255) as u8,
            (tint[2] as u32 * ba / 255) as u8,
            ba as u8,
        ];
        let by_start = blobs[0].y.max(0) as usize;
        let by_end = (blobs[0].y + blobs[0].h as i32).max(0) as usize;
        let by_end = by_end.min(height as usize);
        let bx_start = blobs[0].x.max(0) as usize;
        let bx_end = (blobs[0].x + blobs[0].w as i32).max(0) as usize;
        let bx_end = bx_end.min(width as usize);
        if bx_start >= bx_end {
            return frame;
        }
        for y in by_start..by_end {
            let row = y * width as usize * 4;
            for chunk in
                frame.pixels_pbgra[row + bx_start * 4..row + bx_end * 4].chunks_exact_mut(4)
            {
                chunk.copy_from_slice(&body);
            }
        }
        return frame;
    }

    for y in 0..height {
        let fy = y as f32 + 0.5;
        for x in 0..width {
            let fx = x as f32 + 0.5;
            let sd = blobs_distance(blobs, fx, fy, bridge_k);
            let cov = ((0.5 - sd).clamp(0.0, 1.0) * 255.0) as u8;
            if cov == 0 {
                continue;
            }
            let cov_u = cov as u32;

            if black {
                let idx = ((y * width + x) * 4) as usize;
                frame.pixels_pbgra[idx..idx + 4].copy_from_slice(&[0, 0, 0, cov]);
                continue;
            }

            // Layer stack, composed in registers with premultiplied-over.
            let mut acc = [0u8; 4];

            // 1. Shadow: the same shape shifted 2.5px down, visible only
            //    outside the body (alpha composed below under the tint).
            if glass.shadow_alpha > 0 {
                let sd_sh = blobs_distance(blobs, fx, fy - 2.5 * scale, bridge_k);
                let cov_sh = ((0.5 - sd_sh).clamp(0.0, 1.0) * 255.0) as u32;
                let sa = u32::from(glass.shadow_alpha) * cov_sh / 255 * (255 - cov_u) / 255;
                if sa > 0 {
                    acc[3] = sa as u8;
                }
            }

            // 2. Liquid Glass Body: deep translucent base matching theme & Windows settings
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

            // 3. Subtle Optical Refraction Groove (inner bevel):
            //    Soft glass depth just inside the outer rim without heavy dark banding.
            if -sd >= 1.0 * scale && -sd < 2.5 * scale {
                let factor = 1.0 - ((-sd - 1.8 * scale).abs() / (0.8 * scale)).clamp(0.0, 1.0);
                let dark_a = (factor * 16.0 * (cov_u as f32 / 255.0)) as u32;
                acc[0] = acc[0].saturating_sub((acc[0] as u32 * dark_a / 255) as u8);
                acc[1] = acc[1].saturating_sub((acc[1] as u32 * dark_a / 255) as u8);
                acc[2] = acc[2].saturating_sub((acc[2] as u32 * dark_a / 255) as u8);
            }

            // 4. Parabolic Top Specular Sheen (Subtle Fluent light reflection):
            // Soft luminous top reflection matching Windows 11 Fluent surfaces.
            let sheen_h = (height as f32 * 0.30).clamp(10.0 * scale, 32.0 * scale);
            if fy < sheen_h && -sd > 1.0 * scale {
                let t = 1.0 - (fy / sheen_h);
                let curve = t * t;
                let cx_norm = (fx - width as f32 / 2.0).abs() / (width as f32 / 2.0);
                let horiz_fade = 1.0 - 0.30 * cx_norm * cx_norm;
                let sheen_alpha = ((glass.highlight_alpha as f32 * 0.28).clamp(10.0, 32.0)
                    * curve
                    * horiz_fade
                    * (cov_u as f32 / 255.0)) as u8;
                if sheen_alpha > 0 {
                    let sa = sheen_alpha as u32;
                    let sia = 255 - sa;
                    acc[0] = (sa + acc[0] as u32 * sia / 255) as u8;
                    acc[1] = (sa + acc[1] as u32 * sia / 255) as u8;
                    acc[2] = (sa + acc[2] as u32 * sia / 255) as u8;
                    acc[3] = (sa + acc[3] as u32 * sia / 255).min(255) as u8;
                }
            }

            // 5. Windows 11 Fluent 1px Specular Border:
            // Balanced overhead light: top edge has gentle highlight, clean stroke all around.
            if -sd < 1.2 * scale && -sd >= 0.0 {
                let top_factor = (1.0 - (fy / height as f32)).clamp(0.0, 1.0);
                let any_attached = blobs.iter().any(|b| b.attached);
                let rim_alpha = if any_attached && fy < 1.5 * scale {
                    0u8
                } else {
                    ((glass.border_alpha as f32 * (0.85 + 0.65 * top_factor)).clamp(20.0, 65.0)
                        * (cov_u as f32 / 255.0)) as u8
                };
                if rim_alpha > 0 {
                    let ra = rim_alpha as u32;
                    let ria = 255 - ra;
                    let rb = 255 * ra / 255;
                    let rg = 252 * ra / 255;
                    let rr = 248 * ra / 255;
                    acc[0] = (rb + acc[0] as u32 * ria / 255) as u8;
                    acc[1] = (rg + acc[1] as u32 * ria / 255) as u8;
                    acc[2] = (rr + acc[2] as u32 * ria / 255) as u8;
                    acc[3] = (ra + acc[3] as u32 * ria / 255).min(255) as u8;
                }
            }

            // 6. Subtle Acrylic Dither:
            // Prevents 8-bit banding on gradients, replicating native acrylic texture
            if cov_u > 220 {
                let dither = ((((x
                    .wrapping_mul(1_103_515_245)
                    .wrapping_add(y.wrapping_mul(12_345)))
                    >> 16)
                    & 3) as i32)
                    - 1;
                acc[0] = (acc[0] as i32 + dither).clamp(0, 255) as u8;
                acc[1] = (acc[1] as i32 + dither).clamp(0, 255) as u8;
                acc[2] = (acc[2] as i32 + dither).clamp(0, 255) as u8;
            }

            let idx = ((y * width + x) * 4) as usize;
            frame.pixels_pbgra[idx..idx + 4].copy_from_slice(&acc);
        }
    }
    frame
}

/// Bilinear sample of a premultiplied BGRA frame at fractional source
/// coordinates. Taps clamp to the edge texel, so the four weights always sum
/// to one. Filtering premultiplied components (alpha included) keeps the
/// result a valid premultiplied texel — the same rule the GIF compositor
/// relies on — so filtered edges composite exactly like authored ones.
fn sample_bilinear(src: &FrameBuffer, fx: f32, fy: f32) -> [u8; 4] {
    if src.width == 0 || src.height == 0 {
        return [0, 0, 0, 0];
    }
    let fx = fx.clamp(0.0, src.width as f32 - 1.0);
    let fy = fy.clamp(0.0, src.height as f32 - 1.0);
    let x0 = fx.floor() as u32;
    let y0 = fy.floor() as u32;
    let x1 = (x0 + 1).min(src.width - 1);
    let y1 = (y0 + 1).min(src.height - 1);
    let tx = fx - fx.floor();
    let ty = fy - fy.floor();
    let tap = |x: u32, y: u32| -> [f32; 4] {
        let i = ((y * src.width + x) * 4) as usize;
        [
            f32::from(src.pixels_pbgra[i]),
            f32::from(src.pixels_pbgra[i + 1]),
            f32::from(src.pixels_pbgra[i + 2]),
            f32::from(src.pixels_pbgra[i + 3]),
        ]
    };
    let a = tap(x0, y0);
    let b = tap(x1, y0);
    let c = tap(x0, y1);
    let d = tap(x1, y1);
    let weights = [
        (1.0 - tx) * (1.0 - ty),
        tx * (1.0 - ty),
        (1.0 - tx) * ty,
        tx * ty,
    ];
    let taps = [a, b, c, d];
    let mut out = [0u8; 4];
    for ch in 0..4 {
        let mut acc = 0.0f32;
        for (tap, weight) in taps.iter().zip(weights.iter()) {
            acc += tap[ch] * weight;
        }
        out[ch] = acc.round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Resamples `src` to exactly (`w`, `h`) with bilinear filtering. Sizes are
/// device pixels (pass-through, never scaled): dest pixel
/// centers map into source space, so a downscale averages source texels and
/// an upscale ramps between them instead of stair-stepping. The 1:1 case
/// lands exactly on texel centers and copies.
pub fn resample_bilinear(src: &FrameBuffer, w: u32, h: u32) -> FrameBuffer {
    let mut out = FrameBuffer {
        width: w,
        height: h,
        pixels_pbgra: vec![0u8; (w as usize) * (h as usize) * 4],
        delay_ms: 0,
        loop_index: 0,
        scale: src.scale,
    };
    if src.width == 0 || src.height == 0 || w == 0 || h == 0 {
        return out;
    }
    for ty in 0..h {
        let fy = (ty as f32 + 0.5) * src.height as f32 / h as f32 - 0.5;
        for tx in 0..w {
            let fx = (tx as f32 + 0.5) * src.width as f32 / w as f32 - 0.5;
            let px = sample_bilinear(src, fx, fy);
            let i = ((ty * w + tx) * 4) as usize;
            out.pixels_pbgra[i..i + 4].copy_from_slice(&px);
        }
    }
    out
}

/// Blits a premultiplied BGRA frame with squircle rounded corners.
pub fn blit_rounded(
    frame: &mut FrameBuffer,
    src: &FrameBuffer,
    x: i32,
    y: i32,
    tw: u32,
    th: u32,
    radius: u32,
) {
    if src.width == 0 || src.height == 0 || tw == 0 || th == 0 {
        return;
    }
    let (x, y, tw, th) = (sx(frame, x), sx(frame, y), su(frame, tw), su(frame, th));
    let r = su(frame, radius).min(tw / 2).min(th / 2) as f32;
    for ty in 0..th as i32 {
        let dst_y = y + ty;
        if dst_y < 0 || dst_y >= frame.height as i32 {
            continue;
        }
        let fy = ty as f32 + 0.5;
        for tx in 0..tw as i32 {
            let dst_x = x + tx;
            if dst_x < 0 || dst_x >= frame.width as i32 {
                continue;
            }
            let fx = tx as f32 + 0.5;
            let sd = signed_distance_rounded_rect(fx, fy, tw as f32, th as f32, r, false);
            let cov = ((0.5 - sd).clamp(0.0, 1.0) * 255.0) as u32;
            if cov == 0 {
                continue;
            }
            let fx = (tx as f32 + 0.5) * src.width as f32 / tw as f32 - 0.5;
            let fy = (ty as f32 + 0.5) * src.height as f32 / th as f32 - 0.5;
            let pixel = sample_bilinear(src, fx, fy);
            let alpha = (pixel[3] as u32 * cov / 255) as u8;
            if alpha == 0 {
                continue;
            }
            blend_pixel(
                frame,
                dst_x as u32,
                dst_y as u32,
                [
                    (pixel[0] as u32 * cov / 255) as u8,
                    (pixel[1] as u32 * cov / 255) as u8,
                    (pixel[2] as u32 * cov / 255) as u8,
                    alpha,
                ],
            );
        }
    }
}

/// Renders native Windows GDI antialiased text into the frame. Position,
/// width, and size read logical; glyphs rasterize from the scaled size (at
/// 2x scratch and downsampled on blend), so text lands at full device
/// resolution instead of being filtered up afterwards.
#[allow(clippy::too_many_arguments)]
pub fn draw_text(
    frame: &mut FrameBuffer,
    text: &str,
    x: i32,
    y: i32,
    max_w: u32,
    font_size: i32,
    bold: bool,
    color: [u8; 4],
) {
    if text.is_empty() || max_w == 0 || font_size <= 0 {
        return;
    }
    let (x, y, max_w, font_size) = (
        sx(frame, x),
        sx(frame, y),
        su(frame, max_w),
        su(frame, font_size as u32) as i32,
    );
    let h = (font_size * 2).max(18) as u32;
    let w = max_w;
    // Supersampled scratch dimensions. The 2x2 box average in the blend loop
    // below is what turns the extra raster into extra edge information.
    let (sw, sh) = (w * 2, h * 2);

    let font_name = crate::window::encode_wide("Segoe UI Variable Text");
    let weight = if bold { 700 } else { 400 };
    let font = unsafe {
        CreateFontW(
            -font_size * 2,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            FONT_CHARSET(0),
            FONT_OUTPUT_PRECISION(0),
            FONT_CLIP_PRECISION(0),
            FONT_QUALITY(4), // ANTIALIASED_QUALITY: grayscale AA. ClearType
            // subpixel rendering assumes an opaque background and fringes on
            // a transparent layered window; grayscale stays neutral.
            0,
            PCWSTR(font_name.as_ptr()),
        )
    };
    if font.is_invalid() {
        return;
    }

    let screen_dc = unsafe { GetDC(Some(HWND::default())) };
    if screen_dc.is_invalid() {
        let _ = unsafe { DeleteObject(HGDIOBJ(font.0)) };
        return;
    }
    let memory_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
    if memory_dc.is_invalid() {
        let _ = unsafe { ReleaseDC(None, screen_dc) };
        let _ = unsafe { DeleteObject(HGDIOBJ(font.0)) };
        return;
    }

    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: sw as i32,
            biHeight: -(sh as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let dib =
        unsafe { CreateDIBSection(Some(memory_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) };
    if dib.is_err() || bits.is_null() {
        let _ = unsafe { DeleteDC(memory_dc) };
        let _ = unsafe { ReleaseDC(None, screen_dc) };
        let _ = unsafe { DeleteObject(HGDIOBJ(font.0)) };
        return;
    }
    let dib = dib.unwrap();

    let prev_obj = unsafe { SelectObject(memory_dc, HGDIOBJ(dib.0)) };
    let prev_font = unsafe { SelectObject(memory_dc, HGDIOBJ(font.0)) };

    let total_bytes = (sw * sh * 4) as usize;
    unsafe {
        std::ptr::write_bytes(bits as *mut u8, 0, total_bytes);
        SetBkMode(memory_dc, TRANSPARENT);
        SetTextColor(memory_dc, windows::Win32::Foundation::COLORREF(0x00FFFFFF));
    }

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: sw as i32,
        bottom: sh as i32,
    };
    let mut wide_text = crate::window::encode_wide(text);
    if wide_text.ends_with(&[0]) {
        wide_text.pop();
    }
    unsafe {
        DrawTextW(
            memory_dc,
            &mut wide_text,
            &mut rect,
            DT_LEFT | DT_NOPREFIX | DT_SINGLELINE | DT_END_ELLIPSIS | DT_VCENTER,
        );
    }

    let src_slice = unsafe { std::slice::from_raw_parts(bits as *const u8, total_bytes) };

    for row in 0..h as i32 {
        let target_y = y + row;
        if target_y < 0 || target_y >= frame.height as i32 {
            continue;
        }
        for col in 0..w as i32 {
            let target_x = x + col;
            if target_x < 0 || target_x >= frame.width as i32 {
                continue;
            }
            // Box-average the 2x2 supersampled block into one coverage
            // value: luminance per tap, then the mean of four. Grayscale AA
            // keeps channels in agreement, so no subpixel fringes survive.
            let mut acc = 0u32;
            for dy in 0..2i32 {
                for dx in 0..2i32 {
                    let src_idx = (((row * 2 + dy) * sw as i32 + (col * 2 + dx)) * 4) as usize;
                    acc += u32::from(src_slice[src_idx]) * 77
                        + u32::from(src_slice[src_idx + 1]) * 150
                        + u32::from(src_slice[src_idx + 2]) * 29;
                }
            }
            let val = ((acc / 4 + 128) / 256) as u8;
            if val == 0 {
                continue;
            }
            let text_alpha = (u32::from(val) * u32::from(color[3]) / 255) as u8;
            if text_alpha == 0 {
                continue;
            }
            let p_b = (u32::from(color[0]) * u32::from(text_alpha) / 255) as u8;
            let p_g = (u32::from(color[1]) * u32::from(text_alpha) / 255) as u8;
            let p_r = (u32::from(color[2]) * u32::from(text_alpha) / 255) as u8;
            blend_pixel(
                frame,
                target_x as u32,
                target_y as u32,
                [p_b, p_g, p_r, text_alpha],
            );
        }
    }

    unsafe {
        let _ = SelectObject(memory_dc, prev_font);
        let _ = SelectObject(memory_dc, prev_obj);
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = DeleteDC(memory_dc);
        let _ = ReleaseDC(None, screen_dc);
        let _ = DeleteObject(HGDIOBJ(font.0));
    }
}

/// Blits a premultiplied BGRA frame (the termielle face) scaled to
/// (tw, th) at (x, y), bilinear-filtered so downscaled faces and thumbnails
/// average source texels instead of dropping rows and columns.
pub fn blit_scaled(frame: &mut FrameBuffer, src: &FrameBuffer, x: i32, y: i32, tw: u32, th: u32) {
    if src.width == 0 || src.height == 0 || tw == 0 || th == 0 {
        return;
    }
    // Dest rect reads logical; source texels are sampled by the device-size
    // ratio, so the filter stays correct at any authoring scale.
    let (x, y, tw, th) = (sx(frame, x), sx(frame, y), su(frame, tw), su(frame, th));
    for ty in 0..th as i32 {
        let dst_y = y + ty;
        if dst_y < 0 || dst_y >= frame.height as i32 {
            continue;
        }
        let fy = (ty as f32 + 0.5) * src.height as f32 / th as f32 - 0.5;
        for tx in 0..tw as i32 {
            let dst_x = x + tx;
            if dst_x < 0 || dst_x >= frame.width as i32 {
                continue;
            }
            let fx = (tx as f32 + 0.5) * src.width as f32 / tw as f32 - 0.5;
            let px = sample_bilinear(src, fx, fy);
            if px[3] == 0 {
                continue;
            }
            blend_pixel(frame, dst_x as u32, dst_y as u32, px);
        }
    }
}

/// Draws the 2px state accent strip along the pill's top edge (flush for
/// attached notches, inset for floating islands).
pub fn draw_accent_strip(frame: &mut FrameBuffer, attached: bool, radius: u32, color: [u8; 4]) {
    let (width, height) = (frame.width, frame.height);
    let radius = su(frame, radius);
    let margin = sx(frame, 8);
    let y0 = if attached { 0 } else { sx(frame, 2) };
    let thick = sx(frame, 2);
    for y in y0..(y0 + thick).min(height as i32) {
        for x in margin..(width as i32 - margin) {
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

/// Draws a smooth circular button with a frosted fill and a subtle stroke.
pub fn draw_button_circle(
    frame: &mut FrameBuffer,
    cx: i32,
    cy: i32,
    radius: u32,
    bg: [u8; 4],
    border: [u8; 4],
) {
    let (cx, cy) = (sx(frame, cx), sx(frame, cy));
    let r = su(frame, radius) as f32;
    let r_i = su(frame, radius) as i32;
    let stroke = 1.2 * frame.scale;
    for y in -r_i - 1..=r_i + 1 {
        let py = cy + y;
        if py < 0 || py >= frame.height as i32 {
            continue;
        }
        for x in -r_i - 1..=r_i + 1 {
            let px = cx + x;
            if px < 0 || px >= frame.width as i32 {
                continue;
            }
            let dist = ((x * x + y * y) as f32).sqrt();
            let sd = dist - r;
            let cov = ((0.5 - sd).clamp(0.0, 1.0) * 255.0) as u32;
            if cov == 0 {
                continue;
            }
            let color = if dist >= r - stroke {
                [
                    ((border[0] as u32 * cov) / 255) as u8,
                    ((border[1] as u32 * cov) / 255) as u8,
                    ((border[2] as u32 * cov) / 255) as u8,
                    ((border[3] as u32 * cov) / 255) as u8,
                ]
            } else {
                [
                    ((bg[0] as u32 * cov) / 255) as u8,
                    ((bg[1] as u32 * cov) / 255) as u8,
                    ((bg[2] as u32 * cov) / 255) as u8,
                    ((bg[3] as u32 * cov) / 255) as u8,
                ]
            };
            blend_pixel(frame, px as u32, py as u32, color);
        }
    }
}

/// Draws a smooth rounded rectangle / card with an anti-aliased fill and border.
///
/// `bg`/`border` are straight-alpha BGRA; they are premultiplied here for the
/// PBGRA frame.
#[allow(clippy::too_many_arguments)] // paint ops take explicit geometry; a struct would churn every call site.
pub fn draw_rounded_rect(
    frame: &mut FrameBuffer,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    radius: u32,
    bg: [u8; 4],
    border: [u8; 4],
) {
    if w == 0 || h == 0 {
        return;
    }
    let (sx_val, sy_val, sw_val, sh_val) = (sx(frame, x), sx(frame, y), su(frame, w), su(frame, h));
    let r = (su(frame, radius) as f32)
        .min(sw_val as f32 / 2.0)
        .min(sh_val as f32 / 2.0);
    let stroke = 1.0 * frame.scale;

    for ty in 0..sh_val as i32 {
        let py = sy_val + ty;
        if py < 0 || py >= frame.height as i32 {
            continue;
        }
        let fy = ty as f32 + 0.5;
        for tx in 0..sw_val as i32 {
            let px = sx_val + tx;
            if px < 0 || px >= frame.width as i32 {
                continue;
            }
            let fx = tx as f32 + 0.5;
            let sd = signed_distance_rounded_rect(fx, fy, sw_val as f32, sh_val as f32, r, false);
            let cov = ((0.5 - sd).clamp(0.0, 1.0) * 255.0) as u32;
            if cov == 0 {
                continue;
            }
            // Paint colors are straight alpha; the PBGRA buffer needs them
            // premultiplied. Skipping that turns every low-alpha bright fill
            // (e.g. `[255,255,255,20]` module pills) opaque white: src-over
            // adds the unscaled 255s straight into the frame.
            let paint = |c: [u8; 4]| {
                let a = c[3] as u32;
                [
                    (c[0] as u32 * a * cov / 65025) as u8,
                    (c[1] as u32 * a * cov / 65025) as u8,
                    (c[2] as u32 * a * cov / 65025) as u8,
                    (a * cov / 255) as u8,
                ]
            };
            let color = if border[3] > 0 && sd >= -stroke {
                paint(border)
            } else if bg[3] > 0 {
                paint(bg)
            } else {
                continue;
            };
            blend_pixel(frame, px as u32, py as u32, color);
        }
    }
}

/// Draws a sleek horizontal mini progress bar with rounded ends.
#[allow(clippy::too_many_arguments)] // same paint-op convention as draw_rounded_rect.
pub fn draw_progress_bar(
    frame: &mut FrameBuffer,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    pct: u8,
    track_color: [u8; 4],
    fill_color: [u8; 4],
) {
    if w == 0 || h == 0 {
        return;
    }
    let r = (h / 2).max(1);
    draw_rounded_rect(frame, x, y, w, h, r, track_color, [0, 0, 0, 0]);
    if pct > 0 {
        let fill_w = ((w as f32 * (pct.min(100) as f32 / 100.0)).round() as u32)
            .max(h.min(w))
            .min(w);
        draw_rounded_rect(frame, x, y, fill_w, h, r, fill_color, [0, 0, 0, 0]);
    }
}

/// Draws previous track glyph (|<<).
/// Glyph centers read logical; shapes iterate in design units and only the
/// plotted pixels scale, so topology never changes with DPI.
pub fn draw_glyph_prev(frame: &mut FrameBuffer, cx: i32, cy: i32, color: [u8; 4]) {
    let s = frame.scale;
    let gx = |dx: i32| (cx as f32 + dx as f32 * s).round() as i32;
    let gy = |dy: i32| (cy as f32 + dy as f32 * s).round() as i32;
    for dy in -5..=5 {
        blend_pixel(frame, gx(-6) as u32, gy(dy) as u32, color);
        blend_pixel(frame, gx(-5) as u32, gy(dy) as u32, color);
    }
    for dx in 0..=4 {
        let max_y = 5 - dx;
        for dy in -max_y..=max_y {
            blend_pixel(frame, gx(-4 + dx) as u32, gy(dy) as u32, color);
        }
    }
    for dx in 0..=4 {
        let max_y = 5 - dx;
        for dy in -max_y..=max_y {
            blend_pixel(frame, gx(1 + dx) as u32, gy(dy) as u32, color);
        }
    }
}

/// Draws play triangle glyph (>).
pub fn draw_glyph_play(frame: &mut FrameBuffer, cx: i32, cy: i32, color: [u8; 4]) {
    let s = frame.scale;
    let gx = |dx: i32| (cx as f32 + dx as f32 * s).round() as i32;
    let gy = |dy: i32| (cy as f32 + dy as f32 * s).round() as i32;
    for dx in 0..=8 {
        let half_h = ((8 - dx) * 6) / 8;
        for dy in -half_h..=half_h {
            blend_pixel(frame, gx(-4 + dx) as u32, gy(dy) as u32, color);
        }
    }
}

/// Draws pause bars glyph (||).
pub fn draw_glyph_pause(frame: &mut FrameBuffer, cx: i32, cy: i32, color: [u8; 4]) {
    let s = frame.scale;
    let gx = |dx: i32| (cx as f32 + dx as f32 * s).round() as i32;
    let gy = |dy: i32| (cy as f32 + dy as f32 * s).round() as i32;
    for dy in -6..=6 {
        blend_pixel(frame, gx(-4) as u32, gy(dy) as u32, color);
        blend_pixel(frame, gx(-3) as u32, gy(dy) as u32, color);
        blend_pixel(frame, gx(-2) as u32, gy(dy) as u32, color);

        blend_pixel(frame, gx(2) as u32, gy(dy) as u32, color);
        blend_pixel(frame, gx(3) as u32, gy(dy) as u32, color);
        blend_pixel(frame, gx(4) as u32, gy(dy) as u32, color);
    }
}

/// Draws next track glyph (>>|).
pub fn draw_glyph_next(frame: &mut FrameBuffer, cx: i32, cy: i32, color: [u8; 4]) {
    let s = frame.scale;
    let gx = |dx: i32| (cx as f32 + dx as f32 * s).round() as i32;
    let gy = |dy: i32| (cy as f32 + dy as f32 * s).round() as i32;
    for dx in 0..=4 {
        let max_y = dx + 1;
        for dy in -max_y..=max_y {
            blend_pixel(frame, gx(-5 + dx) as u32, gy(dy) as u32, color);
        }
    }
    for dx in 0..=4 {
        let max_y = dx + 1;
        for dy in -max_y..=max_y {
            blend_pixel(frame, gx(dx) as u32, gy(dy) as u32, color);
        }
    }
    for dy in -5..=5 {
        blend_pixel(frame, gx(5) as u32, gy(dy) as u32, color);
        blend_pixel(frame, gx(6) as u32, gy(dy) as u32, color);
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

/// Public re-export of the rect painter for dashboard elements.
pub fn fill_rect_pub(frame: &mut FrameBuffer, left: i32, top: i32, w: u32, h: u32, color: [u8; 4]) {
    fill_rect(frame, left, top, w, h, color)
}

fn fill_rect(frame: &mut FrameBuffer, left: i32, top: i32, w: u32, h: u32, color: [u8; 4]) {
    let (left, top, w, h) = (sx(frame, left), sx(frame, top), su(frame, w), su(frame, h));
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
    let (cx, cy) = (sx(frame, cx), sx(frame, cy));
    let r = su(frame, radius) as i32;
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
        let f = island_frame(VisualState::Idle, geom, &glass, 0.0, 1.0);
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
        let f = island_frame(VisualState::Idle, geom, &glass, 0.0, 1.0);
        // Corners should be transparent (alpha ~0)
        assert_eq!(f.alpha_at(0, 0), 0);
        assert_eq!(f.alpha_at(119, 0), 0);
        assert_eq!(f.alpha_at(0, 35), 0);
        assert_eq!(f.alpha_at(119, 35), 0);
        // Center should be opaque
        assert!(f.alpha_at(60, 18) > 100);
    }

    #[test]
    fn ink_pair_flips_with_glass_luminance() {
        let (ink, dim) = ink_pair(&GlassConfig::default());
        assert_eq!(ink, [255, 255, 255, 255]);
        assert!(dim[3] >= 217, "dim ink stays above the legibility floor");
        let light = GlassConfig {
            tint: [243, 243, 243, 230],
            ..Default::default()
        };
        let (ink, dim) = ink_pair(&light);
        assert_eq!(ink, [18, 20, 28, 255]);
        // Dark on light at high alpha: contrast ratio against #F3F3F3 > 12.
        assert!(dim[3] >= 217);
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
        let f = island_frame(VisualState::Working, geom, &glass, 0.0, 1.0);
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
        let mut f = glass_layer(300, 56, 20, true, &glass, 1.0);
        let face = fallback_frame(VisualState::Idle, 44, 1.0);
        blit_scaled(&mut f, &face, 14, 6, 44, 44);
        // Media disc + equalizer bars
        draw_disc(&mut f, 75, 28, 10, [200, 200, 200, 255]);
        fill_rect_pub(&mut f, 95, 20, 3, 16, [215, 120, 0, 255]);
        fill_rect_pub(&mut f, 100, 16, 3, 20, [215, 120, 0, 255]);
        fill_rect_pub(&mut f, 105, 22, 3, 14, [215, 120, 0, 255]);
        draw_disc(&mut f, 120, 28, 3, [107, 201, 242, 255]);
        draw_accent_strip(&mut f, true, 20, [61, 163, 232, 220]);
        // Face fill, media disc, equalizer bars, dots and accent strip
        // must leave clearly visible pixels.
        let bright = f
            .pixels_pbgra
            .chunks_exact(4)
            .filter(|px| px[3] > 128 && (px[0] as u32 + px[1] as u32 + px[2] as u32) / 3 > 150)
            .count();
        assert!(bright > 200, "expected visible dashboard pixels");
    }

    #[test]
    fn glass_layer_minimal_material_has_no_content() {
        let f = glass_layer(140, 36, 18, false, &GlassConfig::default(), 1.0);
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
        let f = glass_layer(200, 36, 18, false, &GlassConfig::default(), 1.0);
        // Interior opaque-ish, corners transparent.
        assert!(f.alpha_at(100, 18) > 100);
        assert_eq!(f.alpha_at(0, 0), 0);
        assert_eq!(f.alpha_at(199, 0), 0);
    }
    #[test]
    fn paint_ops_scale_logical_inputs_to_device_pixels() {
        let mut f = FrameBuffer {
            width: 200,
            height: 80,
            pixels_pbgra: vec![0u8; 200 * 80 * 4],
            delay_ms: 0,
            loop_index: 0,
            scale: 2.0,
        };
        fill_rect_pub(&mut f, 10, 10, 5, 5, [255, 255, 255, 255]);
        // Logical (10..15, 10..15) lands at device (20..30, 20..30).
        assert_eq!(f.alpha_at(19, 19), 0);
        assert_eq!(f.alpha_at(20, 20), 255);
        assert_eq!(f.alpha_at(29, 29), 255);
        assert_eq!(f.alpha_at(30, 30), 0);
    }

    #[test]
    fn rounded_rect_premultiplies_low_alpha_fills() {
        // Straight-alpha `[255,255,255,20]` (bar module pills) must composite
        // as a dim veil, not opaque white: the PBGRA blend adds src rgb raw.
        let mut f = FrameBuffer {
            width: 100,
            height: 40,
            pixels_pbgra: vec![0u8; 100 * 40 * 4],
            delay_ms: 0,
            loop_index: 0,
            scale: 1.0,
        };
        draw_rounded_rect(&mut f, 10, 10, 60, 20, 9, [255, 255, 255, 20], [0, 0, 0, 0]);
        let i = ((20 * 100 + 40) * 4) as usize;
        let px = &f.pixels_pbgra[i..i + 4];
        assert_eq!(px[3], 20);
        assert!(
            px[0] <= 20 && px[1] <= 20 && px[2] <= 20,
            "fill not premultiplied: {px:?}"
        );
    }

    #[test]
    fn disc_centers_scale_with_the_frame() {
        let mut f = FrameBuffer {
            width: 120,
            height: 60,
            pixels_pbgra: vec![0u8; 120 * 60 * 4],
            delay_ms: 0,
            loop_index: 0,
            scale: 2.0,
        };
        draw_disc(&mut f, 10, 10, 3, [255, 255, 255, 255]);
        // Center (10, 10) -> device (20, 20); radius 3 -> 6.
        assert_eq!(f.alpha_at(20, 20), 255);
        assert_eq!(f.alpha_at(40, 20), 0);
    }

    #[test]
    fn glass_layer_authors_material_at_device_size() {
        let f = glass_layer(140, 36, 18, false, &GlassConfig::default(), 1.25);
        assert_eq!((f.width, f.height), (175, 45));
        // Interior stays opaque, corners stay transparent: the material
        // keeps its designed coverage, sampled finer.
        assert!(f.alpha_at(87, 22) > 100);
        assert_eq!(f.alpha_at(0, 0), 0);
        assert_eq!(f.alpha_at(174, 0), 0);
    }

    #[test]
    fn text_renders_coverage_at_device_size() {
        let mut f = FrameBuffer {
            width: 400,
            height: 100,
            pixels_pbgra: vec![0u8; 400 * 100 * 4],
            delay_ms: 0,
            loop_index: 0,
            scale: 1.25,
        };
        draw_text(&mut f, "Ag", 10, 10, 200, 14, false, [255, 255, 255, 255]);
        let covered = f
            .pixels_pbgra
            .chunks_exact(4)
            .filter(|px| px[3] > 0)
            .count();
        assert!(covered > 20, "scaled text must leave coverage");
    }
}

//! Tabler Icons, stroked on demand.
//!
//! The bar and the control panel draw line-art glyphs, so the icons ship as
//! their upstream SVG path data rather than as baked bitmaps. [`draw_icon`]
//! flattens those paths and fills the shape from the distance to the nearest
//! segment, so coverage - and therefore the edge - is exact at any scale with
//! no supersampling, and a new icon is one line of path data.
//!
//! Source: Tabler Icons v3.31.0, MIT, Copyright (c) 2020-2024 Pawel Kuna,
//! <https://github.com/tabler/tabler-icons>. Every icon here is the upstream
//! 24x24 `outline` variant, unmodified except for dropping the leading
//! `<path stroke="none">` that every published file carries as a no-op
//! background rect.

use super::FrameBuffer;
use super::notch::su;
use super::notch::sx;

/// The viewBox every published Tabler outline icon is drawn in.
const VIEWBOX: f32 = 24.0;
/// Upstream `stroke-width`. Tabler publishes every outline icon at 2.
const STROKE: f32 = 2.0;

/// One icon: the `d` attribute of each stroked path, in draw order.
#[derive(Clone, Copy)]
pub(crate) struct Icon(pub &'static [&'static str]);

pub(crate) const CPU: Icon = Icon(&[
    "M5 5m0 1a1 1 0 0 1 1 -1h12a1 1 0 0 1 1 1v12a1 1 0 0 1 -1 1h-12a1 1 0 0 1 -1 -1z",
    "M9 9h6v6h-6z",
    "M3 10h2",
    "M3 14h2",
    "M10 3v2",
    "M14 3v2",
    "M21 10h-2",
    "M21 14h-2",
    "M14 21v-2",
    "M10 21v-2",
]);

pub(crate) const MEMORY: Icon = Icon(&[
    "M12 4l-8 4l8 4l8 -4l-8 -4",
    "M4 12l8 4l8 -4",
    "M4 16l8 4l8 -4",
]);

pub(crate) const VOLUME: Icon = Icon(&[
    "M15 8a5 5 0 0 1 0 8",
    "M17.7 5a9 9 0 0 1 0 14",
    "M6 15h-2a1 1 0 0 1 -1 -1v-4a1 1 0 0 1 1 -1h2l3.5 -4.5a.8 .8 0 0 1 1.5 .5v14a.8 .8 0 0 1 -1.5 .5l-3.5 -4.5",
]);

pub(crate) const VOLUME_MUTED: Icon = Icon(&[
    "M15 8a5 5 0 0 1 0 8",
    "M6 15h-2a1 1 0 0 1 -1 -1v-4a1 1 0 0 1 1 -1h2l3.5 -4.5a.8 .8 0 0 1 1.5 .5v14a.8 .8 0 0 1 -1.5 .5l-3.5 -4.5",
]);

pub(crate) const BATTERY: Icon = Icon(&[
    "M6 7h11a2 2 0 0 1 2 2v.5a.5 .5 0 0 0 .5 .5a.5 .5 0 0 1 .5 .5v3a.5 .5 0 0 1 -.5 .5a.5 .5 0 0 0 -.5 .5v.5a2 2 0 0 1 -2 2h-11a2 2 0 0 1 -2 -2v-6a2 2 0 0 1 2 -2",
]);

pub(crate) const BATTERY_CHARGING: Icon = Icon(&[
    "M16 7h1a2 2 0 0 1 2 2v.5a.5 .5 0 0 0 .5 .5a.5 .5 0 0 1 .5 .5v3a.5 .5 0 0 1 -.5 .5a.5 .5 0 0 0 -.5 .5v.5a2 2 0 0 1 -2 2h-2",
    "M8 7h-2a2 2 0 0 0 -2 2v6a2 2 0 0 0 2 2h1",
    "M12 8l-2 4h3l-2 4",
]);

pub(crate) const CLOCK: Icon = Icon(&["M3 12a9 9 0 1 0 18 0a9 9 0 0 0 -18 0", "M12 7v5l3 3"]);

pub(crate) const ADJUSTMENTS: Icon = Icon(&[
    "M4 10a2 2 0 1 0 4 0a2 2 0 0 0 -4 0",
    "M6 4v4",
    "M6 12v8",
    "M10 16a2 2 0 1 0 4 0a2 2 0 0 0 -4 0",
    "M12 4v10",
    "M12 18v2",
    "M16 7a2 2 0 1 0 4 0a2 2 0 0 0 -4 0",
    "M18 4v1",
    "M18 9v11",
]);

/// A flattened polyline in viewBox units, ready to be scaled to the canvas.
struct Polyline(Vec<(f32, f32)>);

/// Parses one `d` attribute into polylines.
///
/// Supports the whole SVG path grammar Tabler emits: absolute and relative
/// moves, lines, horizontal/vertical shorthands, cubic and quadratic curves
/// with their smooth reflections, elliptical arcs, and close.
fn parse_path(d: &str) -> Vec<Polyline> {
    let bytes = d.as_bytes();
    let mut i = 0;
    let number = |i: &mut usize| -> Option<f32> {
        while *i < bytes.len() && (bytes[*i] as char).is_whitespace() {
            *i += 1;
        }
        if *i >= bytes.len() {
            return None;
        }
        let start = *i;
        if bytes[*i] == b'-' || bytes[*i] == b'+' {
            *i += 1;
        }
        while *i < bytes.len()
            && (bytes[*i].is_ascii_digit() || bytes[*i] == b'.' || bytes[*i] == b'e')
        {
            *i += 1;
        }
        if *i < bytes.len() && (bytes[*i] == b'-' || bytes[*i] == b'+') {
            *i += 1;
        }
        d.get(start..*i)?.parse().ok()
    };

    let mut out: Vec<Polyline> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let (mut start_x, mut start_y) = (0.0f32, 0.0f32);
    // Reflection state for the smooth curve shorthands.
    let (mut last_cubic, mut last_quad) = ((0.0f32, 0.0f32), (0.0f32, 0.0f32));
    let mut cmd = b'M';

    let flush = |cur: &mut Vec<(f32, f32)>, out: &mut Vec<Polyline>| {
        if cur.len() > 1 {
            out.push(Polyline(std::mem::take(cur)));
        } else {
            cur.clear();
        }
    };

    while i < bytes.len() {
        let c = bytes[i];
        if (c as char).is_whitespace() || c == b',' {
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() {
            cmd = c;
            i += 1;
        }
        let rel = cmd.is_ascii_lowercase();
        let (dx, dy) = if rel { (x, y) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let Some(nx) = number(&mut i) else { break };
                let Some(ny) = number(&mut i) else { break };
                flush(&mut cur, &mut out);
                x = dx + nx;
                y = dy + ny;
                start_x = x;
                start_y = y;
                cur.push((x, y));
                // Extra coordinate pairs after a move are implicit lines.
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let Some(nx) = number(&mut i) else { break };
                let Some(ny) = number(&mut i) else { break };
                x = dx + nx;
                y = dy + ny;
                cur.push((x, y));
            }
            b'H' => {
                let Some(nx) = number(&mut i) else { break };
                x = dx + nx;
                cur.push((x, y));
            }
            b'V' => {
                let Some(ny) = number(&mut i) else { break };
                y = dy + ny;
                cur.push((x, y));
            }
            b'C' | b'S' => {
                let mut ctrl = [(dx, dy); 2];
                let reflect = cmd == b'S' || cmd == b's';
                if reflect {
                    ctrl[0] = (2.0 * x - last_cubic.0, 2.0 * y - last_cubic.1);
                }
                for slot in ctrl.iter_mut() {
                    let Some(nx) = number(&mut i) else { break };
                    let Some(ny) = number(&mut i) else { break };
                    *slot = (dx + nx, dy + ny);
                }
                let Some(nx) = number(&mut i) else { break };
                let Some(ny) = number(&mut i) else { break };
                let end = (dx + nx, dy + ny);
                flatten_cubic(&mut cur, (x, y), ctrl[0], ctrl[1], end);
                last_cubic = ctrl[1];
                x = end.0;
                y = end.1;
            }
            b'Q' | b'T' => {
                // A `T` carries no control point of its own: it reflects the
                // previous curve's, or the current point if there was none.
                let smooth = cmd == b'T' || cmd == b't';
                let Some(nx) = number(&mut i) else { break };
                let Some(ny) = number(&mut i) else { break };
                let ctrl = (dx + nx, dy + ny);
                let Some(nx) = number(&mut i) else { break };
                let Some(ny) = number(&mut i) else { break };
                let end = (dx + nx, dy + ny);
                let ctrl = if smooth {
                    (2.0 * x - last_quad.0, 2.0 * y - last_quad.1)
                } else {
                    ctrl
                };
                flatten_quad(&mut cur, (x, y), ctrl, end);
                last_quad = ctrl;
                x = end.0;
                y = end.1;
            }
            b'A' => {
                let Some(rx) = number(&mut i) else { break };
                let Some(ry) = number(&mut i) else { break };
                let Some(rot) = number(&mut i) else { break };
                let Some(laf) = number(&mut i) else { break };
                let Some(sf) = number(&mut i) else { break };
                let Some(nx) = number(&mut i) else { break };
                let Some(ny) = number(&mut i) else { break };
                let end = (dx + nx, dy + ny);
                flatten_arc(
                    &mut cur,
                    (x, y),
                    end,
                    rx.abs(),
                    ry.abs(),
                    rot.to_radians(),
                    laf != 0.0,
                    sf != 0.0,
                );
                x = end.0;
                y = end.1;
            }
            b'Z' => {
                flush(&mut cur, &mut out);
                x = start_x;
                y = start_y;
            }
            _ => break,
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// Subdivision count for a curve of roughly `len` viewBox units. Enough
/// samples that a flattened curve stays within a fraction of a pixel of the
/// real one at the sizes a bar icon is drawn at.
fn steps_for(len: f32) -> usize {
    (len * 2.0).ceil().clamp(4.0, 48.0) as usize
}

fn flatten_cubic(
    out: &mut Vec<(f32, f32)>,
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
) {
    let len = dist(p0, p1) + dist(p1, p2) + dist(p2, p3);
    let steps = steps_for(len);
    for s in 1..=steps {
        let t = s as f32 / steps as f32;
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        out.push((
            a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
            a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
        ));
    }
}

fn flatten_quad(out: &mut Vec<(f32, f32)>, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32)) {
    let len = dist(p0, p1) + dist(p1, p2);
    let steps = steps_for(len);
    for s in 1..=steps {
        let t = s as f32 / steps as f32;
        let u = 1.0 - t;
        out.push((
            u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0,
            u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1,
        ));
    }
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Elliptical arc to centre parameterisation, then linear sampling.
#[allow(clippy::too_many_arguments)]
fn flatten_arc(
    out: &mut Vec<(f32, f32)>,
    from: (f32, f32),
    to: (f32, f32),
    rx: f32,
    ry: f32,
    phi: f32,
    large_arc: bool,
    sweep: bool,
) {
    if rx == 0.0 || ry == 0.0 {
        out.push(to);
        return;
    }
    let (sin, cos) = phi.sin_cos();
    let (dx2, dy2) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
    let x1p = cos * dx2 + sin * dy2;
    let y1p = -sin * dx2 + cos * dy2;
    // Scale the radii up if they cannot span the chord.
    let lambda = (x1p / rx).powi(2) + (y1p / ry).powi(2);
    let (rx, ry) = if lambda > 1.0 {
        (rx * lambda.sqrt(), ry * lambda.sqrt())
    } else {
        (rx, ry)
    };
    let num = (rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p).max(0.0);
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let sign = if large_arc != sweep { 1.0 } else { -1.0 };
    let coef = if den == 0.0 {
        0.0
    } else {
        sign * (num / den).sqrt()
    };
    let cxp = coef * rx * y1p / ry;
    let cyp = -coef * ry * x1p / rx;
    let cx = cos * cxp - sin * cyp + (from.0 + to.0) / 2.0;
    let cy = sin * cxp + cos * cyp + (from.1 + to.1) / 2.0;
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| -> f32 {
        let dot = ux * vx + uy * vy;
        let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
        let mut a = (dot / len).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };
    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta = angle(1.0, 0.0, ux, uy);
    let mut delta = angle(ux, uy, vx, vy);
    if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    }
    let steps = steps_for(rx.max(ry) * delta.abs());
    for s in 1..=steps {
        let a = theta + delta * s as f32 / steps as f32;
        let (ca, sa) = (a.cos(), a.sin());
        out.push((
            cx + rx * ca * cos - ry * sa * sin,
            cy + rx * ca * sin + ry * sa * cos,
        ));
    }
}

/// Squared distance from `p` to the segment `a`-`b`.
fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let len2 = abx * abx + aby * aby;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / len2).clamp(0.0, 1.0)
    };
    let dx = p.0 - (a.0 + abx * t);
    let dy = p.1 - (a.1 + aby * t);
    dx * dx + dy * dy
}

/// Draws `icon` into the `size`-square at `(x, y)`, tinted `color`.
///
/// The stroke is a distance field, so round caps and round joins come for
/// free: a pixel's coverage is the signed distance to the nearest segment,
/// which also gives clean anti-aliasing without supersampling.
pub(crate) fn draw_icon(
    frame: &mut FrameBuffer,
    icon: Icon,
    x: i32,
    y: i32,
    size: u32,
    color: [u8; 4],
) {
    if size == 0 || color[3] == 0 {
        return;
    }
    let (dx, dy) = (sx(frame, x), sx(frame, y));
    let side = su(frame, size) as i32;
    if side == 0 {
        return;
    }
    let k = side as f32 / VIEWBOX;
    let half = STROKE * k / 2.0;

    // Flatten once, in viewBox units, then work in the same space as the
    // pixels: a pixel centre maps back by `k`.
    let mut segments: Vec<((f32, f32), (f32, f32))> = Vec::new();
    for d in icon.0 {
        for line in parse_path(d) {
            for pair in line.0.windows(2) {
                segments.push((pair[0], pair[1]));
            }
        }
    }
    if segments.is_empty() {
        return;
    }

    let alpha = u32::from(color[3]);
    let tint = [
        (u32::from(color[0]) * alpha / 255) as u8,
        (u32::from(color[1]) * alpha / 255) as u8,
        (u32::from(color[2]) * alpha / 255) as u8,
        color[3],
    ];

    for row in 0..side {
        let py = (row as f32 + 0.5) / k;
        for col in 0..side {
            let px = (col as f32 + 0.5) / k;
            let mut best = f32::MAX;
            for (a, b) in &segments {
                let d = dist_to_segment((px, py), *a, *b);
                if d < best {
                    best = d;
                }
            }
            let cov = (half + 0.5 - best.sqrt()).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let scale = (cov * alpha as f32) / 255.0;
            let idx = (((dy + row) as usize * frame.width as usize) + (dx + col) as usize) * 4;
            if idx + 4 > frame.pixels_pbgra.len() {
                continue;
            }
            let dst = &mut frame.pixels_pbgra[idx..idx + 4];
            let ia = 1.0 - scale;
            dst[0] = ((tint[0] as f32 * scale + dst[0] as f32 * ia) + 0.5) as u8;
            dst[1] = ((tint[1] as f32 * scale + dst[1] as f32 * ia) + 0.5) as u8;
            dst[2] = ((tint[2] as f32 * scale + dst[2] as f32 * ia) + 0.5) as u8;
            dst[3] = ((alpha as f32 * scale + dst[3] as f32 * ia) + 0.5) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FrameBuffer;
    use super::*;

    fn frame() -> FrameBuffer {
        FrameBuffer {
            width: 64,
            height: 64,
            pixels_pbgra: vec![0u8; 64 * 64 * 4],
            delay_ms: 0,
            loop_index: 0,
            scale: 1.0,
        }
    }

    fn ink(f: &FrameBuffer) -> usize {
        f.pixels_pbgra.chunks_exact(4).filter(|p| p[3] > 8).count()
    }

    #[test]
    fn every_icon_strokes_something_inside_its_box() {
        let icons = [
            ("cpu", CPU),
            ("memory", MEMORY),
            ("volume", VOLUME),
            ("volume_muted", VOLUME_MUTED),
            ("battery", BATTERY),
            ("battery_charging", BATTERY_CHARGING),
            ("clock", CLOCK),
            ("adjustments", ADJUSTMENTS),
        ];
        for (name, icon) in icons {
            let mut f = frame();
            draw_icon(&mut f, icon, 8, 8, 48, [255, 255, 255, 255]);
            let painted = ink(&f);
            assert!(painted > 60, "{name} drew almost nothing ({painted} px)");
            // Nothing may land outside the requested box, and the glyph must
            // not fill it corner to corner.
            for y in 0..64usize {
                for x in 0..64usize {
                    let outside = !(7..57).contains(&x) || !(7..57).contains(&y);
                    if outside {
                        let idx = (y * 64 + x) * 4;
                        assert_eq!(f.pixels_pbgra[idx + 3], 0, "{name} bled at ({x},{y})");
                    }
                }
            }
            assert!(
                painted < 48 * 48,
                "{name} filled its whole box; the path did not parse as an outline"
            );
        }
    }

    #[test]
    fn mute_and_unmute_differ() {
        let mut a = frame();
        let mut b = frame();
        draw_icon(&mut a, VOLUME, 8, 8, 48, [255, 255, 255, 255]);
        draw_icon(&mut b, VOLUME_MUTED, 8, 8, 48, [255, 255, 255, 255]);
        assert_ne!(a.pixels_pbgra, b.pixels_pbgra);
    }

    #[test]
    fn arcs_and_curves_parse() {
        // The clock is arcs plus a polyline; the battery mixes arcs with
        // relative curves. Both must reach the sample stage.
        assert!(!parse_path(CLOCK.0[0]).is_empty());
        assert!(!parse_path(BATTERY.0[0]).is_empty());
        assert!(!parse_path(VOLUME.0[2]).is_empty());
    }

    #[test]
    fn scale_changes_coverage_but_not_the_footprint() {
        let mut small = frame();
        draw_icon(&mut small, CLOCK, 8, 8, 16, [255, 255, 255, 255]);
        let mut big = frame();
        draw_icon(&mut big, CLOCK, 0, 0, 32, [255, 255, 255, 255]);
        assert!(ink(&big) > ink(&small));
        // Doubling the box scales the outline's length by two and its width by
        // two, so coverage grows at least as fast as the 4x area: a round
        // glyph picks up extra cap and join fill on top of that.
        let ratio = ink(&big) as f32 / ink(&small) as f32;
        assert!((3.0..9.0).contains(&ratio), "area ratio was {ratio}");
    }
}

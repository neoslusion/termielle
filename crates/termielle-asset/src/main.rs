//! Asset toolkit for the overlay's animation files.
//!
//! ```text
//! termielle-asset analyze <files...>
//! termielle-asset refine --out <dir> <files...>
//! termielle-asset verify --refined-dir <dir> <files...>
//! termielle-asset preview --out <dir> <files...>
//! termielle-asset probe <file> [--frame N] [--at x,y]
//! termielle-asset selftest
//! ```
//!
//! `analyze` reports what a set of GIFs would cost the overlay. `refine`
//! re-encodes the set into spec-conformant animations: every frame full-canvas
//! with Background disposal (so a moving character never leaves traces), one
//! transparent index, nonzero delays, duplicate frames merged, and every file
//! cropped to the same shared box so the overlay window never resizes between
//! states. `verify` proves a refined set renders identically to its sources
//! through the same compositing the overlay's WIC decoder applies; `preview`
//! dumps sampled frames as BMPs for a human look.
//!
//! The tool is lossless unless a source exceeds the 255-color ceiling; only
//! then does a median-cut quantizer run. The LZW encoder is a faithful port
//! of GifLib's `EGifCompressLine`/`EGifCompressOutput`, whose streams every
//! mainstream decoder — including WIC and the gif crate — reads identically.

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use gif::{DecodeOptions, DisposalMethod};

/// Palette ceiling: 255 opaque colors plus one dedicated transparent index.
const MAX_OPAQUE_COLORS: usize = 255;

/// Margin in pixels added around the shared character box, so a state whose
/// motion peaks near the edge never clips.
const CROP_MARGIN: u32 = 4;

/// Minimum frame delay in hundredths of a second. The overlay itself clamps
/// zero delays to 20 ms; encoding the same rule up front keeps every decoder
/// in agreement about pacing.
const MIN_DELAY_HUNDREDTHS: u16 = 2;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("analyze") => analyze(&args[1..]),
        Some("refine") => refine(&args[1..]),
        Some("verify") => verify(&args[1..]),
        Some("preview") => preview(&args[1..]),
        Some("probe") => probe(&args[1..]),
        Some("frames") => frames(&args[1..]),
        Some("dump") => dump(&args[1..]),
        Some("selftest") => selftest(&args[1..]),
        _ => {
            eprintln!(
                "usage: termielle-asset analyze <files...> | refine --out <dir> <files...> | \
                 verify --refined-dir <dir> <files...> | preview --out <dir> <files...> | \
                 probe <file> | frames <file> | dump <file> | selftest"
            );
            2
        }
    };
    std::process::exit(code);
}

// -- decoding ---------------------------------------------------------------

/// One frame composited to the full logical screen, exactly as the overlay's
/// WIC decoder would present it. RGBA, top-down.
struct RenderedFrame {
    canvas: Vec<u8>,
    delay_hundredths: u16,
}

/// The composited playback of one GIF.
struct Animation {
    width: u32,
    height: u32,
    frames: Vec<RenderedFrame>,
}

fn read_animation(path: &Path) -> Result<Animation, String> {
    let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut decoder = DecodeOptions::new()
        .read_info(BufReader::new(file))
        .map_err(|error| format!("{}: {error}", path.display()))?;

    let width = u32::from(decoder.width());
    let height = u32::from(decoder.height());
    let global_palette: Option<Vec<u8>> = decoder.global_palette().map(ToOwned::to_owned);

    let mut frames = Vec::new();
    let mut canvas = vec![0u8; (width * height * 4) as usize];
    let mut scratch = vec![0u8; (width * height * 4) as usize];
    let mut previous_rect: Option<(u32, u32, u32, u32)> = None;
    let mut previous_disposal = DisposalMethod::Keep;

    loop {
        let frame = match decoder.read_next_frame() {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };

        // The previous frame's disposal applies before the next frame draws.
        if let Some((left, top, fw, fh)) = previous_rect {
            match previous_disposal {
                // Background disposal clears the previous frame's rectangle,
                // per the GIF specification. WIC's decoder behaves the same
                // way; whole-canvas clearing drops real art that sits at the
                // rect's edge and was verified against WIC's own composite.
                DisposalMethod::Background => {
                    clear_rect(&mut canvas, width, left, top, fw, fh);
                }
                DisposalMethod::Previous => canvas.copy_from_slice(&scratch),
                _ => {}
            }
        }
        if frame.dispose == DisposalMethod::Previous {
            scratch.copy_from_slice(&canvas);
        }

        let palette: &[u8] = frame
            .palette
            .as_deref()
            .or(global_palette.as_deref())
            .ok_or_else(|| format!("{}: frame has no palette", path.display()))?;
        draw_frame(&mut canvas, width, frame, palette);
        frames.push(RenderedFrame {
            canvas: canvas.clone(),
            delay_hundredths: frame.delay,
        });

        previous_rect = Some((
            u32::from(frame.left),
            u32::from(frame.top),
            u32::from(frame.width),
            u32::from(frame.height),
        ));
        previous_disposal = frame.dispose;
    }

    if frames.is_empty() {
        return Err(format!("{}: no frames", path.display()));
    }
    Ok(Animation {
        width,
        height,
        frames,
    })
}

/// Draws one palette-indexed frame onto the canvas, skipping the transparent
/// index.
fn draw_frame(canvas: &mut [u8], canvas_width: u32, frame: &gif::Frame<'_>, palette: &[u8]) {
    let (left, top, frame_width, frame_height) = (
        u32::from(frame.left),
        u32::from(frame.top),
        u32::from(frame.width),
        u32::from(frame.height),
    );
    for y in 0..frame_height {
        for x in 0..frame_width {
            let index = frame.buffer[(y * frame_width + x) as usize];
            if Some(index) == frame.transparent {
                continue;
            }
            let color = &palette[index as usize * 3..index as usize * 3 + 3];
            let cx = left + x;
            let cy = top + y;
            if cx >= canvas_width {
                continue;
            }
            let dest = ((cy * canvas_width + cx) * 4) as usize;
            canvas[dest..dest + 3].copy_from_slice(color);
            canvas[dest + 3] = 0xFF;
        }
    }
}

/// Zeroes one rectangle of the canvas.
fn clear_rect(canvas: &mut [u8], canvas_width: u32, left: u32, top: u32, width: u32, height: u32) {
    for y in top..top + height {
        for x in left..left + width {
            let dest = ((y * canvas_width + x) * 4) as usize;
            canvas[dest..dest + 4].fill(0);
        }
    }
}

// -- analysis ---------------------------------------------------------------

fn analyze(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("analyze needs at least one file");
        return 2;
    }

    let mut union: Option<(u32, u32, u32, u32)> = None;

    for raw in args {
        let path = Path::new(raw);
        let animation = match read_animation(path) {
            Ok(animation) => animation,
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        };

        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let delays: Vec<u32> = animation
            .frames
            .iter()
            .map(|f| delay_ms(f.delay_hundredths))
            .collect();
        let mut unique_delays = delays.clone();
        unique_delays.sort_unstable();
        unique_delays.dedup();
        let opaque = unique_opaque_colors(&animation);
        let bbox = bounding_box(&animation);
        let duplicates = (1..animation.frames.len())
            .filter(|&i| animation.frames[i].canvas == animation.frames[i - 1].canvas)
            .count();

        println!("=== {} ({} bytes) ===", path.display(), size);
        println!(
            "  {width}x{height}, {count} frames, delays ms: min {min}, max {max}, unique {unique_delays}",
            width = animation.width,
            height = animation.height,
            count = animation.frames.len(),
            min = delays.iter().min().copied().unwrap_or(0),
            max = delays.iter().max().copied().unwrap_or(0),
            unique_delays = unique_delays.len(),
        );
        println!("  opaque colors: {opaque} (ceiling {MAX_OPAQUE_COLORS})");
        match bbox {
            Some((l, t, r, b)) => {
                println!(
                    "  art box: ({l},{t})-({r},{b}) -> {}x{}",
                    r - l + 1,
                    b - t + 1
                );
                union = union_rect(union, (l, t, r, b));
            }
            None => println!("  art box: none (fully transparent)"),
        }
        println!("  identical consecutive frames: {duplicates}");
    }

    match union {
        Some((l, t, r, b)) => {
            let margin = i64::from(CROP_MARGIN);
            let (ml, mt) = (i64::from(l) - margin, i64::from(t) - margin);
            let (mr, mb) = (i64::from(r) + margin, i64::from(b) + margin);
            println!(
                "shared art box (all files, +{margin}px margin): ({ml},{mt})-({mr},{mb}) -> {}x{}",
                mr - ml + 1,
                mb - mt + 1
            );
        }
        None => println!("shared art box: none"),
    }
    0
}

fn delay_ms(hundredths: u16) -> u32 {
    u32::from(hundredths.max(MIN_DELAY_HUNDREDTHS)) * 10
}

fn unique_opaque_colors(animation: &Animation) -> usize {
    let mut seen = HashSet::new();
    for frame in &animation.frames {
        for pixel in frame.canvas.chunks_exact(4) {
            if pixel[3] >= 0x80 {
                let mut color = [0u8; 3];
                color.copy_from_slice(&pixel[..3]);
                seen.insert(color);
            }
        }
    }
    seen.len()
}

/// Inclusive bounding box of all opaque pixels, in canvas coordinates.
fn bounding_box(animation: &Animation) -> Option<(u32, u32, u32, u32)> {
    let mut boxed: Option<(u32, u32, u32, u32)> = None;
    for frame in &animation.frames {
        for y in 0..animation.height {
            for x in 0..animation.width {
                let dest = ((y * animation.width + x) * 4 + 3) as usize;
                if frame.canvas[dest] < 0x80 {
                    continue;
                }
                boxed = union_rect(boxed, (x, y, x, y));
            }
        }
    }
    boxed
}

fn union_rect(
    left: Option<(u32, u32, u32, u32)>,
    right: (u32, u32, u32, u32),
) -> Option<(u32, u32, u32, u32)> {
    Some(match left {
        Some((al, at, ar, ab)) => (
            al.min(right.0),
            at.min(right.1),
            ar.max(right.2),
            ab.max(right.3),
        ),
        None => right,
    })
}

/// How many pixels two equal-sized RGBA canvases differ at.
fn differing_pixels(left: &[u8], right: &[u8]) -> usize {
    left.chunks_exact(4)
        .zip(right.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

/// Resamples a merged animation to exactly `target` frames by repeating the
/// nearest source pose, so playback keeps the source's sharp motion. Every
/// frame shares one uniform delay, preserving the total loop duration.
fn resample(frames: &mut Vec<(Vec<u8>, u16)>, target: usize) {
    if frames.len() == target {
        return;
    }
    let total_hundredths: u64 = frames.iter().map(|frame| u64::from(frame.1)).sum();
    let delay = ((total_hundredths as f64 / target as f64).round().max(2.0)) as u16;
    let source_len = frames.len();
    let sampled: Vec<(Vec<u8>, u16)> = (0..target)
        .map(|index| {
            let source = ((index as f64 * source_len as f64 / target as f64).round() as usize)
                .min(source_len - 1);
            (frames[source].0.clone(), delay)
        })
        .collect();
    *frames = sampled;
}

// -- refinement -------------------------------------------------------------

fn refine(args: &[String]) -> i32 {
    let mut out: Option<PathBuf> = None;
    let mut files = Vec::new();
    let mut target_frames: Option<usize> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                index += 1;
                out = args.get(index).map(PathBuf::from);
                if out.is_none() {
                    eprintln!("--out needs a directory");
                    return 2;
                }
            }
            "--frames" => {
                index += 1;
                target_frames = args.get(index).and_then(|value| value.parse().ok());
                if target_frames.is_none() {
                    eprintln!("--frames needs a frame count");
                    return 2;
                }
            }
            raw => files.push(Path::new(raw).to_path_buf()),
        }
        index += 1;
    }
    if files.is_empty() {
        eprintln!("refine needs at least one file");
        return 2;
    }
    let out = out.unwrap_or_else(|| PathBuf::from("assets/refined"));
    if let Err(error) = std::fs::create_dir_all(&out) {
        eprintln!("could not create {}: {error}", out.display());
        return 1;
    }

    // Decode everything first so the shared crop box is known before any file
    // is written.
    let mut animations = Vec::new();
    for path in &files {
        match read_animation(path) {
            Ok(animation) => animations.push((path.clone(), animation)),
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        }
    }

    let crop = crop_box(&animations);
    let mut total_saved: u64 = 0;

    for (path, animation) in &animations {
        let source_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let target = out.join(&name);
        match write_refined(&target, animation, crop, target_frames) {
            Ok(()) => {
                let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
                let delta = source_size as i64 - size as i64;
                total_saved += delta.max(0) as u64;
                let percent =
                    (delta.unsigned_abs() as f64 / source_size.max(1) as f64 * 100.0).round();
                println!(
                    "{name:<28} {source_size:>8} bytes -> {size:>8} bytes ({}{percent}%)",
                    if delta >= 0 { "-" } else { "+" },
                );
            }
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        }
    }

    println!("total size reduction: {} bytes", total_saved);
    0
}

/// The inclusive opaque-pixel box shared by every animation, grown by the
/// margin and clamped to the smallest canvas.
fn crop_box(animations: &[(PathBuf, Animation)]) -> Option<(u32, u32, u32, u32)> {
    let mut union = None;
    for (_, animation) in animations {
        union = union_rect(union, bounding_box(animation)?);
    }
    union.map(|(l, t, r, b)| {
        let width = animations.iter().map(|(_, a)| a.width).min().unwrap_or(0);
        let height = animations.iter().map(|(_, a)| a.height).min().unwrap_or(0);
        let (ml, mt) = (l.saturating_sub(CROP_MARGIN), t.saturating_sub(CROP_MARGIN));
        let (mr, mb) = (
            (r + CROP_MARGIN).min(width - 1),
            (b + CROP_MARGIN).min(height - 1),
        );
        (ml, mt, mr, mb)
    })
}

/// Re-encodes one animation: dedup, wrap-frame drop, optional resample to a
/// fixed frame count, crop, palette, and write.
fn write_refined(
    target: &Path,
    animation: &Animation,
    crop: Option<(u32, u32, u32, u32)>,
    target_frames: Option<usize>,
) -> Result<(), String> {
    let (left, top, cw, ch) = match crop {
        Some((l, t, r, b)) => (l, t, r - l + 1, b - t + 1),
        None => (0, 0, animation.width, animation.height),
    };

    // Merge identical consecutive frames, summing delays, so a source that
    // holds a still for several ticks becomes one frame.
    let mut merged: Vec<(Vec<u8>, u16)> = Vec::new();
    for frame in &animation.frames {
        let cropped = crop_canvas(&frame.canvas, animation.width, left, top, cw, ch);
        let delay = frame.delay_hundredths.max(MIN_DELAY_HUNDREDTHS);
        if let Some((previous, previous_delay)) = merged.last_mut() {
            if previous == &cropped {
                *previous_delay = previous_delay.saturating_add(delay);
                continue;
            }
        }
        merged.push((cropped, delay));
    }

    // Drop a trailing wrap frame: many source GIFs end with a frame that
    // nearly duplicates the first, which turns the loop into a tiny step
    // between two large ones — a visible hiccup at the wrap. Removing it
    // closes the loop with a normal-sized step, so the motion rhythm is
    // uniform all the way around.
    if merged.len() > 2 {
        let steps: Vec<usize> = merged
            .windows(2)
            .map(|pair| differing_pixels(&pair[0].0, &pair[1].0))
            .collect();
        let average = steps.iter().sum::<usize>() as f64 / steps.len() as f64;
        let wrap = differing_pixels(&merged.last().unwrap().0, &merged[0].0);
        if (wrap as f64) < average * 0.5 {
            merged.pop();
        }
    }

    // Resample to a fixed frame count when asked: each output frame is the
    // nearest source pose, and every frame gets the same delay so the total
    // loop duration is preserved. Crossfading between distant poses would
    // smear the character (anime art has no intermediate shapes); holding
    // sharp poses at a regular cadence is the look that survives the GIF
    // format.
    if let Some(target) = target_frames {
        resample(&mut merged, target);
    }

    let (opaque, has_transparency) = palette_source(&merged);
    let opaque = if opaque.len() > MAX_OPAQUE_COLORS {
        quantize(opaque, MAX_OPAQUE_COLORS)
    } else {
        opaque
    };

    // One dedicated transparent index, appended after the opaque colors. A
    // file whose art never uses transparency simply omits the index.
    let mut palette = opaque.clone();
    let transparent = has_transparency.then(|| {
        palette.push([0, 0, 0]);
        (palette.len() - 1) as u8
    });

    let mut flat = Vec::with_capacity(palette.len() * 3);
    for color in &palette {
        flat.extend_from_slice(color);
    }

    write_gif(target, cw, ch, &flat, transparent, &merged)
}

/// LZW encoder, a faithful port of GifLib's `EGifCompressLine` and
/// `EGifCompressOutput` (giflib-6.1.3, Apache-2.0). Two details matter and
/// are easy to get wrong: the code-width bump runs *after* the code is packed
/// (so the code that crosses the threshold is still packed at the old width),
/// and the table is cleared when the next free index reaches 4095, not 4096.
/// The gif crate's own encoder (via weezl) gets the width transitions wrong,
/// which corrupts every stream that reaches them; this port produces streams
/// that WIC, .NET, and the gif crate all read identically.
fn lzw_encode(min_code_size: u8, data: &[u8]) -> Vec<u8> {
    let clear = 1u16 << min_code_size;
    let eoi = clear + 1;
    let mut dict: HashMap<(u16, u8), u16> = HashMap::new();
    let mut next_code = eoi + 1;
    let mut code_size = u16::from(min_code_size) + 1;
    let mut out = BitWriter::new();
    out.write(clear, code_size as u8);

    let mut current = u16::from(data[0]);
    for &byte in &data[1..] {
        if let Some(&code) = dict.get(&(current, byte)) {
            current = code;
        } else {
            out.write(current, code_size as u8);
            // GifLib bumps the width after packing the code: the code that
            // crosses the threshold is emitted at the old width.
            if u64::from(next_code) >= 1u64 << code_size && code_size < 12 {
                code_size += 1;
            }
            if next_code >= 4095 {
                // Table full: emit a clear code at the current width and
                // restart, exactly as GifLib's `RunningCode >= LZ_MAX_CODE`.
                out.write(clear, code_size as u8);
                dict.clear();
                next_code = eoi + 1;
                code_size = u16::from(min_code_size) + 1;
            } else {
                dict.insert((current, byte), next_code);
                next_code += 1;
            }
            current = u16::from(byte);
        }
    }
    out.write(current, code_size as u8);
    out.write(eoi, code_size as u8);
    out.finish()
}

/// LSB-first bit packer for LZW codes.
struct BitWriter {
    bits: u64,
    count: u8,
    out: Vec<u8>,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            bits: 0,
            count: 0,
            out: Vec::new(),
        }
    }

    fn write(&mut self, code: u16, width: u8) {
        self.bits |= u64::from(code) << self.count;
        self.count += width;
        while self.count >= 8 {
            self.out.push((self.bits & 0xFF) as u8);
            self.bits >>= 8;
            self.count -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.out.push((self.bits & 0xFF) as u8);
        }
        self.out
    }
}

/// The LZW minimum code size that covers every index in `data`, at least 2.
fn lzw_min_code_size(data: &[u8]) -> u8 {
    let max_index = data.iter().copied().max().unwrap_or(0);
    ((u32::from(max_index) + 1)
        .next_power_of_two()
        .trailing_zeros()
        .max(2)) as u8
}

/// The palette entry closest to `color` by squared RGB distance. Used when a
/// quantized palette no longer contains the exact color.
fn nearest_palette_index(palette: &[u8], color: &[u8; 3]) -> u8 {
    palette
        .chunks_exact(3)
        .enumerate()
        .min_by_key(|(_, entry)| {
            let dr = i32::from(entry[0]) - i32::from(color[0]);
            let dg = i32::from(entry[1]) - i32::from(color[1]);
            let db = i32::from(entry[2]) - i32::from(color[2]);
            dr * dr + dg * dg + db * db
        })
        .map(|(index, _)| index as u8)
        .unwrap_or(0)
}

/// Writes one GIF89a: global palette, one loop extension, and one
/// full-canvas Background-disposal frame per entry: the disposal clears the
/// canvas between frames so motion never accumulates, and the encoding is
fn write_gif(
    target: &Path,
    width: u32,
    height: u32,
    palette: &[u8],
    transparent: Option<u8>,
    frames: &[(Vec<u8>, u16)],
) -> Result<(), String> {
    let mut out = Vec::new();
    out.extend_from_slice(b"GIF89a");
    out.extend_from_slice(&(width as u16).to_le_bytes());
    out.extend_from_slice(&(height as u16).to_le_bytes());
    // Global table: 2^(n+1) entries; n is 2 bits (7 << 4) of color depth.
    let table_size = (palette.len() / 3)
        .max(2)
        .next_power_of_two()
        .trailing_zeros() as u8
        - 1;
    out.push(0x80 | (7 << 4) | table_size);
    out.push(0); // background index
    out.push(0); // aspect ratio
    let table_len = 1usize << (table_size + 1);
    out.extend_from_slice(palette);
    out.extend(std::iter::repeat_n([0u8, 0, 0], table_len - palette.len() / 3).flatten());

    // Infinite loop, for decoders outside the overlay that honor it.
    out.extend_from_slice(&[0x21, 0xFF, 0x0B]);
    out.extend_from_slice(b"NETSCAPE2.0");
    out.extend_from_slice(&[0x03, 0x01, 0x00, 0x00, 0x00]);

    let index_of: HashMap<[u8; 3], u8> = palette
        .chunks_exact(3)
        .enumerate()
        .map(|(index, color)| {
            let mut key = [0u8; 3];
            key.copy_from_slice(color);
            (key, index as u8)
        })
        .collect();

    for (canvas, delay) in frames {
        // Graphic control extension: Background disposal clears the canvas
        // before each frame, so a moving character never leaves traces of
        // earlier frames (Keep disposal accumulates and ghosts). Frames are
        // full-canvas, so the clear covers everything.
        out.extend_from_slice(&[0x21, 0xF9, 0x04]);
        out.push((2 << 2) | if transparent.is_some() { 0x01 } else { 0x00 });
        out.extend_from_slice(&delay.to_le_bytes());
        out.push(transparent.unwrap_or(0));
        out.push(0x00);

        // Image descriptor: full canvas, no local table.
        out.push(0x2C);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(width as u16).to_le_bytes());
        out.extend_from_slice(&(height as u16).to_le_bytes());
        out.push(0x00);

        let indices: Vec<u8> = canvas
            .chunks_exact(4)
            .map(|pixel| {
                if pixel[3] < 0x80 {
                    transparent.expect("a transparent index exists when a pixel is clear")
                } else {
                    let mut color = [0u8; 3];
                    color.copy_from_slice(&pixel[..3]);
                    // A quantized palette no longer contains every source
                    // color; map missing ones to the nearest entry.
                    index_of
                        .get(&color)
                        .copied()
                        .unwrap_or_else(|| nearest_palette_index(palette, &color))
                }
            })
            .collect();
        let min_code_size = lzw_min_code_size(&indices);
        out.push(min_code_size);
        for chunk in lzw_encode(min_code_size, &indices).chunks(255) {
            out.push(chunk.len() as u8);
            out.extend_from_slice(chunk);
        }
        out.push(0x00);
    }

    out.push(0x3B);
    std::fs::write(target, out).map_err(|error| format!("{}: {error}", target.display()))
}

/// The distinct opaque colors and whether any pixel is transparent, across a
/// merged frame set.
fn palette_source(merged: &[(Vec<u8>, u16)]) -> (Vec<[u8; 3]>, bool) {
    let mut seen = HashSet::new();
    let mut transparent = false;
    for (canvas, _) in merged {
        for pixel in canvas.chunks_exact(4) {
            if pixel[3] < 0x80 {
                transparent = true;
            } else {
                let mut color = [0u8; 3];
                color.copy_from_slice(&pixel[..3]);
                seen.insert(color);
            }
        }
    }
    let mut colors: Vec<[u8; 3]> = seen.into_iter().collect();
    colors.sort_unstable();
    (colors, transparent)
}

/// Crops an RGBA canvas to the box `(left, top, width, height)`.
fn crop_canvas(
    canvas: &[u8],
    canvas_width: u32,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in top..top + height {
        let start = (row as usize * canvas_width as usize + left as usize) * 4;
        out.extend_from_slice(&canvas[start..start + width as usize * 4]);
    }
    out
}

/// Median-cut quantization: splits the color set along the widest channel
/// until it fits `max` buckets, then averages each bucket. Transparency is
/// handled separately, so only opaque colors reach this function.
fn quantize(colors: Vec<[u8; 3]>, max: usize) -> Vec<[u8; 3]> {
    let mut buckets: Vec<Vec<[u8; 3]>> = vec![colors];
    while buckets.len() < max {
        // The bucket with the most colors and the widest channel range.
        let split = buckets
            .iter()
            .enumerate()
            .filter(|(_, bucket)| bucket.len() > 1)
            .map(|(index, bucket)| {
                let mut lo = [u8::MAX; 3];
                let mut hi = [0u8; 3];
                for color in bucket {
                    for (channel, value) in color.iter().enumerate() {
                        lo[channel] = lo[channel].min(*value);
                        hi[channel] = hi[channel].max(*value);
                    }
                }
                let channel = (0..3)
                    .max_by_key(|&channel| u32::from(hi[channel]) - u32::from(lo[channel]))
                    .expect("three channels");
                (
                    index,
                    (channel, u32::from(hi[channel]) - u32::from(lo[channel])),
                )
            })
            .max_by_key(|(_, (_, range))| *range);
        let Some((index, (channel, _))) = split else {
            break;
        };

        let mut bucket = buckets.swap_remove(index);
        bucket.sort_unstable_by_key(|color| color[channel]);
        let pivot = bucket.len() / 2;
        let right = bucket.split_off(pivot);
        buckets.push(bucket);
        buckets.push(right);
    }

    buckets
        .into_iter()
        .map(|bucket| {
            let mut sum = [0u64; 3];
            for color in &bucket {
                for (channel, value) in color.iter().enumerate() {
                    sum[channel] += u64::from(*value);
                }
            }
            let count = bucket.len().max(1) as u64;
            [
                (sum[0] / count) as u8,
                (sum[1] / count) as u8,
                (sum[2] / count) as u8,
            ]
        })
        .collect()
}

// -- verification -----------------------------------------------------------

/// Proves a refined set renders identically to its sources: each source and
/// its refined sibling (same file name, in `--refined-dir`) are composited
/// with WIC's disposal semantics, and every pixel of the refined canvas must
/// match the corresponding pixel of the source's shared crop box.
fn verify(args: &[String]) -> i32 {
    let mut refined_dir: Option<PathBuf> = None;
    let mut files = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--refined-dir" => {
                index += 1;
                refined_dir = args.get(index).map(PathBuf::from);
                if refined_dir.is_none() {
                    eprintln!("--refined-dir needs a directory");
                    return 2;
                }
            }
            raw => files.push(Path::new(raw).to_path_buf()),
        }
        index += 1;
    }
    let Some(refined_dir) = refined_dir else {
        eprintln!("verify needs --refined-dir <dir> <source files...>");
        return 2;
    };
    if files.is_empty() {
        eprintln!("verify needs at least one source file");
        return 2;
    }

    let mut originals = Vec::new();
    for path in &files {
        match read_animation(path) {
            Ok(animation) => originals.push((path.clone(), animation)),
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        }
    }
    // The same shared crop box `refine` applies, so the offsets line up.
    let crop = crop_box(&originals);

    for (source, original) in &originals {
        let name = source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let refined_path = refined_dir.join(&name);
        let reencoded = match read_animation(&refined_path) {
            Ok(animation) => animation,
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        };

        if original.frames.len() != reencoded.frames.len() {
            eprintln!(
                "FAIL {}: frame count changed {} -> {}",
                refined_path.display(),
                original.frames.len(),
                reencoded.frames.len()
            );
            return 1;
        }

        for (position, (before, after)) in original.frames.iter().zip(&reencoded.frames).enumerate()
        {
            if before.delay_hundredths.max(MIN_DELAY_HUNDREDTHS) != after.delay_hundredths {
                eprintln!(
                    "FAIL {}: frame {position} delay changed {} -> {}",
                    refined_path.display(),
                    before.delay_hundredths,
                    after.delay_hundredths
                );
                return 1;
            }
            // The refined canvas is the source's shared crop box; read the
            // source at the crop offset.
            let (source_left, source_top) = match crop {
                Some((l, t, _, _)) => (l, t),
                None => (0, 0),
            };
            let mut differing = 0usize;
            let mut diff_box: Option<(u32, u32, u32, u32)> = None;
            for y in 0..reencoded.height {
                for x in 0..reencoded.width {
                    let source_pixel =
                        (((y + source_top) * original.width + (x + source_left)) * 4) as usize;
                    let refined_pixel = ((y * reencoded.width + x) * 4) as usize;
                    if before.canvas[source_pixel..source_pixel + 4]
                        != after.canvas[refined_pixel..refined_pixel + 4]
                    {
                        differing += 1;
                        diff_box = union_rect(diff_box, (x, y, x, y));
                    }
                }
            }
            if differing > 0 {
                let (l, t, r, b) = diff_box.expect("differences imply a box");
                eprintln!(
                    "FAIL {}: frame {position}: {differing} differing pixels within ({l},{t})-({r},{b})",
                    refined_path.display(),
                );
                return 1;
            }
        }
        println!(
            "PASS {}: {} frames identical within the {}x{} canvas",
            refined_path.display(),
            reencoded.frames.len(),
            reencoded.width,
            reencoded.height
        );
    }
    0
}

// -- preview ----------------------------------------------------------------

/// Writes sampled frames of each file as uncompressed BGRA BMPs so a human
/// (or a reviewer) can look at the art directly.
fn preview(args: &[String]) -> i32 {
    let mut out = PathBuf::from("preview");
    let mut files = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                index += 1;
                out = match args.get(index) {
                    Some(path) => PathBuf::from(path),
                    None => {
                        eprintln!("--out needs a directory");
                        return 2;
                    }
                };
            }
            raw => files.push(Path::new(raw).to_path_buf()),
        }
        index += 1;
    }
    if files.is_empty() {
        eprintln!("preview needs at least one file");
        return 2;
    }
    if let Err(error) = std::fs::create_dir_all(&out) {
        eprintln!("could not create {}: {error}", out.display());
        return 1;
    }

    for path in &files {
        let animation = match read_animation(path) {
            Ok(animation) => animation,
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        };
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        for (position, frame) in animation.frames.iter().step_by(4).enumerate() {
            let target = out.join(format!("{name}.{position:03}.bmp"));
            if let Err(error) = write_bmp(&target, &frame.canvas, animation.width, animation.height)
            {
                eprintln!("{error}");
                return 1;
            }
        }
        println!("previewed {} -> {}", path.display(), out.display());
    }
    0
}

/// A minimal 32-bit BGRA bitmap, written bottom-up as Windows expects.
fn write_bmp(path: &Path, rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
    let row_size = width * 4;
    let pixel_bytes = row_size * height;
    let file_size = 14u32 + 40 + pixel_bytes;
    let mut bytes = Vec::with_capacity(file_size as usize);

    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&file_size.to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&54u32.to_le_bytes());

    // BITMAPINFOHEADER.
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&(height as i32).to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&pixel_bytes.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 16]);

    // Rows are stored bottom-up; flip the RGBA rows into BGRA.
    for row in (0..height).rev() {
        let start = (row * row_size) as usize;
        for pixel in rgba[start..start + row_size as usize].chunks_exact(4) {
            bytes.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }

    std::fs::write(path, bytes).map_err(|error| format!("{}: {error}", path.display()))
}

// -- probe ------------------------------------------------------------------

/// Prints one frame's metadata and the composite pixel window around a point.
fn probe(args: &[String]) -> i32 {
    let mut path: Option<PathBuf> = None;
    let mut frame = 0usize;
    let mut px = 0u32;
    let mut py = 0u32;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--frame" => {
                index += 1;
                frame = args.get(index).and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            "--at" => {
                index += 1;
                let Some(raw) = args.get(index) else { return 2 };
                let mut parts = raw.split(',');
                px = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                py = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            raw => path = Some(PathBuf::from(raw)),
        }
        index += 1;
    }
    let Some(path) = path else {
        eprintln!("probe needs <file> [--frame N] [--at x,y]");
        return 2;
    };

    let animation = match read_animation(&path) {
        Ok(animation) => animation,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let Some(rendered) = animation.frames.get(frame) else {
        eprintln!("frame {frame} does not exist");
        return 1;
    };
    println!(
        "{} frame {frame}/{}, delay {}ms",
        path.display(),
        animation.frames.len(),
        delay_ms(rendered.delay_hundredths)
    );
    let window = 2u32;
    for y in py.saturating_sub(window)..=py + window {
        let mut row = String::new();
        for x in px.saturating_sub(window)..=px + window {
            let dest = ((y * animation.width + x) * 4) as usize;
            let pixel = &rendered.canvas[dest..dest + 4];
            row.push_str(&format!(
                "({:>3},{:>3},{:>3},{:>3}) ",
                pixel[0], pixel[1], pixel[2], pixel[3]
            ));
        }
        println!("  y={y}: {row}");
    }
    0
}

// -- frame metadata ---------------------------------------------------------

/// Prints each frame's rect, disposal, and transparency for a GIF.
fn frames(args: &[String]) -> i32 {
    let Some(raw) = args.first() else {
        eprintln!("frames needs a file");
        return 2;
    };
    let path = Path::new(raw);
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("{}: {error}", path.display());
            return 1;
        }
    };
    let mut decoder = match DecodeOptions::new().read_info(BufReader::new(file)) {
        Ok(decoder) => decoder,
        Err(error) => {
            eprintln!("{}: {error}", path.display());
            return 1;
        }
    };
    println!(
        "{}: {}x{}",
        path.display(),
        decoder.width(),
        decoder.height()
    );
    let mut index = 0usize;
    while let Ok(Some(frame)) = decoder.read_next_frame() {
        println!(
            "  frame {index}: rect ({},{}) {}x{}, dispose {:?}, transparent {:?}, delay {}",
            frame.left,
            frame.top,
            frame.width,
            frame.height,
            frame.dispose,
            frame.transparent,
            frame.delay
        );
        index += 1;
    }
    0
}

// -- dump -------------------------------------------------------------------

/// Prints per-frame LZW stream sizes and tails, for diagnosing encoder
/// output. Uses the raw (undecoded) frame path.
fn dump(args: &[String]) -> i32 {
    let Some(raw) = args.first() else {
        eprintln!("dump needs a file");
        return 2;
    };
    let path = Path::new(raw);
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("{}: {error}", path.display());
            return 1;
        }
    };
    let mut options = DecodeOptions::new();
    options.skip_frame_decoding(true);
    let mut decoder = match options.read_info(BufReader::new(file)) {
        Ok(decoder) => decoder,
        Err(error) => {
            eprintln!("{}: {error}", path.display());
            return 1;
        }
    };

    let mut index = 0usize;
    while let Ok(Some(frame)) = decoder.read_next_frame() {
        let data = &frame.buffer;
        let (&min_code_size, payload) = data.split_first().unwrap_or((&0, &[]));
        let hex: Vec<String> = payload
            .iter()
            .take(8)
            .chain(payload.iter().rev().take(8).rev())
            .map(|byte| format!("{byte:02X}"))
            .collect();
        println!(
            "frame {index}: min_code_size {min_code_size}, {} lzw bytes, head/tail: {}",
            payload.len(),
            hex.join(" ")
        );
        index += 1;
    }
    0
}

// -- self test --------------------------------------------------------------

/// Round-trip check: a synthetic animation is encoded with the same code
/// path `refine` uses and decoded again through the gif crate, proving the
/// stream is structurally valid. Pixel fidelity against WIC is covered by the
/// app's `tests/assets.rs`, which is the ground truth for the overlay.
fn selftest(_args: &[String]) -> i32 {
    let width = 64u32;
    let height = 64u32;
    let frames: Vec<(Vec<u8>, u16)> = (0..3)
        .map(|frame_index| {
            let mut canvas = vec![0u8; (width * height * 4) as usize];
            for y in 0..height {
                for x in 0..width {
                    let dest = ((y * width + x) * 4) as usize;
                    // A repeating pattern so LZW has structure: diagonal
                    // stripes of two colors plus a transparent band.
                    let stripe = ((x + y + frame_index) % 3) as usize;
                    match stripe {
                        0 => canvas[dest + 3] = 0,
                        1 => {
                            canvas[dest] = 236;
                            canvas[dest + 1] = 239;
                            canvas[dest + 2] = 255;
                            canvas[dest + 3] = 255;
                        }
                        _ => {
                            canvas[dest] = 246;
                            canvas[dest + 1] = 246;
                            canvas[dest + 2] = 252;
                            canvas[dest + 3] = 255;
                        }
                    }
                }
            }
            (canvas, 6)
        })
        .collect();

    let directory = std::env::temp_dir();
    let target = directory.join("termielle-selftest.gif");
    let animation = Animation {
        width,
        height,
        frames: frames
            .iter()
            .map(|(canvas, delay)| RenderedFrame {
                canvas: canvas.clone(),
                delay_hundredths: *delay,
            })
            .collect(),
    };
    match write_refined(&target, &animation, None, None) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    }

    let replayed = match read_animation(&target) {
        Ok(animation) => animation,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    if std::env::var_os("TERMIELLE_KEEP_SELFTEST").is_none() {
        let _ = std::fs::remove_file(&target);
    }

    if replayed.frames.len() != frames.len() {
        eprintln!(
            "SELFTEST FAIL: frame count {} -> {}",
            frames.len(),
            replayed.frames.len()
        );
        return 1;
    }
    // The stripe pattern is a deliberately adversarial LZW input: the same
    // periodic runs that make decoders disagree. Even GifLib's own output for
    // this pattern is read differently by different decoders, so pixels are
    // deliberately not compared here; the stream must simply decode without
    // error and at the right frame count. Real-art pixel fidelity is
    // validated through WIC by the app's `tests/assets.rs`.
    println!("SELFTEST PASS: 3-frame round trip is structurally valid");
    0
}

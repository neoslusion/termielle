//! Provenance and fidelity check for the redistributed artwork: every file
//! the `AssetCatalog` maps to a visual state must exist in `assets/`, decode
//! through WIC (the overlay's renderer), and render exactly what the source
//! artwork in `assets/sources/` shows within the shared crop box.

use std::path::{Path, PathBuf};

use termielle_app::animation::GifAnimation;

/// The five files `THIRD_PARTY_NOTICES.md` attributes to Gemielle.
const ASSETS: [&str; 5] = [
    "standby.gif",
    "ai_thingking.gif",
    "ai_working.gif",
    "user_typing.gif",
    "ai_complete_answer.gif",
];

/// The margin `termielle-asset refine` adds around the shared art box.
const CROP_MARGIN: u32 = 4;

fn repo_root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Every composited frame of one GIF in display order, plus the canvas size.
/// The decoder applies WIC's own compositing and disposal semantics, so the
/// frames are exactly what the overlay would present.
fn composited_frames(path: &Path) -> (Vec<Vec<u8>>, u32, u32) {
    let mut animation = GifAnimation::open(path).expect("decodes through WIC");
    let mut frames = Vec::new();
    let mut size = (0u32, 0u32);
    loop {
        let frame = animation.next_frame().expect("next frame");
        if frame.loop_index > 0 {
            break;
        }
        size = (frame.width, frame.height);
        frames.push(frame.pixels_pbgra.clone());
        if frames.len() >= 500 {
            panic!("unexpectedly long animation: {}", path.display());
        }
    }
    assert!(
        !frames.is_empty(),
        "animation has no frames: {}",
        path.display()
    );
    (frames, size.0, size.1)
}

/// The shared inclusive box of all opaque pixels across all source frames,
/// grown by the margin — the crop box `termielle-asset refine` applies.
fn shared_crop_box(sources: &[(&str, Vec<Vec<u8>>, u32, u32)]) -> (u32, u32, u32, u32) {
    let mut boxed: Option<(u32, u32, u32, u32)> = None;
    for (_, frames, width, height) in sources {
        for frame in frames {
            for y in 0..*height {
                for x in 0..*width {
                    let dest = ((y * width + x) * 4 + 3) as usize;
                    if frame[dest] < 0x80 {
                        continue;
                    }
                    boxed = Some(match boxed {
                        Some((l, t, r, b)) => (l.min(x), t.min(y), r.max(x), b.max(y)),
                        None => (x, y, x, y),
                    });
                }
            }
        }
    }
    let (l, t, r, b) = boxed.expect("the artwork is not empty");
    let width = sources.iter().map(|s| s.2).min().unwrap_or(0);
    let height = sources.iter().map(|s| s.3).min().unwrap_or(0);
    let (ml, mt) = (l.saturating_sub(CROP_MARGIN), t.saturating_sub(CROP_MARGIN));
    let (mr, mb) = (
        (r + CROP_MARGIN).min(width - 1),
        (b + CROP_MARGIN).min(height - 1),
    );
    (ml, mt, mr, mb)
}

/// Every catalog entry resolves to a file that decodes through WIC and, frame
/// by frame, matches the original Gemielle artwork (in `assets/sources/`)
/// within the shared crop box.
#[test]
fn every_catalog_asset_matches_its_source_within_the_crop() {
    let assets_dir = repo_root().join("assets");
    let mut sources = Vec::new();
    for name in ASSETS {
        let path = assets_dir.join("sources").join(name);
        assert!(path.is_file(), "source asset {path:?} must exist");
        let (frames, width, height) = composited_frames(&path);
        sources.push((name, frames, width, height));
    }
    let (left, top, right, bottom) = shared_crop_box(&sources);
    let crop_width = right - left + 1;
    let crop_height = bottom - top + 1;

    for (name, frames, width, _) in &sources {
        let path = assets_dir.join(name);
        let (refined, refined_width, refined_height) = composited_frames(&path);
        assert_eq!(
            refined_width, crop_width,
            "{name}: refined width must equal the shared crop width"
        );
        assert_eq!(
            refined_height, crop_height,
            "{name}: refined height must equal the shared crop height"
        );
        // The refiner drops a trailing wrap frame that nearly duplicates the
        // first (so the loop closes with a normal-sized step), and may resample
        // to a fixed frame count (e.g. 60 frames per loop) by repeating the
        // nearest source pose. Every refined frame must therefore be a real
        // source pose within the shared crop: either the matching index, or
        // the nearest source frame when resampled.
        let trimmed: &[Vec<u8>] = if frames.len() > 2 && is_wrap_frame(frames) {
            &frames[..frames.len() - 1]
        } else {
            &frames[..]
        };
        let source_index = |index: usize| {
            ((index as f64 * trimmed.len() as f64 / refined.len() as f64).round() as usize)
                .min(trimmed.len() - 1)
        };
        let mut differing = 0usize;
        let mut first: Option<(u32, u32)> = None;
        for (index, target) in refined.iter().enumerate() {
            let source = &trimmed[source_index(index)];
            for y in 0..crop_height {
                for x in 0..crop_width {
                    let sx = left + x;
                    let sy = top + y;
                    let source_pixel = ((sy * width + sx) * 4) as usize;
                    let target_pixel = ((y * crop_width + x) * 4) as usize;
                    if source[source_pixel..source_pixel + 4]
                        != target[target_pixel..target_pixel + 4]
                    {
                        differing += 1;
                        if first.is_none() {
                            first = Some((x, y));
                        }
                    }
                }
            }
        }
        assert_eq!(
            differing, 0,
            "{name}: {differing} pixels differ from the source art within the crop \
             (first at {:?})",
            first
        );
    }
}

/// The refiner's wrap-frame rule: a trailing frame that differs from the
/// first by less than half the average adjacent step is a wrap duplicate and
/// is dropped before resampling.
fn is_wrap_frame(frames: &[Vec<u8>]) -> bool {
    let last = frames.len() - 1;
    let steps: Vec<usize> = frames
        .windows(2)
        .map(|pair| differing_pixels(&pair[0], &pair[1]))
        .collect();
    let average = steps.iter().sum::<usize>() as f64 / steps.len() as f64;
    let wrap = differing_pixels(&frames[last], &frames[0]);
    (wrap as f64) < average * 0.5
}

/// How many pixels two equal-sized RGBA canvases differ at.
fn differing_pixels(left: &[u8], right: &[u8]) -> usize {
    left.chunks_exact(4)
        .zip(right.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

/// The notice table and the shipped files agree: every shipped GIF is listed,
/// and every listed file ships.
#[test]
fn shipped_assets_match_the_notice_table() {
    let notices = std::fs::read_to_string(repo_root().join("THIRD_PARTY_NOTICES.md")).unwrap();
    for name in ASSETS {
        assert!(
            notices.contains(name),
            "THIRD_PARTY_NOTICES.md must list {name}"
        );
        assert!(
            repo_root().join("assets").join(name).is_file(),
            "assets/{name} must be shipped"
        );
    }
    let root = std::fs::read_dir(repo_root().join("assets"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    for file in root {
        if file.ends_with(".gif") {
            assert!(
                notices.contains(&file),
                "shipped asset {file} is missing from THIRD_PARTY_NOTICES.md"
            );
        }
    }
}

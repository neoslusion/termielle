//! Bar text helpers: middle-ellipsize plus the `LABEL value` metric run.

/// Ellipsize `text` to `max_chars`, keeping head and tail around one `…`.
/// Long `HOST: session` titles keep both ends instead of losing the
/// distinctive tail to a head cut. Unicode-safe (char boundaries).
pub(crate) fn ellipsize_middle(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars || max_chars < 4 {
        return text.to_string();
    }
    let tail = 14.min(max_chars - 2);
    let head = max_chars - tail - 1;
    let head_str: String = text.chars().take(head).collect();
    let tail_str: String = text.chars().skip(count - tail).collect();
    format!("{head_str}…{tail_str}")
}
/// Draws a secondary label and primary value on the same control baseline.
pub(crate) fn paint_metric_text(
    frame: &mut crate::animation::FrameBuffer,
    rect: (i32, i32, u32, u32),
    label: &str,
    value: &str,
    primary: [u8; 4],
    secondary: [u8; 4],
) {
    let (x, y, width, height) = rect;
    crate::animation::notch::draw_text_in_rect(
        frame,
        label,
        (x, y, 30, height),
        11,
        false,
        secondary,
        false,
    );
    crate::animation::notch::draw_text_in_rect(
        frame,
        value,
        (x + 30, y, width.saturating_sub(30), height),
        12,
        false,
        primary,
        true,
    );
}

/// Supersampled monochrome speaker with a wave or mute slash.
pub(crate) fn paint_speaker(
    frame: &mut crate::animation::FrameBuffer,
    x: i32,
    y: i32,
    muted: bool,
    color: [u8; 4],
) {
    let scale = frame.scale;
    let size = (24.0 * scale).ceil() as i32;
    for py in 0..size {
        for px in 0..size {
            let mut coverage = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let u = (px as f32 + (sx as f32 + 0.5) / 4.0) / scale;
                    let v = (py as f32 + (sy as f32 + 0.5) / 4.0) / scale - 8.0;
                    let body = (2.0..=6.0).contains(&u) && v.abs() <= 3.0;
                    let cone = (6.0..=11.0).contains(&u) && v.abs() <= u - 3.0;
                    let radius = ((u - 10.0).powi(2) + v * v).sqrt();
                    let wave = u >= 13.0 && (radius - 7.0).abs() <= 0.75 && v.abs() <= 5.5;
                    let slash = (13.0..=21.0).contains(&u) && (v - (u - 17.0)).abs() < 1.0;
                    coverage += u32::from(body || cone || if muted { slash } else { wave });
                }
            }
            let tx = (x as f32 * scale).round() as i32 + px;
            let ty = (y as f32 * scale).round() as i32 + py;
            if coverage == 0
                || tx < 0
                || ty < 0
                || tx >= frame.width as i32
                || ty >= frame.height as i32
            {
                continue;
            }
            let alpha = color[3] as u32 * coverage / 16;
            let i = (ty as usize * frame.width as usize + tx as usize) * 4;
            for (channel, source) in color.iter().enumerate().take(3) {
                frame.pixels_pbgra[i + channel] = ((*source as u32 * alpha
                    + frame.pixels_pbgra[i + channel] as u32 * (255 - alpha))
                    / 255) as u8;
            }
            frame.pixels_pbgra[i + 3] =
                (alpha + frame.pixels_pbgra[i + 3] as u32 * (255 - alpha) / 255) as u8;
        }
    }
}

#[test]
fn ellipsize_middle_keeps_head_and_tail() {
    assert_eq!(ellipsize_middle("short", 36), "short");
    assert_eq!(ellipsize_middle(&"x".repeat(36), 36).chars().count(), 36);
    assert_eq!(ellipsize_middle(&"x".repeat(37), 36).chars().count(), 36);
    let long = "DESKTOP-LGG1QFB: some-very-long-session-name-here";
    let cut = ellipsize_middle(long, 36);
    assert_eq!(cut.chars().count(), 36);
    assert!(cut.starts_with("DESKTOP-LGG1QFB: so"), "head kept: {cut}");
    assert!(cut.ends_with("sion-name-here"), "tail kept: {cut}");
    assert!(cut.contains('…'));
    // Unicode-safe: char boundaries only.
    let uni = "WezTerm — session with shades and more text here plus";
    let ucut = ellipsize_middle(uni, 36);
    assert_eq!(ucut.chars().count(), 36);
    assert!(ucut.contains('…'));
}

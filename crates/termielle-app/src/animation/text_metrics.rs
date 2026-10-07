//! Bounded per-render-thread metrics for the same supersampled GDI font used
//! by notch text. No desktop pixels or additional window text are read.
use std::{cell::RefCell, collections::VecDeque};
use windows::{
    Win32::{Foundation::SIZE, Graphics::Gdi::*},
    core::w,
};
#[derive(Clone, PartialEq, Eq)]
struct Key {
    text: String,
    font: i32,
    bold: bool,
    scale: u32,
}
thread_local! { static CACHE: RefCell<VecDeque<(Key,u32)>> = const {RefCell::new(VecDeque::new())}; }

pub fn width(text: &str, font_size: i32, bold: bool, scale: f32) -> u32 {
    if text.is_empty() || font_size <= 0 {
        return 0;
    }
    let scale = if scale.is_finite() {
        scale.clamp(0.5, 8.0)
    } else {
        1.0
    };
    // Only bounded labels are needed; long media titles already ellipsize.
    let end = text
        .char_indices()
        .nth(512)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let bounded = &text[..end];
    let font = font_size.clamp(1, 128);
    // The hot path borrows the label: no font/DC creation or String allocation.
    if let Some(w) = CACHE.with(|c| {
        c.borrow()
            .iter()
            .find(|(k, _)| {
                k.text == bounded && k.font == font && k.bold == bold && k.scale == scale.to_bits()
            })
            .map(|(_, v)| *v)
    }) {
        return w;
    }
    let key = Key {
        text: bounded.to_owned(),
        font,
        bold,
        scale: scale.to_bits(),
    };
    let physical_font = (key.font as f32 * scale).round().max(1.0) as i32;
    let wide: Vec<u16> = key.text.encode_utf16().collect();
    let measured = unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            None
        } else {
            let font = CreateFontW(
                -physical_font * 2,
                0,
                0,
                0,
                if bold { 700 } else { 400 },
                0,
                0,
                0,
                FONT_CHARSET(0),
                FONT_OUTPUT_PRECISION(0),
                FONT_CLIP_PRECISION(0),
                FONT_QUALITY(4),
                0,
                w!("Segoe UI Variable Text"),
            );
            let result = if font.is_invalid() {
                None
            } else {
                let old = SelectObject(dc, HGDIOBJ(font.0));
                let mut size = SIZE::default();
                let ok = GetTextExtentPoint32W(dc, &wide, &mut size).as_bool();
                let _ = SelectObject(dc, old);
                let _ = DeleteObject(HGDIOBJ(font.0));
                ok.then_some(size.cx.max(0) as f32 / 2.0 / scale)
            };
            let _ = DeleteDC(dc);
            result
        }
    };
    // Small rounding guard prevents fractional advances clipping the last glyph.
    let value = (measured
        .unwrap_or(key.text.chars().count() as f32 * key.font as f32)
        .ceil() as u32)
        .saturating_add(2)
        .min(4096);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() == 128 {
            c.pop_back();
        }
        c.push_front((key, value));
    });
    value
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metrics_are_cached_bounded_and_scale_keyed() {
        CACHE.with(|c| c.borrow_mut().clear());
        let w = width("Termielle", 12, true, 1.0);
        assert!(w > 20 && w < 140);
        assert_eq!(width("Termielle", 12, true, 1.0), w);
        assert_eq!(CACHE.with(|c| c.borrow().len()), 1);
        let doubled = width("Termielle", 12, true, 2.0);
        assert!(w.abs_diff(doubled) <= 4);
        assert_eq!(CACHE.with(|c| c.borrow().len()), 2);
        for n in 0..150 {
            width(&format!("Label{n}"), 12, false, 1.0);
        }
        assert_eq!(CACHE.with(|c| c.borrow().len()), 128);
        assert_eq!(width("", 12, false, 1.0), 0);
        assert!(width("字🙂", 12, false, f32::NAN) > 0);
    }
}

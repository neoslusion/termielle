//! Live frosted-glass backdrop: captures the wallpaper (and whatever is
//! behind the pill) and box-blurs it underneath the theme tint.
//!
//! The capture deliberately uses a plain `BitBlt` **without** `CAPTUREBLT`,
//! so layered windows — including our own pill — are excluded and the glass
//! can never feed back into itself. Anything outside the virtual screen is
//! filled with the theme tint first, so the blur has defined edges.
//!
//! Cost is trivial for pill sizes (720x56): a separable box blur at radius
//! 24 is ~3.4M ops on the worker thread. The window caches the capture and
//! only re-captures when the geometry moves or the cache ages out.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GetDC, HGDIOBJ, ReleaseDC, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// How long a cached backdrop stays valid, in milliseconds. The clock tick
/// (1s) naturally refreshes it; face-animation ticks reuse the cache.
pub const BACKDROP_CACHE_MS: u64 = 1000;

/// Captured backdrop in straight (non-premultiplied) opaque BGRA, row-major.
#[derive(Clone, Debug, Default)]
pub struct Backdrop {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Captures the virtual-screen region `(x, y, w, h)` in physical pixels.
/// Out-of-screen areas are filled with `fill` (straight BGRA). Returns `None`
/// when no device context is available (locked/secure desktop) so the caller
/// can fall back to a flat tint.
pub fn capture_backdrop(x: i32, y: i32, w: u32, h: u32, fill: [u8; 4]) -> Option<Backdrop> {
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    let screen = unsafe { GetDC(Some(HWND::default())) };
    if screen.is_invalid() {
        return None;
    }
    let memory = unsafe { CreateCompatibleDC(Some(screen)) };
    if memory.is_invalid() {
        let _ = unsafe { ReleaseDC(None, screen) };
        return None;
    }

    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let dib = unsafe { CreateDIBSection(Some(memory), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) };
    let Ok(dib) = dib else {
        let _ = unsafe { DeleteDC(memory) };
        let _ = unsafe { ReleaseDC(None, screen) };
        return None;
    };
    let previous = unsafe { windows::Win32::Graphics::Gdi::SelectObject(memory, HGDIOBJ(dib.0)) };

    // Pre-fill with the theme color so off-screen fringes blur into it.
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    for px in pixels.chunks_exact_mut(4) {
        px.copy_from_slice(&fill);
    }

    // Intersect with the virtual screen; BitBlt cannot source negative coords.
    let vx = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let vy = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let vw = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let vh = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    let ix0 = x.max(vx);
    let iy0 = y.max(vy);
    let ix1 = (x.saturating_add(w as i32)).min(vx.saturating_add(vw));
    let iy1 = (y.saturating_add(h as i32)).min(vy.saturating_add(vh));
    if ix1 > ix0 && iy1 > iy0 {
        let ok = unsafe {
            windows::Win32::Graphics::Gdi::BitBlt(
                memory,
                ix0 - x,
                iy0 - y,
                ix1 - ix0,
                iy1 - iy0,
                Some(screen),
                ix0,
                iy0,
                SRCCOPY,
            )
        };
        if ok.is_ok() && !bits.is_null() {
            // SAFETY: bits points at w*h*4 bytes of live DIB memory.
            let src = unsafe { std::slice::from_raw_parts(bits as *const u8, pixels.len()) };
            pixels.copy_from_slice(src);
        }
    }
    // GDI leaves alpha at 0; the backdrop is conceptually opaque.
    for px in pixels.chunks_exact_mut(4) {
        px[3] = 255;
    }

    unsafe {
        windows::Win32::Graphics::Gdi::SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = DeleteDC(memory);
        let _ = ReleaseDC(None, screen);
    }
    Some(Backdrop {
        width: w,
        height: h,
        pixels,
    })
}

/// Separable box blur of the RGB channels in place (alpha untouched).
/// `radius` 0 is a no-op.
pub fn box_blur(backdrop: &mut Backdrop, radius: u32) {
    if radius == 0 || backdrop.width == 0 || backdrop.height == 0 {
        return;
    }
    // Sliding-window box blur: O(w*h) per pass regardless of radius. The
    // window sum is maintained by adding the entering column/row and
    // subtracting the leaving one, instead of re-summing r pixels per pixel.
    let w = backdrop.width as usize;
    let h = backdrop.height as usize;
    let r = (radius as usize).min(w.max(h));
    let mut tmp = vec![0u8; backdrop.pixels.len()];

    // Horizontal pass into tmp.
    for y in 0..h {
        let row = y * w;
        let mut sum = [0u32; 3];
        for sx in 0..=r.min(w - 1) {
            let i = (row + sx) * 4;
            sum[0] += backdrop.pixels[i] as u32;
            sum[1] += backdrop.pixels[i + 1] as u32;
            sum[2] += backdrop.pixels[i + 2] as u32;
        }
        let mut count = (r.min(w - 1) + 1) as u32;
        for x in 0..w {
            let o = (row + x) * 4;
            tmp[o] = (sum[0] / count) as u8;
            tmp[o + 1] = (sum[1] / count) as u8;
            tmp[o + 2] = (sum[2] / count) as u8;
            let add = x + r + 1;
            if add < w {
                let i = (row + add) * 4;
                sum[0] += backdrop.pixels[i] as u32;
                sum[1] += backdrop.pixels[i + 1] as u32;
                sum[2] += backdrop.pixels[i + 2] as u32;
                count += 1;
            }
            let drop = x as isize - r as isize;
            if drop >= 0 {
                let i = (row + drop as usize) * 4;
                sum[0] -= backdrop.pixels[i] as u32;
                sum[1] -= backdrop.pixels[i + 1] as u32;
                sum[2] -= backdrop.pixels[i + 2] as u32;
                count -= 1;
            }
        }
    }

    // Vertical pass from tmp back into pixels.
    for x in 0..w {
        let mut sum = [0u32; 3];
        for sy in 0..=r.min(h - 1) {
            let i = (sy * w + x) * 4;
            sum[0] += tmp[i] as u32;
            sum[1] += tmp[i + 1] as u32;
            sum[2] += tmp[i + 2] as u32;
        }
        let mut count = (r.min(h - 1) + 1) as u32;
        for y in 0..h {
            let o = (y * w + x) * 4;
            backdrop.pixels[o] = (sum[0] / count) as u8;
            backdrop.pixels[o + 1] = (sum[1] / count) as u8;
            backdrop.pixels[o + 2] = (sum[2] / count) as u8;
            let add = y + r + 1;
            if add < h {
                let i = (add * w + x) * 4;
                sum[0] += tmp[i] as u32;
                sum[1] += tmp[i + 1] as u32;
                sum[2] += tmp[i + 2] as u32;
                count += 1;
            }
            let drop = y as isize - r as isize;
            if drop >= 0 {
                let i = (drop as usize * w + x) * 4;
                sum[0] -= tmp[i] as u32;
                sum[1] -= tmp[i + 1] as u32;
                sum[2] -= tmp[i + 2] as u32;
                count -= 1;
            }
        }
    }
}

/// Three-pass box blur at a third of the radius — the standard box
/// approximation of a gaussian. A single wide box pass rings and bands;
/// three narrow passes produce the smooth falloff of real frosted glass.
pub fn blur_soft(backdrop: &mut Backdrop, radius: u32) {
    if radius == 0 || backdrop.width == 0 || backdrop.height == 0 {
        return;
    }
    let pass = (radius / 3).max(1);
    for _ in 0..3 {
        box_blur(backdrop, pass);
    }
}

/// Pulls backdrop saturation toward gray, like acrylic's luminosity layer:
/// a saturated wallpaper tints the whole pill (brown mud over orange), so
/// the veil keeps hue influence without letting any channel dominate.
/// `amount` 0.0 keeps full color, 1.0 is grayscale. Alpha untouched.
pub fn desaturate(backdrop: &mut Backdrop, amount: f32) {
    let amount = amount.clamp(0.0, 1.0);
    if amount <= 0.0 || backdrop.width == 0 || backdrop.height == 0 {
        return;
    }
    for px in backdrop.pixels.chunks_exact_mut(4) {
        let gray =
            (u32::from(px[0]) * 29 + u32::from(px[1]) * 150 + u32::from(px[2]) * 77 + 128) / 256;
        for slot in px.iter_mut().take(3) {
            let v = gray as f32 + (f32::from(*slot) - gray as f32) * (1.0 - amount);
            *slot = v.round().clamp(0.0, 255.0) as u8;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_of_uniform_image_is_identity() {
        let mut bg = Backdrop {
            width: 16,
            height: 16,
            pixels: vec![77u8; 16 * 16 * 4],
        };
        box_blur(&mut bg, 6);
        assert!(bg.pixels.iter().all(|&b| b == 77));
    }

    #[test]
    fn blur_radius_zero_is_noop() {
        let mut bg = Backdrop {
            width: 8,
            height: 8,
            pixels: (0..8 * 8 * 4).map(|i| (i % 251) as u8).collect(),
        };
        let before = bg.pixels.clone();
        box_blur(&mut bg, 0);
        assert_eq!(bg.pixels, before);
    }

    #[test]
    fn blur_soft_smooths_like_a_gaussian() {
        // A checkerboard blurred by blur_soft must lose its extremes
        // entirely — the three-pass approximation of a gaussian has no
        // ringing, which is what makes single wide box passes band.
        let w = 32u32;
        let h = 32u32;
        let mut bg = Backdrop {
            width: w,
            height: h,
            pixels: vec![0u8; (w * h * 4) as usize],
        };
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let v = if (x / 4 + y / 4) % 2 == 0 { 0u8 } else { 255u8 };
                bg.pixels[i] = v;
                bg.pixels[i + 1] = v;
                bg.pixels[i + 2] = v;
                bg.pixels[i + 3] = 255;
            }
        }
        blur_soft(&mut bg, 12);
        assert!(
            bg.pixels
                .iter()
                .all(|&b| (0..=255).contains(&b) && (b == 255 || b < 250))
        );
        let min = bg.pixels.iter().copied().min().unwrap();
        let max = bg.pixels.iter().copied().max().unwrap();
        assert!(max - min < 255, "blur must remove the checker extremes");
    }

    #[test]
    fn blur_softens_a_hard_edge() {
        // Left half black, right half white: the center column must become
        // strictly between after blurring.
        let w = 32u32;
        let h = 8u32;
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let v = if x < w / 2 { 0u8 } else { 255u8 };
                pixels[i] = v;
                pixels[i + 1] = v;
                pixels[i + 2] = v;
                pixels[i + 3] = 255;
            }
        }
        let mut bg = Backdrop {
            width: w,
            height: h,
            pixels,
        };
        box_blur(&mut bg, 4);
        let edge = (((h / 2) * w + w / 2) * 4) as usize;
        assert!(bg.pixels[edge] > 0 && bg.pixels[edge] < 255);
        // Far corners stay put.
        assert_eq!(bg.pixels[3], 255);
        let far = (((h / 2) * w) * 4 + 3) as usize;
        assert_eq!(bg.pixels[far], 255);
        let dark = (((h / 2) * w) * 4) as usize;
        assert_eq!(bg.pixels[dark], 0);
    }

    #[test]
    fn desaturate_pulls_hue_without_touching_alpha_or_gray() {
        // Pure orange pixel: channels must converge, alpha stays.
        let mut bg = Backdrop {
            width: 2,
            height: 1,
            pixels: vec![0, 120, 212, 255, 128, 128, 128, 255],
        };
        desaturate(&mut bg, 0.5);
        let spread_before = 212u32.abs_diff(0);
        let spread_after =
            bg.pixels[2].max(bg.pixels[0]) as u32 - bg.pixels[2].min(bg.pixels[0]) as u32;
        assert!(
            spread_after < spread_before,
            "channels must converge: {:?} spread {spread_after}",
            &bg.pixels[..4]
        );
        assert_eq!(bg.pixels[3], 255);
        // Gray pixel is already neutral: untouched.
        assert_eq!(&bg.pixels[4..8], &[128, 128, 128, 255]);
        // Zero amount is a no-op.
        let mut flat = Backdrop {
            width: 1,
            height: 1,
            pixels: vec![10, 200, 30, 255],
        };
        desaturate(&mut flat, 0.0);
        assert_eq!(flat.pixels, vec![10, 200, 30, 255]);
    }
}

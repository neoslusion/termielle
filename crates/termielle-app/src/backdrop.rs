//! Live frosted-glass backdrop: captures the wallpaper (and whatever is
//! behind the pill) and box-blurs it underneath the theme tint.
//!
//! Modern DWM can include layered windows even without CAPTUREBLT. Production
//! captures temporarily exclude our own HWND, then restore its original
//! capture policy. If exclusion is unavailable, use flat translucent glass.
//! Anything outside the virtual screen is filled before the copy.
//!
//! Cost is trivial for pill sizes (720x56): a separable box blur at radius
//! 24 is ~3.4M ops on the worker thread. The window caches the capture and
//! only re-captures when the geometry moves or the cache ages out.

mod exclusion;
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
    /// Production capture ownership/epoch. Unstamped synthetic images are
    /// supported by render-review examples, never produced by the worker.
    pub capture_token: Option<(isize, u64)>,
    pub origin: (i32, i32),
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Backdrop {
    /// Screen-anchored sampling never stretches a cached image during a morph.
    pub fn sample(&self, x: i32, y: i32) -> Option<&[u8]> {
        let bx = i64::from(x) - i64::from(self.origin.0);
        let by = i64::from(y) - i64::from(self.origin.1);
        if bx < 0 || by < 0 || bx >= i64::from(self.width) || by >= i64::from(self.height) {
            return None;
        }
        let offset = (by as usize * self.width as usize + bx as usize) * 4;
        self.pixels.get(offset..offset + 4)
    }
}

/// Smallest rect containing both `a` and `b`, in physical screen pixels.
///
/// The frosted glass samples a captured image screen-anchored, and a sample
/// outside that image falls back to a flat tint. So a capture published for
/// only the rect that was just drawn leaves every pixel a growing surface has
/// not reached yet showing flat colour — the glass stops reflecting what is
/// behind it exactly while it is moving. Covering the union of where the
/// surface is and where it is going keeps every covered pixel sampleable for
/// the whole travel.
pub fn cover_rect(a: (i32, i32, u32, u32), b: (i32, i32, u32, u32)) -> (i32, i32, u32, u32) {
    let left = a.0.min(b.0);
    let top = a.1.min(b.1);
    let right =
        a.0.saturating_add(a.2 as i32)
            .max(b.0.saturating_add(b.2 as i32));
    let bottom =
        a.1.saturating_add(a.3 as i32)
            .max(b.1.saturating_add(b.3 as i32));
    (
        left,
        top,
        (right - left).max(1) as u32,
        (bottom - top).max(1) as u32,
    )
}

/// Production path: never capture our own visible material into its backdrop.
/// Exclusion lasts only for the desktop copy (not the blur or app lifetime).
pub fn capture_backdrop_excluding(
    hwnd: isize,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    fill: [u8; 4],
) -> Option<Backdrop> {
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    let _exclusion = exclusion::Exclusion::begin(hwnd)?;
    capture_backdrop(x, y, w, h, fill)
}

/// Source-window geometry re-arms the short capture burst without a permanent
/// full-rate desktop poll. No titles or content are read here.
pub(crate) fn foreground_scene(exclude: isize) -> Option<(isize, i32, i32, i32, i32)> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect};
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() || hwnd.0 as isize == exclude {
        return None;
    }
    let mut rect = windows::Win32::Foundation::RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    Some((
        hwnd.0 as isize,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
    ))
}

/// Raw desktop-copy primitive, also used by diagnostics. It does NOT promise
/// to exclude layered windows; use `capture_backdrop_excluding` for glass.
/// Captures the virtual-screen region `(x, y, w, h)` in physical pixels.
/// Out-of-screen areas are filled with `fill` (straight BGRA). Returns `None`
/// when no device context is available (locked/secure desktop) so the caller
/// can fall back to a flat tint.
pub fn capture_backdrop(x: i32, y: i32, w: u32, h: u32, fill: [u8; 4]) -> Option<Backdrop> {
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    let _dpi = exclusion::PhysicalDpi::enter();
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

    // Initialize the actual DIB too: a partial BitBlt leaves off-screen areas
    // untouched. Copying an uninitialized DIB would turn those fringes black.
    if !bits.is_null() {
        unsafe {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
        }
    }
    let mut copied = false;
    // Intersect with the virtual screen, including negative-origin monitors.
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
            copied = true;
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
    if ix1 > ix0 && iy1 > iy0 && !copied {
        return None;
    }
    Some(Backdrop {
        capture_token: None,
        origin: (x, y),
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
    fn off_screen_and_partial_capture_fringe_is_initialized_to_tint() {
        let fill = [11, 22, 33, 255];
        if let Some(bg) = capture_backdrop(i32::MAX - 16, i32::MAX - 16, 8, 1, fill) {
            assert!(bg.pixels.chunks_exact(4).all(|p| p == fill));
        }
        let _dpi = exclusion::PhysicalDpi::enter();
        let right =
            unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        if let Some(bg) = capture_backdrop(right - 1, top, 3, 1, fill) {
            assert_eq!(&bg.pixels[4..], &fill.repeat(2));
        }
    }
    #[test]
    fn desktop_copy_restores_its_callers_dpi_context() {
        use windows::Win32::UI::HiDpi::*;
        let previous = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_UNAWARE) };
        let before = unsafe { GetThreadDpiAwarenessContext() };
        let _ = capture_backdrop(i32::MAX - 16, i32::MAX - 16, 8, 1, [0; 4]);
        let after = unsafe { GetThreadDpiAwarenessContext() };
        let restored = unsafe { AreDpiAwarenessContextsEqual(before, after) }.as_bool();
        if !previous.0.is_null() {
            let _ = unsafe { SetThreadDpiAwarenessContext(previous) };
        }
        assert!(restored);
    }

    #[test]
    fn cached_capture_stays_anchored_when_window_resizes_or_moves() {
        let bg = Backdrop {
            origin: (-10, 20),
            width: 2,
            height: 2,
            pixels: vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255],
            capture_token: None,
        };
        assert_eq!(bg.sample(-9, 21), Some(&[10, 11, 12, 255][..]));
        assert_eq!(bg.sample(-10, 20), Some(&[1, 2, 3, 255][..]));
        assert_eq!(bg.sample(-11, 20), None);
        assert_eq!(bg.sample(-9, 22), None);
    }

    #[test]
    fn blur_of_uniform_image_is_identity() {
        let mut bg = Backdrop {
            origin: (0, 0),
            width: 16,
            height: 16,
            pixels: vec![77u8; 16 * 16 * 4],
            capture_token: None,
        };
        box_blur(&mut bg, 6);
        assert!(bg.pixels.iter().all(|&b| b == 77));
    }

    #[test]
    fn blur_radius_zero_is_noop() {
        let mut bg = Backdrop {
            origin: (0, 0),
            width: 8,
            height: 8,
            pixels: (0..8 * 8 * 4).map(|i| (i % 251) as u8).collect(),
            capture_token: None,
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
            origin: (0, 0),
            width: w,
            height: h,
            pixels: vec![0u8; (w * h * 4) as usize],
            capture_token: None,
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
            origin: (0, 0),
            width: w,
            height: h,
            pixels,
            capture_token: None,
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
            origin: (0, 0),
            width: 2,
            height: 1,
            pixels: vec![0, 120, 212, 255, 128, 128, 128, 255],
            capture_token: None,
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
            origin: (0, 0),
            width: 1,
            height: 1,
            pixels: vec![10, 200, 30, 255],
            capture_token: None,
        };
        desaturate(&mut flat, 0.0);
        assert_eq!(flat.pixels, vec![10, 200, 30, 255]);
    }
}

#[cfg(test)]
mod cover_tests {
    use super::*;

    /// Whether `outer` fully contains the screen rect `inner`.
    fn contains(outer: (i32, i32, u32, u32), inner: (i32, i32, u32, u32)) -> bool {
        inner.0 >= outer.0
            && inner.1 >= outer.1
            && inner.0 + inner.2 as i32 <= outer.0 + outer.2 as i32
            && inner.1 + inner.3 as i32 <= outer.1 + outer.3 as i32
    }

    #[test]
    fn cover_holds_both_rects() {
        let current = (700, 0, 140, 36);
        let target = (600, 0, 340, 210);
        let cover = cover_rect(current, target);
        assert!(
            contains(cover, current),
            "{cover:?} must hold the current rect"
        );
        assert!(
            contains(cover, target),
            "{cover:?} must hold the target rect"
        );
    }

    #[test]
    fn a_growing_surface_is_covered_even_when_it_grows_upwards() {
        // A floating island with a y offset moves its top edge up as it grows,
        // so the cover has to start above both.
        let current = (700, 120, 140, 36);
        let target = (690, 80, 340, 180);
        let cover = cover_rect(current, target);
        assert!(cover.1 <= target.1);
        assert!(contains(cover, target));
    }

    #[test]
    fn identical_rects_collapse_to_themselves() {
        let rect = (10, 20, 30, 40);
        assert_eq!(cover_rect(rect, rect), rect);
    }
}

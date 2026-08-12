//! System-tray presence: a notification icon, a state tooltip, and the
//! Exit / Restart context menu.
//!
//! The icon is the character's idle face, drawn from the standby animation's
//! first frame: scaled to notification size and turned into an `HICON` with
//! per-pixel alpha. When the asset is missing or undecodable, the plain
//! application icon is shown instead, so a broken install still has a tray
//! presence.

use std::mem::size_of;
use std::path::Path;
use std::sync::Mutex;

use windows::Win32::Foundation::{GetLastError, HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos,
    HICON, ICONINFO, IDI_APPLICATION, LoadIconW, MF_STRING, TPM_LEFTALIGN, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TRACK_POPUP_MENU_FLAGS, TrackPopupMenu, WM_APP, WM_CONTEXTMENU, WM_LBUTTONUP,
    WM_RBUTTONUP,
};
use windows::core::{BOOL, PCWSTR};

use crate::animation::GifAnimation;

/// Callback message the shell uses to report tray events to the overlay.
pub const TRAY_MSG: u32 = WM_APP + 2;

/// The tray icon's identifier inside the notification area.
pub(crate) const TRAY_ID: u32 = 1;

/// Menu command: quit the overlay.
pub const TRAY_EXIT: u32 = 100;

/// Menu command: relaunch the overlay after quitting.
pub const TRAY_RESTART: u32 = 101;

/// Edge length of the notification icon, in pixels.
const ICON_SIZE: u32 = 32;

/// The icon currently installed, destroyed on [`remove`]. Stored as a raw
/// handle value so the static stays `Send`.
static CURRENT_ICON: Mutex<Option<isize>> = Mutex::new(None);

/// Tooltip for the tray icon, showing the current face.
pub fn state_tooltip(state: &str) -> String {
    let face = match state {
        "idle" => "Idle",
        "thinking" => "Thinking",
        "working" => "Working",
        "needs_input" => "Needs input",
        "ready" => "Ready",
        "failed" => "Failed",
        other => other,
    };
    format!("Termielle — {face}")
}

/// Creates the notification-area icon, delivering events to `hwnd` through
/// [`TRAY_MSG`]. `character_path` is the idle animation whose first frame
/// becomes the icon; when it is `None` or undecodable, the plain application
/// icon is used. Returns the Win32 error code when the shell rejects the
/// icon.
pub fn add(hwnd: HWND, character_path: Option<&Path>) -> Result<(), u32> {
    let icon = character_path
        .and_then(character_icon)
        .unwrap_or_else(|| unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default());
    if let Ok(mut current) = CURRENT_ICON.lock() {
        *current = Some(icon.0 as isize);
    }
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: TRAY_MSG,
        hIcon: icon,
        szTip: encode_tip("Termielle"),
        ..Default::default()
    };
    if unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
        Ok(())
    } else {
        Err(unsafe { GetLastError().0 })
    }
}

/// Updates the tray tooltip in place. Safe to call when no icon is installed.
pub fn set_tooltip(hwnd: HWND, tip: &str) -> bool {
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_TIP,
        szTip: encode_tip(tip),
        ..Default::default()
    };
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool()
}

/// Removes the notification-area icon and frees the character icon, if any.
pub fn remove(hwnd: HWND) -> bool {
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    };
    let removed = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) }.as_bool();
    if let Ok(mut current) = CURRENT_ICON.lock() {
        if let Some(raw) = current.take() {
            // SAFETY: the icon was created by `character_icon` or `LoadIconW`
            // and is no longer referenced by the shell.
            unsafe {
                let _ = DestroyIcon(HICON(raw as *mut core::ffi::c_void));
            }
        }
    }
    removed
}

/// Shows the tray context menu at the cursor and returns the chosen command
/// id, or 0 when nothing was chosen. The shell owns the message pump while
/// the menu is up, so this call blocks.
pub fn show_menu(hwnd: HWND) -> u32 {
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
        return 0;
    };
    let restart = crate::window::encode_wide("Restart");
    let exit = crate::window::encode_wide("Exit");
    let _ = unsafe {
        AppendMenuW(
            menu,
            MF_STRING,
            TRAY_RESTART as usize,
            PCWSTR(restart.as_ptr()),
        )
    };
    let _ = unsafe { AppendMenuW(menu, MF_STRING, TRAY_EXIT as usize, PCWSTR(exit.as_ptr())) };
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        let _ = unsafe { DestroyMenu(menu) };
        return 0;
    }
    let flags = TRACK_POPUP_MENU_FLAGS(TPM_RETURNCMD.0 | TPM_LEFTALIGN.0 | TPM_RIGHTBUTTON.0);
    let choice = unsafe { TrackPopupMenu(menu, flags, point.x, point.y, None, hwnd, None) }.0;
    let _ = unsafe { DestroyMenu(menu) };
    choice.max(0) as u32
}

/// The mouse message part of a tray callback `LPARAM`.
pub fn callback_mouse_message(lparam: isize) -> u32 {
    (lparam as u32) & 0xFFFF
}

/// Whether a tray callback's mouse message should open the context menu.
pub fn is_menu_message(message: u32) -> bool {
    matches!(message, WM_RBUTTONUP | WM_CONTEXTMENU)
}

/// Whether a tray callback's mouse message is a plain left click.
pub fn is_left_click(message: u32) -> bool {
    message == WM_LBUTTONUP
}

/// Builds the character icon: the first frame of the animation at `path`,
/// scaled to [`ICON_SIZE`] and turned into an `HICON` with per-pixel alpha.
/// Returns `None` when the asset cannot be decoded.
fn character_icon(path: &Path) -> Option<HICON> {
    let mut animation = GifAnimation::open(path).ok()?;
    let frame = animation.next_frame().ok()?;
    let (color, mask) = icon_bits(frame.width, frame.height, &frame.pixels_pbgra);

    // A top-down 32-bit DIB section: `CreateBitmap` does not reliably accept
    // pixel bits for 32bpp bitmaps, so the color surface must be a DIB.
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: ICON_SIZE as i32,
            biHeight: -(ICON_SIZE as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        bmiColors: [Default::default()],
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `info` is a live in-parameter, and `bits` is a live
    // out-parameter receiving the DIB's pixel memory.
    let color_bitmap = unsafe {
        CreateDIBSection(
            None,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            Some(windows::Win32::Foundation::HANDLE::default()),
            0,
        )
    }
    .ok()?;
    if color_bitmap.is_invalid() || bits.is_null() {
        return None;
    }
    // SAFETY: `bits` points at `ICON_SIZE * ICON_SIZE * 4` bytes of DIB
    // memory, and `color` is exactly that many bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(color.as_ptr(), bits as *mut u8, color.len());
    }

    let mask_bitmap = unsafe {
        windows::Win32::Graphics::Gdi::CreateBitmap(
            ICON_SIZE as i32,
            ICON_SIZE as i32,
            1,
            1,
            Some(mask.as_ptr().cast()),
        )
    };
    if mask_bitmap.is_invalid() {
        // SAFETY: `color_bitmap` was created above and is no longer needed.
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(HGDIOBJ(color_bitmap.0));
        }
        return None;
    }

    let info = ICONINFO {
        fIcon: BOOL(1),
        hbmColor: color_bitmap,
        hbmMask: mask_bitmap,
        ..Default::default()
    };
    // SAFETY: both bitmaps are live kernel objects owned by this scope, and
    // `info` is a live in-parameter.
    let icon = unsafe { CreateIconIndirect(&info) }.ok()?;
    // SAFETY: the bitmaps are no longer needed once the icon exists; the
    // icon owns its own copy of the bits.
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(HGDIOBJ(color_bitmap.0));
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(HGDIOBJ(mask_bitmap.0));
    }
    (!icon.0.is_null()).then_some(icon)
}

/// Scales one premultiplied-BGRA frame to [`ICON_SIZE`], producing the
/// 32-bit straight-alpha color bitmap bits and the 1-bit opacity mask.
fn icon_bits(width: u32, height: u32, pixels_pbgra: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut color = Vec::with_capacity((ICON_SIZE * ICON_SIZE * 4) as usize);
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            // Sample the center of the destination pixel, bilinearly.
            let sx = (f64::from(x) + 0.5) * f64::from(width) / f64::from(ICON_SIZE) - 0.5;
            let sy = (f64::from(y) + 0.5) * f64::from(height) / f64::from(ICON_SIZE) - 0.5;
            let (x0, y0) = (sx.max(0.0) as u32, sy.max(0.0) as u32);
            let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
            let fx = (sx - f64::from(x0)).clamp(0.0, 1.0) as f32;
            let fy = (sy - f64::from(y0)).clamp(0.0, 1.0) as f32;
            let mut sample = [0u8; 4];
            for channel in 0..4usize {
                let at = |px: u32, py: u32| {
                    pixels_pbgra[((py * width + px) * 4) as usize + channel] as f32
                };
                let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
                let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
                sample[channel] = (top * (1.0 - fy) + bottom * fy).round() as u8;
            }
            // Un-premultiply for straight alpha, which icon bitmaps expect.
            let alpha = sample[3];
            if alpha > 0 && alpha < 255 {
                for channel in sample[..3].iter_mut() {
                    *channel = ((u32::from(*channel) * 255 + u32::from(alpha) / 2)
                        / u32::from(alpha)) as u8;
                }
            }
            color.extend_from_slice(&sample);
        }
    }

    // One mask bit per pixel: 1 where the character is opaque.
    let mut mask = vec![0u8; ((ICON_SIZE * ICON_SIZE) / 8) as usize];
    for (index, pixel) in color.chunks_exact(4).enumerate() {
        if pixel[3] > 0 {
            mask[index / 8] |= 1 << (index % 8);
        }
    }
    (color, mask)
}

fn encode_tip(tip: &str) -> [u16; 128] {
    let mut wide = [0u16; 128];
    for (slot, unit) in wide.iter_mut().zip(tip.encode_utf16().take(127)) {
        *slot = unit;
    }
    wide
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_names_every_face() {
        assert_eq!(state_tooltip("idle"), "Termielle — Idle");
        assert_eq!(state_tooltip("thinking"), "Termielle — Thinking");
        assert_eq!(state_tooltip("working"), "Termielle — Working");
        assert_eq!(state_tooltip("needs_input"), "Termielle — Needs input");
        assert_eq!(state_tooltip("ready"), "Termielle — Ready");
        assert_eq!(state_tooltip("failed"), "Termielle — Failed");
    }

    #[test]
    fn menu_messages_classify() {
        assert!(is_menu_message(WM_RBUTTONUP));
        assert!(is_menu_message(WM_CONTEXTMENU));
        assert!(!is_menu_message(WM_LBUTTONUP));
        assert!(is_left_click(WM_LBUTTONUP));
    }

    #[test]
    fn tips_are_nul_terminated_within_bounds() {
        let tip = encode_tip(&"x".repeat(200));
        assert!(tip[..128].contains(&0));
    }

    #[test]
    fn icon_bits_are_exactly_icon_sized() {
        let (color, mask) = icon_bits(64, 64, &vec![0u8; 64 * 64 * 4]);
        assert_eq!(color.len(), (ICON_SIZE * ICON_SIZE * 4) as usize);
        assert_eq!(mask.len(), ((ICON_SIZE * ICON_SIZE) / 8) as usize);
    }

    #[test]
    fn icon_bits_mask_follows_alpha() {
        let mut pixels = vec![0u8; 4 * 4 * 4];
        // One opaque pixel at the top-left corner of a 4x4 frame.
        pixels[3] = 255;
        let (_, mask) = icon_bits(4, 4, &pixels);
        assert_ne!(mask[0] & 1, 0, "the opaque corner must set its mask bit");
    }
}

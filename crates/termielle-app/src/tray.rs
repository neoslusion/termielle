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

/// Layout switches
pub const TRAY_LAYOUT_CLASSIC: u32 = 102;
pub const TRAY_LAYOUT_NOTCH: u32 = 103;
pub const TRAY_LAYOUT_ISLAND: u32 = 104;
pub const TRAY_LAYOUT_BAR: u32 = 105;

/// Theme switches
pub const TRAY_THEME_LIQUID_DARK: u32 = 110;
pub const TRAY_THEME_MIDNIGHT: u32 = 111;
pub const TRAY_THEME_LIGHT: u32 = 112;
pub const TRAY_THEME_TRANSPARENT: u32 = 113;
pub const TRAY_THEME_AUTO: u32 = 114;
pub const TRAY_THEME_CATPPUCCIN_MACCHIATO: u32 = 115;

/// Y-offset presets for standalone Island/Notch layouts.
pub const TRAY_YOFFSET_0: u32 = 120;
pub const TRAY_YOFFSET_12: u32 = 121;
pub const TRAY_YOFFSET_80: u32 = 122;
pub const TRAY_YOFFSET_150: u32 = 123;
pub const TRAY_YOFFSET_300: u32 = 124;

/// Bar position commands.
pub const TRAY_BAR_TOP: u32 = 125;
pub const TRAY_BAR_BOTTOM: u32 = 126;

/// Widget toggles
pub const TRAY_TOGGLE_MUSIC: u32 = 130;
pub const TRAY_TOGGLE_HOVER: u32 = 131;
pub const TRAY_TOGGLE_FACE: u32 = 132;
pub const TRAY_TOGGLE_BAR_CPU: u32 = 133;
pub const TRAY_TOGGLE_BAR_MEMORY: u32 = 134;
pub const TRAY_TOGGLE_BAR_VOLUME: u32 = 135;
pub const TRAY_TOGGLE_BAR_BATTERY: u32 = 136;
pub const TRAY_TOGGLE_BAR_NETWORK: u32 = 137;

/// Current user-visible settings used to mark tray menu choices.
#[derive(Clone, Debug, Default)]
pub struct MenuState {
    pub layout: String,
    pub theme: String,
    pub bar_position: String,
    pub y_offset: i32,
    pub music: bool,
    pub face: bool,
    pub hover: bool,
    pub bar_cpu: bool,
    pub bar_memory: bool,
    pub bar_volume: bool,
    pub bar_battery: bool,
    pub bar_network: bool,
}

impl MenuState {
    pub fn from_island(island: &termielle_core::IslandConfig) -> Self {
        Self {
            layout: match island.layout {
                termielle_core::IslandLayout::Classic => "classic",
                termielle_core::IslandLayout::Notch => "notch",
                termielle_core::IslandLayout::Island => "island",
                termielle_core::IslandLayout::Bar => "bar",
            }
            .to_string(),
            theme: island.theme.clone(),
            bar_position: match island.bar.position {
                termielle_core::BarPosition::Top => "top",
                termielle_core::BarPosition::Bottom => "bottom",
            }
            .to_string(),
            y_offset: island.y_offset,
            music: island.has_widget("music"),
            face: island.has_widget("face"),
            hover: island.expand_on_hover,
            bar_cpu: island.bar.modules_right.iter().any(|item| item == "cpu"),
            bar_memory: island.bar.modules_right.iter().any(|item| item == "memory"),
            bar_volume: island.bar.modules_right.iter().any(|item| item == "volume"),
            bar_battery: island
                .bar
                .modules_right
                .iter()
                .any(|item| item == "battery"),
            bar_network: island
                .bar
                .modules_right
                .iter()
                .any(|item| item == "network"),
        }
    }
}
/// Edge length of the notification icon, in pixels.
const ICON_SIZE: u32 = 32;

/// The icon currently installed, destroyed on [`remove`]. Stored as a raw
/// handle value so the static stays `Send`.
static CURRENT_ICON: Mutex<Option<(isize, bool)>> = Mutex::new(None);

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
    let (icon, owned) = character_path
        .and_then(character_icon)
        .map(|icon| (icon, true))
        .unwrap_or_else(|| {
            (
                unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default(),
                false,
            )
        });
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
        if let Ok(mut current) = CURRENT_ICON.lock() {
            if let Some((raw, true)) = current.replace((icon.0 as isize, owned)) {
                let _ = unsafe { DestroyIcon(HICON(raw as *mut core::ffi::c_void)) };
            }
        }
        Ok(())
    } else {
        let error = unsafe { GetLastError().0 };
        if owned {
            let _ = unsafe { DestroyIcon(icon) };
        }
        Err(error)
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
        if let Some((raw, true)) = current.take() {
            // Only CreateIconIndirect transfers ownership. LoadIconW's
            // shared fallback belongs to Windows and must not be destroyed.
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
pub fn show_menu(hwnd: HWND, state: &MenuState) -> u32 {
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
        return 0;
    };
    // Layout submenu
    let Ok(layout_menu) = (unsafe { CreatePopupMenu() }) else {
        let _ = unsafe { DestroyMenu(menu) };
        return 0;
    };
    for (id, label, active) in [
        (
            TRAY_LAYOUT_CLASSIC,
            "Layout: Classic (pet)",
            state.layout == "classic",
        ),
        (
            TRAY_LAYOUT_NOTCH,
            "Layout: Notch (macOS)",
            state.layout == "notch",
        ),
        (
            TRAY_LAYOUT_ISLAND,
            "Layout: Island (floating)",
            state.layout == "island",
        ),
        (
            TRAY_LAYOUT_BAR,
            "Layout: Bar (native)",
            state.layout == "bar",
        ),
    ] {
        let label = if active {
            format!("[x] {label}")
        } else {
            format!("[ ] {label}")
        };
        let w = crate::window::encode_wide(&label);
        let _ = unsafe { AppendMenuW(layout_menu, MF_STRING, id as usize, PCWSTR(w.as_ptr())) };
    }
    let layout_label = crate::window::encode_wide("Layout");
    let _ = unsafe {
        AppendMenuW(
            menu,
            windows::Win32::UI::WindowsAndMessaging::MF_POPUP,
            layout_menu.0 as usize,
            PCWSTR(layout_label.as_ptr()),
        )
    };

    // Theme submenu
    let Ok(theme_menu) = (unsafe { CreatePopupMenu() }) else {
        let _ = unsafe { DestroyMenu(layout_menu) };
        let _ = unsafe { DestroyMenu(menu) };
        return 0;
    };
    for (id, label, active) in [
        (
            TRAY_THEME_LIQUID_DARK,
            "Theme: Liquid Dark",
            state.theme == "liquid-dark",
        ),
        (
            TRAY_THEME_MIDNIGHT,
            "Theme: Midnight",
            state.theme == "midnight",
        ),
        (
            TRAY_THEME_CATPPUCCIN_MACCHIATO,
            "Theme: Catppuccin Macchiato",
            state.theme == "catppuccin-macchiato",
        ),
        (TRAY_THEME_LIGHT, "Theme: Light", state.theme == "light"),
        (
            TRAY_THEME_TRANSPARENT,
            "Theme: Transparent",
            state.theme == "transparent",
        ),
        (TRAY_THEME_AUTO, "Theme: Auto", state.theme == "auto"),
    ] {
        let label = if active {
            format!("[x] {label}")
        } else {
            format!("[ ] {label}")
        };
        let w = crate::window::encode_wide(&label);
        let _ = unsafe { AppendMenuW(theme_menu, MF_STRING, id as usize, PCWSTR(w.as_ptr())) };
    }
    let theme_label = crate::window::encode_wide("Theme");
    let _ = unsafe {
        AppendMenuW(
            menu,
            windows::Win32::UI::WindowsAndMessaging::MF_POPUP,
            theme_menu.0 as usize,
            PCWSTR(theme_label.as_ptr()),
        )
    };

    // Position submenu: Bar has top/bottom placement; Island/Notch retain
    // their floating Y-offset presets.
    let Ok(position_menu) = (unsafe { CreatePopupMenu() }) else {
        let _ = unsafe { DestroyMenu(theme_menu) };
        let _ = unsafe { DestroyMenu(layout_menu) };
        let _ = unsafe { DestroyMenu(menu) };
        return 0;
    };
    if state.layout == "bar" {
        for (id, label, active) in [
            (TRAY_BAR_TOP, "Position: Top", state.bar_position == "top"),
            (
                TRAY_BAR_BOTTOM,
                "Position: Bottom",
                state.bar_position == "bottom",
            ),
        ] {
            let label = if active {
                format!("[x] {label}")
            } else {
                format!("[ ] {label}")
            };
            let w = crate::window::encode_wide(&label);
            let _ =
                unsafe { AppendMenuW(position_menu, MF_STRING, id as usize, PCWSTR(w.as_ptr())) };
        }
    } else {
        for (id, label, active) in [
            (TRAY_YOFFSET_0, "Y Offset: 0 (flush)", state.y_offset == 0),
            (TRAY_YOFFSET_12, "Y Offset: 12", state.y_offset == 12),
            (TRAY_YOFFSET_80, "Y Offset: 80", state.y_offset == 80),
            (TRAY_YOFFSET_150, "Y Offset: 150", state.y_offset == 150),
            (
                TRAY_YOFFSET_300,
                "Y Offset: 300 (center)",
                state.y_offset == 300,
            ),
        ] {
            let label = if active {
                format!("[x] {label}")
            } else {
                format!("[ ] {label}")
            };
            let w = crate::window::encode_wide(&label);
            let _ =
                unsafe { AppendMenuW(position_menu, MF_STRING, id as usize, PCWSTR(w.as_ptr())) };
        }
    }
    let position_label = crate::window::encode_wide("Position");
    let _ = unsafe {
        AppendMenuW(
            menu,
            windows::Win32::UI::WindowsAndMessaging::MF_POPUP,
            position_menu.0 as usize,
            PCWSTR(position_label.as_ptr()),
        )
    };

    let Ok(widgets_menu) = (unsafe { CreatePopupMenu() }) else {
        let _ = unsafe { DestroyMenu(position_menu) };
        let _ = unsafe { DestroyMenu(theme_menu) };
        let _ = unsafe { DestroyMenu(layout_menu) };
        let _ = unsafe { DestroyMenu(menu) };
        return 0;
    };
    for (id, label, active) in [
        (TRAY_TOGGLE_MUSIC, "Media widget", state.music),
        (TRAY_TOGGLE_HOVER, "Hover to expand", state.hover),
        (TRAY_TOGGLE_FACE, "Termielle face", state.face),
    ] {
        let label = if active {
            format!("[x] {label}")
        } else {
            format!("[ ] {label}")
        };
        let w = crate::window::encode_wide(&label);
        let _ = unsafe { AppendMenuW(widgets_menu, MF_STRING, id as usize, PCWSTR(w.as_ptr())) };
    }
    let widgets_label = crate::window::encode_wide("Widgets");
    let _ = unsafe {
        AppendMenuW(
            menu,
            windows::Win32::UI::WindowsAndMessaging::MF_POPUP,
            widgets_menu.0 as usize,
            PCWSTR(widgets_label.as_ptr()),
        )
    };

    if state.layout == "bar" {
        let Ok(status_menu) = (unsafe { CreatePopupMenu() }) else {
            let _ = unsafe { DestroyMenu(menu) };
            return 0;
        };
        for (id, label, active) in [
            (TRAY_TOGGLE_BAR_NETWORK, "Network", state.bar_network),
            (TRAY_TOGGLE_BAR_CPU, "CPU", state.bar_cpu),
            (TRAY_TOGGLE_BAR_MEMORY, "Memory", state.bar_memory),
            (TRAY_TOGGLE_BAR_VOLUME, "Volume", state.bar_volume),
            (TRAY_TOGGLE_BAR_BATTERY, "Battery", state.bar_battery),
        ] {
            let label = format!("{} {label}", if active { "[x]" } else { "[ ]" });
            let wide = crate::window::encode_wide(&label);
            let _ =
                unsafe { AppendMenuW(status_menu, MF_STRING, id as usize, PCWSTR(wide.as_ptr())) };
        }
        let label = crate::window::encode_wide("Menu Bar Items");
        let _ = unsafe {
            AppendMenuW(
                menu,
                windows::Win32::UI::WindowsAndMessaging::MF_POPUP,
                status_menu.0 as usize,
                PCWSTR(label.as_ptr()),
            )
        };
    }

    // Separator + Restart/Exit
    let _ = unsafe {
        AppendMenuW(
            menu,
            windows::Win32::UI::WindowsAndMessaging::MF_SEPARATOR,
            0,
            PCWSTR::null(),
        )
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
        let _ = unsafe { DestroyMenu(widgets_menu) };
        let _ = unsafe { DestroyMenu(position_menu) };
        let _ = unsafe { DestroyMenu(theme_menu) };
        let _ = unsafe { DestroyMenu(layout_menu) };
        let _ = unsafe { DestroyMenu(menu) };
        return 0;
    }
    let flags = TRACK_POPUP_MENU_FLAGS(TPM_RETURNCMD.0 | TPM_LEFTALIGN.0 | TPM_RIGHTBUTTON.0);
    let choice = unsafe { TrackPopupMenu(menu, flags, point.x, point.y, None, hwnd, None) }.0;
    let _ = unsafe { DestroyMenu(widgets_menu) };
    let _ = unsafe { DestroyMenu(position_menu) };
    let _ = unsafe { DestroyMenu(theme_menu) };
    let _ = unsafe { DestroyMenu(layout_menu) };
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
    fn menu_state_reports_bar_position_and_theme() {
        let mut island = termielle_core::IslandConfig {
            layout: termielle_core::IslandLayout::Bar,
            theme: "midnight".into(),
            ..Default::default()
        };
        island.bar.position = termielle_core::BarPosition::Bottom;
        island.y_offset = 80;
        let state = MenuState::from_island(&island);
        assert_eq!(state.layout, "bar");
        assert_eq!(state.theme, "midnight");
        assert_eq!(state.bar_position, "bottom");
        assert_eq!(state.y_offset, 80);
        assert!(!state.bar_cpu);
        assert!(!state.bar_memory);
        assert!(state.bar_volume);
        assert!(state.bar_battery);
        assert!(state.bar_network);
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

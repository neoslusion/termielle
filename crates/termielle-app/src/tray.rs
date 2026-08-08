//! System-tray presence: a notification icon, a state tooltip, and the
//! Exit / Restart context menu.

use std::mem::size_of;

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, IDI_APPLICATION, LoadIconW, MF_STRING,
    TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TRACK_POPUP_MENU_FLAGS, TrackPopupMenu, WM_APP,
    WM_CONTEXTMENU, WM_LBUTTONUP, WM_RBUTTONUP,
};
use windows::core::PCWSTR;

/// Callback message the shell uses to report tray events to the overlay.
pub const TRAY_MSG: u32 = WM_APP + 2;

/// The tray icon's identifier inside the notification area.
pub(crate) const TRAY_ID: u32 = 1;

/// Menu command: quit the overlay.
pub const TRAY_EXIT: u32 = 100;

/// Menu command: relaunch the overlay after quitting.
pub const TRAY_RESTART: u32 = 101;

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
/// [`TRAY_MSG`]. Returns false when the shell rejects the icon.
pub fn add(hwnd: HWND) -> bool {
    let icon = unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default();
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
    unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool()
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

/// Removes the notification-area icon.
pub fn remove(hwnd: HWND) -> bool {
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    };
    unsafe { Shell_NotifyIconW(NIM_DELETE, &data) }.as_bool()
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
}

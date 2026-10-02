//! Media session info for the island via Windows.Globalization / RoInitialize
//! not required — we use the simpler approach: enumerate audio sessions via
//! policy config is complex; instead we read the active media title from the
//! SystemMediaTransportControls through WinRT interop is heavy. For v1 we
//! expose a lightweight hook: read the window title of the foreground app.
//!
//! A full SMTC integration lands in v2; v1 shows the focused app name so the
//! island can display "Chrome — YouTube" style context.

use std::sync::Mutex;

use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW};

/// Last-known foreground window, refreshed at most once per second.
static LAST_WINDOW: Mutex<Option<(u64, isize, String)>> = Mutex::new(None);

/// Returns the foreground window handle and title, cached for 1s.
pub fn foreground_window() -> (isize, String) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if let Ok(guard) = LAST_WINDOW.lock() {
        if let Some((ts, hwnd, title)) = guard.as_ref() {
            if now.saturating_sub(*ts) < 1000 {
                return (*hwnd, title.clone());
            }
        }
    }
    let (hwnd, title) = read_window();
    if let Ok(mut guard) = LAST_WINDOW.lock() {
        *guard = Some((now, hwnd, title.clone()));
    }
    (hwnd, title)
}

/// Returns the title of the foreground window, cached for 1s.
pub fn foreground_title() -> String {
    foreground_window().1
}

fn read_window() -> (isize, String) {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return (0, String::new());
        }
        let mut buf = [0u16; 256];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len <= 0 {
            return (0, String::new());
        }
        (
            hwnd.0 as isize,
            String::from_utf16_lossy(&buf[..len as usize]),
        )
    }
}

/// Extracts just the app name (before " — " or " - " separator) from a title.
pub fn app_name_from_title(title: &str) -> String {
    for sep in [" — ", " - ", " – "] {
        if let Some(idx) = title.find(sep) {
            return title[..idx].trim().to_string();
        }
    }
    title.chars().take(40).collect()
}

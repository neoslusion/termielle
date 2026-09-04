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

/// Last-known foreground app title, refreshed at most once per second.
static LAST_TITLE: Mutex<Option<(u64, String)>> = Mutex::new(None);

/// Returns the title of the foreground window, cached for 1s.
/// Falls back to an empty string when the call fails.
pub fn foreground_title() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if let Ok(guard) = LAST_TITLE.lock() {
        if let Some((ts, title)) = guard.as_ref() {
            if now.saturating_sub(*ts) < 1000 {
                return title.clone();
            }
        }
    }
    let title = read_title();
    if let Ok(mut guard) = LAST_TITLE.lock() {
        *guard = Some((now, title.clone()));
    }
    title
}

fn read_title() -> String {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 256];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
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

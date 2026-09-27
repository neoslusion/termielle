//! Win32 Application Desktop Toolbar (AppBar) and Taskbar Suppressor.
//!
//! Registers the bar with the Windows shell so desktop space is reserved
//! (maximized windows dock cleanly against the bar edge), and optionally
//! hides the native Windows 11 Taskbar (`Shell_TrayWnd`) during runtime,
//! safely restoring it on exit.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Shell::{
    ABE_BOTTOM, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA, SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetSystemMetrics, IsWindowVisible, SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_SHOW,
    ShowWindow,
};
use windows::core::w;

static APPBAR_REGISTERED: AtomicBool = AtomicBool::new(false);
static TASKBAR_HIDDEN: AtomicBool = AtomicBool::new(false);
/// Raw HWND of the registered AppBar, so teardown paths without a window
/// handle (panic hook, Drop) can still send ABM_REMOVE. 0 when unregistered.
static APPBAR_HWND: AtomicIsize = AtomicIsize::new(0);
/// Last reserved geometry (top edge, height px, monitor rect). The shell
/// reservation must track DPI/zoom/monitor changes not just anchor flips:
/// a resized bar with a stale reservation overlaps maximized windows.
static LAST_TOP: AtomicBool = AtomicBool::new(true);
static LAST_HEIGHT: AtomicU32 = AtomicU32::new(0);
static LAST_RECT: std::sync::Mutex<Option<RECT>> = std::sync::Mutex::new(None);

/// Registers `hwnd` as an Application Desktop Toolbar on the top or bottom
/// edge. `height_px` is DEVICE pixels (the shell is per-monitor aware);
/// callers must scale the logical bar height by the render scale first.
/// `monitor_rect` is the full physical monitor bounds the reservation docks
/// against. Stores the HWND so [`leave_bar_shell`] can tear down without one.
pub fn register_appbar(hwnd: HWND, top: bool, height_px: u32, monitor_rect: Option<RECT>) -> bool {
    APPBAR_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
    let mut data = APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        uCallbackMessage: 0,
        uEdge: if top { ABE_TOP } else { ABE_BOTTOM },
        rc: RECT::default(),
        lParam: windows::Win32::Foundation::LPARAM(0),
    };

    let res = unsafe { SHAppBarMessage(ABM_NEW, &raw mut data) };
    if res == 0 {
        return false;
    }

    let rect = monitor_rect.unwrap_or_else(|| {
        let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
        RECT {
            left: 0,
            top: 0,
            right: screen_w,
            bottom: screen_h,
        }
    });

    if top {
        data.rc = RECT {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.top + height_px as i32,
        };
    } else {
        data.rc = RECT {
            left: rect.left,
            top: rect.bottom - height_px as i32,
            right: rect.right,
            bottom: rect.bottom,
        };
    }

    unsafe {
        let _ = SHAppBarMessage(ABM_QUERYPOS, &raw mut data);
        let _ = SHAppBarMessage(ABM_SETPOS, &raw mut data);
    }

    APPBAR_REGISTERED.store(true, Ordering::SeqCst);
    true
}

/// Re-registers the AppBar (monitor follow): removes any prior registration
/// first, so repeated calls never stack `ABM_NEW` on the same HWND.
pub fn reregister_appbar(
    hwnd: HWND,
    top: bool,
    height_px: u32,
    monitor_rect: Option<RECT>,
) -> bool {
    unregister_appbar(hwnd);
    register_appbar(hwnd, top, height_px, monitor_rect)
}
/// Ensures the shell reservation matches the live bar geometry: registers on
/// first call, re-registers when edge, height, or monitor change, and no-ops
/// otherwise. Call on every present; ABM traffic happens only on real change.
/// `height_px` is device pixels, `monitor_rect` the full physical bounds.
pub fn ensure_appbar(hwnd: HWND, top: bool, height_px: u32, monitor_rect: RECT) -> bool {
    let same_rect = LAST_RECT
        .lock()
        .map(|guard| *guard == Some(monitor_rect))
        .unwrap_or(false);
    if APPBAR_REGISTERED.load(Ordering::SeqCst)
        && LAST_TOP.load(Ordering::SeqCst) == top
        && LAST_HEIGHT.load(Ordering::SeqCst) == height_px
        && same_rect
    {
        return true;
    }
    if !reregister_appbar(hwnd, top, height_px, Some(monitor_rect)) {
        return false;
    }
    LAST_TOP.store(top, Ordering::SeqCst);
    LAST_HEIGHT.store(height_px, Ordering::SeqCst);
    if let Ok(mut guard) = LAST_RECT.lock() {
        *guard = Some(monitor_rect);
    }
    true
}

/// Leaves all bar shell state: unregisters the AppBar (via the stored HWND)
/// and restores the taskbar. Idempotent; safe on every exit path including
/// the panic hook and `Drop`, which have no window handle.
pub fn leave_bar_shell() {
    // The flag lives in unregister_appbar; the stored HWND is only routing.
    // Swap first so a concurrent register cannot leak between the two calls.
    let raw = APPBAR_HWND.swap(0, Ordering::SeqCst) as *mut std::ffi::c_void;
    if !raw.is_null() {
        unregister_appbar(HWND(raw));
    }
    restore_taskbar();
}

/// Unregisters `hwnd` as an AppBar and restores the desktop work area.
pub fn unregister_appbar(hwnd: HWND) {
    // Single window for the process lifetime, so an unconditional clear is safe.
    APPBAR_HWND.store(0, Ordering::SeqCst);
    // Forget the reservation geometry so a later ensure re-registers.
    LAST_HEIGHT.store(u32::MAX, Ordering::SeqCst);
    if let Ok(mut guard) = LAST_RECT.lock() {
        *guard = None;
    }
    if !APPBAR_REGISTERED.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut data = APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        uCallbackMessage: 0,
        uEdge: 0,
        rc: RECT::default(),
        lParam: windows::Win32::Foundation::LPARAM(0),
    };
    unsafe {
        let _ = SHAppBarMessage(ABM_REMOVE, &raw mut data);
    }
}

/// Safely hides the Windows 11 taskbar (`Shell_TrayWnd` and secondary taskbars).
pub fn hide_taskbar() {
    if let Ok(main_tray) = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) } {
        if !main_tray.is_invalid() {
            let _ = unsafe { ShowWindow(main_tray, SW_HIDE) };
        }
    }
    if let Ok(sec_tray) = unsafe { FindWindowW(w!("Shell_SecondaryTrayWnd"), None) } {
        if !sec_tray.is_invalid() {
            let _ = unsafe { ShowWindow(sec_tray, SW_HIDE) };
        }
    }
    TASKBAR_HIDDEN.store(true, Ordering::SeqCst);
}

/// Re-asserts the hidden taskbar if something else showed it again.
///
/// Explorer puts its taskbar back whenever it restarts, re-creates the
/// taskbar, or reacts to a settings/display change. There is no notification
/// we can register for on every Windows build, so the replacement mode
/// re-checks on a slow timer instead: two window lookups every few seconds
/// cost nothing and survive shell restarts the user did not ask for.
pub fn ensure_taskbar_hidden() {
    for class in [w!("Shell_TrayWnd"), w!("Shell_SecondaryTrayWnd")] {
        if let Ok(hwnd) = unsafe { FindWindowW(class, None) } {
            if !hwnd.is_invalid() && unsafe { IsWindowVisible(hwnd).as_bool() } {
                let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
            }
        }
    }
    TASKBAR_HIDDEN.store(true, Ordering::SeqCst);
}

/// Restores the native taskbar windows, but only when this process is the one
/// that hid them. A user who keeps the taskbar on auto-hide never sees it
/// forced open by a clean exit.
pub fn restore_taskbar() {
    if TASKBAR_HIDDEN.swap(false, Ordering::SeqCst) {
        show_taskbar_windows();
    }
}

/// Restores the taskbar unconditionally. Used by the watchdog process, which
/// never hid the taskbar itself and therefore has no local flag to consult.
pub fn restore_taskbar_force() {
    TASKBAR_HIDDEN.store(false, Ordering::SeqCst);
    show_taskbar_windows();
}

fn show_taskbar_windows() {
    if let Ok(main_tray) = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) } {
        if !main_tray.is_invalid() {
            let _ = unsafe { ShowWindow(main_tray, SW_SHOW) };
        }
    }
    if let Ok(sec_tray) = unsafe { FindWindowW(w!("Shell_SecondaryTrayWnd"), None) } {
        if !sec_tray.is_invalid() {
            let _ = unsafe { ShowWindow(sec_tray, SW_SHOW) };
        }
    }
}

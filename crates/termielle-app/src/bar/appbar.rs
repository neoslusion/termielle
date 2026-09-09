//! Win32 Application Desktop Toolbar (AppBar) and Taskbar Suppressor.
//!
//! Registers the bar with the Windows shell so desktop space is reserved
//! (maximized windows dock cleanly against the bar edge), and optionally
//! hides the native Windows 11 Taskbar (`Shell_TrayWnd`) during runtime,
//! safely restoring it on exit.

use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Shell::{
    ABE_BOTTOM, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA,
    SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_SHOW, ShowWindow,
};
use windows::core::w;

static APPBAR_REGISTERED: AtomicBool = AtomicBool::new(false);
static TASKBAR_HIDDEN: AtomicBool = AtomicBool::new(false);

/// Registers `hwnd` as an Application Desktop Toolbar on `edge` (top=1, bottom=3).
pub fn register_appbar(hwnd: HWND, top: bool, height: u32, monitor_rect: Option<RECT>) -> bool {
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
            bottom: rect.top + height as i32,
        };
    } else {
        data.rc = RECT {
            left: rect.left,
            top: rect.bottom - height as i32,
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

/// Unregisters `hwnd` as an AppBar and restores the desktop work area.
pub fn unregister_appbar(hwnd: HWND) {
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

/// Restores the native Windows taskbar windows if they were hidden.
pub fn restore_taskbar() {
    if !TASKBAR_HIDDEN.swap(false, Ordering::SeqCst) {
        return;
    }
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

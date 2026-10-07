//! Content-free display/fullscreen metadata; no titles, input or desktop pixels.
use crate::window::WakeHandle;
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, RECT},
        Graphics::{
            Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
            Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW},
        },
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            WindowsAndMessaging::*,
        },
    },
    core::BOOL,
};
#[derive(Clone, Debug)]
pub struct Display {
    pub device: String,
    pub bounds: (i32, i32, i32, i32),
    pub primary: bool,
    pub(crate) handle: HMONITOR,
}
unsafe extern "system" fn enum_display(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    parameter: LPARAM,
) -> BOOL {
    let result = std::panic::catch_unwind(|| {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }.as_bool() {
            let end = info
                .szDevice
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(info.szDevice.len());
            let rect = info.monitorInfo.rcMonitor;
            unsafe { &mut *(parameter.0 as *mut Vec<Display>) }.push(Display {
                device: String::from_utf16_lossy(&info.szDevice[..end]),
                bounds: (rect.left, rect.top, rect.right, rect.bottom),
                primary: info.monitorInfo.dwFlags & 1 != 0,
                handle: monitor,
            });
        }
    });
    BOOL(i32::from(result.is_ok()))
}
pub fn displays() -> Vec<Display> {
    // Stable physical metadata even before the first overlay establishes DPI.
    struct Restore(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT);
    impl Drop for Restore {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                unsafe {
                    windows::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(self.0);
                }
            }
        }
    }
    let _restore = Restore(unsafe {
        windows::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        )
    });
    let mut list: Vec<Display> = Vec::new();
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(enum_display),
            LPARAM((&mut list as *mut Vec<Display>) as isize),
        )
    };
    list.sort_by(|a, b| b.primary.cmp(&a.primary).then(a.device.cmp(&b.device)));
    list
}
pub fn covers_monitor(window: (i32, i32, i32, i32), monitor: (i32, i32, i32, i32)) -> bool {
    let (l, t, r, b) = window;
    let (ml, mt, mr, mb) = monitor;
    l < r
        && t < b
        && ml < mr
        && mt < mb
        && i64::from(l) <= i64::from(ml) + 1
        && i64::from(t) <= i64::from(mt) + 1
        && i64::from(r) >= i64::from(mr) - 1
        && i64::from(b) >= i64::from(mb) - 1
}
pub fn foreground_fullscreen(monitor: RECT) -> bool {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid()
        || !unsafe { IsWindowVisible(hwnd) }.as_bool()
        || unsafe { IsIconic(hwnd) }.as_bool()
    {
        return false;
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == std::process::id() {
        return false;
    }
    let mut class = [0u16; 128];
    let length = unsafe { GetClassNameW(hwnd, &mut class) };
    let class = String::from_utf16_lossy(&class[..length.max(0) as usize]);
    if matches!(
        class.as_str(),
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "MultitaskingViewFrame"
    ) {
        return false;
    }
    let mut rect = RECT::default();
    if unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut rect as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
    }
    .is_err()
        && unsafe { GetWindowRect(hwnd, &mut rect) }.is_err()
    {
        return false;
    }
    covers_monitor(
        (rect.left, rect.top, rect.right, rect.bottom),
        (monitor.left, monitor.top, monitor.right, monitor.bottom),
    )
}
struct Listener {
    wake: WakeHandle,
    pending: Arc<AtomicBool>,
}
thread_local! { static LISTENERS:RefCell<HashMap<isize,Listener>>=RefCell::new(HashMap::new()); }
unsafe extern "system" fn event(
    hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    let _ = std::panic::catch_unwind(|| {
        // Location changes are very frequent; ignore non-window objects and
        // non-foreground windows before touching the coalescing state.
        if event == EVENT_OBJECT_LOCATIONCHANGE
            && (object != OBJID_WINDOW.0 || hwnd != unsafe { GetForegroundWindow() })
        {
            return;
        }
        LISTENERS.with(|map| {
            if let Ok(map) = map.try_borrow() {
                if let Some(listener) = map.get(&(hook.0 as isize)) {
                    if !listener.pending.swap(true, Ordering::AcqRel)
                        && listener.wake.post().is_err()
                    {
                        listener.pending.store(false, Ordering::Release)
                    }
                }
            }
        });
    });
}
/// GUI-thread-owned, out-of-context WinEvent observation. Drop unhooks all
/// callbacks before its owner disappears. No DLL injection/keyboard hooks.
pub struct VisibilityWatch {
    hooks: Vec<HWINEVENTHOOK>,
    pending: Arc<AtomicBool>,
    enabled: bool,
}
impl Default for VisibilityWatch {
    fn default() -> Self {
        Self {
            hooks: Vec::new(),
            pending: Arc::new(AtomicBool::new(false)),
            enabled: false,
        }
    }
}
impl VisibilityWatch {
    pub fn configure(&mut self, enabled: bool, wake: WakeHandle) {
        if enabled == self.enabled {
            return;
        }
        self.clear();
        self.enabled = enabled;
        if !enabled {
            return;
        }
        for (min, max) in [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
            (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
        ] {
            let hook = unsafe {
                SetWinEventHook(min, max, None, Some(event), 0, 0, WINEVENT_OUTOFCONTEXT)
            };
            if !hook.is_invalid() {
                LISTENERS.with(|map| {
                    map.borrow_mut().insert(
                        hook.0 as isize,
                        Listener {
                            wake,
                            pending: self.pending.clone(),
                        },
                    );
                });
                self.hooks.push(hook);
            }
        }
        self.pending.store(true, Ordering::Release);
    }
    pub fn take_change(&self) -> bool {
        self.pending.swap(false, Ordering::AcqRel)
    }
    pub fn needs_fallback(&self) -> bool {
        self.enabled && self.hooks.len() != 3
    }
    fn clear(&mut self) {
        for hook in self.hooks.drain(..) {
            LISTENERS.with(|map| {
                map.borrow_mut().remove(&(hook.0 as isize));
            });
            let _ = unsafe { UnhookWinEvent(hook) };
        }
    }
}
impl Drop for VisibilityWatch {
    fn drop(&mut self) {
        self.clear();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visibility_watch_owns_and_releases_all_registered_callbacks() {
        std::thread::spawn(|| {
            let owner =
                crate::window::OverlayWindow::create(&termielle_core::AppConfig::default(), true)
                    .unwrap();
            let mut watch = VisibilityWatch::default();
            watch.configure(true, owner.wake_handle());
            assert!(watch.take_change());
            assert!(!watch.take_change());
            assert_eq!(watch.needs_fallback(), watch.hooks.len() != 3);
            assert_eq!(LISTENERS.with(|m| m.borrow().len()), watch.hooks.len());
            watch.configure(false, owner.wake_handle());
            assert!(LISTENERS.with(|m| m.borrow().is_empty()));
            watch.configure(true, owner.wake_handle());
            drop(watch);
            assert!(LISTENERS.with(|m| m.borrow().is_empty()));
        })
        .join()
        .unwrap();
    }
    #[test]
    fn fullscreen_coverage_not_maximized_work_area() {
        assert!(covers_monitor((0, 0, 1920, 1080), (0, 0, 1920, 1080)));
        assert!(!covers_monitor((0, 0, 1920, 1040), (0, 0, 1920, 1080)));
        assert!(!covers_monitor((0, 0, 1920, 1080), (1920, 0, 3840, 1080)));
        assert!(covers_monitor((-1920, -1080, 0, 0), (-1920, -1080, 0, 0)));
        assert!(!covers_monitor((0, 0, 0, 0), (0, 0, 1920, 1080)));
        assert!(!covers_monitor((0, 0, 10, 10), (0, 0, 0, 0)));
        assert!(covers_monitor(
            (i32::MIN, i32::MIN, i32::MAX, i32::MAX),
            (i32::MIN, i32::MIN, i32::MAX, i32::MAX)
        ));
    }
}

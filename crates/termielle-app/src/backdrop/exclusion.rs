//! Scope capture exclusion to our own desktop copy, never the app lifetime.
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowDisplayAffinity, GetWindowThreadProcessId, SetWindowDisplayAffinity,
    WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WINDOW_DISPLAY_AFFINITY,
};
use windows::core::{s, w};

pub(super) struct PhysicalDpi(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT);
impl PhysicalDpi {
    pub fn enter() -> Self {
        Self(unsafe {
            windows::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(
                windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            )
        })
    }
}
impl Drop for PhysicalDpi {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            let _ = unsafe { windows::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(self.0) };
        }
    }
}

fn supported() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let Ok(module) = (unsafe { GetModuleHandleW(w!("ntdll.dll")) }) else {
            return false;
        };
        let Some(proc) = (unsafe { GetProcAddress(module, s!("RtlGetVersion")) }) else {
            return false;
        };
        type VersionFn = unsafe extern "system" fn(*mut OSVERSIONINFOW) -> i32;
        // SAFETY: this named SDK export has the RtlGetVersion ABI; ntdll is
        // process-resident. Query the real build, not manifest virtualization.
        let query =
            unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, VersionFn>(proc) };
        let mut version = OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
            ..Default::default()
        };
        (unsafe { query(&mut version) }) >= 0
            && version.dwMajorVersion >= 10
            && version.dwBuildNumber >= 19041
    })
}
fn owner(hwnd: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

pub(super) struct Exclusion {
    hwnd: HWND,
    previous: u32,
    changed: bool,
}
impl Exclusion {
    pub fn begin(hwnd: isize) -> Option<Self> {
        let hwnd = HWND(hwnd as *mut _);
        if hwnd.0.is_null() || owner(hwnd) != std::process::id() || !supported() {
            return None;
        }
        let mut previous = 0;
        unsafe { GetWindowDisplayAffinity(hwnd, &mut previous) }.ok()?;
        // Do not weaken a caller's explicit capture protection.
        if previous != WDA_NONE.0 && previous != WDA_EXCLUDEFROMCAPTURE.0 {
            return None;
        }
        let changed = previous != WDA_EXCLUDEFROMCAPTURE.0;
        if changed {
            unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) }.ok()?;
        }
        let guard = Self {
            hwnd,
            previous,
            changed,
        };
        // Wait only on the background capture thread. The physical window
        // remains visible; the desktop copy must see the updated capture policy.
        unsafe { DwmFlush() }.ok()?;
        Some(guard)
    }
}
impl Drop for Exclusion {
    fn drop(&mut self) {
        if self.changed && owner(self.hwnd) == std::process::id() {
            let _ = unsafe {
                SetWindowDisplayAffinity(self.hwnd, WINDOW_DISPLAY_AFFINITY(self.previous))
            };
            let _ = unsafe { DwmFlush() };
        }
    }
}

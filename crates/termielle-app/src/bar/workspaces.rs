//! Windows virtual desktops: query actual state and switch by absolute index.

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

struct DesktopApartment(bool);
impl Drop for DesktopApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

thread_local! {
    // winvd caches apartment-bound proxies. Short-lived WIC/audio callers must
    // not tear down that apartment between desktop queries.
    static APARTMENT: DesktopApartment = DesktopApartment(
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok()
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub total: usize,
    pub active: usize,
}

pub fn try_query_workspaces() -> winvd::Result<WorkspaceSnapshot> {
    APARTMENT.with(|_| ());
    Ok(WorkspaceSnapshot {
        total: winvd::get_desktop_count()? as usize,
        active: winvd::get_current_desktop()?.get_index()? as usize + 1,
    })
}

pub fn query_workspaces() -> WorkspaceSnapshot {
    // Unsupported shells must not display invented, nonfunctional desktops.
    try_query_workspaces().unwrap_or(WorkspaceSnapshot {
        total: 0,
        active: 0,
    })
}

/// Switches to a one-based desktop number without injecting global shortcuts.
pub fn switch_workspace(target: usize) -> winvd::Result<()> {
    APARTMENT.with(|_| ());
    if target == 0 || target > winvd::get_desktop_count()? as usize {
        return Ok(());
    }
    winvd::switch_desktop((target - 1) as u32)
}

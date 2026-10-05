//! Native boundaries for explicit app navigation. No inferred command lines.
use termielle_core::{AppLaunchTarget, PinnedApp};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Shell::{
    SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowThreadProcessId, IsWindowVisible, PostMessageW, SW_SHOWNORMAL, WM_CLOSE,
};
use windows::core::{PCWSTR, PWSTR, w};

pub(crate) fn application_for_process(pid: u32) -> Option<PinnedApp> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let result = (|| {
        let mut path = vec![0u16; 4096];
        let mut size = path.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(path.as_mut_ptr()),
                &mut size,
            )
        }
        .ok()?;
        let path = String::from_utf16(&path[..size as usize]).ok()?;
        let stem = path
            .rsplit(['\\', '/'])
            .next()?
            .strip_suffix(".exe")
            .unwrap_or("Application");
        let name = match stem.to_lowercase().as_str() {
            "explorer" => "File Explorer",
            "msedge" => "Microsoft Edge",
            "chrome" => "Google Chrome",
            "code" => "Visual Studio Code",
            "windowsterminal" => "Windows Terminal",
            "ms-teams" => "Microsoft Teams",
            _ => stem,
        }
        .to_string();
        let mut id = vec![0u16; 257];
        let mut length = id.len() as u32;
        let status = unsafe {
            GetApplicationUserModelId(process, &mut length, Some(PWSTR(id.as_mut_ptr())))
        };
        let target = if status.0 == 0 && length > 1 && length <= id.len() as u32 {
            AppLaunchTarget::AppUserModelId(String::from_utf16(&id[..length as usize - 1]).ok()?)
        } else {
            AppLaunchTarget::Executable(path)
        };
        let app = PinnedApp { name, target };
        app.is_valid().then_some(app)
    })();
    let _ = unsafe { CloseHandle(process) };
    result
}

/// Posts the application's ordinary Close request. Never force-terminates;
/// unsaved-document prompts remain owned by the application.
pub fn close_window(hwnd: isize, pid: u32) -> bool {
    let hwnd = HWND(hwnd as *mut _);
    let mut owner = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
    if pid == 0 || owner != pid {
        return false;
    }
    if unsafe { IsWindowVisible(hwnd) }.as_bool() {
        crate::tasks::activate_associated_window(hwnd.0 as isize, pid);
    }
    unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }.is_ok()
}

/// Runs off the GUI thread; returns an HRESULT for user-visible failure feedback.
pub fn launch(app: PinnedApp, wake: crate::window::WakeHandle) {
    std::thread::spawn(move || {
        let result = (|| -> windows::core::Result<()> {
            if !app.is_valid() {
                return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80070057u32 as i32,
                )));
            }
            let _apartment = crate::apartment::Apartment::new(
                windows::Win32::System::WinRT::RO_INIT_SINGLETHREADED,
            )?;
            let file = match app.target {
                AppLaunchTarget::Executable(path) => {
                    if !std::path::Path::new(&path).is_file() {
                        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                            0x80070002u32 as i32,
                        )));
                    }
                    path
                }
                AppLaunchTarget::AppUserModelId(id) => format!("shell:AppsFolder\\{id}"),
            };
            let file: Vec<u16> = file.encode_utf16().chain(Some(0)).collect();
            let mut info = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
                lpVerb: w!("open"),
                lpFile: PCWSTR(file.as_ptr()),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            unsafe { ShellExecuteExW(&mut info) }
        })();
        wake.post_app_launch_result(result.err().map_or(0, |e| e.code().0));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_process_identity_is_read_only_and_has_a_valid_explicit_target() {
        let app =
            application_for_process(std::process::id()).expect("own process image is queryable");
        assert!(app.is_valid());
        assert!(matches!(app.target, AppLaunchTarget::Executable(_)));
        assert!(application_for_process(0).is_none());
    }
}

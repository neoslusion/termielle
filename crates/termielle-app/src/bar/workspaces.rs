//! Virtual Desktops / Workspaces tracking and switching.

use std::sync::atomic::{AtomicUsize, Ordering};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, KEY_READ, REG_SAM_FLAGS, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
    VK_CONTROL, VK_LEFT, VK_LWIN, VK_RIGHT,
};
use windows::core::w;

static ACTIVE_WORKSPACE: AtomicUsize = AtomicUsize::new(1);
static TOTAL_WORKSPACES: AtomicUsize = AtomicUsize::new(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub total: usize,
    pub active: usize,
}

/// Discovers total virtual desktops from Windows registry and returns current snapshot.
pub fn query_workspaces() -> WorkspaceSnapshot {
    let mut count = 0usize;
    unsafe {
        let mut hkey = windows::Win32::System::Registry::HKEY::default();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\VirtualDesktops\\Desktops"),
            None,
            REG_SAM_FLAGS(KEY_READ.0),
            &raw mut hkey,
        );
        if status.is_ok() && !hkey.is_invalid() {
            let mut index = 0u32;
            let mut name_buf = [0u16; 256];
            loop {
                let mut name_len = name_buf.len() as u32;
                if RegEnumKeyExW(
                    hkey,
                    index,
                    Some(windows::core::PWSTR(name_buf.as_mut_ptr())),
                    &raw mut name_len,
                    None,
                    None,
                    None,
                    None,
                )
                .is_ok()
                {
                    count += 1;
                    index += 1;
                } else {
                    break;
                }
            }
            let _ = RegCloseKey(hkey);
        }
    }

    let total = if count > 0 { count.clamp(1, 10) } else { 4 };
    TOTAL_WORKSPACES.store(total, Ordering::Relaxed);
    let active = ACTIVE_WORKSPACE.load(Ordering::Relaxed).clamp(1, total);

    WorkspaceSnapshot { total, active }
}

/// Switches to `target` workspace index (1-indexed).
pub fn switch_workspace(target: usize) {
    let total = TOTAL_WORKSPACES.load(Ordering::Relaxed);
    let target = target.clamp(1, total);
    let current = ACTIVE_WORKSPACE.load(Ordering::Relaxed);
    if target == current {
        return;
    }

    if target > current {
        for _ in 0..(target - current) {
            send_desktop_switch(true);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    } else {
        for _ in 0..(current - target) {
            send_desktop_switch(false);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    }

    ACTIVE_WORKSPACE.store(target, Ordering::Relaxed);
}

fn send_desktop_switch(forward: bool) {
    let key = if forward { VK_RIGHT } else { VK_LEFT };
    let inputs = [
        make_key_input(VK_CONTROL, false),
        make_key_input(VK_LWIN, false),
        make_key_input(key, false),
        make_key_input(key, true),
        make_key_input(VK_LWIN, true),
        make_key_input(VK_CONTROL, true),
    ];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn make_key_input(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

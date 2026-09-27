//! Windows shell affordances used by full taskbar-replacement mode.
//!
//! These actions reuse shell-owned surfaces and hotkeys. Termielle does not
//! reimplement the Start menu, search UI, or notification area.

use std::process::Command;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_A, VK_D, VK_LWIN, VK_S, VK_TAB, keybd_event,
};

/// A taskbar-equivalent shell action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellAction {
    Start,
    Search,
    TaskView,
    SystemTray,
    ShowDesktop,
    Clock,
}

impl ShellAction {
    /// Every action owns one stable negative hit id inside the bar range.
    pub fn hit_id(self) -> isize {
        crate::bar::HIT_BAR_SHELL_BASE - self as isize
    }

    /// Resolves a bar hit id back to its shell action.
    pub fn from_hit(id: isize) -> Option<Self> {
        let offset = crate::bar::HIT_BAR_SHELL_BASE - id;
        match offset {
            0 => Some(Self::Start),
            1 => Some(Self::Search),
            2 => Some(Self::TaskView),
            3 => Some(Self::SystemTray),
            4 => Some(Self::ShowDesktop),
            5 => Some(Self::Clock),
            _ => None,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Search => "Find",
            Self::TaskView => "View",
            Self::SystemTray => "Tray",
            Self::ShowDesktop => "Desk",
            Self::Clock => "Clock",
        }
    }

    /// The controls painted in the bar, in render order. The clock is not
    /// one of them: it rides the right module zone with the other metrics.
    pub const ALL: [Self; 5] = [
        Self::Start,
        Self::Search,
        Self::TaskView,
        Self::SystemTray,
        Self::ShowDesktop,
    ];

    /// Painted width of one control in logical pixels. The bar's 10 px type
    /// is never clipped, and a 32 px floor keeps the row reading as evenly
    /// weighted chips rather than a ragged word list.
    pub fn control_width(self) -> u32 {
        (self.label().chars().count() as u32 * 7 + 14).max(32)
    }

    pub(crate) const fn label_of(self) -> &'static str {
        self.label()
    }
}

fn open_shell_uri(uri: &str) {
    let _ = Command::new("explorer.exe").arg(uri).spawn();
}

/// Taps one key with the Windows key held: the chords the taskbar itself
/// uses. Windows 11's Start, Search, Task View, tray, and Show Desktop are
/// shell-owned flyouts, not shell namespaces — `shell:` URIs open File
/// Explorer windows instead of the surface the user asked for.
fn tap_with_win(key: u8) {
    let flags = KEYBD_EVENT_FLAGS(0);
    let up = KEYEVENTF_KEYUP;
    unsafe {
        keybd_event(VK_LWIN.0 as u8, 0, flags, 0);
        keybd_event(key, 0, flags, 0);
        keybd_event(key, 0, up, 0);
        keybd_event(VK_LWIN.0 as u8, 0, up, 0);
    }
}

/// Taps a single key, held for long enough to register as a tap.
fn tap(key: u8) {
    let flags = KEYBD_EVENT_FLAGS(0);
    let up = KEYEVENTF_KEYUP;
    unsafe {
        keybd_event(key, 0, flags, 0);
        keybd_event(key, 0, up, 0);
    }
}

/// Opens the shell-owned surface for one bar action.
pub fn activate(action: ShellAction) {
    match action {
        ShellAction::Start => tap(VK_LWIN.0 as u8),
        ShellAction::Search => tap_with_win(VK_S.0 as u8),
        ShellAction::TaskView => tap_with_win(VK_TAB.0 as u8),
        ShellAction::SystemTray => tap_with_win(VK_A.0 as u8),
        ShellAction::ShowDesktop => tap_with_win(VK_D.0 as u8),
        // The clock flyout is a real shell URI, not a key chord.
        ShellAction::Clock => open_shell_uri("ms-clock:"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_hit_ids_round_trip() {
        for action in ShellAction::ALL {
            assert_eq!(ShellAction::from_hit(action.hit_id()), Some(action));
        }
        assert_eq!(ShellAction::from_hit(0), None);
    }
}

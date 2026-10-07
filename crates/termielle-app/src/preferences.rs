//! Native, keyboard/screen-reader-friendly preferences. Drafts never write files.
pub mod model;
use crate::win32_ptr::{
    set_window_long_ptr as SetWindowLongPtrW, window_long_ptr as GetWindowLongPtrW,
};
use model::Request;
use std::{cell::Cell, collections::HashMap, sync::mpsc};
use termielle_core::{BarPosition, IslandConfig, IslandLayout};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{
            CLIP_DEFAULT_PRECIS, COLOR_WINDOW, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
            DEFAULT_QUALITY, DeleteObject, FF_DONTCARE, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
    },
    core::{PCWSTR, w},
};
thread_local! {static DIALOG:Cell<HWND>=const {Cell::new(HWND(std::ptr::null_mut()))};}
const PAGE: i32 = 101;
const LAYOUT: i32 = 102;
const EDGE: i32 = 103;
const HEIGHT: i32 = 104;
const THEME: i32 = 105;
const HOVER: i32 = 106;
const FACE: i32 = 107;
const BLUR: i32 = 108;
const SHOW_NAME: i32 = 109;
const DISPLAY: i32 = 160;
const FULLSCREEN: i32 = 161;
const ANIMATION: i32 = 162;
const LEFT: i32 = 111;
const RIGHT: i32 = 112;
const PINS: i32 = 120;
const UP: i32 = 121;
const DOWN: i32 = 122;
const REMOVE: i32 = 123;
const PREVIEW: i32 = 130;
const APPLY: i32 = 131;
const REVERT: i32 = 132;
const CLOSE: i32 = 133;
const STATUS: i32 = 140;
const SOUND: i32 = 150;
const NETWORK: i32 = 151;
const POWER: i32 = 152;
const DATE: i32 = 153;
const TASKVIEW: i32 = 154;
const TURN_OFF: i32 = 155;
struct Child {
    hwnd: HWND,
    page: Option<usize>,
    rect: (i32, i32, i32, i32),
}
struct State {
    baseline: IslandConfig,
    draft: IslandConfig,
    sender: mpsc::Sender<Request>,
    wake: crate::window::WakeHandle,
    children: Vec<Child>,
    controls: HashMap<i32, HWND>,
    font: HFONT,
    page: usize,
    themes: Vec<String>,
    heights: Vec<u32>,
    monitors: Vec<termielle_core::MonitorSelection>,
}
impl State {
    fn emit(&self, request: Request) {
        if self.sender.send(request).is_ok() {
            let _ = self.wake.post();
        }
    }
    fn text(&self, id: i32) -> String {
        let hwnd = self.controls[&id];
        let length = unsafe { GetWindowTextLengthW(hwnd) }.clamp(0, 4096);
        let mut buf = vec![0u16; length as usize + 1];
        let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
    fn selected(&self, id: i32) -> usize {
        unsafe { SendMessageW(self.controls[&id], CB_GETCURSEL, None, None) }
            .0
            .max(0) as usize
    }
    fn check(&self, id: i32) -> bool {
        unsafe { SendMessageW(self.controls[&id], BM_GETCHECK, None, None) }.0 == 1
    }
    fn read(&mut self) -> Result<IslandConfig, String> {
        self.draft.layout = [
            IslandLayout::Bar,
            IslandLayout::Notch,
            IslandLayout::Island,
            IslandLayout::Classic,
        ][self.selected(LAYOUT).min(3)];
        self.draft.bar.position = if self.selected(EDGE) == 0 {
            BarPosition::Top
        } else {
            BarPosition::Bottom
        };
        self.draft.bar.height = self.heights[self.selected(HEIGHT).min(self.heights.len() - 1)];
        self.draft.theme = self.themes[self.selected(THEME).min(self.themes.len() - 1)].clone();
        self.draft.expand_on_hover = self.check(HOVER);
        self.draft.face_animated = self.check(FACE);
        self.draft.show_name = self.check(SHOW_NAME);
        self.draft.monitor =
            self.monitors[self.selected(DISPLAY).min(self.monitors.len() - 1)].clone();
        self.draft.hide_on_fullscreen = self.check(FULLSCREEN);
        let feel = self.selected(ANIMATION);
        model::set_animation_feel(&mut self.draft, feel);
        self.draft.glass.blur_radius = if self.selected(BLUR) == 1 {
            0
        } else {
            if self.baseline.glass.blur_radius > 0 {
                self.baseline.glass.blur_radius
            } else {
                12
            }
        };
        self.draft.bar.modules_left = self
            .text(LEFT)
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        self.draft.bar.modules_right = self
            .text(RIGHT)
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        model::validate(self.draft.clone(), &self.baseline)
    }
    fn status(&self, text: &str) {
        set_text(self.controls[&STATUS], text);
    }
    fn fill(&mut self) {
        self.draft = self.baseline.clone();
        choose(
            self.controls[&LAYOUT],
            match self.draft.layout {
                IslandLayout::Bar => 0,
                IslandLayout::Notch => 1,
                IslandLayout::Island => 2,
                IslandLayout::Classic => 3,
            },
        );
        choose(
            self.controls[&EDGE],
            usize::from(self.draft.bar.position == BarPosition::Bottom),
        );
        self.heights = vec![28, 36, 44];
        if !self.heights.contains(&self.draft.bar.height) {
            self.heights.push(self.draft.bar.height);
        }
        items(
            self.controls[&HEIGHT],
            &self
                .heights
                .iter()
                .map(|h| format!("{h} logical pixels"))
                .collect::<Vec<_>>(),
        );
        choose(
            self.controls[&HEIGHT],
            self.heights
                .iter()
                .position(|h| *h == self.draft.bar.height)
                .unwrap_or(0),
        );
        self.themes = [
            "auto",
            "catppuccin-macchiato",
            "liquid-dark",
            "light",
            "midnight",
            "transparent",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        if !self.themes.contains(&self.draft.theme) {
            self.themes.push(self.draft.theme.clone());
        }
        items(self.controls[&THEME], &self.themes);
        choose(
            self.controls[&THEME],
            self.themes
                .iter()
                .position(|t| *t == self.draft.theme)
                .unwrap_or(0),
        );
        check(self.controls[&HOVER], self.draft.expand_on_hover);
        check(self.controls[&FACE], self.draft.face_animated);
        check(self.controls[&SHOW_NAME], self.draft.show_name);
        choose(
            self.controls[&BLUR],
            usize::from(self.draft.glass.blur_radius == 0),
        );
        set_text(
            self.controls[&LEFT],
            &self.draft.bar.modules_left.join(", "),
        );
        set_text(
            self.controls[&RIGHT],
            &self.draft.bar.modules_right.join(", "),
        );
        use termielle_core::MonitorSelection;
        self.monitors = vec![
            MonitorSelection::Automatic,
            MonitorSelection::Primary,
            MonitorSelection::Pointer,
        ];
        let mut labels = vec![
            "Automatic — existing layout behavior".into(),
            "Primary display".into(),
            "Follow pointer — recheck at 500ms".into(),
        ];
        for display in crate::desktop::displays() {
            labels.push(format!(
                "{} — {}×{}{}",
                display.device,
                i64::from(display.bounds.2) - i64::from(display.bounds.0),
                i64::from(display.bounds.3) - i64::from(display.bounds.1),
                if display.primary { " (primary)" } else { "" }
            ));
            self.monitors.push(MonitorSelection::Named(display.device));
        }
        if !self.monitors.contains(&self.draft.monitor) {
            labels.push(format!(
                "{:?} — unavailable; primary fallback",
                self.draft.monitor
            ));
            self.monitors.push(self.draft.monitor.clone());
        }
        items(self.controls[&DISPLAY], &labels);
        choose(
            self.controls[&DISPLAY],
            self.monitors
                .iter()
                .position(|v| v == &self.draft.monitor)
                .unwrap_or(0),
        );
        check(self.controls[&FULLSCREEN], self.draft.hide_on_fullscreen);
        items(
            self.controls[&ANIMATION],
            &model::ANIMATION_FEELS
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        );
        choose(
            self.controls[&ANIMATION],
            model::animation_feel(&self.draft),
        );
        self.fill_pins(None);
        self.status(if crate::hosted::is_hosted() {
            "Windhawk edition — Preview is temporary; Apply saves the shared profile."
        } else {
            "Native edition — Preview is temporary; Apply saves your profile."
        });
    }
    fn fill_pins(&self, selection: Option<usize>) {
        let list = self.controls[&PINS];
        unsafe { SendMessageW(list, LB_RESETCONTENT, None, None) };
        for pin in &self.draft.bar.pinned_apps {
            let name = wide(&pin.name);
            unsafe {
                SendMessageW(
                    list,
                    LB_ADDSTRING,
                    None,
                    Some(LPARAM(name.as_ptr() as isize)),
                )
            };
        }
        if let Some(index) = selection {
            unsafe { SendMessageW(list, LB_SETCURSEL, Some(WPARAM(index)), None) };
        }
    }
    fn layout(&mut self, hwnd: HWND) {
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        let scale = dpi as f32 / 96.0;
        let font = unsafe {
            CreateFontW(
                -(15.0 * scale).round() as i32,
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                DEFAULT_QUALITY,
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                w!("Segoe UI"),
            )
        };
        for child in &self.children {
            let (x, y, w, h) = child.rect;
            let s = |v: i32| (v as f32 * scale).round() as i32;
            let _ = unsafe {
                SetWindowPos(
                    child.hwnd,
                    None,
                    s(x),
                    s(y),
                    s(w),
                    s(h),
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            unsafe {
                SendMessageW(
                    child.hwnd,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                )
            };
            let _ = unsafe {
                ShowWindow(
                    child.hwnd,
                    if child.page.is_none_or(|p| p == self.page) {
                        SW_SHOW
                    } else {
                        SW_HIDE
                    },
                )
            };
        }
        let old = std::mem::replace(&mut self.font, font);
        if !old.0.is_null() {
            let _ = unsafe { DeleteObject(HGDIOBJ(old.0)) };
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        if !self.font.0.is_null() {
            let _ = unsafe { DeleteObject(HGDIOBJ(self.font.0)) };
        }
    }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn set_text(hwnd: HWND, text: &str) {
    let text = wide(text);
    let _ = unsafe { SetWindowTextW(hwnd, PCWSTR(text.as_ptr())) };
}
fn choose(hwnd: HWND, index: usize) {
    unsafe { SendMessageW(hwnd, CB_SETCURSEL, Some(WPARAM(index)), None) };
}
fn check(hwnd: HWND, value: bool) {
    unsafe { SendMessageW(hwnd, BM_SETCHECK, Some(WPARAM(usize::from(value))), None) };
}
fn items(hwnd: HWND, values: &[String]) {
    unsafe { SendMessageW(hwnd, CB_RESETCONTENT, None, None) };
    for value in values {
        let value = wide(value);
        unsafe {
            SendMessageW(
                hwnd,
                CB_ADDSTRING,
                None,
                Some(LPARAM(value.as_ptr() as isize)),
            )
        };
    }
}
unsafe extern "system" fn proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize) };
    }
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
    if !ptr.is_null() {
        // Never unwind through user32. Commands operate only on GUI-owned state.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let state = unsafe { &mut *ptr };
            match message {
                WM_CLOSE => {
                    state.emit(Request::Revert);
                    let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
                    return Some(LRESULT(0));
                }
                WM_DPICHANGED => {
                    let rect = unsafe { &*(lparam.0 as *const windows::Win32::Foundation::RECT) };
                    let _ = unsafe {
                        SetWindowPos(
                            hwnd,
                            None,
                            rect.left,
                            rect.top,
                            rect.right - rect.left,
                            rect.bottom - rect.top,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        )
                    };
                    state.layout(hwnd);
                    return Some(LRESULT(0));
                }
                WM_COMMAND => {
                    let id = (wparam.0 & 0xffff) as i32;
                    let code = (wparam.0 >> 16) as u16;
                    if id == PAGE && code == CBN_SELCHANGE as u16 {
                        state.page = state.selected(PAGE);
                        state.layout(hwnd);
                        return Some(LRESULT(0));
                    }
                    if code != 0 {
                        return None;
                    }
                    match id {
                        PREVIEW | APPLY | 1 => match state.read() {
                            Ok(draft) => {
                                if id == PREVIEW {
                                    state.emit(Request::Preview(Box::new(draft)));
                                } else {
                                    state.emit(Request::Apply {
                                        baseline: Box::new(state.baseline.clone()),
                                        draft: Box::new(draft),
                                    });
                                }
                                state.status("Requesting changes...");
                            }
                            Err(error) => state.status(&error),
                        },
                        REVERT => state.emit(Request::Revert),
                        CLOSE | 2 => {
                            state.emit(Request::Revert);
                            let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
                        }
                        UP | DOWN | REMOVE => {
                            let index = unsafe {
                                SendMessageW(state.controls[&PINS], LB_GETCURSEL, None, None)
                            }
                            .0;
                            if index >= 0 {
                                let index = index as usize;
                                if id == REMOVE && index < state.draft.bar.pinned_apps.len() {
                                    state.draft.bar.pinned_apps.remove(index);
                                    state.fill_pins(Some(index.saturating_sub(1)));
                                } else if let Some(target) = model::move_pin(
                                    &mut state.draft,
                                    index,
                                    if id == UP { -1 } else { 1 },
                                ) {
                                    state.fill_pins(Some(target));
                                }
                            }
                        }
                        TURN_OFF => state.emit(Request::TurnOff),
                        SOUND | NETWORK | POWER | DATE | TASKVIEW => {
                            use crate::bar::shell::{self, ShellAction};
                            shell::activate(match id {
                                SOUND => ShellAction::Sound,
                                NETWORK => ShellAction::Network,
                                POWER => ShellAction::Power,
                                DATE => ShellAction::Clock,
                                _ => ShellAction::TaskView,
                            });
                        }
                        _ => return None,
                    }
                    return Some(LRESULT(0));
                }
                WM_NCDESTROY => {
                    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
                    return None;
                }
                _ => {}
            }
            None
        }));
        if let Ok(Some(result)) = result {
            return result;
        }
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

pub struct PreferencesWindow {
    hwnd: HWND,
    state: Box<State>,
    receiver: mpsc::Receiver<Request>,
}
impl PreferencesWindow {
    pub fn create(
        owner: HWND,
        wake: crate::window::WakeHandle,
        config: &IslandConfig,
    ) -> windows::core::Result<Self> {
        let instance = HINSTANCE(unsafe { GetModuleHandleW(None) }?.0);
        static CLASS: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
        let _ = CLASS.get_or_init(|| unsafe {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance,
                lpszClassName: w!("termielle_preferences"),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(
                    (COLOR_WINDOW.0 + 1) as *mut _,
                ),
                ..Default::default()
            })
        });
        let (sender, receiver) = mpsc::channel();
        let mut state = Box::new(State {
            baseline: config.clone(),
            draft: config.clone(),
            sender,
            wake,
            children: Vec::new(),
            controls: HashMap::new(),
            font: HFONT::default(),
            page: 0,
            themes: Vec::new(),
            heights: Vec::new(),
            monitors: Vec::new(),
        });
        let dpi = unsafe { GetDpiForWindow(owner) }.max(96) as f32 / 96.0;
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_CONTROLPARENT,
                w!("termielle_preferences"),
                w!("Termielle preferences"),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                (610.0 * dpi) as i32,
                (510.0 * dpi) as i32,
                Some(owner),
                None,
                Some(instance),
                Some((&mut *state) as *mut State as *const _),
            )
        }?;
        // The owner wrapper is created before controls so partial construction errors
        // destroy the HWND before releasing callback state.
        let mut window = Self {
            hwnd,
            state,
            receiver,
        };
        window.child(
            "STATIC",
            "&Page",
            0,
            None,
            (18, 18, 60, 24),
            WINDOW_STYLE(0),
        )?;
        window.child(
            "COMBOBOX",
            "",
            PAGE,
            None,
            (86, 14, 300, 180),
            WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_TABSTOP | WS_VSCROLL,
        )?;
        items(
            window.state.controls[&PAGE],
            &[
                "Appearance",
                "Bar items",
                "Pinned applications",
                "System / readiness",
                "Behavior / display",
            ]
            .map(str::to_owned),
        );
        choose(window.state.controls[&PAGE], 0);
        for (label, id, y) in [
            ("&Layout", LAYOUT, 66),
            ("&Position", EDGE, 108),
            ("&Density / height", HEIGHT, 150),
            ("&Theme", THEME, 192),
            ("Glass / &recording", BLUR, 234),
        ] {
            window.child(
                "STATIC",
                label,
                0,
                Some(0),
                (18, y, 160, 26),
                WINDOW_STYLE(0),
            )?;
            window.child(
                "COMBOBOX",
                "",
                id,
                Some(0),
                (185, y - 3, 360, 180),
                WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_TABSTOP | WS_VSCROLL,
            )?;
        }
        items(
            window.state.controls[&LAYOUT],
            &["Bar", "Notch", "Island", "Classic"].map(str::to_owned),
        );
        items(
            window.state.controls[&EDGE],
            &["Top", "Bottom"].map(str::to_owned),
        );
        items(
            window.state.controls[&BLUR],
            &[
                "Frosted — brief capture exclusion",
                "Recording-safe — translucent, no desktop copy",
            ]
            .map(str::to_owned),
        );
        window.child(
            "BUTTON",
            "Hover to reveal live activity",
            HOVER,
            Some(0),
            (18, 278, 300, 26),
            WINDOW_STYLE(BS_AUTOCHECKBOX as u32) | WS_TABSTOP,
        )?;
        window.child(
            "BUTTON",
            "Animate the character",
            FACE,
            Some(0),
            (18, 312, 300, 26),
            WINDOW_STYLE(BS_AUTOCHECKBOX as u32) | WS_TABSTOP,
        )?;
        window.child(
            "BUTTON",
            "Show Termielle &name",
            SHOW_NAME,
            Some(0),
            (18, 346, 300, 26),
            WINDOW_STYLE(BS_AUTOCHECKBOX as u32) | WS_TABSTOP,
        )?;
        for (label, id, y) in [
            ("&Left items (comma-separated, in order)", LEFT, 74),
            ("&Right items (comma-separated, in order)", RIGHT, 174),
        ] {
            window.child(
                "STATIC",
                label,
                0,
                Some(1),
                (18, y, 520, 24),
                WINDOW_STYLE(0),
            )?;
            window.child(
                "EDIT",
                "",
                id,
                Some(1),
                (18, y + 30, 530, 28),
                WS_BORDER | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            )?;
        }
        window.child("STATIC","Left: apps, window, workspaces\nRight: network, volume, battery, control_center, clock, cpu, memory\nApps, Control Center and Clock stay reachable during review.",0,Some(1),(18,262,530,78),WINDOW_STYLE(0))?;
        window.child(
            "STATIC",
            "&Pinned apps — changes stay in this draft until Apply",
            0,
            Some(2),
            (18, 66, 540, 26),
            WINDOW_STYLE(0),
        )?;
        window.child(
            "LISTBOX",
            "Pinned applications",
            PINS,
            Some(2),
            (18, 98, 390, 232),
            WS_BORDER | WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(LBS_NOTIFY as u32),
        )?;
        for (label, id, y) in [
            ("Move &up", UP, 100),
            ("Move &down", DOWN, 144),
            ("&Remove pin", REMOVE, 188),
        ] {
            window.child("BUTTON", label, id, Some(2), (425, y, 125, 30), WS_TABSTOP)?;
        }
        window.child("STATIC","Use the app rail's Pin action to add an application.\nThese controls provide a keyboard-accessible alternative to dragging.",0,Some(2),(18,338,540,40),WINDOW_STYLE(0))?;
        for (label, id, x, y) in [
            ("Sound / output devices", SOUND, 18, 70),
            ("Network details", NETWORK, 295, 70),
            ("Battery / power settings", POWER, 18, 112),
            ("Clock / calendar", DATE, 295, 112),
            ("Windows Task View", TASKVIEW, 18, 154),
            ("Turn &Off Termielle...", TURN_OFF, 295, 154),
        ] {
            window.child("BUTTON", label, id, Some(3), (x, y, 250, 32), WS_TABSTOP)?;
        }
        window.child("STATIC","Windows taskbar and notification-area icons remain available.\nQuick Settings is not tray overflow.\n\nReplacement is not ready for automatic enablement: native tray,\ncustom-bar screen readers, multiple monitors, fullscreen, wake and\nrecording checks still require acceptance.\n\nRecording-safe glass avoids capture-exclusion omissions.\nTurn Off exits fully and persists across logins.\nUse Start > Turn Termielle On to resume; settings are retained.",0,Some(3),(18,210,540,168),WINDOW_STYLE(0))?;
        for (label, id, y) in [
            ("&Display", DISPLAY, 70),
            ("Animation &feel", ANIMATION, 130),
        ] {
            window.child(
                "STATIC",
                label,
                0,
                Some(4),
                (18, y, 150, 26),
                WINDOW_STYLE(0),
            )?;
            window.child(
                "COMBOBOX",
                "",
                id,
                Some(4),
                (175, y - 3, 370, 180),
                WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_TABSTOP | WS_VSCROLL,
            )?;
        }
        window.child(
            "BUTTON",
            "Hide pill in &fullscreen (Island / Notch)",
            FULLSCREEN,
            Some(4),
            (18, 195, 530, 28),
            WINDOW_STYLE(BS_AUTOCHECKBOX as u32) | WS_TABSTOP,
        )?;
        window.child("STATIC", "Explicit displays apply to Bar, Island and Notch; Classic freely drags.\nUnavailable displays fall back to primary without rewriting the choice.\nWindows may renumber display names after hardware changes.\nFullscreen hiding never changes native taskbar visibility/reservations.\nThe hidden pill parks its animation clock; session events still arrive.\nCustom preserves your existing animation timings and bounce.", 0, Some(4), (18,238,540,132), WINDOW_STYLE(0))?;
        window.child(
            "STATIC",
            "",
            STATUS,
            None,
            (18, 378, 540, 35),
            WINDOW_STYLE(0),
        )?;
        for (label, id, x) in [
            ("&Preview", PREVIEW, 18),
            ("&Apply", APPLY, 157),
            ("&Revert", REVERT, 296),
            ("&Close", CLOSE, 435),
        ] {
            window.child("BUTTON", label, id, None, (x, 414, 114, 30), WS_TABSTOP)?;
        }
        window.state.fill();
        if crate::hosted::is_hosted() {
            // The host's temporary pill view must not rewrite the standalone layout.
            let _ = unsafe {
                windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
                    window.state.controls[&LAYOUT],
                    false,
                )
            };
        }
        window.state.layout(hwnd);
        Ok(window)
    }
    fn child(
        &mut self,
        class: &str,
        text: &str,
        id: i32,
        page: Option<usize>,
        rect: (i32, i32, i32, i32),
        style: WINDOW_STYLE,
    ) -> windows::core::Result<()> {
        let class = wide(class);
        let text = wide(text);
        let (x, y, w, h) = rect;
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                PCWSTR(text.as_ptr()),
                WS_CHILD | style,
                x,
                y,
                w,
                h,
                Some(self.hwnd),
                Some(HMENU(id as usize as *mut _)),
                None,
                None,
            )
        }?;
        self.state.children.push(Child { hwnd, page, rect });
        if id != 0 {
            self.state.controls.insert(id, hwnd);
        }
        Ok(())
    }
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
    pub fn show(&mut self, config: &IslandConfig) {
        if !unsafe { IsWindowVisible(self.hwnd) }.as_bool() {
            self.state.baseline = config.clone();
            self.state.fill();
        }
        DIALOG.with(|slot| slot.set(self.hwnd));
        let _ = unsafe { ShowWindow(self.hwnd, SW_SHOW) };
        let _ = unsafe { SetForegroundWindow(self.hwnd) };
        let _ = unsafe { SetFocus(Some(self.state.controls[&PAGE])) };
    }
    pub fn take_request(&self) -> Option<Request> {
        self.receiver.try_recv().ok()
    }
    pub fn acknowledge(
        &mut self,
        current: &IslandConfig,
        result: Result<(), String>,
        commit: bool,
    ) {
        match result {
            Ok(()) => {
                if commit {
                    self.state.baseline = current.clone();
                    self.state.fill();
                }
                self.state.status(if commit {
                    "Applied. Your profile was saved."
                } else {
                    "Preview active. Apply to save, or Revert to restore."
                });
            }
            Err(error) => self.state.status(&error),
        }
    }
    pub fn revert(&mut self, current: &IslandConfig) {
        self.state.baseline = current.clone();
        self.state.fill();
    }
}
impl Drop for PreferencesWindow {
    fn drop(&mut self) {
        DIALOG.with(|slot| {
            if slot.get() == self.hwnd {
                slot.set(HWND::default());
            }
        });
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}
pub(crate) fn translate_message(message: &MSG) -> bool {
    DIALOG.with(|slot| {
        let hwnd = slot.get();
        !hwnd.0.is_null()
            && unsafe { IsWindowVisible(hwnd) }.as_bool()
            && unsafe { IsDialogMessageW(hwnd, message) }.as_bool()
    })
}

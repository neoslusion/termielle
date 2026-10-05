//! Standard Windows tooltips. Owned UTF-16 strings outlive registered tools.
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, RECT, WPARAM},
        UI::{
            Controls::{
                TOOLTIPS_CLASSW, TTDT_INITIAL, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_DELTOOLW, TTM_POP,
                TTM_SETDELAYTIME, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
            },
            WindowsAndMessaging::*,
        },
    },
    core::w,
};
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hint {
    pub id: isize,
    pub rect: (i32, i32, u32, u32),
    pub text: String,
}
struct Tool {
    info: TTTOOLINFOW,
    _text: Vec<u16>,
}
pub(super) struct Tooltips {
    hwnd: HWND,
    owner: HWND,
    hints: Vec<Hint>,
    tools: Vec<Tool>,
}
impl Tooltips {
    pub fn create(owner: HWND) -> windows::core::Result<Self> {
        unsafe { windows::Win32::UI::Controls::InitCommonControls() };
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST,
                TOOLTIPS_CLASSW,
                w!(""),
                WS_POPUP | WINDOW_STYLE(TTS_NOPREFIX | TTS_ALWAYSTIP),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                Some(owner),
                None,
                None,
                None,
            )
        }?;
        unsafe {
            SendMessageW(
                hwnd,
                TTM_SETDELAYTIME,
                Some(WPARAM(TTDT_INITIAL as usize)),
                Some(LPARAM(600)),
            )
        };
        Ok(Self {
            hwnd,
            owner,
            hints: Vec::new(),
            tools: Vec::new(),
        })
    }
    pub fn update(&mut self, hints: Vec<Hint>) {
        if self.hints == hints {
            return;
        }
        self.clear();
        self.hints = hints;
        for hint in &self.hints {
            let mut text: Vec<u16> = hint.text.encode_utf16().chain(Some(0)).collect();
            let (x, y, w, h) = hint.rect;
            let info = TTTOOLINFOW {
                cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                uFlags: TTF_SUBCLASS,
                hwnd: self.owner,
                uId: hint.id as usize,
                rect: RECT {
                    left: x,
                    top: y,
                    right: x.saturating_add(w as i32),
                    bottom: y.saturating_add(h as i32),
                },
                lpszText: windows::core::PWSTR(text.as_mut_ptr()),
                ..Default::default()
            };
            if unsafe {
                SendMessageW(
                    self.hwnd,
                    TTM_ADDTOOLW,
                    None,
                    Some(LPARAM(&info as *const _ as isize)),
                )
            }
            .0 != 0
            {
                self.tools.push(Tool { info, _text: text });
            }
        }
    }
    fn clear(&mut self) {
        unsafe { SendMessageW(self.hwnd, TTM_POP, None, None) };
        for tool in self.tools.drain(..) {
            unsafe {
                SendMessageW(
                    self.hwnd,
                    TTM_DELTOOLW,
                    None,
                    Some(LPARAM(&tool.info as *const _ as isize)),
                )
            };
        }
    }
}
impl Drop for Tooltips {
    fn drop(&mut self) {
        self.clear();
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

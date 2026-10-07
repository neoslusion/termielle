use super::{catalog, model::Model, paint};
use crate::win32_ptr::{
    set_window_long_ptr as SetWindowLongPtrW, window_long_ptr as GetWindowLongPtrW,
};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::mem::size_of;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};
use termielle_core::{GlassConfig, IslandConfig};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DIB_RGB_COLORS, DeleteObject, EndPaint,
    GetMonitorInfoW, HBRUSH, HDC, HFONT, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromWindow, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SetBkColor, SetDIBitsToDevice,
    SetTextColor,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::RO_INIT_SINGLETHREADED;
use windows::Win32::UI::Controls::{EM_LIMITTEXT, EM_SETCUEBANNER, EM_SETSEL};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, MOD_ALT, MOD_NOREPEAT, RegisterHotKey, SetFocus, UnregisterHotKey, VK_CONTROL,
    VK_DOWN, VK_ESCAPE, VK_RETURN, VK_SPACE, VK_UP,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_DBLCLKS, CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, DestroyWindow,
    EN_CHANGE, ES_AUTOHSCROLL, GWLP_USERDATA, GetForegroundWindow, GetParent, GetWindowTextLengthW,
    GetWindowTextW, IDC_ARROW, LoadCursorW, PostMessageW, RegisterClassW, SW_HIDE, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SendMessageW, SetForegroundWindow, SetWindowPos,
    SetWindowTextW, ShowWindow, WA_INACTIVE, WINDOW_STYLE, WM_ACTIVATE, WM_APP, WM_CHAR, WM_CLOSE,
    WM_COMMAND, WM_CTLCOLOREDIT, WM_DPICHANGED, WM_ERASEBKGND, WM_HOTKEY, WM_IME_ENDCOMPOSITION,
    WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_LBUTTONUP, WM_NCCREATE, WM_NCDESTROY, WM_PAINT,
    WM_SETFONT, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    WS_VISIBLE,
};
use windows::core::w;

const UPDATE_MSG: u32 = WM_APP + 101;
const OPEN_MSG: u32 = WM_APP + 102;
const HOTKEY_ID: i32 = 101;
static CLASS: OnceLock<Result<u16, u32>> = OnceLock::new();

enum Update {
    Catalog(windows::core::Result<Vec<super::model::App>>),
    Icons(Vec<(Vec<u8>, Option<Arc<crate::animation::FrameBuffer>>)>),
    Launched(u64, windows::core::Result<()>),
}

enum Command {
    Refresh,
    Icons,
}

#[derive(Clone)]
struct Notifier {
    hwnd: isize,
    sender: mpsc::Sender<Update>,
}

impl Notifier {
    fn send(&self, update: Update) {
        if self.sender.send(update).is_ok() {
            let _ = unsafe {
                PostMessageW(Some(HWND(self.hwnd as _)), UPDATE_MSG, WPARAM(0), LPARAM(0))
            };
        }
    }
}

struct State {
    hwnd: Cell<HWND>,
    edit: Cell<HWND>,
    font: Cell<HFONT>,
    brush: Cell<HBRUSH>,
    scale: Cell<f32>,
    visible: Cell<bool>,
    composing: Cell<bool>,
    hotkey: Cell<bool>,
    generation: Cell<u64>,
    previous_window: Cell<HWND>,
    glass: RefCell<GlassConfig>,
    model: RefCell<Model>,
    receiver: RefCell<mpsc::Receiver<Update>>,
    notifier: Notifier,
    refresh: mpsc::Sender<Command>,
    pending_icons: Arc<Mutex<Vec<Vec<u8>>>>,
    icons_done: RefCell<HashSet<Vec<u8>>>,
    last_refresh: Cell<Instant>,
}

impl State {
    fn request_icons(&self) {
        if !self.visible.get() {
            return;
        }
        let targets = {
            let model = self.model.borrow();
            let mut done = self.icons_done.borrow_mut();
            done.retain(|target| {
                model
                    .results
                    .iter()
                    .any(|index| model.apps[*index].target == *target)
            });
            model
                .results
                .iter()
                .map(|index| &model.apps[*index])
                .filter(|app| !done.contains(&app.target))
                .map(|app| app.target.clone())
                .collect::<Vec<_>>()
        };
        let needed = !targets.is_empty();
        *self.pending_icons.lock().unwrap() = targets;
        if needed {
            let _ = self.refresh.send(Command::Icons);
        }
    }

    fn redraw(&self) {
        let width = (paint::WIDTH as f32 * self.scale.get()).round() as i32;
        let height = (paint::height(&self.model.borrow()) as f32 * self.scale.get()).round() as i32;
        let _ = unsafe {
            SetWindowPos(
                self.hwnd.get(),
                None,
                0,
                0,
                width,
                height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        let _ = unsafe { InvalidateRect(Some(self.hwnd.get()), None, false) };
    }

    fn hide(&self, restore_focus: bool) {
        if !self.visible.replace(false) {
            return;
        }
        self.pending_icons.lock().unwrap().clear();
        let _ = unsafe { ShowWindow(self.hwnd.get(), SW_HIDE) };
        if restore_focus {
            let previous = self.previous_window.get();
            if !previous.is_invalid() {
                let _ = unsafe { SetForegroundWindow(previous) };
            }
        }
    }

    fn show(&self) {
        self.previous_window.set(unsafe { GetForegroundWindow() });
        self.generation.set(self.generation.get().wrapping_add(1));
        {
            let mut model = self.model.borrow_mut();
            model.launching = false;
            model.set_query(String::new());
        }
        let _ = unsafe { SetWindowTextW(self.edit.get(), w!("")) };
        let monitor =
            unsafe { MonitorFromWindow(self.previous_window.get(), MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            let width = (paint::WIDTH as f32 * self.scale.get()).round() as i32;
            let height =
                (paint::height(&self.model.borrow()) as f32 * self.scale.get()).round() as i32;
            let work = info.rcWork;
            let _ = unsafe {
                SetWindowPos(
                    self.hwnd.get(),
                    None,
                    work.left + (work.right - work.left - width) / 2,
                    work.top + (work.bottom - work.top - height).max(0) / 4,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
        }
        self.visible.set(true);
        let _ = unsafe { ShowWindow(self.hwnd.get(), SW_SHOW) };
        let _ = unsafe { SetForegroundWindow(self.hwnd.get()) };
        let _ = unsafe { SetFocus(Some(self.edit.get())) };
        if self.last_refresh.get().elapsed() >= Duration::from_secs(60) {
            self.last_refresh.set(Instant::now());
            let _ = self.refresh.send(Command::Refresh);
        }
        self.request_icons();
        self.redraw();
    }

    fn open_selected(&self) {
        let target = {
            let mut model = self.model.borrow_mut();
            if model.launching {
                return;
            }
            let Some(app) = model.selected_app() else {
                return;
            };
            let target = app.target.clone();
            model.launching = true;
            model.error = None;
            target
        };
        let notifier = self.notifier.clone();
        let generation = self.generation.get();
        std::thread::spawn(move || {
            let result = crate::apartment::Apartment::new(RO_INIT_SINGLETHREADED)
                .and_then(|_apartment| catalog::launch(&target));
            notifier.send(Update::Launched(generation, result));
        });
        self.redraw();
    }

    fn drain_updates(&self) {
        let updates: Vec<_> = self.receiver.borrow().try_iter().collect();
        for update in updates {
            match update {
                Update::Catalog(Ok(apps)) => {
                    self.icons_done.borrow_mut().clear();
                    self.model.borrow_mut().set_apps(apps);
                }
                Update::Icons(icons) => {
                    if !self.visible.get() {
                        continue;
                    }
                    let mut model = self.model.borrow_mut();
                    let mut done = self.icons_done.borrow_mut();
                    for (target, icon) in icons {
                        if let Some(index) = model
                            .results
                            .iter()
                            .copied()
                            .find(|index| model.apps[*index].target == target)
                        {
                            model.apps[index].icon = icon;
                            done.insert(target);
                        }
                    }
                }
                Update::Catalog(Err(error)) => {
                    let mut model = self.model.borrow_mut();
                    model.indexing = false;
                    model.error = Some(format!("Could not index apps ({:08X})", error.code().0));
                }
                Update::Launched(generation, result) if generation == self.generation.get() => {
                    self.model.borrow_mut().launching = false;
                    match result {
                        Ok(()) => self.hide(false),
                        Err(error) => {
                            self.model.borrow_mut().error =
                                Some(format!("Could not open app ({:08X})", error.code().0))
                        }
                    }
                }
                Update::Launched(_, _) => {}
            }
        }
        self.request_icons();
        self.redraw();
    }

    fn restyle(&self) {
        let scale = self.scale.get();
        let font = unsafe {
            CreateFontW(
                -(22.0 * scale).round() as i32,
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
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            )
        };
        let previous = self.font.replace(font);
        unsafe {
            SendMessageW(
                self.edit.get(),
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            )
        };
        if !previous.is_invalid() {
            let _ = unsafe { DeleteObject(previous.into()) };
        }
        let background = colorref(self.glass.borrow().tint);
        let previous = self.brush.replace(unsafe { CreateSolidBrush(background) });
        if !previous.is_invalid() {
            let _ = unsafe { DeleteObject(previous.into()) };
        }
        let _ = unsafe {
            SetWindowPos(
                self.edit.get(),
                None,
                (66.0 * scale).round() as i32,
                (24.0 * scale).round() as i32,
                ((paint::WIDTH - 90) as f32 * scale).round() as i32,
                (36.0 * scale).round() as i32,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        let dark: u32 =
            u32::from(crate::animation::notch::ink_pair(&self.glass.borrow()).0[0] > 128);
        let _ = unsafe {
            DwmSetWindowAttribute(
                self.hwnd.get(),
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&raw const dark).cast(),
                size_of::<u32>() as u32,
            )
        };
    }

    fn paint(&self) {
        let frame = paint::render(
            &self.model.borrow(),
            &self.glass.borrow(),
            self.scale.get(),
            self.hotkey.get(),
        );
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: frame.width as i32,
                biHeight: -(frame.height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut paint = PAINTSTRUCT::default();
        let dc = unsafe { BeginPaint(self.hwnd.get(), &mut paint) };
        if !dc.is_invalid() {
            unsafe {
                SetDIBitsToDevice(
                    dc,
                    0,
                    0,
                    frame.width,
                    frame.height,
                    0,
                    0,
                    0,
                    frame.height,
                    frame.pixels_pbgra.as_ptr().cast(),
                    &mut info,
                    DIB_RGB_COLORS,
                )
            };
        }
        let _ = unsafe { EndPaint(self.hwnd.get(), &paint) };
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if !self.font.get().is_invalid() {
            let _ = unsafe { DeleteObject(self.font.get().into()) };
        }
        if !self.brush.get().is_invalid() {
            let _ = unsafe { DeleteObject(self.brush.get().into()) };
        }
    }
}

pub struct LauncherWindow {
    state: Box<State>,
}

impl LauncherWindow {
    pub fn create(island: &IslandConfig) -> windows::core::Result<Self> {
        Self::create_window(island, true)
    }

    fn create_window(island: &IslandConfig, register_hotkey: bool) -> windows::core::Result<Self> {
        let instance = unsafe { GetModuleHandleW(None)? };
        let class = CLASS.get_or_init(|| {
            let class = WNDCLASSW {
                style: CS_DROPSHADOW | CS_DBLCLKS,
                lpfnWndProc: Some(window_proc),
                hInstance: HINSTANCE(instance.0),
                hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
                lpszClassName: w!("termielle_launcher"),
                ..Default::default()
            };
            let atom = unsafe { RegisterClassW(&class) };
            if atom == 0 {
                Err(unsafe { windows::Win32::Foundation::GetLastError().0 })
            } else {
                Ok(atom)
            }
        });
        if let Err(code) = class {
            return Err(windows::core::Error::from_hresult(
                windows::core::HRESULT::from_win32(*code),
            ));
        }
        let (sender, receiver) = mpsc::channel();
        let (refresh, refresh_receiver) = mpsc::channel();
        let state = Box::new(State {
            hwnd: Cell::new(HWND::default()),
            edit: Cell::new(HWND::default()),
            font: Cell::new(HFONT::default()),
            brush: Cell::new(HBRUSH::default()),
            scale: Cell::new(1.0),
            visible: Cell::new(false),
            composing: Cell::new(false),
            hotkey: Cell::new(false),
            generation: Cell::new(0),
            previous_window: Cell::new(HWND::default()),
            glass: RefCell::new(island.glass.clone()),
            model: RefCell::new(Model::default()),
            receiver: RefCell::new(receiver),
            notifier: Notifier { hwnd: 0, sender },
            refresh,
            pending_icons: Arc::new(Mutex::new(Vec::new())),
            icons_done: RefCell::new(HashSet::new()),
            last_refresh: Cell::new(Instant::now()),
        });
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                w!("termielle_launcher"),
                w!("Termielle Search"),
                WS_POPUP | WS_CLIPCHILDREN,
                0,
                0,
                paint::WIDTH as i32,
                184,
                None,
                None,
                Some(HINSTANCE(instance.0)),
                Some((&*state as *const State).cast()),
            )?
        };
        let mut launcher = Self { state };
        launcher.state.notifier.hwnd = hwnd.0 as isize;
        let edit = unsafe {
            CreateWindowExW(
                Default::default(),
                w!("EDIT"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                66,
                24,
                550,
                36,
                Some(hwnd),
                None,
                Some(HINSTANCE(instance.0)),
                None,
            )?
        };
        launcher.state.edit.set(edit);
        unsafe { SetWindowSubclass(edit, Some(edit_proc), 1, 0) }.ok()?;
        unsafe {
            SendMessageW(edit, EM_LIMITTEXT, Some(WPARAM(256)), Some(LPARAM(0)));
            SendMessageW(
                edit,
                EM_SETCUEBANNER,
                Some(WPARAM(1)),
                Some(LPARAM(w!("Search apps…").as_ptr() as isize)),
            );
        }
        launcher
            .state
            .scale
            .set(unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0);
        launcher.state.restyle();
        let corners = DWMWCP_ROUND;
        let _ = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const corners).cast(),
                size_of_val(&corners) as u32,
            )
        };
        if register_hotkey {
            let registered = unsafe {
                RegisterHotKey(
                    Some(hwnd),
                    HOTKEY_ID,
                    MOD_ALT | MOD_NOREPEAT,
                    VK_SPACE.0 as u32,
                )
            };
            launcher.state.hotkey.set(registered.is_ok());
            if let Err(error) = registered {
                eprintln!("launcher Alt+Space unavailable: {error}");
            }
        }
        let notifier = launcher.state.notifier.clone();
        let pending_icons = launcher.state.pending_icons.clone();
        std::thread::spawn(move || {
            let _apartment = match crate::apartment::Apartment::new(RO_INIT_SINGLETHREADED) {
                Ok(apartment) => apartment,
                Err(error) => {
                    notifier.send(Update::Catalog(Err(error)));
                    return;
                }
            };
            notifier.send(Update::Catalog(catalog::enumerate()));
            let mut cache = super::icon_cache::IconCache::default();
            while let Ok(first) = refresh_receiver.recv() {
                let mut refresh = false;
                let mut icons = false;
                for command in std::iter::once(first).chain(refresh_receiver.try_iter().take(63)) {
                    match command {
                        Command::Refresh => refresh = true,
                        Command::Icons => icons = true,
                    }
                }
                if refresh {
                    cache.clear();
                    notifier.send(Update::Catalog(catalog::enumerate()));
                }
                if icons {
                    let targets = std::mem::take(&mut *pending_icons.lock().unwrap());
                    let icons = targets
                        .into_iter()
                        .map(|target| {
                            let icon = cache.get(&target, || catalog::read_icon(&target));
                            (target, icon)
                        })
                        .collect();
                    notifier.send(Update::Icons(icons));
                }
            }
        });
        Ok(launcher)
    }

    pub fn show(&self) {
        self.state.show();
    }

    pub fn configure(&self, island: &IslandConfig) {
        if *self.state.glass.borrow() != island.glass {
            *self.state.glass.borrow_mut() = island.glass.clone();
            self.state.restyle();
            self.state.redraw();
        }
    }
}

impl Drop for LauncherWindow {
    fn drop(&mut self) {
        if self.state.hotkey.get() {
            let _ = unsafe { UnregisterHotKey(Some(self.state.hwnd.get()), HOTKEY_ID) };
        }
        let _ = unsafe { DestroyWindow(self.state.hwnd.get()) };
    }
}

fn colorref(bgra: [u8; 4]) -> COLORREF {
    COLORREF(u32::from(bgra[2]) | u32::from(bgra[1]) << 8 | u32::from(bgra[0]) << 16)
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        dispatch(hwnd, message, wparam, lparam)
    }))
    .unwrap_or_else(|_| unsafe { DefWindowProcW(hwnd, message, wparam, lparam) })
}

unsafe fn dispatch(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize) };
        let state = unsafe { &*(create.lpCreateParams as *const State) };
        state.hwnd.set(hwnd);
        return LRESULT(1);
    }
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const State;
    if pointer.is_null() {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    let state = unsafe { &*pointer };
    match message {
        WM_HOTKEY if wparam.0 == HOTKEY_ID as usize => {
            if state.visible.get() {
                state.hide(true);
            } else {
                state.show();
            }
        }
        WM_ACTIVATE if wparam.0 as u32 & 0xffff == WA_INACTIVE => state.hide(false),
        WM_CLOSE => state.hide(true),
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => state.paint(),
        WM_COMMAND if (wparam.0 >> 16) as u32 == EN_CHANGE => {
            let mut text =
                vec![0; unsafe { GetWindowTextLengthW(state.edit.get()) }.max(0) as usize + 1];
            let length = unsafe { GetWindowTextW(state.edit.get(), &mut text) }.max(0) as usize;
            state
                .model
                .borrow_mut()
                .set_query(String::from_utf16_lossy(&text[..length]));
            state.request_icons();
            state.redraw();
        }
        WM_CTLCOLOREDIT => {
            let dc = HDC(wparam.0 as _);
            let glass = state.glass.borrow();
            let primary = crate::animation::notch::ink_pair(&glass).0;
            unsafe {
                SetBkColor(dc, colorref(glass.tint));
                SetTextColor(dc, colorref(primary));
            }
            return LRESULT(state.brush.get().0 as isize);
        }
        WM_DPICHANGED => {
            state.scale.set((wparam.0 & 0xffff) as f32 / 96.0);
            state.restyle();
            let rect = unsafe { &*(lparam.0 as *const RECT) };
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
            state.redraw();
        }
        WM_LBUTTONUP => {
            let pointer_y = ((lparam.0 >> 16) as i16) as f32 / state.scale.get();
            let row = ((pointer_y as i32 - paint::ROW_TOP) / paint::ROW_HEIGHT as i32) as usize;
            let selected =
                pointer_y >= paint::ROW_TOP as f32 && row < state.model.borrow().results.len();
            if selected {
                state.model.borrow_mut().selected = row;
                let _ = unsafe { PostMessageW(Some(hwnd), OPEN_MSG, WPARAM(0), LPARAM(0)) };
            }
        }
        UPDATE_MSG => state.drain_updates(),
        OPEN_MSG => state.open_selected(),
        WM_NCDESTROY => {
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
        }
        _ => return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
    LRESULT(0)
}

unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass: usize,
    _data: usize,
) -> LRESULT {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        dispatch_edit(hwnd, message, wparam, lparam)
    }))
    .unwrap_or_else(|_| unsafe { DefSubclassProc(hwnd, message, wparam, lparam) })
}

unsafe fn dispatch_edit(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let parent = unsafe { GetParent(hwnd) }.unwrap_or_default();
    let pointer = unsafe { GetWindowLongPtrW(parent, GWLP_USERDATA) } as *const State;
    if !pointer.is_null() {
        let state = unsafe { &*pointer };
        if message == WM_IME_STARTCOMPOSITION {
            state.composing.set(true);
        } else if message == WM_IME_ENDCOMPOSITION {
            state.composing.set(false);
        }
        if message == WM_KEYDOWN && !state.composing.get() {
            match wparam.0 as u16 {
                key if key == VK_UP.0 || key == VK_DOWN.0 => {
                    state
                        .model
                        .borrow_mut()
                        .move_selection(if key == VK_UP.0 { -1 } else { 1 });
                    state.redraw();
                    return LRESULT(0);
                }
                key if key == VK_RETURN.0 => {
                    let _ = unsafe { PostMessageW(Some(parent), OPEN_MSG, WPARAM(0), LPARAM(0)) };
                    return LRESULT(0);
                }
                key if key == VK_ESCAPE.0 => {
                    state.hide(true);
                    return LRESULT(0);
                }
                65 if unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0 => {
                    unsafe { SendMessageW(hwnd, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1))) };
                    return LRESULT(0);
                }
                _ => {}
            }
        }
        if message == WM_CHAR && !state.composing.get() && matches!(wparam.0, 13 | 27) {
            return LRESULT(0);
        }
    }
    if message == WM_NCDESTROY {
        let _ = unsafe { RemoveWindowSubclass(hwnd, Some(edit_proc), 1) };
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::model::App;

    fn pump_until_overlay_timer(overlay: &mut crate::window::OverlayWindow) {
        unsafe {
            PostMessageW(
                Some(overlay.hwnd()),
                windows::Win32::UI::WindowsAndMessaging::WM_TIMER,
                WPARAM(1),
                LPARAM(0),
            )
        }
        .unwrap();
        for _ in 0..100 {
            if overlay.next_event().unwrap() == Some(crate::window::WindowEvent::Timer) {
                return;
            }
        }
        panic!("overlay timer was not delivered");
    }

    #[test]
    fn production_message_pump_dispatches_launcher_hotkey_and_edit_input() {
        let mut overlay =
            crate::window::OverlayWindow::create(&termielle_core::AppConfig::default(), true)
                .unwrap();
        let launcher = LauncherWindow::create_window(&IslandConfig::default(), false).unwrap();
        unsafe {
            PostMessageW(
                Some(launcher.state.hwnd.get()),
                WM_HOTKEY,
                WPARAM(HOTKEY_ID as usize),
                LPARAM(0),
            )
        }
        .unwrap();
        pump_until_overlay_timer(&mut overlay);
        assert!(
            launcher.state.visible.get(),
            "the shared GUI pump must deliver the launcher's queued hotkey"
        );
        for letter in ['b', 'e'] {
            unsafe {
                PostMessageW(
                    Some(launcher.state.edit.get()),
                    WM_CHAR,
                    WPARAM(letter as usize),
                    LPARAM(0),
                )
            }
            .unwrap();
        }
        pump_until_overlay_timer(&mut overlay);
        assert_eq!(
            launcher.state.model.borrow().query,
            "be",
            "queued input must reach the native edit control"
        );
        unsafe {
            PostMessageW(
                Some(launcher.state.edit.get()),
                WM_KEYDOWN,
                WPARAM(VK_ESCAPE.0 as usize),
                LPARAM(0),
            )
        }
        .unwrap();
        pump_until_overlay_timer(&mut overlay);
        assert!(!launcher.state.visible.get());
    }

    #[test]
    fn native_edit_updates_query_and_routes_navigation_without_search_handoff() {
        let launcher = LauncherWindow::create_window(&IslandConfig::default(), false).unwrap();
        let state = &launcher.state;
        state.model.borrow_mut().set_apps(
            ["Alpha", "Beta"]
                .into_iter()
                .map(|name| App {
                    name: name.into(),
                    key: name.to_lowercase(),
                    target: vec![0, 0],
                    icon: None,
                })
                .collect(),
        );
        unsafe { SetWindowTextW(state.edit.get(), w!("be")) }.unwrap();
        assert_eq!(state.model.borrow().query, "be");
        assert_eq!(state.model.borrow().selected_app().unwrap().name, "Beta");
        unsafe { SetWindowTextW(state.edit.get(), w!("")) }.unwrap();
        unsafe {
            SendMessageW(
                state.edit.get(),
                WM_KEYDOWN,
                Some(WPARAM(VK_DOWN.0 as usize)),
                Some(LPARAM(0)),
            )
        };
        assert_eq!(state.model.borrow().selected_app().unwrap().name, "Beta");
        unsafe {
            SendMessageW(
                state.edit.get(),
                WM_KEYDOWN,
                Some(WPARAM(VK_UP.0 as usize)),
                Some(LPARAM(0)),
            )
        };
        assert_eq!(state.model.borrow().selected_app().unwrap().name, "Alpha");
        unsafe { SetWindowTextW(state.edit.get(), w!("not found")) }.unwrap();
        state.open_selected();
        assert!(!state.model.borrow().launching);
        assert!(!state.hotkey.get());
        assert!(!state.visible.get());
        state.visible.set(true);
        unsafe {
            SendMessageW(
                state.hwnd.get(),
                WM_ACTIVATE,
                Some(WPARAM(WA_INACTIVE as usize)),
                Some(LPARAM(0)),
            )
        };
        assert!(!state.visible.get());
    }

    #[test]
    fn stale_launch_completion_cannot_dismiss_a_reopened_launcher() {
        let launcher = LauncherWindow::create_window(&IslandConfig::default(), false).unwrap();
        launcher.state.generation.set(2);
        launcher.state.visible.set(true);
        launcher.state.model.borrow_mut().launching = true;
        launcher
            .state
            .notifier
            .sender
            .send(Update::Launched(1, Ok(())))
            .unwrap();
        launcher.state.drain_updates();
        assert!(launcher.state.visible.get());
        assert!(launcher.state.model.borrow().launching);
        launcher.state.visible.set(false);
    }
}

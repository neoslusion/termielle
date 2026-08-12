use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use termielle_core::{AppConfig, RenderMode};
use windows::Win32::Foundation::{
    COLORREF, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
    GetMonitorInfoW, HGDIOBJ, MONITOR_DEFAULTTONEAREST, MONITORINFOEXW, MonitorFromRect,
    MonitorFromWindow, RGBQUAD, ReleaseDC, SetDIBitsToDevice,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, GetWindowRect, HTCAPTION, HTTRANSPARENT,
    IDC_ARROW, KillTimer, LWA_COLORKEY, LoadCursorW, MSG, PostMessageW, PostQuitMessage,
    RegisterClassW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOSENDCHANGING,
    SWP_NOZORDER, SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, TranslateMessage, ULW_ALPHA, UpdateLayeredWindow, WM_APP, WM_CLOSE, WM_DESTROY,
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_EXITSIZEMOVE, WM_NCHITTEST, WM_SETCURSOR, WM_TIMER,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::PCWSTR;

use crate::animation::FrameBuffer;
use crate::tray;

/// Minimum per-pixel alpha (0-255) for the overlay to claim a hit; any pixel
/// below the threshold passes clicks through to the terminal underneath.
pub const ALPHA_HIT_THRESHOLD: u8 = 16;

/// The color the [`RenderMode::ColorKey`] renderer erases: magenta is not
/// part of the Termielle palette. Stored as BGR, matching `COLORREF`.
pub const COLOR_KEY_BGRA: [u8; 3] = [255, 0, 255];

/// The RGB value of [`COLOR_KEY_BGRA`] as a `COLORREF`.
pub const COLOR_KEY_REF: u32 = 0x00FF00FF;

/// Alpha threshold (0-255) for the binary color-key decision: pixels at or
/// above it are painted straight, everything below becomes the key color.
pub const COLOR_KEY_ALPHA_THRESHOLD: u8 = 128;

/// The row stride of a 24-bit top-down DIB: every row is padded to a 4-byte
/// boundary. `SetDIBitsToDevice` reads rows at this stride, so the paint
/// buffer must match it exactly.
pub fn dib_stride(width: u32) -> usize {
    (width * 3).div_ceil(4) as usize * 4
}

/// The color-key renderer's per-pixel decision: pixels at or above
/// [`COLOR_KEY_ALPHA_THRESHOLD`] become straight RGB (un-premultiplied from
/// BGRA), everything below becomes the key color for the compositor to erase.
pub fn straight_or_key(alpha: u8, b: u8, g: u8, r: u8) -> [u8; 3] {
    if alpha < COLOR_KEY_ALPHA_THRESHOLD {
        return COLOR_KEY_BGRA;
    }
    if alpha == u8::MAX {
        return [b, g, r];
    }
    let scale = u32::from(alpha);
    [
        ((u32::from(b) * u32::from(u8::MAX) + scale / 2) / scale) as u8,
        ((u32::from(g) * u32::from(u8::MAX) + scale / 2) / scale) as u8,
        ((u32::from(r) * u32::from(u8::MAX) + scale / 2) / scale) as u8,
    ]
}

/// Message id used by [`WakeHandle::post`] to wake the message loop.
const WAKE_MSG: u32 = WM_APP + 1;

/// Timer id used by [`OverlayWindow::set_timer`].
const TIMER_ID: usize = 1;

/// Name of the registered overlay window class.
const CLASS_NAME: &str = "termielle_overlay";

/// The window class is registered once per process; later `create` calls
/// reuse it.
static CLASS_ATOM: std::sync::OnceLock<Result<u16, u32>> = std::sync::OnceLock::new();

/// Axis-aligned rectangle in screen (logical pixel) coordinates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

/// Where a click at a given local position should be routed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HitTestResult {
    /// The pixel is transparent; the click goes to the terminal window.
    Transparent,
    /// The pixel is visible; the click drags the overlay.
    Caption,
}

/// Window-local events the overlay loop has to react to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowEvent {
    /// The repaint/advance deadline set by [`OverlayWindow::set_timer`] was reached.
    Timer,
    /// Display topology or DPI changed; the caller should re-clamp and present.
    DisplayChanged,
    /// The tray menu asked for a graceful exit.
    Quit,
    /// The tray menu asked for a graceful exit followed by a relaunch.
    Restart,
}

/// Why a window operation could not be completed.
#[derive(Debug, thiserror::Error)]
pub enum WindowError {
    #[error("Windows API call failed with error {0}")]
    Win32(u32),
    #[error("frame buffer does not match its declared dimensions")]
    FrameBuffer,
    #[error("windows crate error: {0}")]
    Crate(#[from] windows::core::Error),
}

/// Logical overlay size scaled by `scale`, rounding half up and never
/// collapsing to zero.
pub fn scaled_size(logical: (u32, u32), scale: f32) -> (u32, u32) {
    let width = (logical.0 as f32 * scale).round() as i64;
    let height = (logical.1 as f32 * scale).round() as i64;
    (
        width.clamp(1, i64::from(u32::MAX)) as u32,
        height.clamp(1, i64::from(u32::MAX)) as u32,
    )
}

/// Moves `position` into `work_area` so the `size`-sized rect stays fully
/// visible.
pub fn clamp_to_work_area(position: (i32, i32), size: (i32, i32), work_area: Rect) -> (i32, i32) {
    let max_x = work_area.right.saturating_sub(size.0);
    let max_y = work_area.bottom.saturating_sub(size.1);
    (
        position.0.clamp(work_area.left, max_x),
        position.1.clamp(work_area.top, max_y),
    )
}

/// Whether the `size`-sized rect at `position` lies fully inside `work_area`.
pub fn position_within_work_area(position: (i32, i32), size: (i32, i32), work_area: Rect) -> bool {
    position.0 >= work_area.left
        && position.1 >= work_area.top
        && position.0.saturating_add(size.0) <= work_area.right
        && position.1.saturating_add(size.1) <= work_area.bottom
}

/// Classifies a click at local overlay coordinates against the scaled alpha
/// map (one alpha byte per pixel, row-major).
pub fn alpha_hit_test(
    alpha_map: &[u8],
    map_width: u32,
    local_x: i32,
    local_y: i32,
) -> HitTestResult {
    if local_x < 0 || local_y < 0 || local_x as u32 >= map_width {
        return HitTestResult::Transparent;
    }
    let index = local_y as usize * map_width as usize + local_x as usize;
    match alpha_map.get(index) {
        Some(&alpha) if alpha >= ALPHA_HIT_THRESHOLD => HitTestResult::Caption,
        _ => HitTestResult::Transparent,
    }
}

/// Scaled alpha map (one byte per pixel) used by [`alpha_hit_test`].
#[derive(Default)]
struct AlphaMap {
    bytes: Vec<u8>,
    width: u32,
}

/// Per-window state reachable from the window procedure through `GWLP_USERDATA`.
struct WindowState {
    events: Sender<WindowEvent>,
    alpha: RefCell<AlphaMap>,
}

/// Converts an `LPARAM` mouse message payload into client coordinates.
fn lparam_point(lparam: LPARAM) -> (i32, i32) {
    let value = lparam.0;
    (value as i16 as i32, (value >> 16) as i16 as i32)
}

pub(crate) fn encode_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_to_string(bytes: &[u16]) -> String {
    let length = bytes.iter().take_while(|&&unit| unit != 0).count();
    String::from_utf16_lossy(&bytes[..length])
}

/// Clamps `rect` to the work area of the monitor it lies on.
unsafe fn reposition_to_work_area(hwnd: HWND, rect: &RECT) {
    let monitor = unsafe { MonitorFromRect(rect, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }.as_bool() {
        let work = Rect {
            left: info.monitorInfo.rcWork.left,
            top: info.monitorInfo.rcWork.top,
            right: info.monitorInfo.rcWork.right,
            bottom: info.monitorInfo.rcWork.bottom,
        };
        let (x, y) = clamp_to_work_area(
            (rect.left, rect.top),
            (rect.right - rect.left, rect.bottom - rect.top),
            work,
        );
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                None,
                x,
                y,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
        };
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const WindowState;
    if state.is_null() {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    match message {
        WM_NCHITTEST => {
            let (x, y) = lparam_point(lparam);
            let mut rect = RECT::default();
            let result = if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
                let alpha = unsafe { (*state).alpha.borrow() };
                alpha_hit_test(&alpha.bytes, alpha.width, x - rect.left, y - rect.top)
            } else {
                HitTestResult::Transparent
            };
            let value = match result {
                HitTestResult::Transparent => HTTRANSPARENT as isize,
                HitTestResult::Caption => HTCAPTION as isize,
            };
            return LRESULT(value);
        }
        WM_TIMER => {
            if wparam.0 == TIMER_ID {
                let _ = unsafe { (*state).events.send(WindowEvent::Timer) };
            }
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let suggested = unsafe { &*(lparam.0 as *const RECT) };
            unsafe { reposition_to_work_area(hwnd, suggested) };
            let _ = unsafe { (*state).events.send(WindowEvent::DisplayChanged) };
            return LRESULT(0);
        }
        WM_DISPLAYCHANGE => {
            let mut rect = RECT::default();
            if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
                unsafe { reposition_to_work_area(hwnd, &rect) };
            }
            let _ = unsafe { (*state).events.send(WindowEvent::DisplayChanged) };
            return LRESULT(0);
        }
        WM_EXITSIZEMOVE => {
            let _ = unsafe { (*state).events.send(WindowEvent::DisplayChanged) };
            return LRESULT(0);
        }
        WM_SETCURSOR => return LRESULT(1),
        WM_CLOSE => return LRESULT(0),
        WM_DESTROY => return LRESULT(0),
        tray::TRAY_MSG => {
            if wparam.0 as u32 == tray::TRAY_ID {
                let mouse = tray::callback_mouse_message(lparam.0);
                if tray::is_menu_message(mouse) {
                    match tray::show_menu(hwnd) {
                        tray::TRAY_EXIT => {
                            let _ = unsafe { (*state).events.send(WindowEvent::Quit) };
                        }
                        tray::TRAY_RESTART => {
                            let _ = unsafe { (*state).events.send(WindowEvent::Restart) };
                        }
                        _ => {}
                    }
                }
            }
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// A thread-owned layered, click-through-capable overlay window.
///
/// Owns the window and its message loop; the window is created hidden and
/// only becomes visible when the first frame is presented.
pub struct OverlayWindow {
    hwnd: HWND,
    state: Box<WindowState>,
    receiver: Receiver<WindowEvent>,
    repositioned: bool,
    render: RenderMode,
    tray: bool,
}

impl OverlayWindow {
    /// Creates the overlay window, sized 1x1 and hidden unless `hidden` is
    /// false. The first `present` picks the real size and placement.
    pub fn create(config: &AppConfig, hidden: bool) -> Result<Self, WindowError> {
        let _ =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let module = unsafe { GetModuleHandleW(None) }?;
        let hinstance = HINSTANCE(module.0);

        let class_name = encode_wide(CLASS_NAME);
        let atom = *CLASS_ATOM.get_or_init(|| {
            let class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(window_proc),
                hInstance: hinstance,
                hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
                hbrBackground: Default::default(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            let atom = unsafe { RegisterClassW(&class) };
            if atom == 0 {
                Err(unsafe { GetLastError().0 })
            } else {
                Ok(atom)
            }
        });
        if let Err(code) = atom {
            return Err(WindowError::Win32(code));
        }

        let ex_style = if config.always_on_top {
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TOOLWINDOW
        } else {
            WS_EX_LAYERED | WS_EX_TOOLWINDOW
        };
        let (x, y) = match &config.position {
            Some(position) => (position.x_logical, position.y_logical),
            None => (0, 0),
        };
        let hwnd = unsafe {
            CreateWindowExW(
                ex_style,
                PCWSTR(class_name.as_ptr()),
                PCWSTR::null(),
                WS_POPUP,
                x,
                y,
                1,
                1,
                None,
                None,
                Some(hinstance),
                None,
            )
        }?;

        let (sender, receiver) = channel();
        let state = Box::new(WindowState {
            events: sender,
            alpha: RefCell::new(AlphaMap::default()),
        });
        let state_ptr = &*state as *const WindowState as isize;
        let _ = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr) };

        let window = Self {
            hwnd,
            state,
            receiver,
            repositioned: false,
            render: config.render,
            tray: false,
        };
        if matches!(config.render, RenderMode::ColorKey) {
            // The color-key path paints straight RGB into the window surface
            // and asks the compositor to erase the key color; see
            // `draw_color_key`. This is the legacy layered-window technique
            // that keeps working on drivers whose ULW redirection is broken.
            unsafe {
                SetLayeredWindowAttributes(hwnd, COLORREF(COLOR_KEY_REF), 255, LWA_COLORKEY)
            }?;
        }
        if !hidden {
            let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        }
        Ok(window)
    }

    /// Presents `frame` scaled by `scale`, resizing and re-clamping the window
    /// on size or first-present, then blitting the frame onto the layered
    /// surface with per-pixel alpha (or the configured fallback renderer).
    pub fn present(&mut self, frame: &FrameBuffer, scale: f32) -> Result<(), WindowError> {
        let (scaled_w, scaled_h) = scaled_size((frame.width, frame.height), scale);
        let mut rect = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut rect) }?;
        let current = Rect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        };
        if current.width() != scaled_w as i32
            || current.height() != scaled_h as i32
            || !self.repositioned
        {
            let (x, y) = self.clamped_position(
                scaled_w as i32,
                scaled_h as i32,
                (current.left, current.top),
            );
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    None,
                    x,
                    y,
                    scaled_w as i32,
                    scaled_h as i32,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOSENDCHANGING,
                )
            }?;
            self.repositioned = true;
        }
        match self.render {
            RenderMode::PerPixel => self.draw(frame, scaled_w, scaled_h),
            RenderMode::ColorKey => self.draw_color_key(frame, scaled_w, scaled_h),
        }
    }

    /// Blocks until a window message arrives, dispatches it, and returns the
    /// event it produced, if any.
    pub fn next_event(&mut self) -> Result<Option<WindowEvent>, WindowError> {
        let mut message = MSG::default();
        let result = unsafe { GetMessageW(&mut message, Some(self.hwnd), 0, 0) };
        if result.0 == 0 {
            return Ok(None);
        }
        if result.0 == -1 {
            return Err(WindowError::Win32(unsafe { GetLastError().0 }));
        }
        let _ = unsafe { TranslateMessage(&message) };
        let _ = unsafe { DispatchMessageW(&message) };
        match self.receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => Ok(None),
        }
    }

    /// Current window rectangle and the device name of its nearest monitor.
    pub fn position(&self) -> (Rect, String) {
        let mut rect = RECT::default();
        let rect = if unsafe { GetWindowRect(self.hwnd, &mut rect) }.is_ok() {
            Rect {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            }
        } else {
            Rect::default()
        };
        let monitor = unsafe { MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        let name = if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }.as_bool() {
            wide_to_string(&info.szDevice)
        } else {
            String::new()
        };
        (rect, name)
    }

    /// Arms (or disarms) the repaint timer; `Some(delay)` arms it for `delay`
    /// milliseconds, `None` disarms it.
    pub fn set_timer(&mut self, delay_ms: Option<u32>) -> Result<(), WindowError> {
        match delay_ms {
            Some(delay) => {
                let timer = unsafe { SetTimer(Some(self.hwnd), TIMER_ID, delay, None) };
                if timer == 0 {
                    return Err(WindowError::Win32(unsafe { GetLastError().0 }));
                }
            }
            None => unsafe { KillTimer(Some(self.hwnd), TIMER_ID) }?,
        }
        Ok(())
    }

    /// Handle that wakes the message loop from another thread.
    pub fn wake_handle(&self) -> WakeHandle {
        WakeHandle { hwnd: self.hwnd }
    }

    /// Adds the notification-area icon for the production overlay. The icon
    /// is the character's idle face when `character_path` resolves, else the
    /// plain application icon. Returns the Win32 error code when the shell
    /// rejects the icon.
    pub fn install_tray(&mut self, character_path: Option<&std::path::Path>) -> Result<(), u32> {
        match tray::add(self.hwnd, character_path) {
            Ok(()) => {
                self.tray = true;
                Ok(())
            }
            Err(code) => Err(code),
        }
    }

    /// Refreshes the tray tooltip to `tip`; a no-op when no icon is installed.
    pub fn update_tray(&self, tip: &str) -> bool {
        !self.tray || tray::set_tooltip(self.hwnd, tip)
    }

    /// Destroys the window and posts `WM_QUIT` to the owning thread's queue.
    /// Must be called from the thread that created the window.
    pub fn destroy(&self) {
        if self.tray {
            let _ = tray::remove(self.hwnd);
        }
        let _ = unsafe { DestroyWindow(self.hwnd) };
        unsafe { PostQuitMessage(0) };
    }

    fn clamped_position(&self, width: i32, height: i32, preferred: (i32, i32)) -> (i32, i32) {
        let monitor = unsafe { MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }.as_bool() {
            let work = Rect {
                left: info.monitorInfo.rcWork.left,
                top: info.monitorInfo.rcWork.top,
                right: info.monitorInfo.rcWork.right,
                bottom: info.monitorInfo.rcWork.bottom,
            };
            return clamp_to_work_area(preferred, (width, height), work);
        }
        preferred
    }

    fn draw(&self, frame: &FrameBuffer, scaled_w: u32, scaled_h: u32) -> Result<(), WindowError> {
        let expected = frame.width as usize * frame.height as usize * 4;
        if frame.pixels_pbgra.len() != expected {
            return Err(WindowError::FrameBuffer);
        }

        let screen = unsafe { GetDC(None) };
        if screen.is_invalid() {
            return Err(WindowError::Win32(unsafe { GetLastError().0 }));
        }
        let memory = unsafe { CreateCompatibleDC(Some(screen)) };
        if memory.is_invalid() {
            let _ = unsafe { ReleaseDC(None, screen) };
            return Err(WindowError::Win32(unsafe { GetLastError().0 }));
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: scaled_w as i32,
                biHeight: -(scaled_h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                ..Default::default()
            },
            bmiColors: [RGBQUAD::default()],
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let dib =
            unsafe { CreateDIBSection(Some(memory), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) };
        let Ok(dib) = dib else {
            let error = unsafe { GetLastError().0 };
            let _ = unsafe { DeleteDC(memory) };
            let _ = unsafe { ReleaseDC(None, screen) };
            return Err(WindowError::Win32(error));
        };

        let mut alpha_map = Vec::with_capacity(scaled_w as usize * scaled_h as usize);
        let len = scaled_w as usize * scaled_h as usize * 4;
        let dst = unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, len) };
        for y in 0..scaled_h {
            for x in 0..scaled_w {
                let source_x = (x * frame.width / scaled_w) as usize;
                let source_y = (y * frame.height / scaled_h) as usize;
                let source =
                    &frame.pixels_pbgra[(source_y * frame.width as usize + source_x) * 4..][..4];
                let target = (y as usize * scaled_w as usize + x as usize) * 4;
                dst[target..target + 4].copy_from_slice(source);
                alpha_map.push(source[3]);
            }
        }

        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let origin = POINT { x: 0, y: 0 };
        // The window is already sized and positioned by `present` via
        // SetWindowPos; leaving pptDst and psize NULL updates only the surface
        // contents, which is the one variant that also works on systems where
        // DWM rejects a full redirection update.
        let update = unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                None,
                None,
                None,
                Some(memory),
                Some(&origin),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        };
        let _ = unsafe { DeleteObject(HGDIOBJ(dib.0)) };
        let _ = unsafe { DeleteDC(memory) };
        let _ = unsafe { ReleaseDC(None, screen) };
        update?;

        *self.state.alpha.borrow_mut() = AlphaMap {
            bytes: alpha_map,
            width: scaled_w,
        };
        Ok(())
    }

    /// Paints `frame` into the window's own DC as straight RGB, replacing
    /// every pixel below [`COLOR_KEY_ALPHA_THRESHOLD`] with the key color
    /// that `SetLayeredWindowAttributes(LWA_COLORKEY)` removes from the
    /// composite. The whole window is otherwise opaque; the per-pixel alpha
    /// map is still produced for click-through hit testing.
    fn draw_color_key(
        &self,
        frame: &FrameBuffer,
        scaled_w: u32,
        scaled_h: u32,
    ) -> Result<(), WindowError> {
        let expected = frame.width as usize * frame.height as usize * 4;
        if frame.pixels_pbgra.len() != expected {
            return Err(WindowError::FrameBuffer);
        }

        // 24-bit DIB rows are padded to a 4-byte boundary. The width times
        // three is not always divisible by four (329 px -> 987 bytes -> 988
        // padded), and `SetDIBitsToDevice` reads rows at the padded stride:
        // a buffer without the padding shifts every row by one byte and
        // renders diagonal garbage.
        let stride = dib_stride(scaled_w);
        let mut rgb = vec![0u8; stride * scaled_h as usize];
        let mut alpha_map = Vec::with_capacity(scaled_w as usize * scaled_h as usize);
        for y in 0..scaled_h {
            for x in 0..scaled_w {
                let source_x = (x * frame.width / scaled_w) as usize;
                let source_y = (y * frame.height / scaled_h) as usize;
                let source =
                    &frame.pixels_pbgra[(source_y * frame.width as usize + source_x) * 4..][..4];
                let target = y as usize * stride + x as usize * 3;
                rgb[target..target + 3]
                    .copy_from_slice(&straight_or_key(source[3], source[0], source[1], source[2]));
                alpha_map.push(source[3]);
            }
        }

        let dc = unsafe { GetDC(Some(self.hwnd)) };
        if dc.is_invalid() {
            return Err(WindowError::Win32(unsafe { GetLastError().0 }));
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: scaled_w as i32,
                biHeight: -(scaled_h as i32),
                biPlanes: 1,
                biBitCount: 24,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                ..Default::default()
            },
            bmiColors: [RGBQUAD::default()],
        };
        // SAFETY: `rgb` holds `scaled_h` top-down rows of `scaled_w` 24-bit
        // pixels and outlives the call; `dc` is the window's own DC.
        let lines = unsafe {
            SetDIBitsToDevice(
                dc,
                0,
                0,
                scaled_w,
                scaled_h,
                0,
                0,
                0,
                scaled_h,
                rgb.as_ptr() as *const c_void,
                &bmi,
                DIB_RGB_COLORS,
            )
        };
        let _ = unsafe { ReleaseDC(Some(self.hwnd), dc) };
        if lines == 0 {
            return Err(WindowError::Win32(unsafe { GetLastError().0 }));
        }

        *self.state.alpha.borrow_mut() = AlphaMap {
            bytes: alpha_map,
            width: scaled_w,
        };
        Ok(())
    }
}

impl Drop for OverlayWindow {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// Handle that can wake the overlay's message loop from another thread.
#[derive(Clone, Copy)]
pub struct WakeHandle {
    hwnd: HWND,
}

/// `HWND` is a pointer, so the derived bound would not hold, but a window
/// handle is a plain token: `PostMessageW` is documented thread-safe, and the
/// handle never dereferences anything. The pipe thread needs this to wake the
/// GUI thread out of `GetMessageW`.
unsafe impl Send for WakeHandle {}

/// A precise animation clock: a background thread sleeps until the next
/// deadline and wakes the GUI thread, which decodes and presents the frame.
/// `WM_TIMER` cannot pace 16 ms frames — its messages are delivered only when
/// the queue is idle, adding unpredictable latency — so the deadlines live on
/// this thread instead.
pub struct AnimationClock {
    deadline: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Kept for the wake handle's lifetime; the spawned thread holds its own
    /// copy, so this field keeps the handle alive for as long as the clock.
    _wake: WakeHandle,
}

/// Sentinel: no deadline is armed.
const NO_DEADLINE: u64 = u64::MAX;

impl AnimationClock {
    /// Starts the clock thread. The overlay arms deadlines with
    /// [`AnimationClock::arm`] after every frame.
    pub fn spawn(wake: WakeHandle) -> Self {
        let deadline = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(NO_DEADLINE));
        let clock = Self {
            deadline: deadline.clone(),
            _wake: wake,
        };
        std::thread::spawn(move || {
            loop {
                let at = deadline.load(std::sync::atomic::Ordering::Relaxed);
                if at == NO_DEADLINE {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    continue;
                }
                let now = now_ms();
                if at > now {
                    std::thread::sleep(std::time::Duration::from_millis(at - now));
                }
                let _ = wake.post();
                // Give the GUI thread a moment to re-arm before checking again,
                // so a due-but-not-yet-rearmed deadline does not spin.
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        });
        clock
    }

    /// Arms the next deadline; `None` disarms the clock.
    pub fn arm(&self, deadline: Option<u64>) {
        self.deadline.store(
            deadline.unwrap_or(NO_DEADLINE),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

/// Milliseconds since the Unix epoch.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |elapsed| elapsed.as_millis() as u64)
}

impl WakeHandle {
    /// Posts an empty message to the overlay's message queue.
    pub fn post(&self) -> Result<(), WindowError> {
        unsafe { PostMessageW(Some(self.hwnd), WAKE_MSG, WPARAM(0), LPARAM(0)) }?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_strings_round_trip() {
        let wide = encode_wide("\\\\.\\DISPLAY1");
        assert_eq!(wide_to_string(&wide), "\\\\.\\DISPLAY1");
        assert_eq!(wide.last(), Some(&0));
    }

    #[test]
    fn straight_or_key_is_key_below_threshold() {
        assert_eq!(straight_or_key(0, 9, 9, 9), COLOR_KEY_BGRA);
        assert_eq!(straight_or_key(127, 9, 9, 9), COLOR_KEY_BGRA);
    }

    #[test]
    fn straight_or_key_passes_opaque_pixels_through() {
        assert_eq!(straight_or_key(255, 1, 2, 3), [1, 2, 3]);
        assert_eq!(straight_or_key(255, 0, 255, 0), [0, 255, 0]);
    }

    #[test]
    fn straight_or_key_un_premultiplies_partial_alpha() {
        // 64 is premultiplied by 128/255, so straight is 128.
        assert_eq!(straight_or_key(128, 64, 32, 16), [128, 64, 32]);
        // A semi-transparent magenta pixel must convert back to the key
        // color itself so it does not leak a tinted fringe.
        assert_eq!(straight_or_key(200, 200, 0, 200), COLOR_KEY_BGRA);
    }
}

#[test]
fn dib_stride_pads_rows_to_four_bytes() {
    // 360 px * 3 = 1080 bytes: already aligned, the case that masked the
    // bug. 329 px * 3 = 987 bytes: padded to 988, the refined assets.
    assert_eq!(dib_stride(360), 1080);
    assert_eq!(dib_stride(329), 988);
    assert_eq!(dib_stride(1), 4);
    assert_eq!(dib_stride(2), 8);
    assert_eq!(dib_stride(4), 12);
}

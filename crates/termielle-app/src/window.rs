use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use termielle_core::{AppConfig, GlassConfig, IslandLayout, RenderMode, island_anchored_position};
use windows::Win32::Foundation::{
    COLORREF, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
    GetMonitorInfoW, HGDIOBJ, MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW,
    MonitorFromRect, MonitorFromWindow, RGBQUAD, ReleaseDC, SelectObject, SetDIBitsToDevice,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GWLP_USERDATA, GetCursorPos, GetMessageW, GetWindowLongPtrW, GetWindowRect, HTCAPTION,
    HTCLIENT, HTTRANSPARENT, IDC_ARROW, IDC_HAND, KillTimer, LWA_COLORKEY, LoadCursorW, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SWP_NOOWNERZORDER, SWP_NOSENDCHANGING, SWP_NOZORDER, SetCursor, SetLayeredWindowAttributes,
    SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, ULW_ALPHA,
    UpdateLayeredWindow, WM_APP, WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED,
    WM_DWMCOLORIZATIONCOLORCHANGED, WM_EXITSIZEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_NCHITTEST, WM_SETCURSOR, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
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

/// `WM_MOUSELEAVE` (0x02A3), posted after `TrackMouseEvent(TME_LEAVE)`.
/// Not exported by windows 0.62's `WindowsAndMessaging`, so it lives here.
const WM_MOUSELEAVE: u32 = 0x02A3;

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
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WindowEvent {
    /// The repaint/advance deadline set by [`OverlayWindow::set_timer`] was reached.
    Timer,
    /// Display topology or DPI changed; the caller should re-clamp and present.
    DisplayChanged,
    /// The tray menu asked for a graceful exit.
    Quit,
    /// The tray menu asked for a graceful exit followed by a relaunch.
    Restart,
    /// Tray requested a layout switch.
    LayoutChanged(termielle_core::IslandLayout),
    /// Tray requested a theme switch.
    ThemeChanged(String),
    /// Tray requested a Y-offset change.
    YOffsetChanged(i32),
    /// User clicked the island at frame-local (x, y).
    ClickAt(i32, i32),
    /// The pointer was pressed down on (`true`) or released from the island,
    /// for the Dynamic Island press swell.
    PressChanged(bool),
    /// Cursor entered (`true`) or left (`false`) the island pill.
    HoverChanged(bool),
    /// Tray toggled task thumbnails on/off.
    ToggleTasks,
    /// Tray toggled hover-to-expand on/off.
    ToggleHoverExpand,
    /// Tray toggled the termielle face widget on/off.
    ToggleFace,
    /// Windows light/dark theme setting changed; re-resolve `auto`.
    SystemThemeChanged,
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

/// Whether the cursor is currently over an opaque pill pixel. Used by the
/// wndproc to distrust spurious `WM_MOUSELEAVE`s (ULW resizes synthesize
/// them) and by the hover poll as a backstop.
fn cursor_over_pill_raw(hwnd: HWND, alpha: &AlphaMap) -> bool {
    unsafe {
        let mut point = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point).is_err() {
            return false;
        }
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return false;
        }
        if point.x < rect.left
            || point.x >= rect.right
            || point.y < rect.top
            || point.y >= rect.bottom
        {
            return false;
        }
        alpha_hit_test(
            &alpha.bytes,
            alpha.width,
            point.x - rect.left,
            point.y - rect.top,
        ) == HitTestResult::Caption
    }
}

/// Per-window state reachable from the window procedure through `GWLP_USERDATA`.
struct WindowState {
    events: Sender<WindowEvent>,
    alpha: RefCell<AlphaMap>,
    is_island: bool,
    /// Whether the cursor is currently inside the opaque pill. Used to emit
    /// each hover transition exactly once.
    hover_inside: RefCell<bool>,
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
///
/// The overlay owns its size (the present loop derives it from the frame and
/// the render scale), so only the position comes from the suggested rect: an
/// OS DPI suggestion would otherwise resize us behind the present loop's
/// back for one frame — and race its size assertions in tests.
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
        let mut current = RECT::default();
        let (w, h) = if unsafe { GetWindowRect(hwnd, &mut current) }.is_ok() {
            (current.right - current.left, current.bottom - current.top)
        } else {
            (rect.right - rect.left, rect.bottom - rect.top)
        };
        let (x, y) = clamp_to_work_area((rect.left, rect.top), (w, h), work);
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                None,
                x,
                y,
                w,
                h,
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
            let is_island = unsafe { (*state).is_island };
            let value = match result {
                HitTestResult::Transparent => HTTRANSPARENT as isize,
                HitTestResult::Caption => {
                    if is_island {
                        HTCLIENT as isize
                    } else {
                        HTCAPTION as isize
                    }
                }
            };
            return LRESULT(value);
        }
        WM_LBUTTONDOWN => {
            // Press feedback: swell the pill while the pointer is held.
            // Capture so the release is reported even if the swell moves
            // the pill under the cursor.
            let is_island = unsafe { (*state).is_island };
            if is_island {
                let (x, y) = lparam_point(lparam);
                let hit = {
                    let alpha = unsafe { (*state).alpha.borrow() };
                    alpha_hit_test(&alpha.bytes, alpha.width, x, y)
                };
                if hit == HitTestResult::Caption {
                    let _ = unsafe { (*state).events.send(WindowEvent::PressChanged(true)) };
                    let _ = unsafe { SetCapture(hwnd) };
                    return LRESULT(0);
                }
            }
        }
        WM_LBUTTONUP => {
            let is_island = unsafe { (*state).is_island };
            if is_island {
                let _ = unsafe { ReleaseCapture() };
                let _ = unsafe { (*state).events.send(WindowEvent::PressChanged(false)) };
                let (x, y) = lparam_point(lparam);
                let hit = {
                    let alpha = unsafe { (*state).alpha.borrow() };
                    alpha_hit_test(&alpha.bytes, alpha.width, x, y)
                };
                if hit == HitTestResult::Caption {
                    let _ = unsafe { (*state).events.send(WindowEvent::ClickAt(x, y)) };
                    return LRESULT(0);
                }
            }
        }
        WM_MOUSEMOVE => {
            // Hover-to-expand: report the enter transition once, then arm
            // leave tracking. Transparent pixels never reach us — NCHITTEST
            // already returns HTTRANSPARENT there — so any MOVE here is over
            // the pill... except the alpha map may lag a fresh frame by one
            // present, hence the re-test.
            let is_island = unsafe { (*state).is_island };
            if is_island {
                let (x, y) = lparam_point(lparam);
                let over_pill = {
                    let alpha = unsafe { (*state).alpha.borrow() };
                    alpha_hit_test(&alpha.bytes, alpha.width, x, y) == HitTestResult::Caption
                };
                let mut inside = unsafe { (*state).hover_inside.borrow_mut() };
                if over_pill && !*inside {
                    *inside = true;
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = unsafe { TrackMouseEvent(&mut track) };
                    let _ = unsafe { (*state).events.send(WindowEvent::HoverChanged(true)) };
                }
            }
        }
        WM_MOUSELEAVE => {
            let is_island = unsafe { (*state).is_island };
            let mut inside = unsafe { (*state).hover_inside.borrow_mut() };
            if *inside {
                // UpdateLayeredWindow resizes synthesize spurious leaves
                // while the cursor is still over the pill. Verify with the
                // real cursor position; a false leave collapsed the pill
                // under a stationary cursor (the "hover makes it disappear"
                // bug) because no further WM_MOUSEMOVE would re-enter.
                let still_inside = is_island && {
                    let alpha = unsafe { (*state).alpha.borrow() };
                    cursor_over_pill_raw(hwnd, &alpha)
                };
                if still_inside {
                    // Re-arm so a genuine later leave is still reported.
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = unsafe { TrackMouseEvent(&mut track) };
                } else {
                    *inside = false;
                    let _ = unsafe { (*state).events.send(WindowEvent::HoverChanged(false)) };
                }
            }
            return LRESULT(0);
        }
        WM_SETTINGCHANGE => {
            // lParam points at a NUL-terminated string naming the changed
            // setting; the light/dark toggle broadcasts "ImmersiveColorSet".
            if lparam.0 != 0 {
                let name =
                    unsafe { windows::core::PCWSTR::from_raw(lparam.0 as *const u16).to_string() };
                if matches!(name.as_deref(), Ok("ImmersiveColorSet")) {
                    let _ = unsafe { (*state).events.send(WindowEvent::SystemThemeChanged) };
                }
            }
        }
        WM_DWMCOLORIZATIONCOLORCHANGED => {
            // Accent color (and its prevalence) changed: same re-resolve as
            // a light/dark flip, so `auto` tints track Settings live.
            let _ = unsafe { (*state).events.send(WindowEvent::SystemThemeChanged) };
            return LRESULT(0);
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
        WM_SETCURSOR => {
            // Interactive affordance: a hand cursor over the pill tells the
            // user the island is clickable, matching Dynamic Island behavior.
            let is_island = unsafe { (*state).is_island };
            if is_island {
                let mut point = POINT { x: 0, y: 0 };
                let mut rect = RECT::default();
                if unsafe { GetCursorPos(&mut point) }.is_ok()
                    && unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok()
                {
                    let height = rect.bottom - rect.top;
                    if height > 4 {
                        let alpha = unsafe { (*state).alpha.borrow() };
                        if alpha_hit_test(
                            &alpha.bytes,
                            alpha.width,
                            point.x - rect.left,
                            point.y - rect.top,
                        ) == HitTestResult::Caption
                        {
                            if let Ok(hand) = unsafe { LoadCursorW(None, IDC_HAND) } {
                                unsafe { SetCursor(Some(hand)) };
                                return LRESULT(1);
                            }
                        }
                    }
                }
            }
            return LRESULT(1);
        }
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
                        tray::TRAY_LAYOUT_CLASSIC => {
                            let _ = unsafe {
                                (*state).events.send(WindowEvent::LayoutChanged(
                                    termielle_core::IslandLayout::Classic,
                                ))
                            };
                        }
                        tray::TRAY_LAYOUT_NOTCH => {
                            let _ = unsafe {
                                (*state).events.send(WindowEvent::LayoutChanged(
                                    termielle_core::IslandLayout::Notch,
                                ))
                            };
                        }
                        tray::TRAY_LAYOUT_ISLAND => {
                            let _ = unsafe {
                                (*state).events.send(WindowEvent::LayoutChanged(
                                    termielle_core::IslandLayout::Island,
                                ))
                            };
                        }
                        tray::TRAY_THEME_LIQUID_DARK => {
                            let _ = unsafe {
                                (*state)
                                    .events
                                    .send(WindowEvent::ThemeChanged("liquid-dark".into()))
                            };
                        }
                        tray::TRAY_THEME_MIDNIGHT => {
                            let _ = unsafe {
                                (*state)
                                    .events
                                    .send(WindowEvent::ThemeChanged("midnight".into()))
                            };
                        }
                        tray::TRAY_THEME_LIGHT => {
                            let _ = unsafe {
                                (*state)
                                    .events
                                    .send(WindowEvent::ThemeChanged("light".into()))
                            };
                        }
                        tray::TRAY_THEME_TRANSPARENT => {
                            let _ = unsafe {
                                (*state)
                                    .events
                                    .send(WindowEvent::ThemeChanged("transparent".into()))
                            };
                        }
                        tray::TRAY_YOFFSET_0 => {
                            let _ = unsafe { (*state).events.send(WindowEvent::YOffsetChanged(0)) };
                        }
                        tray::TRAY_YOFFSET_12 => {
                            let _ =
                                unsafe { (*state).events.send(WindowEvent::YOffsetChanged(12)) };
                        }
                        tray::TRAY_YOFFSET_80 => {
                            let _ =
                                unsafe { (*state).events.send(WindowEvent::YOffsetChanged(80)) };
                        }
                        tray::TRAY_YOFFSET_150 => {
                            let _ =
                                unsafe { (*state).events.send(WindowEvent::YOffsetChanged(150)) };
                        }
                        tray::TRAY_YOFFSET_300 => {
                            let _ =
                                unsafe { (*state).events.send(WindowEvent::YOffsetChanged(300)) };
                        }
                        tray::TRAY_TOGGLE_TASKS => {
                            let _ = unsafe { (*state).events.send(WindowEvent::ToggleTasks) };
                        }
                        tray::TRAY_TOGGLE_HOVER => {
                            let _ = unsafe { (*state).events.send(WindowEvent::ToggleHoverExpand) };
                        }
                        tray::TRAY_TOGGLE_FACE => {
                            let _ = unsafe { (*state).events.send(WindowEvent::ToggleFace) };
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
    glass: GlassConfig,
    /// Blurred wallpaper captured by the worker thread, sampled under the
    /// pill during draw. `None` until the first worker round lands.
    backdrop: RefCell<Option<crate::backdrop::Backdrop>>,
    /// Last ULW destination (x, y, w, h) in physical pixels.
    last_dest: (i32, i32, u32, u32),
    /// Monitor the island is anchored to. Picked from the cursor position on
    /// the first present and re-picked on display changes, so the notch
    /// follows the display the user is on — but never jumps mid-morph just
    /// because the cursor crossed a screen edge.
    anchor_monitor: Option<windows::Win32::Graphics::Gdi::HMONITOR>,
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

        let island_enabled = config.island.layout != IslandLayout::Classic;
        let ex_style = if config.always_on_top || island_enabled {
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
            is_island: config.island.is_enabled(),
            hover_inside: RefCell::new(false),
        });
        let state_ptr = &*state as *const WindowState as isize;
        let _ = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr) };

        let window = Self {
            hwnd,
            state,
            receiver,
            repositioned: false,
            anchor_monitor: None,
            render: config.render,
            tray: false,
            glass: config.island.glass.clone(),
            backdrop: RefCell::new(None),
            last_dest: (0, 0, 1, 1),
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

    /// Presents `frame`, resizing and re-clamping the window on size or
    /// first-present, then blitting the frame onto the layered surface with
    /// per-pixel alpha (or the configured fallback renderer).
    pub fn present(&mut self, frame: &FrameBuffer) -> Result<(), WindowError> {
        self.present_with_anchor(frame, None)
    }

    /// Present with optional island anchoring. When `island` is `Some` the
    /// window is centered at the top edge (notch) or just below it (island)
    /// instead of clamping to the work area.
    ///
    /// Frames arrive authored at device pixels (see
    /// [`Controller::render_scale`]): the window takes their size as-is and
    /// presents 1:1 — there is no filtering step anymore.
    pub fn present_with_anchor(
        &mut self,
        frame: &FrameBuffer,
        island: Option<(bool, i32)>,
    ) -> Result<(), WindowError> {
        let (w, h) = (frame.width, frame.height);
        let mut rect = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut rect) }?;
        let current = Rect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        };
        let needs_move =
            current.width() != w as i32 || current.height() != h as i32 || !self.repositioned;
        let (x, y) = if let Some((attached, y_offset)) = island {
            self.island_anchored_position(w as i32, h as i32, attached, y_offset)
        } else {
            self.clamped_position(w as i32, h as i32, (current.left, current.top))
        };
        if self.render == RenderMode::PerPixel {
            // The per-pixel path updates position, size, and content in ONE
            // UpdateLayeredWindow call. The content-only variant (pptDst and
            // psize NULL) is silently ignored by some DWM configurations,
            // which left the window composited empty.
            self.repositioned = true;
            self.last_dest = (x, y, w, h);
            return self.draw_at(frame, x, y);
        }
        if needs_move {
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    None,
                    x,
                    y,
                    w as i32,
                    h as i32,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOSENDCHANGING,
                )
            }?;
            self.repositioned = true;
        }
        self.draw_color_key(frame)
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

    /// Updates the island hit-test flag live when layout changes via the tray.
    pub fn set_island(&mut self, is_island: bool) {
        self.state.is_island = is_island;
    }

    /// Updates the glass material live (theme switches); the new tint and
    /// blur apply on the next present.
    pub fn set_glass(&mut self, glass: &GlassConfig) {
        self.glass = glass.clone();
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
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            let work = Rect {
                left: info.rcWork.left,
                top: info.rcWork.top,
                right: info.rcWork.right,
                bottom: info.rcWork.bottom,
            };
            return clamp_to_work_area(preferred, (width, height), work);
        }
        preferred
    }

    /// Supplies a freshly captured+blurred backdrop from the worker thread.
    pub fn set_backdrop(&mut self, bg: crate::backdrop::Backdrop) {
        *self.backdrop.borrow_mut() = Some(bg);
    }

    /// The last destination rect the pill was drawn at (physical pixels), so
    /// the worker knows what region to capture for the glass.
    pub fn last_dest(&self) -> (i32, i32, u32, u32) {
        self.last_dest
    }

    /// Monitor DPI scale for the window's current monitor: physical pixels
    /// per logical pixel. Falls back to 1.0 when the query fails — the
    /// overlay keeps presenting, just unscaled.
    pub fn dpi_scale(&self) -> f32 {
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
    }

    /// Whether the cursor is currently over an opaque pill pixel. Polled by
    /// the GUI loop as a backstop for spurious `WM_MOUSELEAVE`s.
    pub fn cursor_over_pill(&self) -> bool {
        let alpha = self.state.alpha.borrow();
        cursor_over_pill_raw(self.hwnd, &alpha)
    }

    /// Cursor position in frame-local coordinates when it is over the pill.
    pub fn cursor_client_pos(&self) -> Option<(i32, i32)> {
        unsafe {
            let mut point = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut point).is_err() {
                return None;
            }
            let mut rect = RECT::default();
            if GetWindowRect(self.hwnd, &mut rect).is_err() {
                return None;
            }
            let (x, y) = (point.x - rect.left, point.y - rect.top);
            let alpha = self.state.alpha.borrow();
            if alpha_hit_test(&alpha.bytes, alpha.width, x, y) == HitTestResult::Caption {
                Some((x, y))
            } else {
                None
            }
        }
    }
    /// Clears the tracked anchor monitor so the next present re-picks it
    /// from the cursor position. Called on display/DPI changes, when the
    /// monitor set may have been rearranged out from under the island.
    pub fn reset_anchor_monitor(&mut self) {
        self.anchor_monitor = None;
    }

    /// Top-center position on the tracked anchor monitor. The monitor is
    /// picked once from the cursor position — the display the user is
    /// looking at — and then kept: re-picking on every morph would teleport
    /// the island whenever the cursor crosses a screen edge mid-animation.
    fn island_anchored_position(
        &mut self,
        width: i32,
        height: i32,
        attached: bool,
        y_offset: i32,
    ) -> (i32, i32) {
        if self.anchor_monitor.is_none() {
            let mut point = POINT { x: 0, y: 0 };
            let _ = unsafe { GetCursorPos(&mut point) };
            self.anchor_monitor = Some(unsafe {
                windows::Win32::Graphics::Gdi::MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST)
            });
        }
        if let Some(monitor) = self.anchor_monitor {
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
                let work = (
                    info.rcWork.left,
                    info.rcWork.top,
                    info.rcWork.right,
                    info.rcWork.bottom,
                );
                return island_anchored_position(width, height, attached, y_offset, work);
            }
            // A tracked monitor that no longer resolves (unplugged between
            // the pick and this present): drop it so the next present
            // re-picks instead of pinning to a dead rectangle.
            self.anchor_monitor = None;
        }
        // Fallback: center on 1920 if no monitor info
        island_anchored_position(width, height, attached, y_offset, (0, 0, 1920, 1080))
    }

    fn draw_at(&self, frame: &FrameBuffer, x: i32, y: i32) -> Result<(), WindowError> {
        // Frames arrive authored at device pixels: every blit below is 1:1,
        // so the old bilinear upscale is gone and pixels land exactly.
        let (w, h) = (frame.width, frame.height);
        let expected = w as usize * h as usize * 4;
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
                biWidth: w as i32,
                biHeight: -(h as i32),
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

        // Frosted glass: capture the live backdrop once (cached) and blur
        // it under the frame's own tint. The backdrop excludes layered windows,
        // so the pill can never feed back into itself. `None` means blur is off
        // or capture failed — fall back to the flat procedural frame.
        let backdrop = if self.glass.blur_radius > 0 {
            self.backdrop.borrow().clone()
        } else {
            None
        };

        let is_hidden_sensor = h <= 4;
        let mut alpha_map = Vec::with_capacity(w as usize * h as usize);
        let len = w as usize * h as usize * 4;
        let dst = unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, len) };
        if is_hidden_sensor {
            // Assign alpha = 1 for the hidden sensor: 1/255 opacity is completely invisible
            // to the human eye, but ensures Windows DWM treats the window as an active
            // mouse hit target and dispatches WM_NCHITTEST / WM_MOUSEMOVE to window_proc.
            for px in dst.chunks_exact_mut(4) {
                px[0] = 0;
                px[1] = 0;
                px[2] = 0;
                px[3] = 1;
            }
            alpha_map.resize(w as usize * h as usize, ALPHA_HIT_THRESHOLD);
        } else {
            // 1:1 blit: the frame already matches the window surface.
            match backdrop.as_ref() {
                Some(bg) => {
                    for yy in 0..h {
                        for xx in 0..w {
                            let source = &frame.pixels_pbgra
                                [(yy as usize * w as usize + xx as usize) * 4..][..4];
                            let target = (yy as usize * w as usize + xx as usize) * 4;
                            if source[3] == 0 {
                                // Outside the pill: stay fully transparent so clicks
                                // pass through and the desktop shows untouched.
                                dst[target..target + 4].fill(0);
                            } else {
                                // Sample the backdrop proportionally and clamped:
                                // during morphs the stored backdrop was captured
                                // for the previous pill size, and blur hides the
                                // stretch. Never panics on size mismatch.
                                let bx = ((u64::from(xx) * u64::from(bg.width)) / u64::from(w))
                                    .min(u64::from(bg.width.saturating_sub(1)))
                                    as usize;
                                let by = ((u64::from(yy) * u64::from(bg.height)) / u64::from(h))
                                    .min(u64::from(bg.height.saturating_sub(1)))
                                    as usize;
                                let bi = (by * bg.width as usize + bx) * 4;
                                let ia = 255 - u32::from(source[3]);
                                let bg_b = u32::from(bg.pixels.get(bi).copied().unwrap_or(0));
                                let bg_g = u32::from(bg.pixels.get(bi + 1).copied().unwrap_or(0));
                                let bg_r = u32::from(bg.pixels.get(bi + 2).copied().unwrap_or(0));
                                let comp_b = (u32::from(source[0]) + bg_b * ia / 255).min(255);
                                let comp_g = (u32::from(source[1]) + bg_g * ia / 255).min(255);
                                let comp_r = (u32::from(source[2]) + bg_r * ia / 255).min(255);
                                let tint_alpha = u32::from(self.glass.tint[3]).max(1);
                                let is_interior = source[3] >= self.glass.tint[3].saturating_sub(15);
                                if is_interior {
                                    dst[target] = comp_b as u8;
                                    dst[target + 1] = comp_g as u8;
                                    dst[target + 2] = comp_r as u8;
                                    dst[target + 3] = 255;
                                } else {
                                    // Anti-aliased outer edge: scale colors by coverage so PBGRA stays valid
                                    let edge_cov = ((u32::from(source[3]) * 255) / tint_alpha).min(255);
                                    dst[target] = (comp_b * edge_cov / 255) as u8;
                                    dst[target + 1] = (comp_g * edge_cov / 255) as u8;
                                    dst[target + 2] = (comp_r * edge_cov / 255) as u8;
                                    dst[target + 3] = edge_cov as u8;
                                }
                            }
                            // Hit-testing still follows the frame's own alpha, so the
                            // rounded corners stay click-through.
                            alpha_map.push(source[3]);
                        }
                    }
                }
                None => {
                    for yy in 0..h {
                        for xx in 0..w {
                            let source = &frame.pixels_pbgra
                                [(yy as usize * w as usize + xx as usize) * 4..][..4];
                            let target = (yy as usize * w as usize + xx as usize) * 4;
                            dst[target..target + 4].copy_from_slice(source);
                            alpha_map.push(source[3]);
                        }
                    }
                }
            }
        }

        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        // The DIB must be selected into the source DC before UpdateLayeredWindow:
        // CreateDIBSection only allocates the section and returns the `bits`
        // pointer, it does not select the bitmap. Without this the DC still has
        // the stock 1x1 monochrome bitmap, ULW composites nothing, and the
        // window shows an empty (fully transparent) surface.
        let previous = unsafe { SelectObject(memory, HGDIOBJ(dib.0)) };
        let origin = POINT { x: 0, y: 0 };
        // The window is already sized and positioned by `present` via
        // SetWindowPos; leaving pptDst and psize NULL updates only the surface
        // contents, which is the one variant that also works on systems where
        // DWM rejects a full redirection update.
        // Full-form ULW: destination position, size, and content are set
        // atomically. The content-only variant (pptDst/psize NULL) is
        // silently ignored by some DWM configurations, which left this
        // window composited empty — invisible, despite "success".
        let dest = POINT { x, y };
        let size = SIZE {
            cx: w as i32,
            cy: h as i32,
        };
        let update = unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                Some(screen),
                Some(&dest),
                Some(&size),
                Some(memory),
                Some(&origin),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        };
        // Restore the stock bitmap so DeleteDC's cleanup bookkeeping stays
        // symmetric, then release everything. ULW has already copied the
        // pixels into the redirection surface at this point.
        unsafe { SelectObject(memory, previous) };
        let _ = unsafe { DeleteObject(HGDIOBJ(dib.0)) };
        let _ = unsafe { DeleteDC(memory) };
        let _ = unsafe { ReleaseDC(None, screen) };
        update?;

        *self.state.alpha.borrow_mut() = AlphaMap {
            bytes: alpha_map,
            width: w,
        };
        Ok(())
    }

    /// Paints `frame` into the window's own DC as straight RGB, replacing
    /// every pixel below [`COLOR_KEY_ALPHA_THRESHOLD`] with the key color
    /// that `SetLayeredWindowAttributes(LWA_COLORKEY)` removes from the
    /// composite. The whole window is otherwise opaque; the per-pixel alpha
    /// map is still produced for click-through hit testing.
    fn draw_color_key(&self, frame: &FrameBuffer) -> Result<(), WindowError> {
        let (w, h) = (frame.width, frame.height);
        let expected = w as usize * h as usize * 4;
        if frame.pixels_pbgra.len() != expected {
            return Err(WindowError::FrameBuffer);
        }
        // 24-bit DIB rows are padded to a 4-byte boundary. The width times
        // three is not always divisible by four (329 px -> 987 bytes -> 988
        // padded), and `SetDIBitsToDevice` reads rows at the padded stride:
        // a buffer without the padding shifts every row by one byte and
        // renders diagonal garbage.
        let is_hidden_sensor = h <= 4;
        let stride = dib_stride(w);
        let mut rgb = vec![0u8; stride * h as usize];
        let mut alpha_map = Vec::with_capacity(w as usize * h as usize);
        if is_hidden_sensor {
            for y in 0..h {
                for x in 0..w {
                    let target = y as usize * stride + x as usize * 3;
                    rgb[target..target + 3].copy_from_slice(&COLOR_KEY_BGRA);
                }
            }
            alpha_map.resize(w as usize * h as usize, ALPHA_HIT_THRESHOLD);
        } else {
            // 1:1 blit: the frame already matches the window surface.
            for y in 0..h {
                for x in 0..w {
                    let source =
                        &frame.pixels_pbgra[(y as usize * w as usize + x as usize) * 4..][..4];
                    let target = y as usize * stride + x as usize * 3;
                    rgb[target..target + 3].copy_from_slice(&straight_or_key(
                        source[3], source[0], source[1], source[2],
                    ));
                    alpha_map.push(source[3]);
                }
            }
        }

        let dc = unsafe { GetDC(Some(self.hwnd)) };
        if dc.is_invalid() {
            return Err(WindowError::Win32(unsafe { GetLastError().0 }));
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 24,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                ..Default::default()
            },
            bmiColors: [RGBQUAD::default()],
        };
        // SAFETY: `rgb` holds `h` top-down rows of `w` 24-bit
        // pixels and outlives the call; `dc` is the window's own DC.
        let lines = unsafe {
            SetDIBitsToDevice(
                dc,
                0,
                0,
                w,
                h,
                0,
                0,
                0,
                h,
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
            width: w,
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

    #[test]
    fn test_window_position_after_present() {
        let mut config = AppConfig::default();
        config.island.layout = IslandLayout::Island;
        let mut win = OverlayWindow::create(&config, false).unwrap();
        let frame = FrameBuffer {
            width: 140,
            height: 36,
            pixels_pbgra: vec![255; 140 * 36 * 4],
            delay_ms: 0,
            loop_index: 0,
            scale: 1.0,
        };
        win.present_with_anchor(&frame, Some((false, 10))).unwrap();
        let (rect, mon) = win.position();
        println!("Window pos: {:?}, monitor: {}", rect, mon);
        let vis = unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(win.hwnd) };
        println!("IsWindowVisible: {}", vis.as_bool());
        assert!(vis.as_bool());
        assert_eq!(rect.width(), 140);
        assert_eq!(rect.height(), 36);
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

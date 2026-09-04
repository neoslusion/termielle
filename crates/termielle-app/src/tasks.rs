//! Running-task icons and now-playing media for the notch dashboard.
//!
//! The expanded pill shows one small app icon per visible top-level window
//! plus the current media title/artist while something plays. Icon reads are
//! cheap (`WM_GETICON`/class-icon handles + one 24px `DrawIconEx` blit each)
//! and media comes from the System Media Transport Controls; both are polled
//! on a background worker thread and posted to the GUI thread, so hovering
//! and morphing never stall.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HGDIOBJ,
    ReleaseDC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DI_NORMAL, DrawIconEx, EnumWindows, GCLP_HICON, GCLP_HICONSM, GWL_EXSTYLE, GetClassLongPtrW,
    GetWindowLongPtrW, GetWindowRect, HICON, ICON_SMALL2, IsIconic, IsWindowVisible,
    SEND_MESSAGE_TIMEOUT_FLAGS, SMTO_ABORTIFHUNG, SMTO_NORMAL, SendMessageTimeoutW, WM_GETICON,
};

/// Edge length of a captured app icon, in pixels.
pub const ICON_PX: u32 = 24;

/// How often the worker repolls while enabled, in milliseconds.
pub const TASKS_REFRESH_MS: u64 = 1500;

/// One enumerated top-level window: a candidate dashboard task.
#[derive(Clone, Debug)]
pub struct TaskWindow {
    pub hwnd: isize,
    pub title: String,
}

/// One app icon for the dashboard row, premultiplied BGRA at [`ICON_PX`].
#[derive(Clone, Debug)]
pub struct TaskIcon {
    pub hwnd: isize,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub pixels_pbgra: Vec<u8>,
}

/// Now-playing media from the System Media Transport Controls.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    /// Source app user-model id (e.g. `Spotify.exe`), may be empty.
    pub app: String,
    pub playing: bool,
}

/// One worker round: fresh icons, the current media state, and (when a
/// capture is due) the blurred wallpaper backdrop for the glass.
#[derive(Clone, Debug, Default)]
pub struct WorkerUpdate {
    pub icons: Vec<TaskIcon>,
    pub media: Option<MediaInfo>,
    pub backdrop: Option<crate::backdrop::Backdrop>,
}

/// The pill rect the worker should capture behind, updated by the GUI thread
/// on every present. Physical pixels.
#[derive(Clone, Copy, Debug)]
pub struct BackdropRequest {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub radius: u32,
    pub tint: [u8; 4],
}

/// Shared worker controls, owned by the GUI thread and read by the capture
/// worker. Lets tray toggles enable/disable capture without restarting the
/// thread.
#[derive(Debug, Default)]
pub struct WorkerConfig {
    /// When false the worker sleeps instead of polling.
    pub enabled: AtomicBool,
    /// Maximum windows captured per round (0-6).
    pub max_thumbs: AtomicU32,
}

impl WorkerConfig {
    pub fn new(enabled: bool, max_thumbs: u32) -> Self {
        Self {
            enabled: AtomicBool::new(enabled),
            max_thumbs: AtomicU32::new(max_thumbs.min(6)),
        }
    }
}

/// Starts the background worker. It enumerates top-level windows, reads up to
/// `max_thumbs` app icons, samples the media session, then sends the batch
/// and wakes the GUI thread. Everything here may block (hung windows time
/// out via `SMTO_ABORTIFHUNG`; WinRT calls fail soft), and none of it ever
/// runs on the GUI thread — that was the hover freeze.
///
/// The worker exits when the receiver is dropped.
pub fn spawn_worker(
    wake: crate::window::WakeHandle,
    sender: Sender<WorkerUpdate>,
    config: Arc<WorkerConfig>,
    backdrop_request: Arc<std::sync::Mutex<Option<BackdropRequest>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        // WinRT media queries need a multithreaded apartment on THIS thread
        // (initializing on the spawner would leave `RequestAsync` failing
        // here, and media would stay empty forever).
        ensure_winrt();
        let mut last_sig: Option<(i32, i32, u32, u32, u32, [u8; 4])> = None;
        let mut last_capture_ms: u64 = 0;
        // Capture immediately so the first expansion already has content.
        loop {
            if config.enabled.load(Ordering::Relaxed) {
                let max = config.max_thumbs.load(Ordering::Relaxed) as usize;
                let icons = enumerate()
                    .into_iter()
                    .take(max)
                    .filter_map(|task| window_icon(task.hwnd, &task.title))
                    .collect::<Vec<_>>();

                // Backdrop: capture when the pill geometry/material changed
                // or the previous capture is over a second old.
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let request = backdrop_request
                    .lock()
                    .ok()
                    .and_then(|mut guard| guard.take());
                let mut backdrop = None;
                if let Some(req) = request {
                    let sig = (req.x, req.y, req.w, req.h, req.radius, req.tint);
                    let due =
                        last_sig != Some(sig) || now_ms.saturating_sub(last_capture_ms) > 1000;
                    if req.w > 0 && req.h > 0 && due {
                        let mut bg = crate::backdrop::capture_backdrop(
                            req.x,
                            req.y,
                            req.w,
                            req.h,
                            [req.tint[0], req.tint[1], req.tint[2], 255],
                        );
                        if let Some(bg) = bg.as_mut() {
                            crate::backdrop::box_blur(bg, req.radius);
                        }
                        backdrop = bg;
                        last_sig = Some(sig);
                        last_capture_ms = now_ms;
                    } else if !due {
                        // Keep the request for the next cycle.
                        *backdrop_request.lock().unwrap() = Some(req);
                    }
                }

                let update = WorkerUpdate {
                    icons,
                    media: current_media(),
                    backdrop,
                };
                if sender.send(update).is_err() {
                    break;
                }
                let _ = wake.post();
            }
            std::thread::sleep(Duration::from_millis(TASKS_REFRESH_MS));
        }
    })
}

fn ensure_winrt() {
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    });
}

thread_local! {
    static ENUM_BUF: RefCell<Vec<TaskWindow>> = const { RefCell::new(Vec::new()) };
}

/// Lists candidate task windows, newest-z-order first.
pub fn enumerate() -> Vec<TaskWindow> {
    ENUM_BUF.with(|cell| {
        cell.borrow_mut().clear();
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(0));
        }
        cell.borrow().clone()
    })
}

unsafe extern "system" fn enum_proc(hwnd: HWND, _lparam: LPARAM) -> windows::core::BOOL {
    // NOTE: must not lock a shared Mutex here — `enumerate` runs on this same
    // thread and `EnumWindows` invokes us synchronously, so a shared lock
    // would deadlock. The thread-local buffer above is reentrancy-safe.
    let push = |task: TaskWindow| {
        ENUM_BUF.with(|cell| cell.borrow_mut().push(task));
    };
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return windows::core::BOOL(1);
    }
    if unsafe { IsIconic(hwnd) }.as_bool() {
        return windows::core::BOOL(1);
    }
    // Skip our own overlay.
    let mut class = [0u16; 64];
    let len = unsafe { windows::Win32::UI::WindowsAndMessaging::GetClassNameW(hwnd, &mut class) };
    if len > 0 && String::from_utf16_lossy(&class[..len as usize]) == "termielle_overlay" {
        return windows::core::BOOL(1);
    }
    // Skip cloaked (UWP hosted / hidden) windows.
    let mut cloaked: u32 = 0;
    let _ = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    };
    if cloaked != 0 {
        return windows::core::BOOL(1);
    }
    // Skip tool windows (tooltips, floating palettes).
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if ex_style & 0x80 != 0 {
        return windows::core::BOOL(1);
    }
    // Must have a title.
    let mut title = [0u16; 128];
    let len = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut title) };
    if len <= 0 {
        return windows::core::BOOL(1);
    }
    let text = String::from_utf16_lossy(&title[..len as usize]);
    // Must be reasonably sized.
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return windows::core::BOOL(1);
    }
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if w < 120 || h < 60 {
        return windows::core::BOOL(1);
    }
    push(TaskWindow {
        hwnd: hwnd.0 as isize,
        title: text.chars().take(64).collect(),
    });
    windows::core::BOOL(1)
}

/// Reads one window's app icon: per-window icon first (favicons etc.),
/// falling back to the window-class icons. Hung windows time out instead of
/// hanging the worker.
pub fn window_icon(hwnd: isize, title: &str) -> Option<TaskIcon> {
    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    let hicon = icon_handle(hwnd)?;

    let screen = unsafe { GetDC(None) };
    if screen.is_invalid() {
        return None;
    }
    let memory = unsafe { CreateCompatibleDC(Some(screen)) };
    if memory.is_invalid() {
        let _ = unsafe { ReleaseDC(None, screen) };
        return None;
    }

    let bmi = windows::Win32::Graphics::Gdi::BITMAPINFO {
        bmiHeader: windows::Win32::Graphics::Gdi::BITMAPINFOHEADER {
            biSize: std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32,
            biWidth: ICON_PX as i32,
            biHeight: -(ICON_PX as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: windows::Win32::Graphics::Gdi::BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let dib = unsafe { CreateDIBSection(Some(memory), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) };
    let Ok(dib) = dib else {
        let _ = unsafe { DeleteDC(memory) };
        let _ = unsafe { ReleaseDC(None, screen) };
        return None;
    };
    let previous = unsafe { windows::Win32::Graphics::Gdi::SelectObject(memory, HGDIOBJ(dib.0)) };

    let drawn = unsafe {
        DrawIconEx(
            memory,
            0,
            0,
            hicon,
            ICON_PX as i32,
            ICON_PX as i32,
            0,
            None,
            DI_NORMAL,
        )
        .is_ok()
    };

    let mut pixels = vec![0u8; (ICON_PX * ICON_PX * 4) as usize];
    if drawn && !bits.is_null() {
        // SAFETY: bits points at ICON_PX*ICON_PX*4 bytes of DIB memory that
        // outlives the read.
        let src = unsafe { std::slice::from_raw_parts(bits as *const u8, pixels.len()) };
        pixels.copy_from_slice(src);
    }

    unsafe {
        windows::Win32::Graphics::Gdi::SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = DeleteDC(memory);
        let _ = ReleaseDC(None, screen);
    }

    if !drawn {
        return None;
    }
    Some(TaskIcon {
        hwnd: hwnd.0 as isize,
        title: title.to_string(),
        width: ICON_PX,
        height: ICON_PX,
        pixels_pbgra: pixels,
    })
}

fn icon_handle(hwnd: HWND) -> Option<HICON> {
    // Per-window icon first (hung-app safe: 100 ms timeout).
    let mut out: usize = 0;
    let flags = SEND_MESSAGE_TIMEOUT_FLAGS(SMTO_ABORTIFHUNG.0 | SMTO_NORMAL.0);
    let _ = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETICON,
            WPARAM(ICON_SMALL2 as usize),
            LPARAM(0),
            flags,
            100,
            Some(&mut out as *mut usize),
        )
    };
    if out != 0 {
        return Some(HICON(out as *mut core::ffi::c_void));
    }
    // Window-class icons.
    for id in [GCLP_HICONSM, GCLP_HICON] {
        let raw = unsafe { GetClassLongPtrW(hwnd, id) };
        if raw != 0 {
            return Some(HICON(raw as *mut core::ffi::c_void));
        }
    }
    None
}

/// Samples the System Media Transport Controls: `Some` only while something
/// is actually playing with a non-empty title/artist. Fail-soft by design —
/// no session, no COM, or no media all yield `None`, never an error surface.
pub fn current_media() -> Option<MediaInfo> {
    let info = current_media_inner().ok()??;
    if !info.playing || (info.title.is_empty() && info.artist.is_empty()) {
        return None;
    }
    Some(info)
}

fn current_media_inner() -> windows::core::Result<Option<MediaInfo>> {
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSessionManager as Sessions,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Playback,
    };
    let manager = block_async(
        Sessions::RequestAsync()?,
        |op| op.Status(),
        |op| op.GetResults(),
    )?;
    let session = manager.GetCurrentSession()?;
    if session.GetPlaybackInfo()?.PlaybackStatus()? != Playback::Playing {
        return Ok(None);
    }
    let props = block_async(
        session.TryGetMediaPropertiesAsync()?,
        |op| op.Status(),
        |op| op.GetResults(),
    )?;
    Ok(Some(MediaInfo {
        title: props.Title()?.to_string(),
        artist: props.Artist()?.to_string(),
        app: session
            .SourceAppUserModelId()
            .map(|id| id.to_string())
            .unwrap_or_default(),
        playing: true,
    }))
}

/// Blocks a WinRT async operation by polling `Status()` on the worker thread
/// (2.5s budget). There is no blocking `get()` on `IAsyncOperation`, and this
/// must never run on the GUI thread.
fn block_async<T, Op>(
    op: Op,
    status: impl Fn(&Op) -> windows::core::Result<windows_future::AsyncStatus>,
    results: impl FnOnce(&Op) -> windows::core::Result<T>,
) -> windows::core::Result<T> {
    use windows_future::AsyncStatus;
    for _ in 0..50 {
        match status(&op)? {
            AsyncStatus::Completed => return results(&op),
            AsyncStatus::Canceled | AsyncStatus::Error => {
                return Err(windows::core::Error::empty());
            }
            _ => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    Err(windows::core::Error::empty())
}

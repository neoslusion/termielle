//! Background dashboard worker: SMTC media state, artwork decoding, and the
//! frosted-glass backdrop capture. Everything here runs off the GUI thread;
//! the worker posts results and wakes the overlay to composite.
//!
//! Note the island's task icons were removed: the pill shows **live
//! activities only** (agent sessions + media), matching the iOS Dynamic
//! Island — not a window switcher.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GetDC, HGDIOBJ, ReleaseDC, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DI_NORMAL, DrawIconEx, EnumWindows, GCLP_HICON, GCLP_HICONSM, GWL_EXSTYLE, GetClassLongPtrW,
    GetClassNameW, GetWindowLongPtrW, GetWindowRect, GetWindowTextW, HICON, ICON_BIG, ICON_SMALL2,
    IsIconic, IsWindowVisible, SEND_MESSAGE_TIMEOUT_FLAGS, SMTO_ABORTIFHUNG, SMTO_NORMAL,
    SW_RESTORE, SendMessageTimeoutW, SetForegroundWindow, ShowWindow, WM_GETICON, WS_EX_TOOLWINDOW,
};

/// Edge length of a decoded media artwork thumbnail, in pixels (high-resolution).
pub const ICON_PX: u32 = 64;

/// Edge length of a decoded window task icon, in pixels.
pub const TASK_ICON_PX: u32 = 36;

/// How often the worker repolls running tasks while enabled, in milliseconds.
pub const TASKS_REFRESH_MS: u64 = 1500;

/// How often the worker repolls the media session, in milliseconds.
pub const MEDIA_REFRESH_MS: u64 = 1000;

/// Worker loop tick. Backdrop requests are evaluated every tick so the
/// frosted glass samples the live desktop content behind the pill (windows
/// dragging, video) instead of a seconds-stale snapshot.
pub const WORKER_TICK_MS: u64 = 120;

/// How often the backdrop is recaptured for an unchanged geometry, in
/// milliseconds — fast enough that glass tracks moving content behind it.
pub const BACKDROP_REFRESH_MS: u64 = 140;

/// A small premultiplied-BGRA bitmap (media artwork or window icon), drawn rounded.
#[derive(Clone, Debug)]
pub struct ThumbBitmap {
    pub width: u32,
    pub height: u32,
    pub pixels_pbgra: Vec<u8>,
}

/// Open application window icon and title.
#[derive(Clone, Debug)]
pub struct TaskIcon {
    pub hwnd: isize,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub pixels_pbgra: Vec<u8>,
}

/// Now-playing media from the System Media Transport Controls.
#[derive(Clone, Debug, Default)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    /// Source app user-model id (e.g. `Spotify.exe`), may be empty.
    pub app: String,
    pub playing: bool,
    /// Decoded artwork thumbnail (premultiplied BGRA), when the source
    /// exposes one. None on decode failure or absent artwork.
    pub thumbnail: Option<ThumbBitmap>,
}

/// Toggles play/pause on the system's active media session via the
/// standard multimedia key. Works for any SMTC source (YouTube in a
/// browser, Spotify, media players) without touching that app directly.
pub fn toggle_media_playback() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_MEDIA_PLAY_PAUSE, keybd_event,
    };
    unsafe {
        keybd_event(VK_MEDIA_PLAY_PAUSE.0 as u8, 0, KEYBD_EVENT_FLAGS(0), 0);
        keybd_event(VK_MEDIA_PLAY_PAUSE.0 as u8, 0, KEYEVENTF_KEYUP, 0);
    }
}

/// Skips to the next track on the system's active media session.
pub fn media_next_track() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_MEDIA_NEXT_TRACK, keybd_event,
    };
    unsafe {
        keybd_event(VK_MEDIA_NEXT_TRACK.0 as u8, 0, KEYBD_EVENT_FLAGS(0), 0);
        keybd_event(VK_MEDIA_NEXT_TRACK.0 as u8, 0, KEYEVENTF_KEYUP, 0);
    }
}

/// Skips to the previous track on the system's active media session.
pub fn media_prev_track() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_MEDIA_PREV_TRACK, keybd_event,
    };
    unsafe {
        keybd_event(VK_MEDIA_PREV_TRACK.0 as u8, 0, KEYBD_EVENT_FLAGS(0), 0);
        keybd_event(VK_MEDIA_PREV_TRACK.0 as u8, 0, KEYEVENTF_KEYUP, 0);
    }
}

/// Brings the specified window to the foreground, restoring it if minimized.
pub fn activate_window(hwnd: isize) {
    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

/// One worker round: the current media state, open window tasks, and (when a capture is due) the
/// blurred wallpaper backdrop for the glass.
#[derive(Clone, Debug, Default)]
pub struct WorkerUpdate {
    pub media: Option<MediaInfo>,
    pub tasks: Vec<TaskIcon>,
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

/// Shared worker controls, owned by the GUI thread and read by the worker.
#[derive(Debug, Default)]
pub struct WorkerConfig {
    /// When false the worker sleeps instead of polling.
    pub enabled: AtomicBool,
}

impl WorkerConfig {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: AtomicBool::new(enabled),
        }
    }
}

/// Starts the background worker: SMTC media sampling and frosted-glass
/// backdrop capture. The worker exits when the receiver is dropped.
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
        let mut last_media_ms: u64 = 0;
        let mut last_tasks_ms: u64 = 0;
        let mut last_media: Option<MediaInfo> = None;
        let mut last_tasks: Vec<TaskIcon> = Vec::new();
        loop {
            if config.enabled.load(Ordering::Relaxed) {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                // Backdrop: capture when the pill geometry/material changed
                // or the previous capture aged out, so the glass tracks the
                // live desktop behind it rather than a stale snapshot.
                let request = backdrop_request
                    .lock()
                    .ok()
                    .and_then(|mut guard| guard.take());
                let mut backdrop = None;
                if let Some(req) = request {
                    let sig = (req.x, req.y, req.w, req.h, req.radius, req.tint);
                    let due = last_sig != Some(sig)
                        || now_ms.saturating_sub(last_capture_ms) > BACKDROP_REFRESH_MS;
                    if req.w > 0 && req.h > 0 && due {
                        let mut bg = crate::backdrop::capture_backdrop(
                            req.x,
                            req.y,
                            req.w,
                            req.h,
                            [req.tint[0], req.tint[1], req.tint[2], 255],
                        );
                        if let Some(bg) = bg.as_mut() {
                            crate::backdrop::blur_soft(bg, req.radius);
                        }
                        backdrop = bg;
                        last_sig = Some(sig);
                        last_capture_ms = now_ms;
                    } else if !due {
                        // Keep the request for the next cycle.
                        *backdrop_request.lock().unwrap() = Some(req);
                    }
                }

                // Expensive polls stay on their own cadences.
                let media_due = now_ms.saturating_sub(last_media_ms) >= MEDIA_REFRESH_MS;
                if media_due {
                    last_media_ms = now_ms;
                    last_media = current_media();
                }
                let tasks_due = now_ms.saturating_sub(last_tasks_ms) >= TASKS_REFRESH_MS;
                if tasks_due {
                    last_tasks_ms = now_ms;
                    last_tasks = enumerate_tasks(6);
                }

                if backdrop.is_some() || media_due || tasks_due {
                    let update = WorkerUpdate {
                        media: last_media.clone(),
                        tasks: last_tasks.clone(),
                        backdrop,
                    };
                    if sender.send(update).is_err() {
                        break;
                    }
                    let _ = wake.post();
                }
            }
            std::thread::sleep(Duration::from_millis(WORKER_TICK_MS));
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
    let thumbnail = decode_media_thumbnail(&props).unwrap_or(None);
    Ok(Some(MediaInfo {
        title: props.Title()?.to_string(),
        artist: props.Artist()?.to_string(),
        app: session
            .SourceAppUserModelId()
            .map(|id| id.to_string())
            .unwrap_or_default(),
        playing: true,
        thumbnail,
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

/// Decodes the SMTC artwork thumbnail to a small premultiplied BGRA bitmap.
/// The WinRT random-access stream is bridged to a COM IStream with
/// `CreateStreamOverRandomAccessStream`, then decoded through WIC and
/// scaled to [`ICON_PX`]. Fails soft: any error yields `None`.
fn decode_media_thumbnail(
    props: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> windows::core::Result<Option<ThumbBitmap>> {
    use windows::Win32::Graphics::Imaging::{
        GUID_WICPixelFormat32bppBGRA, WICDecodeMetadataCacheOnDemand,
    };
    use windows::Win32::System::WinRT::CreateStreamOverRandomAccessStream;

    let stream_ref = props.Thumbnail()?;
    let stream = block_async(
        stream_ref.OpenReadAsync()?,
        |op| op.Status(),
        |op| op.GetResults(),
    )?;

    // Bridge WinRT stream -> COM IStream for WIC.
    let istream: windows::Win32::System::Com::IStream = unsafe {
        CreateStreamOverRandomAccessStream::<_, windows::Win32::System::Com::IStream>(&stream)?
    };

    let factory: windows::Win32::Graphics::Imaging::IWICImagingFactory = unsafe {
        windows::Win32::System::Com::CoCreateInstance(
            &windows::Win32::Graphics::Imaging::CLSID_WICImagingFactory,
            None,
            windows::Win32::System::Com::CLSCTX_INPROC_SERVER,
        )?
    };
    let decoder = unsafe {
        factory.CreateDecoderFromStream(
            &istream,
            std::ptr::null(),
            WICDecodeMetadataCacheOnDemand,
        )?
    };
    let frame = unsafe { decoder.GetFrame(0)? };

    let converter = unsafe { factory.CreateFormatConverter()? };
    unsafe {
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppBGRA,
            windows::Win32::Graphics::Imaging::WICBitmapDitherTypeNone,
            None,
            0.0,
            windows::Win32::Graphics::Imaging::WICBitmapPaletteTypeCustom,
        )?;
    }

    // Scale to the dashboard icon size through WIC (high-quality Fant filter).
    let scaler = unsafe { factory.CreateBitmapScaler()? };
    unsafe {
        scaler.Initialize(
            &converter,
            ICON_PX,
            ICON_PX,
            windows::Win32::Graphics::Imaging::WICBitmapInterpolationModeFant,
        )?;
    }

    let mut pixels = vec![0u8; (ICON_PX * ICON_PX * 4) as usize];
    unsafe {
        scaler.CopyPixels(std::ptr::null(), ICON_PX * 4, &mut pixels)?;
    }
    // WIC gives straight BGRA; force alpha opaque and premultiply for blit.
    for px in pixels.chunks_exact_mut(4) {
        px[0] = ((px[0] as u32 * 255) / 255) as u8;
        px[1] = ((px[1] as u32 * 255) / 255) as u8;
        px[2] = ((px[2] as u32 * 255) / 255) as u8;
        px[3] = 255;
    }

    Ok(Some(ThumbBitmap {
        width: ICON_PX,
        height: ICON_PX,
        pixels_pbgra: pixels,
    }))
}

#[derive(Clone)]
struct TaskWindow {
    hwnd: isize,
    title: String,
}

thread_local! {
    static ENUM_BUF: RefCell<Vec<TaskWindow>> = const { RefCell::new(Vec::new()) };
}

/// Enumerates up to `max_count` open application windows and captures their icons.
pub fn enumerate_tasks(max_count: usize) -> Vec<TaskIcon> {
    let windows = enumerate_windows();
    let mut icons = Vec::new();
    for win in windows {
        if icons.len() >= max_count {
            break;
        }
        if let Some(icon) = window_icon(win.hwnd, &win.title) {
            icons.push(icon);
        }
    }
    icons
}

fn enumerate_windows() -> Vec<TaskWindow> {
    ENUM_BUF.with(|cell| {
        cell.borrow_mut().clear();
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(0));
        }
        cell.borrow().clone()
    })
}

unsafe extern "system" fn enum_proc(hwnd: HWND, _lparam: LPARAM) -> windows::core::BOOL {
    let push = |task: TaskWindow| {
        ENUM_BUF.with(|cell| cell.borrow_mut().push(task));
    };
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return windows::core::BOOL(1);
    }
    // Skip cloaked windows (UWP background / virtual desktop hidden).
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
    // Skip tool windows.
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
        return windows::core::BOOL(1);
    }
    // Skip our own overlay window.
    let mut class = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut class) };
    if len > 0 && String::from_utf16_lossy(&class[..len as usize]) == "termielle_overlay" {
        return windows::core::BOOL(1);
    }
    // Must have a non-empty title.
    let mut title = [0u16; 128];
    let len = unsafe { GetWindowTextW(hwnd, &mut title) };
    if len <= 0 {
        return windows::core::BOOL(1);
    }
    let text = String::from_utf16_lossy(&title[..len as usize]);
    let text = text.trim();
    if text.is_empty() || text == "Program Manager" || text == "Windows Input Experience" {
        return windows::core::BOOL(1);
    }
    // Must be a real window with size.
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

/// Reads one window's app icon: per-window icon first, falling back to window-class icons.
pub fn window_icon(hwnd: isize, title: &str) -> Option<TaskIcon> {
    let h_wnd = HWND(hwnd as *mut core::ffi::c_void);
    let hicon = icon_handle(h_wnd)?;

    let screen = unsafe { GetDC(None) };
    if screen.is_invalid() {
        return None;
    }
    let memory = unsafe { CreateCompatibleDC(Some(screen)) };
    if memory.is_invalid() {
        let _ = unsafe { ReleaseDC(None, screen) };
        return None;
    }

    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: TASK_ICON_PX as i32,
            biHeight: -(TASK_ICON_PX as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
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
    let previous = unsafe { SelectObject(memory, HGDIOBJ(dib.0)) };

    let drawn = unsafe {
        DrawIconEx(
            memory,
            0,
            0,
            hicon,
            TASK_ICON_PX as i32,
            TASK_ICON_PX as i32,
            0,
            None,
            DI_NORMAL,
        )
        .is_ok()
    };

    let mut pixels = vec![0u8; (TASK_ICON_PX * TASK_ICON_PX * 4) as usize];
    if drawn && !bits.is_null() {
        let src = unsafe { std::slice::from_raw_parts(bits as *const u8, pixels.len()) };
        pixels.copy_from_slice(src);
    }

    unsafe {
        let _ = SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = DeleteDC(memory);
        let _ = ReleaseDC(None, screen);
    }

    if !drawn {
        return None;
    }
    // Check if any pixels are non-zero (avoid invisible icons)
    let has_content = pixels
        .chunks_exact(4)
        .any(|px| px[3] > 10 || (px[0] > 0 || px[1] > 0 || px[2] > 0));
    if !has_content {
        return None;
    }
    // Premultiply alpha
    for px in pixels.chunks_exact_mut(4) {
        if px[3] == 0 && (px[0] > 0 || px[1] > 0 || px[2] > 0) {
            px[3] = 255;
        } else {
            let a = px[3] as u32;
            px[0] = ((px[0] as u32 * a) / 255) as u8;
            px[1] = ((px[1] as u32 * a) / 255) as u8;
            px[2] = ((px[2] as u32 * a) / 255) as u8;
        }
    }

    Some(TaskIcon {
        hwnd,
        title: title.to_string(),
        width: TASK_ICON_PX,
        height: TASK_ICON_PX,
        pixels_pbgra: pixels,
    })
}

fn icon_handle(hwnd: HWND) -> Option<HICON> {
    // 1. Try WM_GETICON with ICON_BIG (hung-app safe: 50 ms timeout)
    let mut out: usize = 0;
    let flags = SEND_MESSAGE_TIMEOUT_FLAGS(SMTO_ABORTIFHUNG.0 | SMTO_NORMAL.0);
    let _ = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETICON,
            WPARAM(ICON_BIG as usize),
            LPARAM(0),
            flags,
            50,
            Some(&mut out as *mut usize),
        )
    };
    if out != 0 {
        return Some(HICON(out as *mut core::ffi::c_void));
    }
    // 2. Try WM_GETICON with ICON_SMALL2
    let _ = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETICON,
            WPARAM(ICON_SMALL2 as usize),
            LPARAM(0),
            flags,
            50,
            Some(&mut out as *mut usize),
        )
    };
    if out != 0 {
        return Some(HICON(out as *mut core::ffi::c_void));
    }
    // 3. Class icons
    for id in [GCLP_HICON, GCLP_HICONSM] {
        let raw = unsafe { GetClassLongPtrW(hwnd, id) };
        if raw != 0 {
            return Some(HICON(raw as *mut core::ffi::c_void));
        }
    }
    None
}

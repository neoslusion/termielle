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
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, HDC, HGDIOBJ, ReleaseDC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GCLP_HICON, GCLP_HICONSM, GWL_EXSTYLE, GetClassLongPtrW, GetClassNameW,
    GetIconInfo, GetWindowLongPtrW, GetWindowRect, GetWindowTextW, HICON, ICON_BIG, ICON_SMALL2,
    ICONINFO, IsIconic, IsWindowVisible, SEND_MESSAGE_TIMEOUT_FLAGS, SMTO_ABORTIFHUNG, SMTO_NORMAL,
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
        // Last pill rect the GUI thread published. Retained so a static pill
        // (no presents, no fresh requests) still gets a live backdrop
        // instead of a capture frozen from its last animation.
        let mut last_req: Option<BackdropRequest> = None;
        let mut last_req_ms: u64 = 0;
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
                // live desktop behind it rather than a stale snapshot. The
                // last published rect is retained: a static pill sends no
                // fresh requests, but its glass must not freeze.
                if let Some(req) = backdrop_request
                    .lock()
                    .ok()
                    .and_then(|mut guard| guard.take())
                {
                    last_req = Some(req);
                    last_req_ms = now_ms;
                } else if now_ms.saturating_sub(last_req_ms) > 5_000 {
                    // The GUI went quiet (idle hidden sensor): stop capturing
                    // until it presents again instead of burning laptop CPU.
                    last_req = None;
                }
                let mut backdrop = None;
                if let Some(req) = last_req {
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
/// Combines DrawIconEx image bits with the AND-mask bits into premultiplied
/// BGRA. `mask` is white where the icon is transparent; `mask_valid` tells
/// whether the mask pass succeeded. Classic XOR/AND icons leave image alpha
/// at 0 with the glyph composited on black — the mask (not a blind opaque
/// force) decides those pixels. Alpha-channel icons carry an empty mask, so
/// they keep per-pixel alpha. A failed mask pass leaves image alpha
/// untouched rather than inventing boxes.
fn apply_icon_mask(pixels: &mut [u8], mask: &[u8], mask_valid: bool) {
    for (px, m) in pixels.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
        if mask_valid {
            let luma = (u32::from(m[0]) + u32::from(m[1]) + u32::from(m[2])) / 3;
            if luma > 127 {
                px[0] = 0;
                px[1] = 0;
                px[2] = 0;
                px[3] = 0;
                continue;
            }
        }
        if px[3] == 0 && (px[0] > 0 || px[1] > 0 || px[2] > 0) {
            // Legacy XOR pixel under an opaque mask bit: solid color.
            px[3] = 255;
        } else {
            let a = u32::from(px[3]);
            px[0] = (u32::from(px[0]) * a / 255) as u8;
            px[1] = (u32::from(px[1]) * a / 255) as u8;
            px[2] = (u32::from(px[2]) * a / 255) as u8;
        }
    }
}

/// Reads one window's app icon: per-window icon first, falling back to window-class icons.
/// Decoded at native size with true per-pixel alpha (see [`decode_icon`]), then
/// filtered to [`TASK_ICON_PX`] on the way out.
pub fn window_icon(hwnd: isize, title: &str) -> Option<TaskIcon> {
    let h_wnd = HWND(hwnd as *mut core::ffi::c_void);
    let hicon = icon_handle(h_wnd)?;

    let (w, h, native) = decode_icon(hicon)?;
    let frame = crate::animation::FrameBuffer {
        width: w,
        height: h,
        pixels_pbgra: native,
        delay_ms: 0,
        loop_index: 0,
    };
    let scaled = if w == TASK_ICON_PX && h == TASK_ICON_PX {
        frame
    } else {
        crate::animation::resample_bilinear(&frame, TASK_ICON_PX, TASK_ICON_PX)
    };
    Some(TaskIcon {
        hwnd,
        title: title.to_string(),
        width: TASK_ICON_PX,
        height: TASK_ICON_PX,
        pixels_pbgra: scaled.pixels_pbgra,
    })
}

/// Native icon dimensions from a GDI bitmap handle.
fn bitmap_dims(hbm: HBITMAP) -> Option<(i32, i32)> {
    let mut bm = BITMAP::default();
    if unsafe {
        GetObjectW(
            HGDIOBJ(hbm.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some((&raw mut bm).cast()),
        )
    } == 0
    {
        return None;
    }
    (bm.bmWidth > 0 && bm.bmHeight > 0).then_some((bm.bmWidth, bm.bmHeight))
}

/// Reads `h` top-down rows of a GDI bitmap as bytes: 32bpp BGRA, or 1bpp
/// stride-padded rows with MSB first for masks.
fn dib_bits(hdc: HDC, hbm: HBITMAP, w: i32, h: i32, bpp: u16) -> Option<Vec<u8>> {
    let stride = if bpp == 1 {
        (w as usize).div_ceil(32) * 4
    } else {
        w as usize * 4
    };
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: bpp,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = vec![0u8; stride * h as usize];
    let lines = unsafe {
        GetDIBits(
            hdc,
            hbm,
            0,
            h as u32,
            Some(bits.as_mut_ptr() as *mut core::ffi::c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        )
    };
    if lines != h {
        return None;
    }
    Some(bits)
}

/// AND-mask bits (true = transparent) for a `w`x`h` icon mask.
fn and_mask(hdc: HDC, hbm: HBITMAP, w: i32, h: i32) -> Option<Vec<bool>> {
    let bits = dib_bits(hdc, hbm, w, h, 1)?;
    let stride = (w as usize).div_ceil(32) * 4;
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h as usize {
        for x in 0..w as usize {
            out.push((bits[y * stride + x / 8] >> (7 - (x % 8))) & 1 == 1);
        }
    }
    Some(out)
}

/// Extracts an HICON into premultiplied BGRA at native size: the color
/// bitmap's alpha channel is real for 32bpp icons, and the 1bpp AND mask
/// decides legacy XOR art. Returns `None` for undecodable or
fn decode_icon(hicon: HICON) -> Option<(u32, u32, Vec<u8>)> {
    let mut info = ICONINFO::default();
    if unsafe { GetIconInfo(hicon, &mut info) }.is_err() {
        return None;
    }
    let screen = unsafe { GetDC(None) };
    if screen.is_invalid() {
        free_icon_bitmaps(&info);
        return None;
    }
    let result = decode_icon_bits(screen, &info);
    unsafe {
        let _ = ReleaseDC(None, screen);
    }
    free_icon_bitmaps(&info);
    result
}

/// Frees the caller-owned bitmaps from [`GetIconInfo`].
fn free_icon_bitmaps(info: &ICONINFO) {
    unsafe {
        if !info.hbmColor.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        }
        if !info.hbmMask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
        }
    }
}

/// Core extraction once the device context and icon info are in hand.
fn decode_icon_bits(screen: HDC, info: &ICONINFO) -> Option<(u32, u32, Vec<u8>)> {
    if info.hbmColor.is_invalid() {
        return decode_monochrome_icon(screen, info);
    }
    let (w, h) = bitmap_dims(info.hbmColor)?;
    if w <= 0 || h <= 0 || w > 256 || h > 256 {
        return None;
    }
    let mut pixels = dib_bits(screen, info.hbmColor, w, h, 32)?;
    if pixels.chunks_exact(4).any(|px| px[3] != 0) {
        // True alpha channel (opaque art included): premultiply in place.
        for px in pixels.chunks_exact_mut(4) {
            let a = u32::from(px[3]);
            px[0] = (u32::from(px[0]) * a / 255) as u8;
            px[1] = (u32::from(px[1]) * a / 255) as u8;
            px[2] = (u32::from(px[2]) * a / 255) as u8;
        }
    } else {
        // Legacy XOR art: the AND mask (same dims for color icons) decides.
        // A mismatched mask would shear the image, so bail instead.
        let (mw, mh) = bitmap_dims(info.hbmMask)?;
        if (mw, mh) != (w, h) {
            return None;
        }
        let mask = and_mask(screen, info.hbmMask, w, h)?;
        let mut mask32 = vec![0u8; pixels.len()];
        for (slot, transparent) in mask32.chunks_exact_mut(4).zip(mask.iter()) {
            let v = u8::from(*transparent) * 255;
            slot.copy_from_slice(&[v, v, v, 255]);
        }
        apply_icon_mask(&mut pixels, &mask32, true);
    }
    if !pixels.chunks_exact(4).any(|px| px[3] > 0) {
        return None;
    }
    Some((w as u32, h as u32, pixels))
}

/// Monochrome fallback: no color bitmap, so the mask stacks the AND mask
/// over the XOR image, each `h` tall. White XOR pixels light up.
fn decode_monochrome_icon(screen: HDC, info: &ICONINFO) -> Option<(u32, u32, Vec<u8>)> {
    let (w, mh) = bitmap_dims(info.hbmMask)?;
    if w <= 0 || mh <= 0 || w > 256 || mh > 512 || mh % 2 != 0 {
        return None;
    }
    let h = mh / 2;
    let bits = dib_bits(screen, info.hbmMask, w, mh, 1)?;
    let stride = (w as usize).div_ceil(32) * 4;
    let bit = |x: usize, y: usize| (bits[y * stride + x / 8] >> (7 - (x % 8))) & 1 == 1;
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let mut visible = false;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let o = (y * w as usize + x) * 4;
            if bit(x, y) {
                // AND set: transparent.
                pixels[o..o + 4].copy_from_slice(&[0, 0, 0, 0]);
            } else {
                visible = true;
                let v = if bit(x, y + h as usize) { 255 } else { 0 };
                pixels[o..o + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
    }
    visible.then_some((w as u32, h as u32, pixels))
}

/// Best-available icon handle for a window: per-window BIG/SMALL plus both
/// class icons, largest native art wins. A 16px per-window icon must never
/// upscale-soften a tile when a bigger class icon exists; ties keep
/// discovery order (per-window first). Handles are owned by their window or
/// class and must not be freed.
fn icon_handle(hwnd: HWND) -> Option<HICON> {
    let flags = SEND_MESSAGE_TIMEOUT_FLAGS(SMTO_ABORTIFHUNG.0 | SMTO_NORMAL.0);
    let query = |which: usize| {
        let mut out: usize = 0;
        let _ = unsafe {
            SendMessageTimeoutW(
                hwnd,
                WM_GETICON,
                WPARAM(which),
                LPARAM(0),
                flags,
                50,
                Some(&mut out as *mut usize),
            )
        };
        out
    };
    let mut candidates = vec![
        query(ICON_BIG as usize),
        query(ICON_SMALL2 as usize),
        unsafe { GetClassLongPtrW(hwnd, GCLP_HICON) as usize },
        unsafe { GetClassLongPtrW(hwnd, GCLP_HICONSM) as usize },
    ];
    candidates.retain(|h| *h != 0);
    candidates.dedup();
    let mut best: Option<(HICON, u64)> = None;
    for raw in &candidates {
        let hicon = HICON(*raw as *mut core::ffi::c_void);
        let area = icon_area(hicon);
        if area > best.map(|(_, b)| b).unwrap_or(0) {
            best = Some((hicon, area));
        }
    }
    // Fall back to first-available when nothing reports a size: decode may
    // still succeed, and a missing tile is worse than a soft one.
    best.map(|(h, _)| h).or_else(|| {
        candidates
            .first()
            .map(|raw| HICON(*raw as *mut core::ffi::c_void))
    })
}

/// Native pixel area of an icon for candidate ranking. Reads headers only,
/// never pixels; 0 when the icon cannot even be measured.
fn icon_area(hicon: HICON) -> u64 {
    let mut info = ICONINFO::default();
    if unsafe { GetIconInfo(hicon, &mut info) }.is_err() {
        return 0;
    }
    let area = if info.hbmColor.is_invalid() {
        bitmap_dims(info.hbmMask).map(|(w, h)| (w as u64) * (h as u64) / 2)
    } else {
        bitmap_dims(info.hbmColor).map(|(w, h)| (w as u64) * (h as u64))
    }
    .unwrap_or(0);
    free_icon_bitmaps(&info);
    area
}

#[cfg(test)]
mod icon_mask_tests {
    use super::apply_icon_mask;

    #[test]
    fn masked_background_goes_transparent() {
        // Legacy icon: red glyph on black, mask white around it.
        let mut pixels = vec![200, 0, 0, 0, 0, 0, 0, 0];
        let mask = vec![0, 0, 0, 255, 255, 255, 255, 255];
        apply_icon_mask(&mut pixels, &mask, true);
        assert_eq!(pixels, vec![200, 0, 0, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn alpha_pixels_premultiply_under_empty_mask() {
        let mut pixels = vec![200, 100, 50, 128];
        let mask = vec![0, 0, 0, 255];
        apply_icon_mask(&mut pixels, &mask, true);
        assert_eq!(pixels, vec![100, 50, 25, 128]);
    }

    #[test]
    fn failed_mask_keeps_legacy_force_opaque_rule() {
        let mut pixels = vec![200, 0, 0, 0, 0, 0, 0, 0];
        let mask = vec![0; 8];
        apply_icon_mask(&mut pixels, &mask, false);
        assert_eq!(pixels[3], 255);
        assert_eq!(pixels[7], 0);
    }
}

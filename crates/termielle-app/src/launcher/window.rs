use super::{
    anchor::{Anchor, Placement, place, rest_edge, reveal_radius, reveal_rect},
    catalog, files,
    model::{Model, Target},
    paint,
};
use crate::animation::motion::{LAUNCHER_COLLAPSE_MS, LAUNCHER_REVEAL_MS, SELECTION_GLIDE_MS};
use crate::animation::transition::Transition;
use crate::win32_ptr::{
    set_window_long_ptr as SetWindowLongPtrW, window_long_ptr as GetWindowLongPtrW,
};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::mem::size_of;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};
use termielle_core::{GlassConfig, IslandConfig};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DEFAULT_CHARSET, DIB_RGB_COLORS,
    DeleteObject, EndPaint, GetMonitorInfoW, HBRUSH, HDC, HFONT, InvalidateRect,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, MonitorFromWindow, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, SetBkColor, SetDIBitsToDevice, SetTextColor, SetWindowRgn,
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
    GetWindowTextW, IDC_ARROW, LWA_ALPHA, LoadCursorW, PostMessageW, RegisterClassW, SW_HIDE,
    SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW,
    SetForegroundWindow, SetLayeredWindowAttributes, SetWindowPos, SetWindowTextW, ShowWindow,
    WA_INACTIVE, WINDOW_STYLE, WM_ACTIVATE, WM_APP, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT,
    WM_DPICHANGED, WM_ERASEBKGND, WM_HOTKEY, WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION,
    WM_KEYDOWN, WM_LBUTTONUP, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_SETFONT, WNDCLASSW, WS_CHILD,
    WS_CLIPCHILDREN, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::w;

const UPDATE_MSG: u32 = WM_APP + 101;
const OPEN_MSG: u32 = WM_APP + 102;
const HOTKEY_ID: i32 = 101;
/// The launcher's motion wake. A dedicated clock thread posts this on the
/// display's vertical blank; a `WM_TIMER` would be delivered through the
/// message queue and dropped while the queue is busy — which is exactly
/// when the launcher is working hardest, with the user typing into it.
const MOTION_MSG: u32 = WM_APP + 103;
/// Space between the pill and the launcher that grows out of it, in logical pixels.
const DOCK_GAP_PX: f32 = 6.0;
/// How thick the reveal starts: a strip against the pill, in logical pixels.
const REVEAL_STRIP_PX: f32 = 12.0;
/// The corner radius Windows 11 rounds a resting window with, in logical pixels.
const RESTING_RADIUS_PX: f32 = 8.0;
static CLASS: OnceLock<Result<u16, u32>> = OnceLock::new();

enum Update {
    Catalog(windows::core::Result<Vec<super::model::App>>),
    Icons(Vec<(Vec<u8>, Option<Arc<crate::animation::FrameBuffer>>)>),
    Launched(u64, windows::core::Result<()>),
    Documents(files::Reply),
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
    palette: Cell<crate::theme::Palette>,
    model: RefCell<Model>,
    receiver: RefCell<mpsc::Receiver<Update>>,
    notifier: Notifier,
    refresh: mpsc::Sender<Command>,
    pending_icons: Arc<Mutex<Vec<Vec<u8>>>>,
    icons_done: RefCell<HashSet<Vec<u8>>>,
    last_refresh: Cell<Instant>,
    file_search: mpsc::Sender<files::Command>,
    file_generation: Cell<u64>,
    clock: Instant,
    reduced_motion: Cell<bool>,
    selection: Cell<Transition>,
    /// The launcher growing out of the pill, 0 (a strip against it) to 1 (the
    /// whole window). Always 1 when there is no pill or motion is reduced.
    reveal: Cell<Transition>,
    /// The pill the runtime last reported, in screen pixels.
    anchor: Cell<Option<Anchor>>,
    /// The anchor this opening is docked to. It is fixed at show so the window
    /// does not follow a pill that changes width while it is open.
    docked: Cell<Option<Anchor>>,
    /// The work area of the monitor the launcher opened on: left, top, right, bottom.
    work: Cell<(i32, i32, i32, i32)>,
    /// The edge the launcher settles on (see `anchor::rest_edge`), fixed when
    /// it opens so typing, which changes its height, does not move it.
    rest: Cell<i32>,
    /// The launcher is collapsing back into the pill and will hide when done.
    closing: Cell<bool>,
    /// Whether focus goes back to the window that had it once it has hidden.
    restore_after_exit: Cell<bool>,
    /// Wakes the overlay's loop when the launcher opens or closes, so the bar
    /// can step aside for it and take its notifications back.
    wake: RefCell<Option<crate::window::WakeHandle>>,
    /// The display-synced clock that drives the reveal and the selection
    /// highlight. `None` until the window handle exists.
    motion_clock: RefCell<Option<crate::window::AnimationClock>>,
    motion_armed: Cell<bool>,
}

impl State {
    fn now_ms(&self) -> u64 {
        self.clock.elapsed().as_millis().min(u64::MAX as u128) as u64
    }

    /// Arms or parks the motion clock.
    ///
    /// The clock waits for the display's vertical blank and then posts
    /// [`MOTION_MSG`], so a frame lands on the refresh instead of whenever
    /// the message queue happens to drain. While the motion runs, each tick
    /// re-arms the clock for the next refresh; when it ends, the clock is
    /// parked.
    fn sync_motion(&self) {
        let now = self.now_ms();
        let active = self.visible.get()
            && (self.selection.get().active(now) || self.reveal.get().active(now));
        if active {
            self.motion_armed.set(true);
            if let Some(clock) = self.motion_clock.borrow().as_ref() {
                clock.arm_after(0);
            }
        } else if self.motion_armed.replace(false) {
            if let Some(clock) = self.motion_clock.borrow().as_ref() {
                clock.arm(None);
            }
        }
    }

    /// Where the window goes for the current results when it is docked to the
    /// pill, with the window's width and height. `None` means there is no pill
    /// to grow from and the launcher sits centred as it always did.
    fn current_placement(&self) -> Option<(Placement, i32, i32)> {
        let anchor = self.docked.get()?;
        let scale = self.scale.get();
        let width = (paint::WIDTH as f32 * scale).round() as i32;
        let height = (paint::height(&self.model.borrow()) as f32 * scale).round() as i32;
        let gap = (DOCK_GAP_PX * scale).round() as i32;
        let strip = (REVEAL_STRIP_PX * scale).round() as i32;
        let placement = place(
            anchor,
            width,
            height,
            self.work.get(),
            gap,
            strip,
            self.rest.get(),
        );
        Some((placement, width, height))
    }

    /// Clips the window to the reveal's shape at `now_ms`, or removes the clip
    /// once the launcher has grown to its full size so Windows rounds and
    /// shadows it as it does every other window.
    fn apply_reveal(&self, now_ms: u64) {
        let progress = self.reveal.get().value(now_ms).clamp(0.0, 1.0);
        let placement = self.current_placement();
        if let Some((placement, ..)) = &placement {
            // The window slides from the pill to where it rests as it grows.
            self.move_window(placement, progress);
        }
        let growing = placement.filter(|_| progress < 1.0);
        let Some((placement, width, height)) = growing else {
            let _ = unsafe { SetWindowRgn(self.hwnd.get(), None, true) };
            return;
        };
        let scale = self.scale.get();
        let (left, top, right, bottom) =
            reveal_rect(placement.start, (0, 0, width, height), progress);
        let radius = reveal_radius(
            (REVEAL_STRIP_PX * scale).round() as i32,
            (RESTING_RADIUS_PX * scale).round() as i32,
            progress,
        );
        // The system owns the region once it is set.
        let region =
            unsafe { CreateRoundRectRgn(left, top, right, bottom, radius * 2, radius * 2) };
        let _ = unsafe { SetWindowRgn(self.hwnd.get(), Some(region), true) };
    }

    fn move_window(&self, placement: &Placement, progress: f32) {
        let _ = unsafe {
            SetWindowPos(
                self.hwnd.get(),
                None,
                placement.x,
                placement.y_at(progress),
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
    }

    fn sync_selection(&self, animate: bool, force: bool) {
        let target = self.model.borrow().selected as f32 * paint::ROW_HEIGHT as f32;
        let mut selection = self.selection.get();
        if animate && !self.reduced_motion.get() {
            selection.retarget(target, self.now_ms(), SELECTION_GLIDE_MS);
        } else if force || selection.target != target {
            selection.snap(target);
        }
        self.selection.set(selection);
        self.sync_motion();
    }

    fn tick(&self, now_ms: u64) {
        if self.closing.get() && !self.reveal.get().active(now_ms) {
            // The collapse has reached the pill: now the window can go.
            self.finish_hide(self.restore_after_exit.replace(false));
            return;
        }
        let repaint_selection = !self.selection.get().is_constant();
        if !self.reveal.get().is_constant() {
            let mut reveal = self.reveal.get();
            if !reveal.active(now_ms) {
                reveal.snap(reveal.target);
                self.reveal.set(reveal);
            }
            self.apply_reveal(now_ms);
        }
        let mut selection = self.selection.get();
        if !selection.active(now_ms) {
            selection.snap(selection.target);
            self.selection.set(selection);
        }
        if repaint_selection {
            let _ = unsafe { InvalidateRect(Some(self.hwnd.get()), None, false) };
        }
        self.sync_motion();
    }
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
                    .any(|index| model.apps[*index].shell_target() == Some(target.as_slice()))
            });
            model
                .results
                .iter()
                .map(|index| &model.apps[*index])
                .filter_map(|app| app.shell_target())
                .filter(|target| !done.contains(*target))
                .map(|target| target.to_vec())
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
        // Docked above a bottom bar the window's bottom edge is the fixed one,
        // so a change in height has to move its top.
        let progress = self.reveal.get().value(self.now_ms()).clamp(0.0, 1.0);
        let docked = self
            .current_placement()
            .map(|(placement, ..)| (placement.x, placement.y_at(progress)));
        let _ = unsafe {
            SetWindowPos(
                self.hwnd.get(),
                None,
                docked.map_or(0, |at| at.0),
                docked.map_or(0, |at| at.1),
                width,
                height,
                if docked.is_some() {
                    SWP_NOZORDER | SWP_NOACTIVATE
                } else {
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE
                },
            )
        };
        let _ = unsafe { InvalidateRect(Some(self.hwnd.get()), None, false) };
    }

    fn request_documents(&self) {
        let generation = self.file_generation.get().wrapping_add(1);
        self.file_generation.set(generation);
        let _ = self.file_search.send(files::Command::Search(
            generation,
            self.model.borrow().query.clone(),
        ));
    }

    fn hide(&self, restore_focus: bool) {
        if !self.visible.get() {
            return;
        }
        self.pending_icons.lock().unwrap().clear();
        self.file_generation
            .set(self.file_generation.get().wrapping_add(1));
        // However it is put away (Escape, Alt+Space, losing focus, launching
        // something) it collapses back into the pill, the reverse of the way it
        // opened, from wherever the opening had got to. The collapse is only a
        // picture: nothing waits on it, because focus has already moved on
        // (`restore_focus` says whether it goes back to the previous window once
        // the collapse is done). A second request while it collapses finishes it.
        if !self.closing.get() && self.docked.get().is_some() && !self.reduced_motion.get() {
            self.closing.set(true);
            self.restore_after_exit.set(restore_focus);
            let mut reveal = self.reveal.get();
            reveal.retarget(0.0, self.now_ms(), LAUNCHER_COLLAPSE_MS);
            self.reveal.set(reveal);
            self.sync_motion();
            return;
        }
        self.finish_hide(restore_focus);
    }

    /// Lets the overlay's loop know the launcher's visibility changed.
    fn wake_overlay(&self) {
        if let Some(wake) = self.wake.borrow().as_ref() {
            let _ = wake.post();
        }
    }

    fn finish_hide(&self, restore_focus: bool) {
        self.visible.set(false);
        self.closing.set(false);
        self.restore_after_exit.set(false);
        self.wake_overlay();
        let mut reveal = self.reveal.get();
        reveal.snap(1.0);
        self.reveal.set(reveal);
        self.docked.set(None);
        self.apply_reveal(self.now_ms());
        let mut selection = self.selection.get();
        selection.snap(selection.target);
        self.selection.set(selection);
        self.sync_motion();
        let foreground = unsafe { GetForegroundWindow() } == self.hwnd.get();
        let _ = unsafe { ShowWindow(self.hwnd.get(), SW_HIDE) };
        if restore_focus && foreground {
            let previous = self.previous_window.get();
            if !previous.is_invalid() {
                let _ = unsafe { SetForegroundWindow(previous) };
            }
        }
    }

    fn show(&self) {
        // Reopening while it is collapsing grows it back from where it is.
        let was_closing = self.closing.replace(false);
        self.restore_after_exit.set(false);
        let already_open = self.visible.get() && !was_closing;
        let foreground = unsafe { GetForegroundWindow() };
        if foreground != self.hwnd.get() {
            self.previous_window.set(foreground);
        }
        self.generation.set(self.generation.get().wrapping_add(1));
        {
            let mut model = self.model.borrow_mut();
            model.launching = false;
            model.set_query(String::new());
        }
        let _ = unsafe { SetWindowTextW(self.edit.get(), w!("")) };
        // Docked to the pill when the runtime has one to report: the launcher
        // opens on the pill's monitor, against the pill, and grows out of it.
        self.docked.set(self.anchor.get());
        let monitor = match self.docked.get() {
            Some(anchor) => unsafe {
                MonitorFromPoint(
                    POINT {
                        x: (anchor.pill.0 + anchor.pill.2) / 2,
                        y: (anchor.pill.1 + anchor.pill.3) / 2,
                    },
                    MONITOR_DEFAULTTONEAREST,
                )
            },
            None => unsafe {
                MonitorFromWindow(self.previous_window.get(), MONITOR_DEFAULTTONEAREST)
            },
        };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            let width = (paint::WIDTH as f32 * self.scale.get()).round() as i32;
            let height =
                (paint::height(&self.model.borrow()) as f32 * self.scale.get()).round() as i32;
            let work = info.rcWork;
            self.work
                .set((work.left, work.top, work.right, work.bottom));
            if let Some(anchor) = self.docked.get() {
                let scale = self.scale.get();
                self.rest.set(rest_edge(
                    anchor,
                    height,
                    self.work.get(),
                    (DOCK_GAP_PX * scale).round() as i32,
                ));
            }
            let (x, y) = match self.current_placement() {
                Some((placement, ..)) => (placement.x, placement.y_start),
                None => (
                    work.left + (work.right - work.left - width) / 2,
                    work.top + (work.bottom - work.top - height).max(0) / 4,
                ),
            };
            let _ = unsafe {
                SetWindowPos(
                    self.hwnd.get(),
                    None,
                    x,
                    y,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
        } else {
            // Without a work area there is nowhere to dock.
            self.docked.set(None);
        }
        // A second request while it is up leaves it as it is. Otherwise it
        // starts as the strip against the pill (or from wherever a collapse
        // had got to), or is simply whole when there is nothing to grow from.
        let grows = !already_open && self.docked.get().is_some() && !self.reduced_motion.get();
        let mut reveal = self.reveal.get();
        if grows {
            if !was_closing {
                reveal.snap(0.0);
            }
        } else if !already_open {
            reveal.snap(1.0);
        }
        self.reveal.set(reveal);
        // The clip is on before the window is shown, so the first frame is the
        // strip and not a flash of the whole launcher.
        self.apply_reveal(self.now_ms());
        self.sync_selection(false, true);
        self.visible.set(true);
        let _ = unsafe { ShowWindow(self.hwnd.get(), SW_SHOW) };
        let _ = unsafe { SetForegroundWindow(self.hwnd.get()) };
        let _ = unsafe { SetFocus(Some(self.edit.get())) };
        if self.last_refresh.get().elapsed() >= Duration::from_secs(60) {
            self.last_refresh.set(Instant::now());
            let _ = self.refresh.send(Command::Refresh);
        }
        self.request_icons();
        self.request_documents();
        self.redraw();
        if grows {
            // The clock starts now, once the window is up and focused, not
            // before the slow part of showing it: the opening is the 160 ms the
            // user sees, not whatever was left after the window manager.
            let now = self.now_ms();
            let mut reveal = self.reveal.get();
            reveal.retarget(1.0, now, LAUNCHER_REVEAL_MS);
            self.reveal.set(reveal);
            self.apply_reveal(now);
        }
        self.sync_motion();
        self.wake_overlay();
    }

    fn open_selected(&self) {
        if !self.visible.get() {
            return;
        }
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
        let file_search = self.file_search.clone();
        std::thread::spawn(move || {
            let result = crate::apartment::Apartment::new(RO_INIT_SINGLETHREADED)
                .and_then(|_apartment| catalog::launch(&target));
            if result.is_ok() {
                if let Target::Document(path) = target {
                    let _ = file_search.send(files::Command::Opened(path));
                }
            }
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
                        if let Some(index) = model.results.iter().copied().find(|index| {
                            model.apps[*index].shell_target() == Some(target.as_slice())
                        }) {
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
                                Some(format!("Could not open item ({:08X})", error.code().0))
                        }
                    }
                }
                Update::Launched(_, _) => {}
                Update::Documents(reply) => {
                    if self.visible.get() && reply.generation == self.file_generation.get() {
                        self.model
                            .borrow_mut()
                            .set_documents(reply.entries, reply.status);
                    }
                }
            }
        }
        self.request_icons();
        self.sync_selection(false, false);
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
        let dark: u32 = u32::from(self.palette.get().primary[0] > 128);
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
        let frame = paint::render_with_selection(
            &self.model.borrow(),
            &self.glass.borrow(),
            self.scale.get(),
            self.hotkey.get(),
            self.selection.get().value(self.now_ms()),
            self.palette.get(),
        );
        let info = BITMAPINFO {
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
                    &info,
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
    pub fn create(island: &IslandConfig, reduced_motion: bool) -> windows::core::Result<Self> {
        let launcher = Self::create_window(island, true)?;
        launcher.state.reduced_motion.set(reduced_motion);
        Ok(launcher)
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
        let (file_search, file_receiver) = mpsc::channel();
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
            palette: Cell::new(crate::theme::Palette::for_island(island)),
            model: RefCell::new(Model::default()),
            receiver: RefCell::new(receiver),
            notifier: Notifier { hwnd: 0, sender },
            refresh,
            pending_icons: Arc::new(Mutex::new(Vec::new())),
            icons_done: RefCell::new(HashSet::new()),
            last_refresh: Cell::new(Instant::now()),
            file_search,
            file_generation: Cell::new(0),
            clock: Instant::now(),
            reduced_motion: Cell::new(false),
            selection: Cell::new(Transition::default()),
            reveal: Cell::new(Transition::default()),
            anchor: Cell::new(None),
            docked: Cell::new(None),
            work: Cell::new((0, 0, 0, 0)),
            rest: Cell::new(0),
            closing: Cell::new(false),
            restore_after_exit: Cell::new(false),
            wake: RefCell::new(None),
            motion_clock: RefCell::new(None),
            motion_armed: Cell::new(false),
        });
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED,
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
        // A layered window paints nothing until its attributes are set. The
        // launcher is always fully opaque: it appears and disappears instantly.
        unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA) }?;
        let mut launcher = Self { state };
        launcher.state.notifier.hwnd = hwnd.0 as isize;
        // The motion clock drives the selection highlight off the display's
        // vertical blank. The launcher needs its own clock
        // because it lives in its own window with its own message queue.
        *launcher.state.motion_clock.borrow_mut() =
            Some(crate::window::AnimationClock::spawn_with(
                crate::window::WakeHandle::for_window(hwnd),
                MOTION_MSG,
            ));
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
                Some(LPARAM(w!("Search apps and documents…").as_ptr() as isize)),
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
        if register_hotkey {
            let notifier = launcher.state.notifier.clone();
            std::thread::spawn(move || {
                files::run(file_receiver, |reply| {
                    notifier.send(Update::Documents(reply))
                })
            });
        }
        Ok(launcher)
    }

    pub fn show(&self) {
        self.state.show();
    }

    /// Reports the bar's pill, in screen pixels, for the next opening to grow
    /// out of. `None` (no bar, or it is hidden) opens the launcher centred.
    pub fn set_anchor(&self, anchor: Option<Anchor>) {
        self.state.anchor.set(anchor);
    }

    /// The overlay's wake handle, so opening and closing the launcher reaches
    /// the bar's loop at once instead of on its next event.
    pub fn set_wake(&self, wake: crate::window::WakeHandle) {
        *self.state.wake.borrow_mut() = Some(wake);
    }

    pub fn is_visible(&self) -> bool {
        self.state.visible.get()
    }

    pub fn configure(&self, island: &IslandConfig, reduced_motion: bool) {
        if self.state.reduced_motion.replace(reduced_motion) != reduced_motion && reduced_motion {
            let mut reveal = self.state.reveal.get();
            reveal.snap(1.0);
            self.state.reveal.set(reveal);
            self.state.apply_reveal(self.state.now_ms());
            self.state.sync_selection(false, true);
            if self.state.visible.get() {
                self.state.redraw();
            }
        }
        let palette = crate::theme::Palette::for_island(island);
        if *self.state.glass.borrow() != island.glass || self.state.palette.get() != palette {
            self.state.palette.set(palette);
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
            if state.visible.get() && !state.closing.get() {
                state.hide(true);
            } else {
                state.show();
            }
        }
        WM_ACTIVATE if wparam.0 as u32 & 0xffff == WA_INACTIVE => state.hide(false),
        WM_CLOSE => state.hide(true),
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => state.paint(),
        MOTION_MSG => state.tick(state.now_ms()),
        WM_COMMAND if (wparam.0 >> 16) as u32 == EN_CHANGE => {
            let mut text =
                vec![0; unsafe { GetWindowTextLengthW(state.edit.get()) }.max(0) as usize + 1];
            let length = unsafe { GetWindowTextW(state.edit.get(), &mut text) }.max(0) as usize;
            state
                .model
                .borrow_mut()
                .set_query(String::from_utf16_lossy(&text[..length]));
            state.request_icons();
            state.request_documents();
            state.sync_selection(false, true);
            state.redraw();
        }
        WM_CTLCOLOREDIT => {
            let dc = HDC(wparam.0 as _);
            let glass = state.glass.borrow();
            let primary = state.palette.get().primary;
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
                    state.sync_selection(true, false);
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

    #[test]
    fn theme_palette_updates_even_when_glass_overrides_are_identical() {
        let mut island = IslandConfig::default();
        let launcher = LauncherWindow::create_window(&island, false).unwrap();
        let original = launcher.state.palette.get();
        island.theme = "macos-dark".into();
        launcher.configure(&island, false);
        assert_eq!(*launcher.state.glass.borrow(), island.glass);
        assert_ne!(launcher.state.palette.get(), original);
        assert_eq!(launcher.state.palette.get().accent, [255, 132, 10, 255]);
        island.theme = "light".into();
        launcher.configure(&island, false);
        assert_eq!(launcher.state.palette.get(), original);
    }

    #[test]
    fn selection_glides_but_typing_snaps_and_parks_the_timer() {
        let launcher = LauncherWindow::create_window(&IslandConfig::default(), false).unwrap();
        let state = &launcher.state;
        state.visible.set(true);
        state.model.borrow_mut().set_apps(vec![
            App::application("Alpha".into(), vec![0, 0]),
            App::application("Beta".into(), vec![1, 0, 0]),
        ]);
        unsafe {
            SendMessageW(
                state.edit.get(),
                WM_KEYDOWN,
                Some(WPARAM(VK_DOWN.0 as usize)),
                Some(LPARAM(0)),
            )
        };
        assert_eq!(state.model.borrow().selected_app().unwrap().name, "Beta");
        assert_eq!(state.selection.get().target, paint::ROW_HEIGHT as f32);
        assert!(state.selection.get().active(state.now_ms()));
        assert!(state.motion_armed.get());
        unsafe { SetWindowTextW(state.edit.get(), w!("be")) }.unwrap();
        assert_eq!(state.model.borrow().selected_app().unwrap().name, "Beta");
        assert_eq!(state.selection.get().value(state.now_ms()), 0.0);
        assert!(!state.motion_armed.get());
        state.hide(false);
    }

    #[test]
    fn the_launcher_appears_and_disappears_instantly_and_parks_its_motion_clock() {
        let island = IslandConfig::default();
        let launcher = LauncherWindow::create_window(&island, false).unwrap();
        let state = &launcher.state;
        state.model.borrow_mut().set_apps(vec![
            App::application("Alpha".into(), vec![0, 0]),
            App::application("Beta".into(), vec![1, 0, 0]),
        ]);
        state.show();
        assert!(state.visible.get());
        assert!(!state.motion_armed.get());
        unsafe {
            SendMessageW(
                state.edit.get(),
                WM_KEYDOWN,
                Some(WPARAM(VK_DOWN.0 as usize)),
                Some(LPARAM(0)),
            )
        };
        assert!(state.motion_armed.get());
        let settled = state.now_ms() + SELECTION_GLIDE_MS + 1;
        state.tick(settled);
        assert_eq!(
            state.selection.get().value(settled),
            paint::ROW_HEIGHT as f32
        );
        assert!(!state.motion_armed.get());
        state.hide(true);
        assert!(!state.visible.get());
        assert!(!state.motion_armed.get());
        launcher.configure(&island, true);
        assert!(!state.motion_armed.get());
    }

    #[test]
    fn the_launcher_grows_out_of_the_pill_settles_lower_and_collapses_back_into_it() {
        use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let launcher = LauncherWindow::create_window(&IslandConfig::default(), false).unwrap();
        let state = &launcher.state;
        state.model.borrow_mut().set_apps(vec![
            App::application("Alpha".into(), vec![0, 0]),
            App::application("Beta".into(), vec![1, 0, 0]),
        ]);
        let top_of = |state: &State| {
            let mut rect = RECT::default();
            unsafe { GetWindowRect(state.hwnd.get(), &mut rect) }.unwrap();
            rect.top
        };
        launcher.set_anchor(Some(Anchor {
            pill: (900, 8, 1100, 40),
            below: true,
        }));
        state.show();

        let gap = (DOCK_GAP_PX * state.scale.get()).round() as i32;
        assert_eq!(top_of(state), 40 + gap, "it starts just under the pill");
        assert!(launcher.is_visible());
        assert!(state.reveal.get().active(state.now_ms()), "it grows out");
        assert!(state.motion_armed.get(), "the display clock is running");

        let grown = state.now_ms() + LAUNCHER_REVEAL_MS + 1;
        state.tick(grown);
        assert_eq!(state.reveal.get().value(grown), 1.0);
        assert!(
            !state.motion_armed.get(),
            "the clock parks when it has grown"
        );
        assert!(
            top_of(state) > 40 + gap,
            "it settles lower than the pill, where it always used to open"
        );

        // Escape collapses it back into the pill instead of hiding it at once.
        state.hide(true);
        assert!(launcher.is_visible(), "still on screen while it collapses");
        assert!(state.closing.get());
        assert!(state.reveal.get().active(state.now_ms()));
        assert!(state.motion_armed.get());
        let collapsed = state.now_ms() + LAUNCHER_COLLAPSE_MS + 1;
        state.tick(collapsed);
        assert!(
            !launcher.is_visible(),
            "hidden once it has reached the pill"
        );
        assert!(!state.closing.get());
        assert!(state.docked.get().is_none(), "the next opening re-docks");
        assert!(!state.motion_armed.get());

        // A second Escape while it collapses finishes it at once.
        state.show();
        state.hide(true);
        assert!(state.closing.get());
        state.hide(true);
        assert!(!launcher.is_visible());

        // Opening again while it is collapsing grows it back from where it is.
        state.show();
        state.hide(true);
        assert!(state.closing.get());
        state.show();
        assert!(!state.closing.get(), "reopened");
        assert_eq!(state.reveal.get().target, 1.0);
        assert!(launcher.is_visible());
        // Losing focus (or launching) collapses it too, but gives focus back to
        // nobody when it is done.
        state.hide(false);
        assert!(state.closing.get());
        assert!(!state.restore_after_exit.get());
        state.tick(state.now_ms() + LAUNCHER_COLLAPSE_MS + 1);
        assert!(!launcher.is_visible());

        // No pill to grow from: today's centred, instant window, with no collapse.
        launcher.set_anchor(None);
        state.show();
        assert!(state.docked.get().is_none());
        assert!(!state.reveal.get().active(state.now_ms()));
        assert!(!state.motion_armed.get());
        state.hide(true);
        assert!(!launcher.is_visible(), "no pill, no collapse");
    }

    #[test]
    fn document_updates_ignore_stale_queries_and_never_request_shell_icons() {
        let launcher = LauncherWindow::create_window(&IslandConfig::default(), false).unwrap();
        let state = &launcher.state;
        state.visible.set(true);
        state.file_generation.set(2);
        state.model.borrow_mut().set_query("report".into());
        let reply = |generation| {
            Update::Documents(files::Reply {
                generation,
                status: "1 document".into(),
                entries: vec![App {
                    name: "Report.pdf".into(),
                    key: "c:\\docs\\report.pdf".into(),
                    target: Target::Document(std::path::PathBuf::from("C:/Docs/Report.pdf")),
                    icon: None,
                    file_rank: Some(10),
                }],
            })
        };
        state.notifier.sender.send(reply(1)).unwrap();
        state.drain_updates();
        assert!(state.model.borrow().results.is_empty());
        state.notifier.sender.send(reply(2)).unwrap();
        state.drain_updates();
        assert_eq!(
            state.model.borrow().selected_app().unwrap().name,
            "Report.pdf"
        );
        assert!(state.pending_icons.lock().unwrap().is_empty());
        state.hide(false);
        state.notifier.sender.send(reply(2)).unwrap();
        state.drain_updates();
        assert!(!state.visible.get());
    }

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
                .map(|name| App::application(name.into(), vec![0, 0]))
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

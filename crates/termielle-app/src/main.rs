#![windows_subsystem = "windows"]

//! The overlay entry point: single instance, pipe wake-up, one timer, and
//! bounded numeric logging. The loop never polls: a deadline change re-arms
//! the window timer, and pipe events wake the loop out of `GetMessageW`.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use termielle_app::animation::fallback_frame;
use termielle_app::app::{Controller, ControllerActions, FALLBACK_FRAME_SIZE};
use termielle_app::log::{BoundedLog, LogComponent, LogEvent, LogLevel, LogRecord};
use termielle_app::theme;
use termielle_app::tray;
use termielle_app::window::{AnimationClock, OverlayWindow, WakeHandle, WindowError, WindowEvent};
use termielle_core::{
    AppConfig, AssetCatalog, DEFAULT_EVENT_LOG_MAX_BYTES, EventKind, EventLog, EventMessage,
    PROTOCOL_VERSION, ProtocolError, ReducedMotion, RenderMode, Source, VisualState,
    decode_event_line, encode_event_line, load_config, save_config_atomic,
};
use termielle_ipc::{DEFAULT_PIPE_NAME, EventClient, EventServer, IpcError};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_PATH_NOT_FOUND, GetLastError, HANDLE, SetLastError,
    WIN32_ERROR,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{BOOL, PCWSTR};

/// All six visual states, in priority order, for the smoke test.
const ALL_STATES: [VisualState; 6] = [
    VisualState::Idle,
    VisualState::Thinking,
    VisualState::Working,
    VisualState::NeedsInput,
    VisualState::Ready,
    VisualState::Failed,
];

/// Raises the process-wide timer resolution to 1 ms for the process lifetime.
/// The animation cadence is 20-40 ms per frame; the default 15.6 ms system
/// timer makes such short deadlines fire irregularly (a 20 ms frame can land
/// at 31 ms), which reads as shutter. `timeBeginPeriod(1)` removes that
/// jitter for as long as the overlay lives.
struct PreciseTimer;

impl PreciseTimer {
    fn enable() -> Self {
        // SAFETY: the call has no pointers and cannot fail; the matching
        // `timeEndPeriod` runs when the guard drops.
        unsafe {
            let _ = windows::Win32::Media::timeBeginPeriod(1);
        }
        Self
    }
}

impl Drop for PreciseTimer {
    fn drop(&mut self) {
        // SAFETY: balances the matching `timeBeginPeriod` from `enable`.
        unsafe {
            let _ = windows::Win32::Media::timeEndPeriod(1);
        }
    }
}

/// How long the smoke test waits for its in-process pipe event.
const SMOKE_TIMEOUT_MS: u32 = 5_000;

/// The diagnostics CLI surface, deliberately undocumented: the doctor and
/// benchmark scripts are the only callers.
struct Cli {
    smoke_test: bool,
    pipe: String,
    ack_file: Option<PathBuf>,
    render: Option<RenderMode>,
}

impl Cli {
    fn parse() -> Self {
        let mut args = std::env::args();
        let _program = args.next();
        let mut smoke_test = false;
        let mut pipe = String::from(DEFAULT_PIPE_NAME);
        let mut ack_file = None;
        let mut render = None;
        let mut iter = args;
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--smoke-test" => smoke_test = true,
                "--pipe" => pipe = iter.next().unwrap_or_else(|| DEFAULT_PIPE_NAME.to_string()),
                "--ack-file" => ack_file = iter.next().map(PathBuf::from),
                "--render" => {
                    render = match iter.next().as_deref() {
                        Some("per-pixel") | Some("per_pixel") => Some(RenderMode::PerPixel),
                        Some("color-key") | Some("color_key") => Some(RenderMode::ColorKey),
                        _ => None,
                    };
                }
                _ => {}
            }
        }
        Self {
            smoke_test,
            pipe,
            ack_file,
            render,
        }
    }
}

fn main() {
    // The frame timer needs millisecond precision; enable it before any
    // timing path runs, including the smoke test.
    let _precise_timer = PreciseTimer::enable();

    // GUI-subsystem panics vanish silently; route them to a file so a hover
    // crash is diagnosable instead of looking like "it disappeared".
    let panic_path = data_dir().map(|d| d.join("panic.log"));
    std::panic::set_hook(Box::new(move |info| {
        if let Some(path) = panic_path.as_ref() {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                use std::io::Write;
                let _ = writeln!(f, "{} panic: {info}", now_ms());
                let _ = writeln!(
                    f,
                    "location: {}",
                    info.location().map(|l| l.to_string()).unwrap_or_default()
                );
            }
        }
    }));

    let cli = Cli::parse();

    // One-time move of user state from the pre-0.2 location.
    migrate_legacy_data();

    // One overlay per session: a second instance must neither appear nor
    // steal events. The claim is held for the whole process lifetime. A
    // smoke run reports the contention loudly so diagnostics never mistake
    // a silent exit for a passing check.
    let instance = match acquire_single_instance(&cli.pipe) {
        Ok(guard) => guard,
        Err(_) => {
            if cli.smoke_test {
                std::process::exit(3);
            }
            return;
        }
    };

    let log = Arc::new(Mutex::new(BoundedLog::new(log_path(), 1_048_576, 3)));

    let mut config = load_config_with_log(&log);
    // Resolve island theme (builtin + file overlay)
    theme::resolve_theme(&mut config.island);

    // The command line wins over the persisted file: a doctor run can force a
    // renderer without touching the user's config.
    if let Some(render) = cli.render {
        config.render = render;
    }

    let mut window =
        match OverlayWindow::create(&config, std::env::args().any(|a| a == "--smoke-test")) {
            Ok(window) => window,
            Err(error) => {
                log_error(
                    &log,
                    LogComponent::Window,
                    LogEvent::PresentFailed,
                    window_error_code(&error),
                );
                return;
            }
        };
    let wake = window.wake_handle();
    if !cli.smoke_test && cli.pipe == DEFAULT_PIPE_NAME {
        // Only the production overlay gets the tray icon: diagnostics runs on
        // custom pipes stay out of the notification area. The icon is the
        // character's idle face when the standby asset is present.
        let catalog = AssetCatalog::new(asset_roots());
        let idle_asset = catalog.resolve(VisualState::Idle);
        if let Err(code) = window.install_tray(idle_asset.as_deref()) {
            log_error(
                &log,
                LogComponent::Window,
                LogEvent::PresentFailed,
                code as i32,
            );
        }
    }
    let mut controller = Controller::new_with_island(
        config.ready_hold_ms,
        config.busy_stall_ms,
        AssetCatalog::new(asset_roots()),
        reduced_motion_enabled(config.reduced_motion),
        // A fixed frame rate overrides each GIF's own delays: the overlay
        // presents frames at exactly this cadence (clamped to a whole
        // millisecond interval).
        config
            .frame_rate
            .map(|rate| ((1000u64 + u64::from(rate) / 2) / u64::from(rate)).max(1)),
        config.island.clone(),
    );

    // Only the production overlay journals and replays: diagnostics runs on
    // custom pipes must neither inherit nor pollute the live state.
    let journal = if !cli.smoke_test && cli.pipe == DEFAULT_PIPE_NAME {
        Some(EventLog::new(events_path(), DEFAULT_EVENT_LOG_MAX_BYTES))
    } else {
        None
    };

    // Rebuild the pre-restart state from the journal before any live event can
    // arrive. The fold is absolute-time driven, so a restart that happened
    // after the ready-hold or busy-stall expired replays straight into Idle,
    // and a restart mid-turn picks up exactly where the crash left off.
    if let Some(journal) = &journal {
        for event in journal.read_all() {
            controller.handle_event(event, now_ms());
        }
    }

    // Claim the pipe on the main thread so a name squatter is rejected at
    // startup, then hand the server to the pipe thread.
    let server = match EventServer::bind(&cli.pipe) {
        Ok(server) => server,
        Err(IpcError::PipeNameOwned) => {
            log_error(&log, LogComponent::Ipc, LogEvent::InvalidEvent, 3);
            return;
        }
        Err(error) => {
            log_error(
                &log,
                LogComponent::Ipc,
                LogEvent::InvalidEvent,
                ipc_error_code(&error),
            );
            return;
        }
    };

    let (sender, receiver) = channel();
    spawn_pipe_thread(server, wake, sender, log.clone());

    if cli.smoke_test {
        let builtin_event = cli.pipe == DEFAULT_PIPE_NAME;
        let code = run_smoke(&mut window, &mut controller, &receiver, &log, builtin_event);
        drop(instance);
        std::process::exit(code);
    }

    // Background dashboard worker: icon reads and media queries never run
    // on the GUI thread, so hovering and morphing never stall. The worker
    // exits when its receiver is dropped at shutdown.
    let thumb_cfg = std::sync::Arc::new(termielle_app::tasks::WorkerConfig::new(
        config.island.is_enabled(),
    ));
    let (thumb_sender, thumb_receiver) = channel();
    let backdrop_request: Arc<std::sync::Mutex<Option<termielle_app::tasks::BackdropRequest>>> =
        Arc::new(std::sync::Mutex::new(None));
    let _thumb_worker = termielle_app::tasks::spawn_worker(
        wake,
        thumb_sender,
        thumb_cfg.clone(),
        backdrop_request.clone(),
    );

    // The acknowledgement file is a diagnostics contract: open failure is
    // logged and the overlay still runs, so a locked temp file never hides
    // the pet itself.
    let mut ack = cli
        .ack_file
        .as_deref()
        .and_then(|path| match AckWriter::open(path) {
            Ok(writer) => Some(writer),
            Err(error) => {
                log_error(
                    &log,
                    LogComponent::Ipc,
                    LogEvent::InvalidEvent,
                    error.raw_os_error().unwrap_or(6),
                );
                None
            }
        });

    present_current(
        &mut window,
        &mut controller,
        &log,
        ack.as_mut(),
        Some(&backdrop_request),
    );
    let restart = run_gui(
        &mut window,
        &mut controller,
        &receiver,
        &thumb_receiver,
        &thumb_cfg,
        &backdrop_request,
        &log,
        &mut ack,
        journal.as_ref(),
        &mut config,
    );
    drop(instance);
    if restart {
        // The single-instance claim is already released, so the new process
        // can take it; the tray "Restart" path lands here.
        if let Ok(exe) = std::env::current_exe() {
            let _ = Command::new(exe).spawn();
        }
    }
}

/// The overlay's data directory: `%USERPROFILE%\.termielle`, the same
/// dot-directory convention as `~/.claude`. All user-facing state lives
/// here; the installed binaries stay under `%LOCALAPPDATA%\Termielle\bin`.
fn data_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")?;
    Some(PathBuf::from(home).join(".termielle"))
}

/// The legacy data directory, `%LOCALAPPDATA%\Termielle`, used before the
/// move to `~/.termielle`.
fn legacy_data_dir() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(local).join("Termielle"))
}

/// Copies user-facing state from the legacy `%LOCALAPPDATA%\Termielle`
/// location into `~/.termielle`, once. The installed binaries stay where the
/// installer put them; config, the user asset folder, and the logs move.
fn migrate_legacy_data() {
    let (Some(legacy), Some(current)) = (legacy_data_dir(), data_dir()) else {
        return;
    };
    if !legacy.join("config.json").is_file() {
        return;
    }
    if current.join("config.json").is_file() {
        return;
    }
    let _ = std::fs::create_dir_all(&current);
    for name in ["config.json", "assets", "termielle.log", "events.log"] {
        let from = legacy.join(name);
        let to = current.join(name);
        if from.is_dir() {
            let _ = copy_tree(&from, &to);
        } else if from.is_file() {
            let _ = std::fs::copy(&from, &to);
        }
    }
}

/// Recursively copies a directory tree.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Where the bounded JSON-lines log lives.
fn log_path() -> PathBuf {
    data_dir()
        .map(|dir| dir.join("termielle.log"))
        .unwrap_or_else(|| PathBuf::from("termielle.log"))
}

/// Where the replayed event journal lives.
fn events_path() -> PathBuf {
    data_dir()
        .map(|dir| dir.join("events.log"))
        .unwrap_or_else(|| PathBuf::from("events.log"))
}

/// Asset roots: the user's folder first, then the install directory.
fn asset_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = data_dir() {
        roots.push(dir.join("assets"));
    }
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
    {
        roots.push(exe_dir.join("assets"));
    }
    roots
}

/// Reads the config, falling back to defaults and one bounded record when the
/// file cannot be used.
fn load_config_with_log(log: &Arc<Mutex<BoundedLog>>) -> AppConfig {
    let Some(path) = data_dir().map(|dir| dir.join("config.json")) else {
        return AppConfig::default();
    };
    match load_config(&path) {
        Ok(config) => config,
        Err(error) => {
            log_error(
                log,
                LogComponent::Config,
                LogEvent::ReadFailed,
                config_error_code(&error),
            );
            AppConfig::default()
        }
    }
}

/// Whether the user's reduced-motion preference turns off GIF animation.
fn reduced_motion_enabled(mode: ReducedMotion) -> bool {
    match mode {
        ReducedMotion::On => true,
        ReducedMotion::Off => false,
        // SPI_GETCLIENTAREAANIMATION reports whether animations run; reduced
        // motion is the negation. A failed query defaults to motion on.
        ReducedMotion::System => !system_animations_enabled(),
    }
}

fn system_animations_enabled() -> bool {
    let mut enabled = BOOL::default();
    let queried = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&raw mut enabled).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    queried.map_or(true, |()| enabled.as_bool())
}

/// Holds the single-instance claim for the process lifetime.
///
/// The claim has two layers. The named mutex is the primary, session-scoped
/// gate. A lock file in the data directory backs it up, because named-object
/// creation is unreliable in sandboxed sessions: the kernel's `Local`
/// namespace subdirectory can be absent for a whole process lifetime, and
/// even root names fail randomly once Win32 UI libraries are loaded. The
/// mutex name therefore lives directly in the `\BaseNamedObjects` root (for
/// session processes the plain root name is redirected per session anyway),
/// and every claim also takes the lock file so a fallback claim stays
/// visible to a healthy mutex-owning instance. The file records the owner
/// PID so a crashed owner's stale lock is stealable.
struct InstanceGuard {
    mutex: Option<HANDLE>,
    lock_file: Option<PathBuf>,
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.mutex.take() {
            // SAFETY: a kernel handle this process created and owns.
            unsafe {
                let _ = CloseHandle(handle);
            };
        }
        if let Some(path) = self.lock_file.take() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Claims the single-instance claim; the first process wins, later ones get
/// `ERROR_ALREADY_EXISTS`. A custom pipe gets its own claim so a
/// diagnostics run never contends with a live overlay, and a live overlay is
/// never validated by a smoke run that did not actually run.
fn acquire_single_instance(pipe: &str) -> Result<InstanceGuard, u32> {
    // The pipe name keeps its `\\.\pipe\` prefix; the mutex name must not,
    // or CreateMutexW fails on the embedded device-path components.
    let label = pipe.strip_prefix(r"\\.\pipe\").unwrap_or(pipe);
    let mut name = if pipe == DEFAULT_PIPE_NAME {
        String::from("Termielle")
    } else {
        format!("Termielle-{label}")
    };
    name.truncate(250);
    let name: Vec<u16> = name.encode_utf16().collect();

    // SAFETY: kernel object creation with an in-bounds name; a returned
    // handle is a process-owned kernel handle kept for the claim lifetime.
    // The last-error slot is cleared first because CreateMutexW reports an
    // already-held mutex through GetLastError without failing the call, and
    // the call itself never touches the slot on success.
    unsafe { SetLastError(WIN32_ERROR(0)) };
    let mutex = match unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) } {
        Ok(handle) => {
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                return Err(ERROR_ALREADY_EXISTS.0);
            }
            Some(handle)
        }
        Err(error) => {
            let code = error.code().0 as u32;
            if code & 0xFFFF != ERROR_PATH_NOT_FOUND.0 {
                return Err(code);
            }
            None
        }
    };

    let lock_file = acquire_lock_file(label)?;
    Ok(InstanceGuard { mutex, lock_file })
}

/// Takes the exclusive lock file for `label`, stealing it when the recorded
/// owner PID is no longer running. Returns `ERROR_ALREADY_EXISTS` when a
/// live process holds the claim.
fn acquire_lock_file(label: &str) -> Result<Option<PathBuf>, u32> {
    let mut path = match data_dir() {
        Some(dir) => dir,
        None => return Ok(None),
    };
    let mut file_name = String::from("instance-");
    for ch in label.chars() {
        file_name.push(if ch.is_ascii_alphanumeric() || ch == '-' || ch == '.' {
            ch
        } else {
            '-'
        });
    }
    path.push(file_name);
    path.set_extension("lock");

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let _ = writeln!(file, "{}", std::process::id());
                return Ok(Some(path));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let owner = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| text.trim().parse::<u32>().ok());
                match owner {
                    Some(pid) if !process_alive(pid) => {
                        let _ = std::fs::remove_file(&path);
                    }
                    _ => return Err(ERROR_ALREADY_EXISTS.0),
                }
            }
            Err(_) => return Err(ERROR_ALREADY_EXISTS.0),
        }
    }
}

/// Whether a process with the given PID is in the live process table.
///
/// PID liveness is decided from a process snapshot rather than `OpenProcess`:
/// some systems answer an open of a dead PID with `ERROR_ACCESS_DENIED` or
/// `ERROR_PATH_NOT_FOUND` instead of `ERROR_INVALID_PARAMETER`, which would
/// keep a stale lock alive forever. A snapshot that cannot be taken reports
/// the process as alive, so a stale lock is never stolen speculatively.
fn process_alive(pid: u32) -> bool {
    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(snapshot) => snapshot,
        Err(_) => return true,
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut alive = false;
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            if entry.th32ProcessID == pid {
                alive = true;
                break;
            }
            if !unsafe { Process32NextW(snapshot, &mut entry) }.is_ok() {
                break;
            }
        }
    }
    // SAFETY: the snapshot handle, when returned, lives only for this call.
    let _ = unsafe { CloseHandle(snapshot) };
    alive
}

/// Appends one JSON line per successful present: the presented state's wire
/// name and the presentation timestamp, the signal the benchmark measures.
struct AckWriter {
    file: BufWriter<File>,
}

impl AckWriter {
    fn open(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: BufWriter::new(file),
        })
    }

    fn record(&mut self, state: VisualState) {
        let line = format!(r#"{{"state":"{}","ts":{}}}"#, state.wire_name(), now_ms());
        let _ = writeln!(self.file, "{line}");
        let _ = self.file.flush();
    }
}

/// One blocking server thread: decode accepted lines, queue valid events,
/// and wake the GUI thread out of `GetMessageW`.
fn spawn_pipe_thread(
    server: EventServer,
    wake: WakeHandle,
    sender: Sender<EventMessage>,
    log: Arc<Mutex<BoundedLog>>,
) {
    std::thread::spawn(move || {
        loop {
            match server.receive_one() {
                Ok(line) => match decode_event_line(&line) {
                    Ok(event) => {
                        let _ = sender.send(event);
                        let _ = wake.post();
                    }
                    Err(error) => {
                        log_error(
                            &log,
                            LogComponent::Ipc,
                            LogEvent::InvalidEvent,
                            protocol_error_code(&error),
                        );
                    }
                },
                Err(error) => {
                    log_error(
                        &log,
                        LogComponent::Ipc,
                        LogEvent::InvalidEvent,
                        ipc_error_code(&error),
                    );
                    // Do not spin if the transport is failing repeatedly.
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    });
}

/// Presents the controller's current frame, retrying once and then falling
/// back to the procedural still, logging one bounded record per failure. Each
/// successful present is recorded in the acknowledgement file, if any.
fn present_current(
    window: &mut OverlayWindow,
    controller: &mut Controller,
    log: &Arc<Mutex<BoundedLog>>,
    ack: Option<&mut AckWriter>,
    backdrop_request: Option<&Arc<std::sync::Mutex<Option<termielle_app::tasks::BackdropRequest>>>>,
) {
    let anchor = controller.island_anchor();
    let attempt = if let Some((attached, y_off)) = anchor {
        window.present_with_anchor(controller.current_frame(), 1.0, Some((attached, y_off)))
    } else {
        window.present(controller.current_frame(), 1.0)
    };
    // Publish the glass capture request: the rect the pill was just drawn
    // at. The worker owns the (potentially slow) capture and blur.
    if let Some(request) = backdrop_request {
        let (x, y, w, h) = window.last_dest();
        if w > 1 {
            let cfg = controller.island_config();
            *request.lock().unwrap() = Some(termielle_app::tasks::BackdropRequest {
                x,
                y,
                w,
                h,
                radius: cfg.glass.blur_radius,
                tint: cfg.glass.tint,
            });
        }
    }
    match attempt {
        Ok(()) => {
            if let Some(ack) = ack {
                ack.record(controller.visible_state());
            }
        }
        Err(error) => {
            log_error(
                log,
                LogComponent::Window,
                LogEvent::PresentFailed,
                window_error_code(&error),
            );
            let retry_anchor = controller.island_anchor();
            let retry = if let Some((attached, y_off)) = retry_anchor {
                window.present_with_anchor(controller.current_frame(), 1.0, Some((attached, y_off)))
            } else {
                window.present(controller.current_frame(), 1.0)
            };
            match retry {
                Ok(()) => {
                    if let Some(ack) = ack {
                        ack.record(controller.visible_state());
                    }
                }
                Err(second) => {
                    log_error(
                        log,
                        LogComponent::Window,
                        LogEvent::PresentFailed,
                        window_error_code(&second),
                    );
                    controller.fallback_to_still();
                    let fallback_anchor = controller.island_anchor();
                    let _ = if let Some((attached, y_off)) = fallback_anchor {
                        window.present_with_anchor(
                            controller.current_frame(),
                            1.0,
                            Some((attached, y_off)),
                        )
                    } else {
                        window.present(controller.current_frame(), 1.0)
                    };
                }
            }
        }
    }
}

/// Polls hover from the real cursor position and folds any transition into
/// `actions`. Backstop for spurious `WM_MOUSELEAVE`s across ULW resizes.
fn poll_hover(
    window: &OverlayWindow,
    controller: &mut Controller,
    actions: &mut ControllerActions,
) {
    controller.set_hover_point(window.cursor_client_pos());
    if controller.set_hover(window.cursor_over_pill(), now_ms()) {
        actions.present_frame = true;
        actions.next_deadline_ms = controller.next_deadline_ms();
    }
}

/// Applies the controller's actions: log decode failures, repaint when asked.
fn apply(
    window: &mut OverlayWindow,
    controller: &mut Controller,
    log: &Arc<Mutex<BoundedLog>>,
    actions: ControllerActions,
    ack: &mut Option<AckWriter>,
    backdrop_request: Option<&Arc<std::sync::Mutex<Option<termielle_app::tasks::BackdropRequest>>>>,
) {
    if let Some(code) = actions.error_code {
        log_error(log, LogComponent::Animation, LogEvent::DecodeFailed, code);
    }
    if let Some(state) = actions.visible_state {
        if !window.update_tray(&tray::state_tooltip(state.wire_name())) {
            log_error(log, LogComponent::Window, LogEvent::PresentFailed, 8);
        }
    }
    if actions.present_frame {
        present_current(window, controller, log, ack.as_mut(), backdrop_request);
    }
}

/// Drains background worker rounds (task icons + media state) into the
/// controller. Returns whether any round wants a repaint.
fn drain_thumbs(
    window: &mut OverlayWindow,
    controller: &mut Controller,
    receiver: &Receiver<termielle_app::tasks::WorkerUpdate>,
) -> bool {
    let mut present = false;
    while let Ok(mut batch) = receiver.try_recv() {
        if let Some(backdrop) = batch.backdrop.take() {
            window.set_backdrop(backdrop);
        }
        if controller.set_task_update(batch) {
            present = true;
        }
    }
    present
}

/// Drains every queued pipe event into the controller, merging the actions.
///
/// Each event is journaled before it is folded: a crash between the two loses
/// the event, and replaying from the journal is exactly what restores it.
fn drain_pipe(
    controller: &mut Controller,
    receiver: &Receiver<EventMessage>,
    journal: Option<&EventLog>,
) -> ControllerActions {
    let mut actions = ControllerActions::default();
    while let Ok(event) = receiver.try_recv() {
        if let Some(journal) = journal {
            // A journal failure is logged nowhere and stalls nothing: the
            // overlay keeps running, it simply forgets the event on restart.
            let _ = journal.append(&event);
        }
        actions = merge(actions, controller.handle_event(event, now_ms()));
    }
    actions
}

/// Combines two action sets from the same loop iteration; the last deadline
/// wins because both were computed against the same clock reading.
fn merge(left: ControllerActions, right: ControllerActions) -> ControllerActions {
    ControllerActions {
        visible_state: right.visible_state.or(left.visible_state),
        present_frame: left.present_frame || right.present_frame,
        next_deadline_ms: right.next_deadline_ms.or(left.next_deadline_ms),
        error_code: right.error_code.or(left.error_code),
    }
}

/// Applies the controller's actions, measuring the present cost when a frame
/// was actually presented so the animation clock can subtract it from the
/// next frame interval (a slow present would otherwise stretch every gap).
fn apply_timed(
    window: &mut OverlayWindow,
    controller: &mut Controller,
    log: &Arc<Mutex<BoundedLog>>,
    actions: ControllerActions,
    ack: &mut Option<AckWriter>,
    backdrop_request: Option<&Arc<std::sync::Mutex<Option<termielle_app::tasks::BackdropRequest>>>>,
) {
    if actions.present_frame {
        let started = std::time::Instant::now();
        apply(window, controller, log, actions, ack, backdrop_request);
        controller.set_present_cost(started.elapsed().as_millis() as u64);
    } else {
        apply(window, controller, log, actions, ack, backdrop_request);
    }
}

/// The production loop: block on the window, react to timer and display
/// changes, drain pipe events and thumbnail batches, present when required,
/// and re-arm one timer. Returns whether the overlay should be relaunched
/// after the graceful quit.
#[allow(clippy::too_many_arguments)] // wiring hub: each channel/config has exactly one consumer here.
fn run_gui(
    window: &mut OverlayWindow,
    controller: &mut Controller,
    receiver: &Receiver<EventMessage>,
    thumb_receiver: &Receiver<termielle_app::tasks::WorkerUpdate>,
    thumb_cfg: &std::sync::Arc<termielle_app::tasks::WorkerConfig>,
    backdrop_request: &Arc<std::sync::Mutex<Option<termielle_app::tasks::BackdropRequest>>>,
    log: &Arc<Mutex<BoundedLog>>,
    ack: &mut Option<AckWriter>,
    journal: Option<&EventLog>,
    config: &mut AppConfig,
) -> bool {
    // The animation clock thread paces frames; the smoke test's timeout timer
    // is separate and unaffected.
    let clock = AnimationClock::spawn(window.wake_handle());
    loop {
        // Catch up on deadlines that became due while we were blocked.
        loop {
            let now = now_ms();
            let Some(at) = controller.next_deadline_ms() else {
                break;
            };
            if at > now {
                break;
            }
            let actions = controller.on_timer(now);
            let mut actions = actions;
            poll_hover(window, controller, &mut actions);
            apply_timed(
                window,
                controller,
                log,
                actions,
                ack,
                Some(backdrop_request),
            );
        }

        // Arm the clock for the nearest deadline; disarming when none. The
        // clock thread wakes the loop out of `next_event`, which lands here
        // via the `Ok(None)` path and re-runs the catch-up above.
        clock.arm(controller.next_deadline_ms());

        let event = match window.next_event() {
            Ok(Some(event)) => event,
            Ok(None) => {
                // A wake message (deadline due, pipe event, or fresh
                // thumbnails) or WM_QUIT: drain everything queued; the
                // catch-up loop above handles any deadline that is now due.
                let iteration = || {
                    let actions = drain_pipe(controller, receiver, journal);
                    let thumb_present = drain_thumbs(window, controller, thumb_receiver);
                    let mut actions = actions;
                    actions.present_frame = actions.present_frame || thumb_present;
                    poll_hover(window, controller, &mut actions);
                    actions.next_deadline_ms = controller.next_deadline_ms();
                    apply_timed(
                        window,
                        controller,
                        log,
                        actions,
                        ack,
                        Some(backdrop_request),
                    );
                };
                // A panic here must never kill the overlay: log (via the
                // hook), rebuild a known-good still, and keep running.
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(iteration)).is_err() {
                    log_error(log, LogComponent::Window, LogEvent::PresentFailed, 66);
                    controller.fallback_to_still();
                    present_current(
                        window,
                        controller,
                        log,
                        ack.as_mut(),
                        Some(backdrop_request),
                    );
                }
                continue;
            }
            Err(error) => {
                log_error(
                    log,
                    LogComponent::Window,
                    LogEvent::PresentFailed,
                    window_error_code(&error),
                );
                continue;
            }
        };

        let mut actions = match event {
            WindowEvent::Timer => controller.on_timer(now_ms()),
            WindowEvent::DisplayChanged => {
                // The wndproc re-clamped classic rects; island needs a re-anchor
                // so force a repaint which will SetWindowPos to top-center.
                ControllerActions {
                    present_frame: true,
                    ..Default::default()
                }
            }
            WindowEvent::Quit => {
                window.destroy();
                return false;
            }
            WindowEvent::Restart => {
                window.destroy();
                return true;
            }
            WindowEvent::LayoutChanged(layout) => {
                config.island.layout = layout;
                config.island.clamp();
                let _ = save_config_atomic(
                    &data_dir()
                        .map(|d| d.join("config.json"))
                        .unwrap_or_else(|| PathBuf::from("config.json")),
                    config,
                );
                theme::resolve_theme(&mut config.island);
                controller.set_island_config(config.island.clone(), now_ms());
                window.set_island(config.island.is_enabled());
                window.set_glass(&config.island.glass);
                thumb_cfg.enabled.store(
                    config.island.is_enabled(),
                    std::sync::atomic::Ordering::Relaxed,
                );
                ControllerActions {
                    present_frame: true,
                    ..Default::default()
                }
            }
            WindowEvent::ThemeChanged(theme) => {
                config.island.theme = theme;
                theme::resolve_theme(&mut config.island);
                window.set_glass(&config.island.glass);
                let _ = save_config_atomic(
                    &data_dir()
                        .map(|d| d.join("config.json"))
                        .unwrap_or_else(|| PathBuf::from("config.json")),
                    config,
                );
                controller.set_island_config(config.island.clone(), now_ms());
                ControllerActions {
                    present_frame: true,
                    ..Default::default()
                }
            }
            WindowEvent::YOffsetChanged(y) => {
                config.island.y_offset = y;
                config.island.clamp();
                let _ = save_config_atomic(
                    &data_dir()
                        .map(|d| d.join("config.json"))
                        .unwrap_or_else(|| PathBuf::from("config.json")),
                    config,
                );
                controller.set_island_config(config.island.clone(), now_ms());
                ControllerActions {
                    present_frame: true,
                    ..Default::default()
                }
            }
            WindowEvent::ClickAt(x, y) => match controller.handle_click(x, y, now_ms()) {
                termielle_app::app::ClickOutcome::MediaToggle => {
                    termielle_app::tasks::toggle_media_playback();
                    ControllerActions {
                        present_frame: true,
                        ..Default::default()
                    }
                }
                termielle_app::app::ClickOutcome::MediaPrev => {
                    termielle_app::tasks::media_prev_track();
                    ControllerActions {
                        present_frame: true,
                        ..Default::default()
                    }
                }
                termielle_app::app::ClickOutcome::MediaNext => {
                    termielle_app::tasks::media_next_track();
                    ControllerActions {
                        present_frame: true,
                        ..Default::default()
                    }
                }
                termielle_app::app::ClickOutcome::ActivateWindow(hwnd) => {
                    termielle_app::tasks::activate_window(hwnd);
                    ControllerActions {
                        present_frame: true,
                        ..Default::default()
                    }
                }
                termielle_app::app::ClickOutcome::AlertDismiss
                | termielle_app::app::ClickOutcome::Expanded
                | termielle_app::app::ClickOutcome::Collapsed => ControllerActions {
                    present_frame: true,
                    ..Default::default()
                },
                termielle_app::app::ClickOutcome::None => ControllerActions::default(),
            },
            WindowEvent::HoverChanged(inside) => {
                let changed = controller.set_hover(inside, now_ms());
                ControllerActions {
                    present_frame: changed,
                    ..Default::default()
                }
            }
            WindowEvent::PressChanged(pressed) => {
                let changed = controller.set_pressed(pressed, now_ms());
                ControllerActions {
                    present_frame: changed,
                    ..Default::default()
                }
            }
            WindowEvent::ToggleTasks => {
                if config.island.has_widget("music") {
                    config.island.widgets.retain(|w| w != "music");
                } else {
                    config.island.widgets.push("music".to_string());
                }
                let _ = save_config_atomic(
                    &data_dir()
                        .map(|d| d.join("config.json"))
                        .unwrap_or_else(|| PathBuf::from("config.json")),
                    config,
                );
                controller.set_island_config(config.island.clone(), now_ms());
                ControllerActions {
                    present_frame: true,
                    ..Default::default()
                }
            }
            WindowEvent::ToggleHoverExpand => {
                config.island.expand_on_hover = !config.island.expand_on_hover;
                let _ = save_config_atomic(
                    &data_dir()
                        .map(|d| d.join("config.json"))
                        .unwrap_or_else(|| PathBuf::from("config.json")),
                    config,
                );
                controller.set_island_config(config.island.clone(), now_ms());
                ControllerActions {
                    present_frame: true,
                    ..Default::default()
                }
            }
            WindowEvent::ToggleFace => {
                if config.island.has_widget("face") {
                    config.island.widgets.retain(|w| w != "face");
                } else {
                    config.island.widgets.push("face".to_string());
                }
                config.island.clamp();
                let _ = save_config_atomic(
                    &data_dir()
                        .map(|d| d.join("config.json"))
                        .unwrap_or_else(|| PathBuf::from("config.json")),
                    config,
                );
                controller.set_island_config(config.island.clone(), now_ms());
                ControllerActions {
                    present_frame: true,
                    ..Default::default()
                }
            }
            WindowEvent::SystemThemeChanged => {
                // The Windows light/dark setting flipped: re-resolve `auto`
                // and repaint. The persisted config keeps `theme: auto`.
                theme::resolve_theme(&mut config.island);
                window.set_glass(&config.island.glass);
                controller.set_island_config(config.island.clone(), now_ms());
                ControllerActions {
                    present_frame: true,
                    ..Default::default()
                }
            }
        };
        actions = merge(actions, drain_pipe(controller, receiver, journal));
        if drain_thumbs(window, controller, thumb_receiver) {
            actions.present_frame = true;
            actions.next_deadline_ms = controller.next_deadline_ms();
        }
        poll_hover(window, controller, &mut actions);
        let iteration = || {
            apply_timed(
                window,
                controller,
                log,
                actions,
                ack,
                Some(backdrop_request),
            );
        };
        // Same fault tolerance as the wake path.
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(iteration)).is_err() {
            log_error(log, LogComponent::Window, LogEvent::PresentFailed, 66);
            controller.fallback_to_still();
            present_current(
                window,
                controller,
                log,
                ack.as_mut(),
                Some(backdrop_request),
            );
        }
    }
}

/// Smoke mode: no external assets. Presents every procedural fallback once,
/// drives one in-process pipe event to `Thinking` (unless a custom pipe asked
/// for the event to come from outside, as the doctor does), then exits `0`.
fn run_smoke(
    window: &mut OverlayWindow,
    controller: &mut Controller,
    receiver: &Receiver<EventMessage>,
    log: &Arc<Mutex<BoundedLog>>,
    builtin_event: bool,
) -> i32 {
    for state in ALL_STATES {
        let frame = fallback_frame(state, FALLBACK_FRAME_SIZE);
        if let Err(error) = window.present(&frame, 1.0) {
            log_error(
                log,
                LogComponent::Window,
                LogEvent::PresentFailed,
                window_error_code(&error),
            );
            window.destroy();
            return 1;
        }
    }

    // One in-process pipe event, through the real transport.
    if builtin_event {
        let line = encode_event_line(&EventMessage {
            version: PROTOCOL_VERSION,
            source: Source::parse("codex").expect("smoke source word is valid"),
            session_id: "smoke-test".into(),
            event: EventKind::PromptSubmitted,
            timestamp_ms: now_ms(),
        })
        .expect("smoke event encodes");
        std::thread::spawn(move || {
            let client = EventClient::new(DEFAULT_PIPE_NAME, Duration::from_secs(5));
            let _ = client.send(&line);
        });
    }

    // The window timer doubles as the smoke timeout.
    let _ = window.set_timer(Some(SMOKE_TIMEOUT_MS));
    loop {
        match window.next_event() {
            Ok(Some(WindowEvent::Timer)) => {
                window.destroy();
                return 1;
            }
            Ok(Some(WindowEvent::DisplayChanged)) => {}
            Ok(Some(WindowEvent::Quit | WindowEvent::Restart)) => {}
            Ok(Some(
                WindowEvent::LayoutChanged(_)
                | WindowEvent::ThemeChanged(_)
                | WindowEvent::YOffsetChanged(_)
                | WindowEvent::SystemThemeChanged
                | WindowEvent::ClickAt(..)
                | WindowEvent::PressChanged(_)
                | WindowEvent::HoverChanged(_)
                | WindowEvent::ToggleTasks
                | WindowEvent::ToggleHoverExpand
                | WindowEvent::ToggleFace,
            )) => {}
            Ok(None) => {}
            Err(error) => {
                log_error(
                    log,
                    LogComponent::Window,
                    LogEvent::PresentFailed,
                    window_error_code(&error),
                );
                window.destroy();
                return 1;
            }
        }
        let actions = drain_pipe(controller, receiver, None);
        if actions.visible_state == Some(VisualState::Thinking) {
            window.destroy();
            return 0;
        }
    }
}

/// Unix epoch milliseconds; 1 when the clock is unusable.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |elapsed| elapsed.as_millis() as u64)
}

/// One bounded error record.
fn log_error(log: &Arc<Mutex<BoundedLog>>, component: LogComponent, event: LogEvent, code: i32) {
    let record = LogRecord {
        timestamp_ms: now_ms(),
        level: LogLevel::Error,
        component,
        event,
        error_code: code,
    };
    let _ = log.lock().unwrap().write(&record);
}

fn window_error_code(error: &WindowError) -> i32 {
    match error {
        WindowError::Win32(code) => *code as i32,
        WindowError::FrameBuffer => 1,
        WindowError::Crate(error) => error.code().0,
    }
}

fn protocol_error_code(error: &ProtocolError) -> i32 {
    match error {
        ProtocolError::TooLarge => 1,
        ProtocolError::Json(_) => 2,
        ProtocolError::UnsupportedVersion(version) => 100 + i32::from(*version),
        ProtocolError::InvalidSessionId => 3,
        ProtocolError::InvalidTimestamp => 4,
        ProtocolError::InvalidSource => 5,
    }
}

fn ipc_error_code(error: &IpcError) -> i32 {
    match error {
        IpcError::Os(code) => *code as i32,
        IpcError::NotFound => 1,
        IpcError::Busy => 2,
        IpcError::PipeNameOwned => 3,
        IpcError::Timeout => 4,
        IpcError::TooLarge => 5,
    }
}

fn config_error_code(error: &termielle_core::ConfigError) -> i32 {
    match error {
        termielle_core::ConfigError::Malformed(_) => 1,
        termielle_core::ConfigError::Io(error) => error.raw_os_error().unwrap_or(2),
    }
}

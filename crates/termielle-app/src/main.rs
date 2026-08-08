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
use termielle_app::tray;
use termielle_app::window::{OverlayWindow, WakeHandle, WindowError, WindowEvent};
use termielle_core::{
    AppConfig, AssetCatalog, EventKind, EventMessage, PROTOCOL_VERSION, ProtocolError,
    ReducedMotion, RenderMode, Source, VisualState, decode_event_line, encode_event_line,
    load_config,
};
use termielle_ipc::{DEFAULT_PIPE_NAME, IpcError, PipeClient, PipeServer};
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
    let cli = Cli::parse();

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

    let config = load_config_with_log(&log);

    // The command line wins over the persisted file: a doctor run can force a
    // renderer without touching the user's config.
    let mut config = config;
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
        // custom pipes stay out of the notification area.
        if !window.install_tray() {
            log_error(&log, LogComponent::Window, LogEvent::PresentFailed, 9);
        }
    }
    let mut controller = Controller::new(
        config.ready_hold_ms,
        config.busy_stall_ms,
        AssetCatalog::new(asset_roots()),
        reduced_motion_enabled(config.reduced_motion),
    );

    // Claim the pipe on the main thread so a name squatter is rejected at
    // startup, then hand the server to the pipe thread.
    let server = match PipeServer::bind(&cli.pipe) {
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

    present_current(&mut window, &mut controller, &log, ack.as_mut());
    let restart = run_gui(&mut window, &mut controller, &receiver, &log, &mut ack);
    drop(instance);
    if restart {
        // The single-instance claim is already released, so the new process
        // can take it; the tray "Restart" path lands here.
        if let Ok(exe) = std::env::current_exe() {
            let _ = Command::new(exe).spawn();
        }
    }
}

/// The overlay's data directory: `%LOCALAPPDATA%\Termielle`.
fn data_dir() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(local).join("Termielle"))
}

/// Where the bounded JSON-lines log lives.
fn log_path() -> PathBuf {
    data_dir()
        .map(|dir| dir.join("termielle.log"))
        .unwrap_or_else(|| PathBuf::from("termielle.log"))
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
    server: PipeServer,
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
) {
    match window.present(controller.current_frame(), 1.0) {
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
            match window.present(controller.current_frame(), 1.0) {
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
                    let _ = window.present(controller.current_frame(), 1.0);
                }
            }
        }
    }
}

/// Applies the controller's actions: log decode failures, repaint when asked.
fn apply(
    window: &mut OverlayWindow,
    controller: &mut Controller,
    log: &Arc<Mutex<BoundedLog>>,
    actions: ControllerActions,
    ack: &mut Option<AckWriter>,
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
        present_current(window, controller, log, ack.as_mut());
    }
}

/// Drains every queued pipe event into the controller, merging the actions.
fn drain_pipe(controller: &mut Controller, receiver: &Receiver<EventMessage>) -> ControllerActions {
    let mut actions = ControllerActions::default();
    while let Ok(event) = receiver.try_recv() {
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

/// The production loop: block on the window, react to timer and display
/// changes, drain pipe events, present when required, and re-arm one timer.
/// Returns whether the overlay should be relaunched after the graceful quit.
fn run_gui(
    window: &mut OverlayWindow,
    controller: &mut Controller,
    receiver: &Receiver<EventMessage>,
    log: &Arc<Mutex<BoundedLog>>,
    ack: &mut Option<AckWriter>,
) -> bool {
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
            apply(window, controller, log, actions, ack);
        }

        // Arm the single timer for the nearest deadline; disarming when none.
        let delay = controller
            .next_deadline_ms()
            .and_then(|at| u32::try_from(at.saturating_sub(now_ms())).ok());
        let _ = window.set_timer(delay);

        let event = match window.next_event() {
            Ok(Some(event)) => event,
            Ok(None) => {
                // A wake message or WM_QUIT: drain queued pipe events.
                let actions = drain_pipe(controller, receiver);
                apply(window, controller, log, actions, ack);
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
                // The wndproc already re-clamped the rect; blit the frame.
                ControllerActions::default()
            }
            WindowEvent::Quit => {
                window.destroy();
                return false;
            }
            WindowEvent::Restart => {
                window.destroy();
                return true;
            }
        };
        actions = merge(actions, drain_pipe(controller, receiver));
        apply(window, controller, log, actions, ack);
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
            source: Source::Codex,
            session_id: "smoke-test".into(),
            event: EventKind::PromptSubmitted,
            timestamp_ms: now_ms(),
        })
        .expect("smoke event encodes");
        std::thread::spawn(move || {
            let client = PipeClient::new(DEFAULT_PIPE_NAME, Duration::from_secs(5));
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
        let actions = drain_pipe(controller, receiver);
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

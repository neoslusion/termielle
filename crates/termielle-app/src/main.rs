#![windows_subsystem = "windows"]

//! The overlay entry point: single instance, pipe wake-up, one timer, and
//! bounded numeric logging. The loop never polls: a deadline change re-arms
//! the window timer, and pipe events wake the loop out of `GetMessageW`.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
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
    AppConfig, AssetCatalog, BarPosition, DEFAULT_EVENT_LOG_MAX_BYTES, EventKind, EventLog,
    EventMessage, IslandLayout, PROTOCOL_VERSION, ProtocolError, ReducedMotion, RenderMode, Source,
    VisualState, decode_event_line, encode_event_line, load_config, save_config_atomic,
};
use termielle_ipc::{DEFAULT_PIPE_NAME, EventClient, EventServer, IpcError};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_PATH_NOT_FOUND, GetLastError, HANDLE, SetLastError,
    WIN32_ERROR,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    CreateMutexW, OpenProcess, PROCESS_ACCESS_RIGHTS, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};

use windows::core::{BOOL, PCWSTR};

/// `SYNCHRONIZE` from winnt.h (0x00100000). The `windows` crate does not bind
/// the constant, and the watchdog needs nothing else from the process.
const PROCESS_SYNCHRONIZE_ACCESS: PROCESS_ACCESS_RIGHTS = PROCESS_ACCESS_RIGHTS(0x0010_0000);

/// `INFINITE` from winbase.h. The `windows` crate does not bind it, and a
/// watchdog that outlives its parent is exactly the wait that never times out.
const WAIT_FOREVER: u32 = u32::MAX;

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
    config_path: Option<PathBuf>,
    layout: Option<IslandLayout>,
    bar_pos: Option<BarPosition>,
    replace_taskbar: Option<bool>,
    restore_taskbar: bool,
    watchdog_parent_pid: Option<u32>,
}

impl Cli {
    fn parse() -> Self {
        let mut args = std::env::args();
        let _program = args.next();
        let mut smoke_test = false;
        let mut pipe = String::from(DEFAULT_PIPE_NAME);
        let mut ack_file = None;
        let mut render = None;
        let mut layout = None;
        let mut bar_pos = None;
        let mut config_path = None;
        let mut replace_taskbar = None;
        let mut restore_taskbar = false;
        let mut watchdog_parent_pid = None;
        let mut iter = args;
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--smoke-test" => smoke_test = true,
                "--pipe" => pipe = iter.next().unwrap_or_else(|| DEFAULT_PIPE_NAME.to_string()),
                "--ack-file" => ack_file = iter.next().map(PathBuf::from),
                "--config" => config_path = iter.next().map(PathBuf::from),
                "--render" => {
                    render = match iter.next().as_deref() {
                        Some("per-pixel") | Some("per_pixel") => Some(RenderMode::PerPixel),
                        Some("color-key") | Some("color_key") => Some(RenderMode::ColorKey),
                        _ => None,
                    };
                }
                "--restore-taskbar" => restore_taskbar = true,
                "--watchdog-parent-pid" => {
                    watchdog_parent_pid = iter.next().and_then(|value| value.parse().ok());
                }
                "--bar" => layout = Some(IslandLayout::Bar),
                "--island" => layout = Some(IslandLayout::Island),
                "--notch" => layout = Some(IslandLayout::Notch),
                "--classic" => layout = Some(IslandLayout::Classic),
                "--layout" => {
                    layout = match iter.next().as_deref() {
                        Some("bar") => Some(IslandLayout::Bar),
                        Some("island") => Some(IslandLayout::Island),
                        Some("notch") => Some(IslandLayout::Notch),
                        Some("classic") => Some(IslandLayout::Classic),
                        _ => None,
                    };
                }
                "--top" => bar_pos = Some(BarPosition::Top),
                "--bottom" => bar_pos = Some(BarPosition::Bottom),
                "--bar-pos" => {
                    bar_pos = match iter.next().as_deref() {
                        Some("top") => Some(BarPosition::Top),
                        Some("bottom") => Some(BarPosition::Bottom),
                        _ => None,
                    };
                }
                "--replace-taskbar" => replace_taskbar = Some(true),
                "--keep-taskbar" => replace_taskbar = Some(false),
                _ => {}
            }
        }
        Self {
            smoke_test,
            pipe,
            ack_file,
            render,
            config_path,
            layout,
            bar_pos,
            replace_taskbar,
            restore_taskbar,
            watchdog_parent_pid,
        }
    }
}

fn main() {
    if let Some(directory) = data_dir() {
        let _ = std::fs::create_dir_all(directory);
    }

    // GUI-subsystem panics vanish silently; route them to a file so a hover
    // crash is diagnosable instead of looking like "it disappeared".
    let panic_path = data_dir().map(|d| d.join("panic.log"));
    std::panic::set_hook(Box::new(move |info| {
        // No HWND here; leave_bar_shell routes ABM_REMOVE via the stored handle.
        termielle_app::bar::appbar::leave_bar_shell();
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
    if cli.restore_taskbar {
        run_taskbar_watchdog(cli.watchdog_parent_pid);
        return;
    }

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
            unsafe {
                unsafe extern "system" {
                    fn AttachConsole(dw_process_id: u32) -> windows::core::BOOL;
                }
                let _ = AttachConsole(u32::MAX);
            }
            eprintln!(
                "Another Termielle instance is already running. Close it before launching, or switch layout via the system tray icon."
            );
            return;
        }
    };

    let log = Arc::new(Mutex::new(BoundedLog::new(log_path(), 1_048_576, 3)));

    let mut config = load_config_with_log(&log, cli.config_path.as_deref());

    // The command line wins over the persisted file:
    if let Some(render) = cli.render {
        config.render = render;
    }
    let layout_cli_override =
        cli.layout.is_some() || cli.bar_pos.is_some() || cli.replace_taskbar.is_some();
    if let Some(layout) = cli.layout {
        config.island.layout = layout;
    }
    if let Some(pos) = cli.bar_pos {
        config.island.bar.position = pos;
    }
    if let Some(replace) = cli.replace_taskbar {
        config.island.bar.replace_taskbar = replace;
    }
    config.island.clamp();
    theme::resolve_theme(&mut config.island);

    if layout_cli_override {
        let _ = save_config_atomic(
            &data_dir()
                .map(|d| d.join("config.json"))
                .unwrap_or_else(|| PathBuf::from("config.json")),
            &config,
        );
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
    sync_taskbar_mode(&config.island);
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
        //
        // The interval is deliberately a millisecond *under* the frame time.
        // The pacer waits for this deadline and then aligns to the next
        // vertical blank, so a deadline that rounds up to 16.7 -> 17 lands
        // just past that blank and the overlay waits a whole extra frame: a
        // requested 60 ran at 30. Landing inside the frame targets the blank
        // the caller meant.
        config.frame_rate.map(|rate| {
            let interval = (1000u64 + u64::from(rate) / 2) / u64::from(rate);
            interval.saturating_sub(1).max(1)
        }),
        config.island.clone(),
    );
    // User zoom rides on top of monitor DPI inside `render_scale`.
    controller.set_user_scale(config.scale);
    // Author the first frame at the live scale: the journal replay below
    // renders immediately, before any present refreshes the DPI.
    if config.island.is_bar() && !config.island.bar.follow_active_monitor {
        window.pin_primary_monitor();
    } else {
        let _ = window.update_active_monitor();
    }
    controller.set_dpi_scale(window.anchor_dpi_scale());
    if config.island.is_bar() {
        let logical_w = (window.monitor_width() as f32 / controller.render_scale()).round() as u32;
        controller.set_bar_width(logical_w);
        if config.island.bar.reserve_space {
            let is_top = config.island.bar.position == termielle_core::BarPosition::Top;
            // SHAppBarMessage takes device pixels; the config height is logical.
            let height_px =
                (config.island.bar.height as f32 * controller.render_scale()).round() as u32;
            termielle_app::bar::appbar::ensure_appbar(
                window.hwnd(),
                is_top,
                height_px,
                window.monitor_bounds(),
            );
        }
    }
    controller.refresh_scale(now_ms());

    // Only the production overlay journals and replays: diagnostics runs on
    // custom pipes must neither inherit nor pollute the live state.
    let journal = if !cli.smoke_test && cli.pipe == DEFAULT_PIPE_NAME {
        Some(EventLog::new(events_path(), DEFAULT_EVENT_LOG_MAX_BYTES))
    } else {
        None
    };

    // Claim the endpoint before replay. A short retry absorbs the previous
    // process's pipe handle during an in-process tray restart.
    let Some(server) = bind_server_with_retry(&cli.pipe, &log) else {
        termielle_app::bar::appbar::leave_bar_shell();
        return;
    };

    let (sender, receiver) = channel();
    spawn_pipe_thread(server, wake, sender, log.clone());

    // Rebuild pre-restart state after the endpoint is live. Newly emitted
    // events queue behind replay and are folded in arrival order. Absolute-time
    // deadlines still make expired holds/stalls replay straight into Idle.
    if let Some(journal) = &journal {
        for event in journal.read_all() {
            controller.handle_event(event, now_ms());
        }
    }

    if cli.smoke_test {
        let builtin_event = cli.pipe == DEFAULT_PIPE_NAME;
        let code = run_smoke(&mut window, &mut controller, &receiver, &log, builtin_event);
        termielle_app::bar::appbar::leave_bar_shell();
        drop(instance);
        std::process::exit(code);
    }

    // Background dashboard worker: icon reads and media queries never run
    // on the GUI thread, so hovering and morphing never stall. The worker
    // exits when its receiver is dropped at shutdown.
    let thumb_cfg = std::sync::Arc::new(termielle_app::tasks::WorkerConfig::new(
        config.island.is_enabled(),
        config.island.has_widget("music"),
        (config.island.has_widget("tasks") && config.island.show_tasks)
            || (config.island.is_bar() && config.island.bar.replace_taskbar),
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
    let config_watcher = if cli.config_path.is_none() {
        data_dir().map(|dir| ConfigWatcher::spawn(dir.join("config.json"), wake))
    } else {
        None
    };

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
        config_watcher.as_ref(),
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

/// Starts the helper that hands the taskbar back if this process dies without
/// getting to its own teardown. Spawned at most once per process: a config
/// reload that re-enters replacement mode must not pile up watchers, and a
/// restart gets a fresh process (and therefore a fresh watchdog) anyway.
fn spawn_taskbar_watchdog() {
    static SPAWNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if SPAWNED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let pid = std::process::id().to_string();
    let _ = Command::new(exe)
        .args(["--restore-taskbar", "--watchdog-parent-pid"])
        .arg(pid)
        .spawn();
}

fn run_taskbar_watchdog(parent_pid: Option<u32>) {
    if let Some(pid) = parent_pid {
        wait_for_process_exit(pid);
        // Give a replacement instance time to claim the taskbar before the
        // watchdog restores it.
        std::thread::sleep(Duration::from_millis(1_200));
    }
    termielle_app::bar::appbar::restore_taskbar_force();
}

/// Blocks until `pid` exits without polling.
///
/// `WaitForSingleObject` on a `SYNCHRONIZE` handle parks the watchdog in the
/// kernel, so it costs nothing while the overlay runs. A parent that cannot be
/// opened with that right falls back to the process snapshot, which keeps its
/// conservative "assume alive" behavior.
fn wait_for_process_exit(pid: u32) {
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE_ACCESS, false, pid) }) else {
        while process_alive(pid) {
            std::thread::sleep(Duration::from_millis(200));
        }
        return;
    };
    // SAFETY: `handle` is a process-owned kernel handle valid for this call.
    unsafe {
        let _ = WaitForSingleObject(handle, WAIT_FOREVER);
        let _ = CloseHandle(handle);
    }
}

/// Applies the taskbar replacement mode of `config`: hide the native taskbar
/// while the bar owns it, hand it back the moment it does not, and keep a
/// watchdog alive for as long as we are the ones holding it.
fn sync_taskbar_mode(config: &termielle_core::IslandConfig) {
    if config.is_bar() && config.bar.replace_taskbar {
        termielle_app::bar::appbar::hide_taskbar();
        spawn_taskbar_watchdog();
    } else {
        termielle_app::bar::appbar::restore_taskbar();
    }
}

/// Cadence for re-asserting taskbar replacement. Explorer can re-show the
/// taskbar on its own (shell restart, taskbar re-creation, settings change)
/// and offers no notification we can rely on everywhere, so the overlay
/// re-checks on this slow tick. Two window lookups per interval.
const TASKBAR_REASSERT_MS: u64 = 2_000;

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

/// Watches the user config and posts a debounced wake after the file settles.
/// Validation happens on the GUI thread; the watcher never applies config.
struct ConfigWatcher {
    path: PathBuf,
    receiver: Receiver<()>,
    stop: Arc<AtomicBool>,
}

impl ConfigWatcher {
    fn spawn(path: PathBuf, wake: WakeHandle) -> Self {
        let (sender, receiver) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread_path = path.clone();
        std::thread::spawn(move || {
            let mut last = config_signature(&thread_path);
            while !thread_stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(200));
                let current = config_signature(&thread_path);
                if current == last {
                    continue;
                }
                // Editors commonly write in several steps. Wait for a quiet
                // window so a half-written JSON document is never applied.
                std::thread::sleep(Duration::from_millis(400));
                let settled = config_signature(&thread_path);
                if settled == current {
                    last = settled;
                    if sender.send(()).is_err() || wake.post().is_err() {
                        break;
                    }
                }
            }
        });
        Self {
            path,
            receiver,
            stop,
        }
    }

    fn take_pending(&self) -> bool {
        let mut pending = false;
        while self.receiver.try_recv().is_ok() {
            pending = true;
        }
        pending
    }
}

impl Drop for ConfigWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn config_signature(path: &Path) -> Option<(u64, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    Some((modified, metadata.len()))
}

/// Reads the config, falling back to defaults and one bounded record when the
/// file cannot be used.
fn load_config_with_log(log: &Arc<Mutex<BoundedLog>>, override_path: Option<&Path>) -> AppConfig {
    let path = override_path
        .map(Path::to_path_buf)
        .or_else(|| data_dir().map(|dir| dir.join("config.json")));
    let Some(path) = path else {
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

/// Applies a validated config file on the GUI thread. A bad or partially
/// written file leaves the running surface untouched.
fn reload_config(
    path: &Path,
    config: &mut AppConfig,
    controller: &mut Controller,
    window: &mut OverlayWindow,
    thumb_cfg: &Arc<termielle_app::tasks::WorkerConfig>,
    log: &Arc<Mutex<BoundedLog>>,
) -> bool {
    let mut next = match load_config(path) {
        Ok(next) => next,
        Err(error) => {
            log_error(
                log,
                LogComponent::Config,
                LogEvent::ReadFailed,
                config_error_code(&error),
            );
            return false;
        }
    };
    next.island.clamp();
    theme::resolve_theme(&mut next.island);
    if next.island == config.island {
        return false;
    }

    let was_bar = config.island.is_bar();
    let next_bar = next.island.is_bar();
    config.island = next.island;

    if was_bar && !next_bar {
        termielle_app::bar::appbar::leave_bar_shell();
    } else if !was_bar && next_bar && !config.island.bar.follow_active_monitor {
        window.pin_primary_monitor();
        controller.set_dpi_scale(window.anchor_dpi_scale());
    }
    if next_bar {
        let logical_w = (window.monitor_width() as f32 / controller.render_scale()).round() as u32;
        controller.set_bar_width(logical_w);
        if config.island.bar.reserve_space {
            let is_top = config.island.bar.position == BarPosition::Top;
            let height_px =
                (config.island.bar.height as f32 * controller.render_scale()).round() as u32;
            termielle_app::bar::appbar::ensure_appbar(
                window.hwnd(),
                is_top,
                height_px,
                window.monitor_bounds(),
            );
        }
    }

    controller.set_island_config(config.island.clone(), now_ms());
    window.set_island(config.island.is_enabled());
    window.set_bar(config.island.is_bar());
    window.set_glass(&config.island.glass);
    sync_taskbar_mode(&config.island);
    thumb_cfg.enabled.store(
        config.island.is_enabled(),
        std::sync::atomic::Ordering::Relaxed,
    );
    thumb_cfg.poll_media.store(
        config.island.has_widget("music"),
        std::sync::atomic::Ordering::Relaxed,
    );
    thumb_cfg.poll_tasks.store(
        (config.island.has_widget("tasks") && config.island.show_tasks)
            || (config.island.is_bar() && config.island.bar.replace_taskbar),
        std::sync::atomic::Ordering::Relaxed,
    );
    true
}

fn drain_config_reload(
    watcher: Option<&ConfigWatcher>,
    config: &mut AppConfig,
    controller: &mut Controller,
    window: &mut OverlayWindow,
    thumb_cfg: &Arc<termielle_app::tasks::WorkerConfig>,
    log: &Arc<Mutex<BoundedLog>>,
) -> bool {
    let Some(watcher) = watcher else {
        return false;
    };
    if !watcher.take_pending() {
        return false;
    }
    reload_config(&watcher.path, config, controller, window, thumb_cfg, log)
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

/// Binds the event endpoint, briefly retrying a just-exiting restart owner.
fn bind_server_with_retry(pipe: &str, log: &Arc<Mutex<BoundedLog>>) -> Option<EventServer> {
    for attempt in 0..8 {
        match EventServer::bind(pipe) {
            Ok(server) => return Some(server),
            Err(IpcError::PipeNameOwned) if attempt < 7 => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(IpcError::PipeNameOwned) => {
                log_error(log, LogComponent::Ipc, LogEvent::InvalidEvent, 3);
                return None;
            }
            Err(error) => {
                log_error(
                    log,
                    LogComponent::Ipc,
                    LogEvent::InvalidEvent,
                    ipc_error_code(&error),
                );
                return None;
            }
        }
    }
    None
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

/// The rect the glass should capture for this frame, in physical pixels.
///
/// The frosted material samples a captured image, and a sample outside that
/// image falls back to a flat tint. A surface that is mid-morph is therefore
/// flat in the area it is growing into unless the capture covers its whole
/// travel. The bar's popup envelope is permanent; an island only needs the
/// envelope while it can still be heading somewhere, so a settled pill keeps
/// its small rect. Pure, so that invariant is testable.
fn backdrop_request_rect(
    current: (i32, i32, u32, u32),
    target: Option<(i32, i32, u32, u32)>,
    is_bar: bool,
    bar_height: u32,
    bar_bottom: bool,
) -> (i32, i32, u32, u32) {
    let (x, y, w, h) = current;
    if is_bar {
        // The bar's popup is always one click away: capture the full envelope
        // up front so the blur kernel and screen origin stay fixed through
        // every morph.
        let envelope = bar_height.saturating_add(320).max(h);
        let y = if bar_bottom {
            y - envelope.saturating_sub(h) as i32
        } else {
            y
        };
        return (x, y, w, h.max(envelope));
    }
    match target {
        Some(target) => termielle_app::backdrop::cover_rect(current, target),
        None => current,
    }
}

/// The mutation half of one of Termielle's own setting toggles: no I/O, no
/// window, no controller. Both the tray menu and the control panel end up
/// here, so the field a toggle flips is the same field the other surface
/// reads. Returns false for events that are not setting toggles.
fn apply_setting(event: &WindowEvent, island: &mut termielle_core::IslandConfig) -> bool {
    match *event {
        WindowEvent::ToggleMusic => {
            if island.has_widget("music") {
                island.widgets.retain(|w| w != "music");
            } else {
                island.widgets.push("music".to_string());
            }
            true
        }
        WindowEvent::ToggleHoverExpand => {
            island.expand_on_hover = !island.expand_on_hover;
            true
        }
        WindowEvent::ToggleFace => {
            if island.has_widget("face") {
                island.widgets.retain(|w| w != "face");
            } else {
                island.widgets.push("face".to_string());
            }
            island.clamp();
            true
        }
        _ => false,
    }
}

/// Applies a setting toggle end to end: mutate the single config owner,
/// persist it, and mirror it into the controller.
fn apply_settings_event(
    event: WindowEvent,
    config: &mut AppConfig,
    controller: &mut Controller,
    thumb_cfg: &Arc<termielle_app::tasks::WorkerConfig>,
) -> Option<ControllerActions> {
    if !apply_setting(&event, &mut config.island) {
        return None;
    }
    if matches!(event, WindowEvent::ToggleMusic) {
        thumb_cfg.poll_media.store(
            config.island.has_widget("music"),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
    let _ = save_config_atomic(
        &data_dir()
            .map(|d| d.join("config.json"))
            .unwrap_or_else(|| PathBuf::from("config.json")),
        config,
    );
    controller.set_island_config(config.island.clone(), now_ms());
    Some(ControllerActions {
        present_frame: true,
        ..Default::default()
    })
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
    // Fresh DPI on every present: dragging the pill across monitors with
    // different DPIs tracks without an invalidation path (the DisplayChanged
    // repaint covers the transition frame).
    let target_dpi = if controller.is_island() {
        window.anchor_dpi_scale()
    } else {
        window.dpi_scale()
    };
    controller.set_dpi_scale(target_dpi);
    if controller.island_config().is_bar() {
        let logical_w = (window.monitor_width() as f32 / controller.render_scale()).round() as u32;
        controller.set_bar_width(logical_w);
        // Keep the shell reservation glued to the live geometry: DPI, zoom,
        // or monitor can change without an anchor flip, and a stale
        // reservation overlaps maximized windows. No-op when unchanged.
        if controller.island_config().bar.reserve_space {
            let bar = &controller.island_config().bar;
            let height_px = (bar.height as f32 * controller.render_scale()).round() as u32;
            termielle_app::bar::appbar::ensure_appbar(
                window.hwnd(),
                bar.position == termielle_core::BarPosition::Top,
                height_px,
                window.monitor_bounds(),
            );
        }
    }
    // The bar's modules are drawn on transparent glass, so without this the
    // only clickable pixels are the ones that got inked. The regions the
    // renderer declared are the layout's own slots, so they become the targets.
    if controller.island_config().is_bar() {
        window.set_hit_targets(controller.click_regions(), controller.render_scale());
    } else {
        window.set_hit_targets(&[], 1.0);
    }
    let anchor = controller.island_anchor();
    let attempt = if controller.island_config().is_bar() {
        window.present_with_bar(
            controller.current_frame(),
            controller.island_config().bar.position,
        )
    } else if let Some((attached, y_off)) = anchor {
        window.present_with_anchor(controller.current_frame(), Some((attached, y_off)))
    } else {
        window.present(controller.current_frame())
    };
    // Publish the glass capture request: the rect the pill was just drawn
    // at. The worker owns the (potentially slow) capture and blur.
    if let Some(request) = backdrop_request {
        let (x, y, w, h) = window.last_dest();
        if w > 1 {
            let cfg = controller.island_config();
            // The glass samples a captured image; a sample outside it falls
            // back to a flat tint. So the capture has to cover where the
            // surface is going, not only the rect that was just drawn, or the
            // glass goes flat in exactly the area a growing surface is
            // filling. `target` is None when the surface has settled, which
            // keeps a resting pill's capture small.
            let target = if controller.surface_can_grow() {
                let (tw, th) = controller.max_surface_size();
                let scale = controller.render_scale();
                Some(window.island_dest_for(
                    (tw as f32 * scale).ceil() as u32,
                    (th as f32 * scale).ceil() as u32,
                    anchor,
                ))
            } else {
                None
            };
            let (x, y, w, h) = backdrop_request_rect(
                (x, y, w, h),
                target,
                cfg.is_bar(),
                cfg.bar.height,
                cfg.bar.position == termielle_core::BarPosition::Bottom,
            );
            *request.lock().unwrap() = Some(termielle_app::tasks::BackdropRequest {
                x,
                y,
                w,
                h,
                radius: cfg.glass.blur_radius,
                tint: cfg.glass.tint,
                blur: cfg.glass.blur_radius > 0 && !(cfg.is_attached() && cfg.glass.notch_black),
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
            let retry = if controller.island_config().is_bar() {
                window.present_with_bar(
                    controller.current_frame(),
                    controller.island_config().bar.position,
                )
            } else if let Some((attached, y_off)) = retry_anchor {
                window.present_with_anchor(controller.current_frame(), Some((attached, y_off)))
            } else {
                window.present(controller.current_frame())
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
                    let _ = if controller.island_config().is_bar() {
                        window.present_with_bar(
                            controller.current_frame(),
                            controller.island_config().bar.position,
                        )
                    } else if let Some((attached, y_off)) = fallback_anchor {
                        window.present_with_anchor(
                            controller.current_frame(),
                            Some((attached, y_off)),
                        )
                    } else {
                        window.present(controller.current_frame())
                    };
                }
            }
        }
    }
}

/// Polls hover from the real cursor position and folds any transition into
/// `actions`. Backstop for spurious `WM_MOUSELEAVE`s across ULW resizes.
/// Also auto-dismisses the manually expanded card if the user clicks outside or presses Escape.
fn poll_hover(
    window: &mut OverlayWindow,
    controller: &mut Controller,
    actions: &mut ControllerActions,
) {
    let now = now_ms();
    if controller.island_config().is_bar()
        && controller.island_config().bar.follow_active_monitor
        && window.update_active_monitor()
    {
        let new_dpi = window.anchor_dpi_scale();
        controller.set_dpi_scale(new_dpi);
        let logical_w = (window.monitor_width() as f32 / controller.render_scale()).round() as u32;
        controller.set_bar_width(logical_w);
        // Reservation itself syncs in present_current (every present), which
        // also covers DPI/zoom changes without an anchor flip.
        actions.present_frame = true;
    }
    if controller.set_hover_point(window.cursor_client_pos()) {
        actions.present_frame = true;
    }
    // A bar window is mostly transparent and its hit map covers every opaque
    // pixel - module text, an open card - so "over an opaque pixel" would
    // open the card when the pointer merely crosses the clock. Hover needs the
    // pill's own rect.
    let over = if controller.island_config().is_bar() {
        // Ungated: the alpha-gated read answers None unless the cursor is over
        // an opaque pixel, and in a bar that is the wrong question.
        // The island's own surface, so an open panel does not widen it.
        window
            .cursor_frame_pos()
            .is_some_and(|point| controller.point_over_notch_surface(point))
    } else {
        window.cursor_over_pill()
    };
    if controller.set_hover(over, now) {
        actions.present_frame = true;
    }
    // A hover can arm a dwell or a grace without changing anything on screen,
    // so the schedule is re-read whether or not the frame did.
    actions.next_deadline_ms = controller.next_deadline_ms();
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON, VK_RBUTTON,
    };
    // Either surface counts as open. The panel no longer sets
    // `manually_expanded` - it is not the island's card - so gating
    // dismissal on that alone left it dismissible only by pressing its own
    // icon: no Escape, no click outside.
    if controller.is_manually_expanded() || controller.is_panel_open() {
        let l_click = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } < 0;
        let r_click = unsafe { GetAsyncKeyState(VK_RBUTTON.0 as i32) } < 0;
        let esc = unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) } < 0;
        if ((!over && (l_click || r_click)) || esc) && controller.collapse_if_expanded(now) {
            actions.present_frame = true;
            actions.next_deadline_ms = controller.next_deadline_ms();
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
            // Fresh glass with no repaint is invisible: force one so the new
            // backdrop actually reaches the screen.
            present = true;
        }
        if controller.set_task_update_at(batch, now_ms()) {
            present = true;
        }
    }
    present
}

/// Drains toast batches into alert banners. Returns whether any banner wants
/// a repaint.
fn drain_toasts(
    controller: &mut Controller,
    receiver: &Receiver<termielle_app::toast::ToastBatch>,
) -> bool {
    let mut present = false;
    while let Ok(batch) = receiver.try_recv() {
        for event in batch.events {
            let key = format!("toast:{}", event.id);
            present |= controller.trigger_alert(
                termielle_app::app::AlertKind::System,
                event.display_title(),
                event.display_subtitle(),
                termielle_app::system::accent_color_bgra(),
                6000,
                now_ms(),
                key,
            );
        }
    }
    present
}

/// Drains every queued pipe event into the controller, merging the actions.
///
/// The IPC endpoint is already live while events are folded, so startup work
/// cannot create a fail-open emitter race. Each event is still journaled before
/// application, making a crash between those steps recoverable.
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
    config_watcher: Option<&ConfigWatcher>,
    config: &mut AppConfig,
) -> bool {
    // The animation clock thread paces frames; the smoke test's timeout timer
    // is separate and unaffected.
    let clock = AnimationClock::spawn(window.wake_handle());
    let mut bar_service =
        termielle_app::bar::metrics::Service::spawn(window.wake_handle(), config.island.clone());
    controller.enable_display_pacing();
    // Windows toast forwarding: own STA thread plus channel. Gated on
    // config only — smoke runs never reach run_gui, so diagnostics stays
    // free of consent prompts.
    let toast_receiver = if config.island.forward_toasts {
        let (toast_sender, toast_receiver) =
            std::sync::mpsc::channel::<termielle_app::toast::ToastBatch>();
        let _toast_watcher =
            termielle_app::toast::spawn_toast_watcher(toast_sender, window.wake_handle());
        Some(toast_receiver)
    } else {
        None
    };
    let mut next_taskbar_reassert = now_ms().saturating_add(TASKBAR_REASSERT_MS);
    loop {
        // Only the display clock advances animation. Unrelated window/worker
        // messages must not bypass vertical-blank pacing.
        bar_service.configure(&config.island);
        if now_ms() >= next_taskbar_reassert {
            next_taskbar_reassert = now_ms().saturating_add(TASKBAR_REASSERT_MS);
            if config.island.is_bar() && config.island.bar.replace_taskbar {
                termielle_app::bar::appbar::ensure_taskbar_hidden();
            }
        }
        if let Some(snapshot) = bar_service.take_snapshot() {
            if controller.set_bar_metrics(snapshot, now_ms()) {
                present_current(
                    window,
                    controller,
                    log,
                    ack.as_mut(),
                    Some(backdrop_request),
                );
            }
        }
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
                    let toast_present = toast_receiver
                        .as_ref()
                        .map(|rx| drain_toasts(controller, rx))
                        .unwrap_or(false);
                    let mut actions = actions;
                    actions.present_frame = actions.present_frame || thumb_present || toast_present;
                    actions.present_frame |= drain_config_reload(
                        config_watcher,
                        config,
                        controller,
                        window,
                        thumb_cfg,
                        log,
                    );
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
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
        };

        let mut actions = match event {
            WindowEvent::Timer | WindowEvent::AnimationFrame => controller.on_timer(now_ms()),
            WindowEvent::DisplayChanged => {
                if controller.island_config().is_bar()
                    && !controller.island_config().bar.follow_active_monitor
                {
                    // Pinned bar: re-resolve the primary (it may have moved)
                    // instead of dropping the anchor to a cursor pick.
                    window.pin_primary_monitor();
                } else {
                    // The wndproc re-clamped classic rects; the island drops its
                    // tracked monitor so the repaint re-picks the display the
                    // cursor is on, then SetWindowPos re-anchors top-center there.
                    window.reset_anchor_monitor();
                }
                // A monitor change can move the pill across DPIs: re-author
                // the frame at the new scale before presenting it.
                controller.set_dpi_scale(window.anchor_dpi_scale());
                controller.refresh_scale(now_ms());
                ControllerActions {
                    present_frame: true,
                    ..Default::default()
                }
            }
            WindowEvent::Quit => {
                termielle_app::bar::appbar::leave_bar_shell();
                window.destroy();
                return false;
            }
            WindowEvent::Restart => {
                termielle_app::bar::appbar::leave_bar_shell();
                window.destroy();
                return true;
            }
            WindowEvent::LayoutChanged(layout) => {
                let was_bar = config.island.is_bar();
                config.island.layout = layout;
                config.island.clamp();
                if was_bar && !config.island.is_bar() {
                    termielle_app::bar::appbar::leave_bar_shell();
                } else if !was_bar && config.island.is_bar() {
                    if !config.island.bar.follow_active_monitor {
                        window.pin_primary_monitor();
                        controller.set_dpi_scale(window.anchor_dpi_scale());
                    }
                    let logical_w =
                        (window.monitor_width() as f32 / controller.render_scale()).round() as u32;
                    controller.set_bar_width(logical_w);
                    if config.island.bar.reserve_space {
                        let is_top = config.island.bar.position == termielle_core::BarPosition::Top;
                        let height_px = (config.island.bar.height as f32
                            * controller.render_scale())
                        .round() as u32;
                        termielle_app::bar::appbar::ensure_appbar(
                            window.hwnd(),
                            is_top,
                            height_px,
                            window.monitor_bounds(),
                        );
                    }
                }
                let _ = save_config_atomic(
                    &data_dir()
                        .map(|d| d.join("config.json"))
                        .unwrap_or_else(|| PathBuf::from("config.json")),
                    config,
                );
                theme::resolve_theme(&mut config.island);
                controller.set_island_config(config.island.clone(), now_ms());
                window.set_island(config.island.is_enabled());
                window.set_bar(config.island.is_bar());
                window.set_glass(&config.island.glass);
                sync_taskbar_mode(&config.island);
                thumb_cfg.enabled.store(
                    config.island.is_enabled(),
                    std::sync::atomic::Ordering::Relaxed,
                );
                thumb_cfg.poll_media.store(
                    config.island.has_widget("music"),
                    std::sync::atomic::Ordering::Relaxed,
                );
                thumb_cfg.poll_tasks.store(
                    (config.island.has_widget("tasks") && config.island.show_tasks)
                        || (config.island.is_bar() && config.island.bar.replace_taskbar),
                    std::sync::atomic::Ordering::Relaxed,
                );
                ControllerActions {
                    present_frame: true,
                    ..Default::default()
                }
            }
            WindowEvent::ThemeChanged(theme) => {
                theme::select_theme(&mut config.island, &theme);
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
            WindowEvent::BarPositionChanged(position) => {
                config.island.bar.position = position;
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
            WindowEvent::ScrollAt(x, y, delta) => {
                // The panel's volume row answers the wheel exactly like the
                // bar's speaker, through the same worker command.
                if delta != 0 && (controller.volume_at(x, y) || controller.panel_volume_at(x, y)) {
                    bar_service.send(termielle_app::bar::metrics::Command::VolumeWheel(delta));
                }
                ControllerActions::default()
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
                termielle_app::app::ClickOutcome::WorkspaceSwitch(idx) => {
                    bar_service.send(termielle_app::bar::metrics::Command::Workspace(
                        idx as usize,
                    ));
                    ControllerActions::default()
                }
                termielle_app::app::ClickOutcome::VolumeToggle => {
                    bar_service.send(termielle_app::bar::metrics::Command::ToggleMute);
                    ControllerActions::default()
                }
                termielle_app::app::ClickOutcome::VolumeSet(level) => {
                    bar_service.send(termielle_app::bar::metrics::Command::SetVolume(level));
                    ControllerActions {
                        present_frame: true,
                        ..Default::default()
                    }
                }
                termielle_app::app::ClickOutcome::PanelToggled => ControllerActions {
                    present_frame: true,
                    ..Default::default()
                },
                termielle_app::app::ClickOutcome::PanelToggle(setting) => {
                    // The panel sends the same commands the tray menu sends,
                    // so one owner (this config) serves both surfaces and the
                    // menu's checkmarks can never disagree with the panel.
                    let event = match setting {
                        termielle_app::app::PanelToggle::HoverExpand => {
                            WindowEvent::ToggleHoverExpand
                        }
                        termielle_app::app::PanelToggle::Face => WindowEvent::ToggleFace,
                        termielle_app::app::PanelToggle::Music => WindowEvent::ToggleMusic,
                    };
                    apply_settings_event(event, config, controller, thumb_cfg).unwrap_or_default()
                }
                termielle_app::app::ClickOutcome::Shell(action) => {
                    termielle_app::bar::shell::activate(action);
                    ControllerActions::default()
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
            WindowEvent::ToggleMusic | WindowEvent::ToggleHoverExpand | WindowEvent::ToggleFace => {
                apply_settings_event(
                    match event {
                        WindowEvent::ToggleMusic => WindowEvent::ToggleMusic,
                        WindowEvent::ToggleHoverExpand => WindowEvent::ToggleHoverExpand,
                        _ => WindowEvent::ToggleFace,
                    },
                    config,
                    controller,
                    thumb_cfg,
                )
                .unwrap_or_default()
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
        window.set_tray_state(termielle_app::tray::MenuState::from_island(&config.island));
        actions = merge(actions, drain_pipe(controller, receiver, journal));
        let thumbs = drain_thumbs(window, controller, thumb_receiver);
        let toasts = toast_receiver
            .as_ref()
            .map(|rx| drain_toasts(controller, rx))
            .unwrap_or(false);
        if thumbs || toasts {
            actions.present_frame = true;
            actions.next_deadline_ms = controller.next_deadline_ms();
        }
        actions.present_frame |=
            drain_config_reload(config_watcher, config, controller, window, thumb_cfg, log);
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
    controller.set_dpi_scale(window.dpi_scale());
    let scale = controller.render_scale();
    for state in ALL_STATES {
        let frame = fallback_frame(state, FALLBACK_FRAME_SIZE, scale);
        if let Err(error) = window.present(&frame) {
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
            Ok(Some(WindowEvent::Timer | WindowEvent::AnimationFrame)) => {
                window.destroy();
                return 1;
            }
            Ok(Some(WindowEvent::DisplayChanged)) => {}
            Ok(Some(WindowEvent::Quit | WindowEvent::Restart)) => {}
            Ok(Some(
                WindowEvent::LayoutChanged(_)
                | WindowEvent::ThemeChanged(_)
                | WindowEvent::YOffsetChanged(_)
                | WindowEvent::BarPositionChanged(_)
                | WindowEvent::SystemThemeChanged
                | WindowEvent::ClickAt(..)
                | WindowEvent::ScrollAt(..)
                | WindowEvent::PressChanged(_)
                | WindowEvent::HoverChanged(_)
                | WindowEvent::ToggleMusic
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

#[cfg(test)]
mod settings_tests {
    use super::*;
    use termielle_app::app::PanelToggle;

    /// The panel's rows and the tray menu are two surfaces over one config.
    /// Each toggle must flip exactly the field the tray menu renders, so the
    /// two can never show different states for the same setting.
    /// The glass samples a captured image, and anything outside it falls back
    /// to a flat tint. A surface mid-morph must therefore be covered for its
    /// whole travel, or the area it is growing into shows flat colour.
    #[test]
    fn backdrop_capture_covers_a_morphing_surface() {
        let current = (700, 0, 140, 36);
        let target = (600, 0, 340, 210);
        let (x, y, w, h) = backdrop_request_rect(current, Some(target), false, 0, false);
        for (rect, label) in [(current, "current"), (target, "target")] {
            assert!(
                rect.0 >= x
                    && rect.1 >= y
                    && rect.0 + rect.2 as i32 <= x + w as i32
                    && rect.1 + rect.3 as i32 <= y + h as i32,
                "the capture must hold the {label} rect: {:?} vs {rect:?}",
                (x, y, w, h)
            );
        }
    }

    #[test]
    fn a_settled_island_keeps_its_small_capture() {
        let current = (700, 0, 140, 36);
        assert_eq!(
            backdrop_request_rect(current, None, false, 0, false),
            current,
            "a resting pill must not pay for an envelope it cannot reach"
        );
    }

    #[test]
    fn the_bar_keeps_its_popup_envelope_always() {
        let strip = (0, 0, 1920, 36);
        let (x, y, w, h) = backdrop_request_rect(strip, None, true, 36, false);
        assert_eq!((x, y, w), (0, 0, 1920));
        assert!(
            h >= 36 + 320,
            "the bar's popup needs room inside the capture"
        );

        // A bottom bar's strip sits at the screen's bottom edge, so its
        // envelope grows upward and must not push the strip off-screen.
        let bottom_strip = (0, 1044, 1920, 36);
        let (_, y, _, h) = backdrop_request_rect(bottom_strip, None, true, 36, true);
        assert_eq!(y + h as i32, bottom_strip.1 + bottom_strip.3 as i32);
        assert!(y < bottom_strip.1, "the envelope grows upward");
    }

    #[test]
    fn panel_toggles_flip_the_fields_the_tray_menu_renders() {
        for (setting, event) in [
            (PanelToggle::HoverExpand, WindowEvent::ToggleHoverExpand),
            (PanelToggle::Face, WindowEvent::ToggleFace),
            (PanelToggle::Music, WindowEvent::ToggleMusic),
        ] {
            let mut island = termielle_core::IslandConfig::default();
            let before = termielle_app::tray::MenuState::from_island(&island);
            assert!(apply_setting(&event, &mut island), "{setting:?} must apply");
            let after = termielle_app::tray::MenuState::from_island(&island);
            match setting {
                PanelToggle::HoverExpand => assert_ne!(before.hover, after.hover),
                PanelToggle::Face => assert_ne!(before.face, after.face),
                PanelToggle::Music => assert_ne!(before.music, after.music),
            }
            // A second application lands back where it started, so the
            // panel row and the menu item are both true toggles.
            assert!(apply_setting(&event, &mut island));
            let round_trip = termielle_app::tray::MenuState::from_island(&island);
            match setting {
                PanelToggle::HoverExpand => assert_eq!(round_trip.hover, before.hover),
                PanelToggle::Face => assert_eq!(round_trip.face, before.face),
                PanelToggle::Music => assert_eq!(round_trip.music, before.music),
            }
        }
    }

    #[test]
    fn unrelated_events_are_not_setting_toggles() {
        let island = termielle_core::IslandConfig::default();
        let mut untouched = island.clone();
        assert!(
            !apply_setting(&WindowEvent::SystemThemeChanged, &mut untouched),
            "only setting toggles may mutate config here"
        );
        assert_eq!(untouched.expand_on_hover, island.expand_on_hover);
    }
}

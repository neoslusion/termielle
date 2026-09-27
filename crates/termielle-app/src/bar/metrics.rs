//! Serial shell/audio worker. Renderers consume values, never COM proxies.

use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use termielle_core::IslandConfig;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub workspaces: super::workspaces::WorkspaceSnapshot,
    pub window_title: String,
    pub time_str: String,
    pub battery: (Option<u8>, bool, bool),
    pub volume: super::volume::VolumeSnapshot,
    pub memory_pct: u8,
    pub cpu_pct: u8,
}

impl Snapshot {
    pub(crate) fn empty() -> Self {
        Self {
            workspaces: super::workspaces::WorkspaceSnapshot {
                total: 0,
                active: 0,
            },
            window_title: String::new(),
            time_str: String::new(),
            battery: (None, false, false),
            volume: super::volume::VolumeSnapshot {
                level: 0,
                muted: true,
            },
            memory_pct: 0,
            cpu_pct: 0,
        }
    }
}

impl Default for Snapshot {
    fn default() -> Self {
        Self::empty()
    }
}

pub enum Command {
    Configure(Box<IslandConfig>),
    VolumeWheel(i16),
    ToggleMute,
    Workspace(usize),
    /// Set an absolute level. The panel's steppers send this instead of a
    /// delta, so the target matches the level the user last saw.
    SetVolume(u8),
}

pub struct Service {
    sender: mpsc::Sender<Command>,
    latest: Arc<Mutex<Option<Snapshot>>>,
    config: IslandConfig,
}

impl Service {
    pub fn spawn(wake: crate::window::WakeHandle, config: IslandConfig) -> Self {
        let (sender, receiver) = mpsc::channel();
        let latest = Arc::new(Mutex::new(None));
        let mailbox = latest.clone();
        let initial = config.clone();
        std::thread::spawn(move || {
            let mut config = initial;
            let mut previous = Snapshot::empty();
            let mut wheel_remainder = 0i32;
            loop {
                // Audio and virtual desktop proxies are acquired and consumed
                // on this worker; none cross into the UI apartment.
                let mut fresh = previous.clone();
                if config.is_bar() {
                    let has = |zone: &[String], name| zone.iter().any(|m| m == name);
                    if has(&config.bar.modules_left, "workspaces") {
                        fresh.workspaces = super::workspaces::query_workspaces();
                    }
                    if has(&config.bar.modules_left, "window") {
                        fresh.window_title = crate::media::foreground_title();
                    }
                    if has(&config.bar.modules_right, "clock") {
                        fresh.time_str = crate::system::current_time_text();
                    }
                    if has(&config.bar.modules_right, "battery") {
                        fresh.battery = crate::system::battery_status();
                    }
                    // The control panel shows a level whether or not the
                    // bar lists a volume module, so bar mode always polls it.
                    if has(&config.bar.modules_right, "volume") || config.is_bar() {
                        if let Ok(volume) = super::volume::try_query_volume() {
                            fresh.volume = volume;
                        }
                    }
                    if has(&config.bar.modules_right, "memory") {
                        fresh.memory_pct = crate::system::mem_percent();
                    }
                    if has(&config.bar.modules_right, "cpu") {
                        fresh.cpu_pct = crate::system::cpu_percent();
                    }
                }
                if fresh != previous {
                    previous = fresh.clone();
                    *mailbox.lock().unwrap() = Some(fresh);
                    if wake.post().is_err() {
                        break;
                    }
                }
                let first = if config.is_bar() {
                    match receiver.recv_timeout(Duration::from_secs(2)) {
                        Ok(command) => command,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                } else {
                    match receiver.recv() {
                        Ok(command) => command,
                        Err(_) => break,
                    }
                };
                let mut wheel = 0i32;
                // Only adjacent wheel messages coalesce: a mute followed by
                // wheel input must still unmute, while the reverse must mute.
                for command in std::iter::once(first).chain(receiver.try_iter().take(63)) {
                    if let Command::VolumeWheel(delta) = command {
                        wheel += i32::from(delta);
                        continue;
                    }
                    apply_wheel(&mut wheel_remainder, wheel);
                    wheel = 0;
                    match command {
                        Command::Configure(next) => config = *next,
                        Command::ToggleMute => {
                            if let Err(error) = super::volume::toggle_mute() {
                                eprintln!("volume toggle failed: {error}");
                            }
                        }
                        Command::SetVolume(level) => {
                            if let Err(error) = super::volume::set_volume(level) {
                                eprintln!("volume set failed: {error}");
                            }
                        }
                        Command::Workspace(index) => {
                            if let Err(error) = super::workspaces::switch_workspace(index) {
                                eprintln!("workspace switch failed: {error:?}");
                            }
                        }
                        Command::VolumeWheel(_) => unreachable!(),
                    }
                }
                apply_wheel(&mut wheel_remainder, wheel);
            }
        });
        Self {
            sender,
            latest,
            config,
        }
    }

    pub fn configure(&mut self, config: &IslandConfig) {
        if self.config != *config {
            self.config = config.clone();
            self.send(Command::Configure(Box::new(config.clone())));
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.sender.send(command);
    }

    pub fn take_snapshot(&self) -> Option<Snapshot> {
        self.latest.lock().ok()?.take()
    }
}

fn wheel_steps(remainder: &mut i32, delta: i32) -> i32 {
    *remainder += delta;
    let steps = *remainder / 120;
    *remainder %= 120;
    steps
}

fn apply_wheel(remainder: &mut i32, delta: i32) {
    let steps = wheel_steps(remainder, delta);
    if steps == 0 {
        return;
    }
    let result = super::volume::try_query_volume().and_then(|volume| {
        super::volume::set_volume((i32::from(volume.level) + 2 * steps).clamp(0, 100) as u8)
    });
    if let Err(error) = result {
        eprintln!("volume adjustment failed: {error}");
    }
}

#[test]
fn partial_wheel_detents_accumulate_and_reverse_without_drift() {
    let mut remainder = 0;
    assert_eq!(wheel_steps(&mut remainder, 30), 0);
    assert_eq!(wheel_steps(&mut remainder, 60), 0);
    assert_eq!(wheel_steps(&mut remainder, 30), 1);
    assert_eq!(remainder, 0);
    assert_eq!(wheel_steps(&mut remainder, -180), -1);
    assert_eq!(wheel_steps(&mut remainder, 60), 0);
    assert_eq!(remainder, 0);
}

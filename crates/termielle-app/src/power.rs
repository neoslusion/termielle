//! Per-user, persistent whole-application power switch, independent of profiles.
//! No resident controller while off. The logon task can still run, but startup
//! exits cleanly before claiming IPC, creating windows or starting workers.
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io,
    path::Path,
    thread::{self, JoinHandle},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
        System::Threading::{CreateEventW, ResetEvent, SetEvent, WaitForMultipleObjects},
    },
    core::PCWSTR,
};

pub const DISABLED_FILE: &str = "disabled";

/// Missing marker means enabled; permission/I/O failures must not silently
/// pretend that a deliberate off preference was absent.
pub fn is_disabled(directory: &Path) -> io::Result<bool> {
    directory.join(DISABLED_FILE).try_exists()
}

struct Event(HANDLE);
// Windows kernel event handles support operations across threads and have no
// thread affinity. The owning wrapper closes only after its waiter has joined.
unsafe impl Send for Event {}
unsafe impl Sync for Event {}
impl Event {
    fn handle(&self) -> HANDLE {
        self.0
    }
}
impl Drop for Event {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
fn named_event(directory: &Path) -> io::Result<Event> {
    // Per-user absolute directory separates users and isolated test profiles;
    // use Windows case-insensitive spelling for callers of the same directory.
    let mut hash = DefaultHasher::new();
    directory.to_string_lossy().to_lowercase().hash(&mut hash);
    let name: Vec<u16> = format!("Local\\TermiellePower-{:016x}", hash.finish())
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe { CreateEventW(None, true, false, PCWSTR(name.as_ptr())) }
        .map(Event)
        .map_err(io::Error::from)
}

/// Writes only the separate off marker, never config.json, hooks or task settings.
/// Turning on clears the marker. Turning off signals every cooperating instance
/// for this user's data directory to take its normal graceful shutdown path.
pub fn set_enabled(directory: &Path, enabled: bool) -> io::Result<()> {
    let event = named_event(directory)?;
    let marker = directory.join(DISABLED_FILE);
    if enabled {
        match std::fs::remove_file(&marker) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        unsafe { ResetEvent(event.0) }.map_err(io::Error::from)?;
    } else {
        std::fs::create_dir_all(directory)?;
        // Empty-file presence is atomic; flush before asking a running app to
        // exit so a later normal startup will honor the persistent opt-out.
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(marker)?
            .sync_all()?;
        unsafe { SetEvent(event.0) }.map_err(io::Error::from)?;
    }
    Ok(())
}

/// A blocked kernel-event waiter: no polling CPU cost, no agent-event protocol
/// changes. Its private cancellation event never disables another instance.
pub struct Watch {
    stop: std::sync::Arc<Event>,
    thread: Option<JoinHandle<()>>,
}
impl Watch {
    pub fn spawn(directory: &Path, on_disable: impl FnOnce() + Send + 'static) -> io::Result<Self> {
        let power = named_event(directory)?;
        let stop = std::sync::Arc::new(Event(
            unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(io::Error::from)?,
        ));
        // Cover disable between the startup check and event registration.
        if is_disabled(directory)? {
            unsafe { SetEvent(power.0) }.map_err(io::Error::from)?;
        }
        let thread_stop = stop.clone();
        let thread = thread::Builder::new()
            .name("termielle-power".into())
            .spawn(move || {
                let result = unsafe {
                    WaitForMultipleObjects(&[thread_stop.handle(), power.handle()], false, u32::MAX)
                };
                if result.0 == WAIT_OBJECT_0.0 + 1 {
                    on_disable();
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        let _ = unsafe { SetEvent(self.stop.0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};
    #[test]
    fn marker_is_persistent_and_does_not_rewrite_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.json");
        std::fs::write(&config, b"custom bytes including unknown fields").unwrap();
        assert!(!is_disabled(dir.path()).unwrap());
        set_enabled(dir.path(), false).unwrap();
        assert!(is_disabled(dir.path()).unwrap());
        set_enabled(dir.path(), false).unwrap();
        set_enabled(dir.path(), true).unwrap();
        set_enabled(dir.path(), true).unwrap();
        assert!(!is_disabled(dir.path()).unwrap());
        assert_eq!(
            std::fs::read(config).unwrap(),
            b"custom bytes including unknown fields"
        );
    }
    #[test]
    fn signal_wakes_all_instances_and_isolated_directories_stay_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let tx2 = tx.clone();
        let a = Watch::spawn(dir.path(), move || {
            tx.send(()).unwrap();
        })
        .unwrap();
        let b = Watch::spawn(dir.path(), move || {
            tx2.send(()).unwrap();
        })
        .unwrap();
        let (other_tx, other_rx) = mpsc::channel();
        let c = Watch::spawn(other.path(), move || {
            let _ = other_tx.send(());
        })
        .unwrap();
        set_enabled(dir.path(), false).unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(other_rx.try_recv().is_err());
        drop((a, b, c));
        assert!(other_rx.try_recv().is_err());
        assert!(!is_disabled(other.path()).unwrap());
    }
    #[test]
    fn enable_rearms_the_signal_for_the_next_off_cycle() {
        let dir = tempfile::tempdir().unwrap();
        set_enabled(dir.path(), false).unwrap();
        set_enabled(dir.path(), true).unwrap();
        let (tx, rx) = mpsc::channel();
        let watch = Watch::spawn(dir.path(), move || {
            tx.send(()).unwrap();
        })
        .unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(30)).is_err());
        set_enabled(dir.path(), false).unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(watch);
    }

    #[test]
    fn late_registration_observes_disabled_marker() {
        let dir = tempfile::tempdir().unwrap();
        set_enabled(dir.path(), false).unwrap();
        let (tx, rx) = mpsc::channel();
        let watch = Watch::spawn(dir.path(), move || {
            tx.send(()).unwrap();
        })
        .unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(watch);
    }
    #[test]
    fn failed_enable_keeps_off_preference_and_failed_disable_keeps_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(DISABLED_FILE)).unwrap();
        assert!(set_enabled(dir.path(), true).is_err());
        assert!(is_disabled(dir.path()).unwrap());
        let file = dir.path().join("not-a-directory");
        std::fs::write(&file, b"keep").unwrap();
        assert!(set_enabled(&file, false).is_err());
        assert_eq!(std::fs::read(file).unwrap(), b"keep");
    }
}

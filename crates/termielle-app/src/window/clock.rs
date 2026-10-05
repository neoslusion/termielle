//! One-shot deadlines paced by the overlay monitor's vertical blank.

use super::{WakeHandle, now_ms};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};

/// Requested system timer period, in milliseconds.
///
/// Without this the pre-wait is quantised to the system timer, which defaults
/// to 15.6 ms: a 17 ms deadline therefore expired on the *second* tick and
/// the overlay presented every other refresh, halving the animation rate.
/// `WaitForVBlank` still does the real alignment - this only stops the wait
/// from overshooting into the next frame before it starts.
const TIMER_PERIOD_MS: u32 = 1;

enum Command {
    Arm(Option<u64>),
    InvalidateOutput,
}

pub struct AnimationClock {
    sender: Sender<Command>,
}

fn find_output(factory: &IDXGIFactory1, monitor: usize) -> Option<IDXGIOutput> {
    let mut adapter_index = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(adapter_index) } {
        let mut output_index = 0;
        while let Ok(output) = unsafe { adapter.EnumOutputs(output_index) } {
            if let Ok(desc) = unsafe { output.GetDesc() } {
                if desc.Monitor.0 as usize == monitor && desc.AttachedToDesktop.as_bool() {
                    return Some(output);
                }
            }
            output_index += 1;
        }
        adapter_index += 1;
    }
    None
}

impl AnimationClock {
    pub fn spawn(wake: WakeHandle) -> Self {
        let (sender, receiver) = mpsc::channel::<Command>();
        let hwnd = wake.hwnd.0 as usize;
        std::thread::spawn(move || {
            unsafe { timeBeginPeriod(TIMER_PERIOD_MS) };
            let mut deadline: Option<u64> = None;
            let mut factory: Option<IDXGIFactory1> = None;
            let mut output = None;
            let mut monitor = 0;
            let mut retry = Instant::now();
            loop {
                let command = match deadline {
                    Some(at) => {
                        receiver.recv_timeout(Duration::from_millis(at.saturating_sub(now_ms())))
                    }
                    None => match receiver.recv() {
                        Ok(value) => Ok(value),
                        Err(_) => break,
                    },
                };
                match command {
                    Ok(Command::Arm(value)) => {
                        deadline = value;
                        continue;
                    }
                    Ok(Command::InvalidateOutput) => {
                        factory = None;
                        output = None;
                        monitor = 0;
                        retry = Instant::now();
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {}
                }
                let current_monitor =
                    unsafe { MonitorFromWindow(HWND(hwnd as *mut _), MONITOR_DEFAULTTONEAREST) }.0
                        as usize;
                let stale = factory
                    .as_ref()
                    .is_some_and(|f| !unsafe { f.IsCurrent() }.as_bool());
                if monitor != current_monitor
                    || stale
                    || (output.is_none() && Instant::now() >= retry)
                {
                    monitor = current_monitor;
                    factory = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }.ok();
                    output = factory.as_ref().and_then(|f| find_output(f, monitor));
                    retry = Instant::now() + Duration::from_secs(1);
                }
                let synced = output
                    .as_ref()
                    .is_some_and(|o: &IDXGIOutput| unsafe { o.WaitForVBlank() }.is_ok());
                if !synced {
                    output = None;
                    // Remote/secure desktops may expose no DXGI output. Remain
                    // interruptible and avoid a hot retry loop in that case.
                    match receiver.recv_timeout(Duration::from_millis(16)) {
                        Ok(Command::Arm(value)) => {
                            deadline = value;
                            continue;
                        }
                        Ok(Command::InvalidateOutput) => {
                            factory = None;
                            monitor = 0;
                            retry = Instant::now();
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                }
                // Disarm before posting: each arm produces at most one wake.
                deadline = None;
                if wake.post_animation().is_err() {
                    break;
                }
            }
            unsafe { timeEndPeriod(TIMER_PERIOD_MS) };
        });
        Self { sender }
    }

    /// A new command interrupts an older wait, including an idle wait.
    pub fn arm(&self, deadline: Option<u64>) {
        let _ = self.sender.send(Command::Arm(deadline));
    }

    /// Re-resolve the output after display topology/resume, even when Windows
    /// reused the HMONITOR and DXGI still reports the old factory as current.
    /// GUI recovery uses its own Win32 timer and never waits on this worker.
    pub fn invalidate_output(&self) {
        let _ = self.sender.send(Command::InvalidateOutput);
    }
}

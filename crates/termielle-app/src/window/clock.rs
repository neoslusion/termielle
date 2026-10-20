//! One-shot deadlines paced by the overlay monitor's vertical blank.

use super::{ANIMATION_MSG, WakeHandle, now_ms};
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
    /// The overlay's clock: wakes the overlay with its own animation message.
    pub fn spawn(wake: WakeHandle) -> Self {
        Self::spawn_with(wake, ANIMATION_MSG)
    }

    /// A clock for another window: it wakes `wake` with `message` once per
    /// display refresh.
    ///
    /// A `WM_TIMER` cannot do this. Windows delivers timer messages through
    /// the window's own message queue and drops them when that queue is
    /// busy, so a timer-driven animation stutters exactly when the window
    /// has work to do — while the user is typing in the launcher, for
    /// instance. This thread waits for the display's vertical blank and
    /// posts a message, so the frame lands on the refresh no matter what
    /// else the queue is carrying.
    pub fn spawn_with(wake: WakeHandle, message: u32) -> Self {
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
                if wake.post_message(message).is_err() {
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

    /// Arms the clock for the next display refresh.
    ///
    /// `delay_ms` is measured from now, in milliseconds; the wake still
    /// lands on a vertical blank, so the delay only says how long the clock
    /// may wait before it starts looking for one.
    pub fn arm_after(&self, delay_ms: u64) {
        self.arm(Some(now_ms().saturating_add(delay_ms)));
    }

    /// Re-resolve the output after display topology/resume, even when Windows
    /// reused the HMONITOR and DXGI still reports the old factory as current.
    /// GUI recovery uses its own Win32 timer and never waits on this worker.
    pub fn invalidate_output(&self) {
        let _ = self.sender.send(Command::InvalidateOutput);
    }
}

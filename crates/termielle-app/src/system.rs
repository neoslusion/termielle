//! System stats for the idle island: time, battery, memory, CPU.

use std::sync::atomic::{AtomicU64, Ordering};

/// Snapshot of system stats shown in the expanded idle pill.
#[derive(Clone, Debug, Default)]
pub struct SystemStats {
    pub time: String,        // "14:30"
    pub battery: Option<u8>, // 0-100
    pub battery_charging: bool,
    pub mem_percent: u8, // 0-100
    pub cpu_percent: u8, // 0-100 (smoothed)
}

static LAST_CPU_IDLE: AtomicU64 = AtomicU64::new(0);
static LAST_CPU_TOTAL: AtomicU64 = AtomicU64::new(0);
static LAST_CPU_SAMPLE_MS: AtomicU64 = AtomicU64::new(0);
/// Smoothed CPU percentage from [`cpu_percent`], so the readout glides like
/// Task Manager instead of jumping between raw instantaneous samples.
static LAST_CPU_PCT: AtomicU64 = AtomicU64::new(0);

/// Exponential moving average factor for the CPU readout.
const CPU_SMOOTHING: f64 = 0.35;

pub fn current_time_text() -> String {
    let st = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:02}:{:02}", st.wHour, st.wMinute)
}

pub fn battery_status() -> (Option<u8>, bool) {
    unsafe {
        let mut s = windows::Win32::System::Power::SYSTEM_POWER_STATUS::default();
        if windows::Win32::System::Power::GetSystemPowerStatus(&mut s).is_ok() {
            let pct = if s.BatteryLifePercent == 255 {
                None
            } else {
                Some(s.BatteryLifePercent)
            };
            let charging = (s.BatteryFlag & 8) != 0 || s.ACLineStatus == 1;
            (pct, charging)
        } else {
            (None, false)
        }
    }
}

pub fn mem_percent() -> u8 {
    unsafe {
        let mut info = windows::Win32::System::SystemInformation::MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<windows::Win32::System::SystemInformation::MEMORYSTATUSEX>(
            ) as u32,
            ..Default::default()
        };
        if windows::Win32::System::SystemInformation::GlobalMemoryStatusEx(&mut info).is_ok() {
            info.dwMemoryLoad as u8
        } else {
            0
        }
    }
}

pub fn cpu_percent() -> u8 {
    // Cheap sample via GetSystemTimes, exponentially smoothed so the readout
    // glides instead of jumping between raw instantaneous samples.
    let instantaneous = sample_cpu();
    let Some(instantaneous) = instantaneous else {
        // No fresh sample yet (or the call failed): hold the last value
        // instead of flashing 0.
        return LAST_CPU_PCT.load(Ordering::Relaxed).min(100) as u8;
    };
    let previous = LAST_CPU_PCT.load(Ordering::Relaxed) as f64;
    let first = LAST_CPU_SAMPLE_MS.load(Ordering::Relaxed) == 0;
    // `LAST_CPU_SAMPLE_MS` doubles as the have-sample flag; `sample_cpu`
    // stores it before returning `Some`.
    let smoothed = if first {
        instantaneous as f64
    } else {
        previous + CPU_SMOOTHING * (instantaneous as f64 - previous)
    };
    LAST_CPU_PCT.store(smoothed.round() as u64, Ordering::Relaxed);
    smoothed.round().min(100.0) as u8
}

/// One raw CPU-busy sample, or `None` when called again too soon after the
/// previous sample (under 400 ms there is not enough delta to measure).
fn sample_cpu() -> Option<u8> {
    unsafe {
        let mut idle = windows::Win32::Foundation::FILETIME::default();
        let mut kernel = windows::Win32::Foundation::FILETIME::default();
        let mut user = windows::Win32::Foundation::FILETIME::default();
        if windows::Win32::System::Threading::GetSystemTimes(
            Some(&mut idle),
            Some(&mut kernel),
            Some(&mut user),
        )
        .is_err()
        {
            return None;
        }
        let to_u64 = |ft: windows::Win32::Foundation::FILETIME| {
            ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
        };
        let idle_u = to_u64(idle);
        let total_u = to_u64(kernel).wrapping_add(to_u64(user));
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let last_idle = LAST_CPU_IDLE.load(Ordering::Relaxed);
        let last_total = LAST_CPU_TOTAL.load(Ordering::Relaxed);
        let last_ms = LAST_CPU_SAMPLE_MS.load(Ordering::Relaxed);
        let dt = now_ms.saturating_sub(last_ms);
        if last_ms != 0 && dt < 400 {
            // Not enough time for a meaningful delta; the caller holds the
            // last smoothed value.
            return None;
        }
        let idle_delta = idle_u.wrapping_sub(last_idle);
        let total_delta = total_u.wrapping_sub(last_total);
        let pct = if total_delta == 0 || last_ms == 0 {
            0
        } else {
            let busy = total_delta.saturating_sub(idle_delta);
            ((busy as f64 / total_delta as f64) * 100.0).round() as u8
        };
        LAST_CPU_IDLE.store(idle_u, Ordering::Relaxed);
        LAST_CPU_TOTAL.store(total_u, Ordering::Relaxed);
        LAST_CPU_SAMPLE_MS.store(now_ms.max(1), Ordering::Relaxed);
        Some(pct.min(100))
    }
}

/// Whether Windows apps are set to the light theme (registry-backed).
/// `None` when the value cannot be read.
pub fn apps_use_light_theme() -> Option<bool> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::w;
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: key/value names are NUL-terminated literals; out-buffer and size
    // are a live pair for the duration of the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        None
    } else {
        Some(value == 1)
    }
}

/// The user's accent color as BGRA, from the Windows personalization
/// registry (AccentColorMenu). Falls back to the default Windows blue.
pub fn accent_color_bgra() -> [u8; 4] {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::w;
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent"),
            w!("AccentColorMenu"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        [215, 120, 0, 255] // default Windows accent #0078D7 in BGRA
    } else {
        [
            (value & 0xff) as u8,
            ((value >> 8) & 0xff) as u8,
            ((value >> 16) & 0xff) as u8,
            255,
        ]
    }
}

/// Whether Windows "transparency effects" are enabled. When off, the
/// taskbar/goal glass is opaque — we match that with a fully opaque tint.
pub fn transparency_enabled() -> bool {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::w;
    let mut value: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("EnableTransparency"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    if status.is_err() { true } else { value == 1 }
}

/// Whether Windows paints the accent color onto Start and the taskbar
/// ("Show accent color on Start and taskbar"). When on, the taskbar acrylic
/// is tinted toward the accent and the island follows it in `auto` mode.
/// `None` is treated as off: a neutral taskbar needs no accent mixing.
pub fn taskbar_shows_accent() -> bool {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::w;
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: same live key/value/size contract as the readers above.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("ColorPrevalence"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    !status.is_err() && value == 1
}

/// Resolves the `auto` theme name from the system light/dark setting.
pub fn auto_theme_name() -> &'static str {
    match apps_use_light_theme() {
        Some(true) => "light",
        _ => "liquid-dark",
    }
}

pub fn collect() -> SystemStats {
    let (bat, charging) = battery_status();
    SystemStats {
        time: current_time_text(),
        battery: bat,
        battery_charging: charging,
        mem_percent: mem_percent(),
        cpu_percent: cpu_percent(),
    }
}

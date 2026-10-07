//! Isolated DLL host, not a Windhawk injection test. No visible windows/input.
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, RECT},
        System::LibraryLoader::{
            GetProcAddress, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
            LoadLibraryExW,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetClassNameW, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
        },
    },
    core::{BOOL, PCSTR, PCWSTR},
};
type Run = unsafe extern "system" fn(*const u8, u32) -> u32;
type Stop = unsafe extern "system" fn();
type Abi = unsafe extern "system" fn() -> u32;
unsafe extern "system" fn check_hidden(hwnd: HWND, parameter: LPARAM) -> BOOL {
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == std::process::id() && unsafe { IsWindowVisible(hwnd) }.as_bool() {
        unsafe {
            *(parameter.0 as *mut bool) = false;
        }
    }
    BOOL(1)
}
unsafe extern "system" fn check_bar(hwnd: HWND, parameter: LPARAM) -> BOOL {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == std::process::id() {
        let mut class = [0u16; 128];
        let length = unsafe { GetClassNameW(hwnd, &mut class) };
        if String::from_utf16_lossy(&class[..length.max(0) as usize]) == "termielle_overlay" {
            let mut rect = RECT::default();
            if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
                let width = i64::from(rect.right) - i64::from(rect.left);
                let height = i64::from(rect.bottom) - i64::from(rect.top);
                if height > 0 && width > height * 8 {
                    unsafe { *(parameter.0 as *mut bool) = true };
                }
            }
        }
    }
    BOOL(1)
}
fn process_cpu_ms() -> u64 {
    use windows::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
        .unwrap()
    };
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    (ticks(kernel) + ticks(user)) / 10_000
}
fn main() {
    let mut args = std::env::args().skip(1);
    let dll = PathBuf::from(args.next().expect("DLL path"))
        .canonicalize()
        .unwrap();
    let mode = args.next().unwrap_or_else(|| "smoke".into());
    assert!(
        [
            "smoke", "stop", "off", "invalid", "busy", "cancel", "bar", "adapter", "perf"
        ]
        .contains(&mode.as_str())
    );
    let seconds: u64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(20);
    assert!((5..=120).contains(&seconds));
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join(".termielle");
    std::fs::create_dir_all(&directory).unwrap();
    let profile = directory.join("config.json");
    let mut config = termielle_core::AppConfig::default();
    config.island.show_name = false;
    config.island.forward_toasts = false;
    config.island.face_animated = false;
    config.island.glass.blur_radius = 0;
    config.island.widgets = Vec::new();
    if mode == "perf" {
        // No navigation/title/status modules: isolate the resting worker,
        // avoiding foreign-window churn and side-module paint differences.
        config.island.bar.modules_left.clear();
        config.island.bar.modules_right.clear();
    }
    // Deliberately reserved/replacement: all hidden diagnostics must suppress
    // shell changes. Bar mode also proves an override of a saved pill layout.
    if mode == "bar" {
        config.island.layout = termielle_core::IslandLayout::Notch;
    }
    config.island.bar.replace_taskbar = true;
    config.island.bar.reserve_space = true;
    termielle_core::save_config_atomic(&profile, &config).unwrap();
    let before = std::fs::read(&profile).unwrap();
    let pipe = format!(r"\\.\pipe\termielle-host-test-{}", std::process::id());
    let json=serde_json::to_vec(&serde_json::json!({"abi_version":1,"payload_dir":dll.parent().unwrap(),"config_path":profile,"data_dir":directory,"pipe":pipe,"smoke_test":mode=="smoke","review_hidden":mode!="smoke","surface":if matches!(mode.as_str(),"bar"|"perf") {"bar"} else {"island"}})).unwrap();
    if mode == "off" {
        termielle_app::power::set_enabled(&directory, false).unwrap();
    }
    let wide: Vec<u16> = dll
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let module = unsafe {
        LoadLibraryExW(
            PCWSTR(wide.as_ptr()),
            None,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        )
    }
    .unwrap();
    let abi: Abi = unsafe {
        std::mem::transmute(
            GetProcAddress(module, PCSTR(c"termielle_runtime_abi_v1".as_ptr().cast())).unwrap(),
        )
    };
    let run: Run = unsafe {
        std::mem::transmute(
            GetProcAddress(module, PCSTR(c"termielle_runtime_run_v1".as_ptr().cast())).unwrap(),
        )
    };
    let stop: Stop = unsafe {
        std::mem::transmute(
            GetProcAddress(module, PCSTR(c"termielle_runtime_stop_v1".as_ptr().cast())).unwrap(),
        )
    };
    assert_eq!(unsafe { abi() }, 1);
    if mode == "adapter" {
        // The integrated DLL must refuse to run/exit/hook an ordinary process.
        let init: unsafe extern "C" fn() -> i32 = unsafe {
            std::mem::transmute(
                GetProcAddress(module, PCSTR(c"_Z10Wh_ModInitv".as_ptr().cast())).unwrap(),
            )
        };
        assert_eq!(unsafe { init() }, 0, "ordinary process must be excluded");
        for name in [
            c"InternalWhModPtr",
            c"_Z15Wh_ModAfterInitv",
            c"_Z12Wh_ModUninitv",
            c"_Z21Wh_ModSettingsChangedv",
        ] {
            assert!(
                unsafe { GetProcAddress(module, PCSTR(name.as_ptr().cast())) }.is_some(),
                "missing integrated SDK export {name:?}"
            );
        }
        for name in [
            c"_Z15Wh_ModAfterInitv",
            c"_Z21Wh_ModSettingsChangedv",
            c"_Z12Wh_ModUninitv",
        ] {
            let callback: unsafe extern "C" fn() = unsafe {
                std::mem::transmute(GetProcAddress(module, PCSTR(name.as_ptr().cast())).unwrap())
            };
            unsafe { callback() };
        }
        assert_eq!(std::fs::read(&profile).unwrap(), before);
    } else if mode == "invalid" {
        assert_eq!(unsafe { run(std::ptr::null(), 0) }, 2);
        let bad = b"{\"abi_version\":2}";
        assert_eq!(unsafe { run(bad.as_ptr(), bad.len() as u32) }, 2);
    } else {
        let repeat_request = json.clone();
        let mutex_name: Vec<u16> = format!("Termielle-termielle-host-test-{}", std::process::id())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let claim = if mode == "busy" {
            Some(
                unsafe {
                    windows::Win32::System::Threading::CreateMutexW(
                        None,
                        false,
                        PCWSTR(mutex_name.as_ptr()),
                    )
                }
                .unwrap(),
            )
        } else {
            None
        };
        if mode == "cancel" {
            unsafe { stop() };
        }
        let worker = std::thread::spawn(move || unsafe { run(json.as_ptr(), json.len() as u32) });
        if !["off", "busy", "cancel"].contains(&mode.as_str()) {
            let client = termielle_ipc::EventClient::new(&pipe, Duration::from_millis(20));
            let event = termielle_core::encode_event_line(&termielle_core::EventMessage {
                version: 1,
                source: termielle_core::Source::parse("codex").unwrap(),
                session_id: "host-review".into(),
                event: if mode == "perf" {
                    termielle_core::EventKind::SessionEnded
                } else {
                    termielle_core::EventKind::PromptSubmitted
                },
                timestamp_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            })
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(8);
            while client.send(&event).is_err() {
                assert!(
                    Instant::now() < deadline,
                    "runtime endpoint never became ready"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            let mut hidden = true;
            unsafe {
                EnumWindows(
                    Some(check_hidden),
                    LPARAM((&mut hidden as *mut bool) as isize),
                )
                .unwrap();
            }
            assert!(hidden, "review must not show any owned window");
            if mode == "bar" {
                let deadline = Instant::now() + Duration::from_secs(8);
                loop {
                    let mut found = false;
                    unsafe {
                        EnumWindows(Some(check_bar), LPARAM((&mut found as *mut bool) as isize))
                            .unwrap();
                    }
                    if found {
                        break;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "hosted override never produced bar geometry"
                    );
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            if mode == "perf" {
                std::thread::sleep(Duration::from_secs(5));
                println!("PERF_READY");
                let start_cpu = process_cpu_ms();
                let start = Instant::now();
                std::thread::sleep(Duration::from_secs(seconds));
                println!(
                    "PERF_CPU {} {}",
                    process_cpu_ms() - start_cpu,
                    start.elapsed().as_millis()
                );
            }
            if matches!(mode.as_str(), "stop" | "bar" | "perf") {
                unsafe { stop() };
            }
        }
        let result = worker.join().unwrap();
        assert_eq!(
            result,
            match mode.as_str() {
                "off" => 4,
                "busy" => 3,
                _ => 0,
            }
        );
        if let Some(claim) = claim {
            unsafe { windows::Win32::Foundation::CloseHandle(claim).unwrap() };
        }
        assert_eq!(
            std::fs::read(&profile).unwrap(),
            before,
            "startup/stop must preserve profile bytes"
        );
        assert_eq!(
            unsafe { run(repeat_request.as_ptr(), repeat_request.len() as u32) },
            5,
            "one-shot host must reject in-process restart/unload"
        );
        // Never FreeLibrary: one-shot, process-owned DLL. Process exit is part
        // of this host contract, just as with the real Windhawk tool host.
    }
    println!("PASS isolated runtime DLL {mode}: ABI, profile preservation, lifecycle");
}

//! Consolidated x86 Windhawk 1.7.3 tool adapter. Rust ABI bindings mirror the
//! installed SDK; callback export names match engine-mod.cpp's cdecl lookups.
//! Dedicated-tool lifecycle adapted from the MIT reference/official snippet;
//! see windhawk/LICENSES/Dynamic-Island-MIT.txt and THIRD_PARTY_NOTICES.md.
//! Never start the runtime in the manager, a service, another tool or Explorer.
use std::{
    ffi::c_void,
    sync::{
        Mutex,
        atomic::{AtomicU8, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HLOCAL, LocalFree},
        System::Environment::GetCommandLineW,
        System::{
            Environment::ExpandEnvironmentStringsW,
            LibraryLoader::{GetModuleFileNameW, GetModuleHandleW, GetProcAddress},
            RemoteDesktop::ProcessIdToSessionId,
            Threading::{
                CreateMutexW, ExitThread, GetCurrentProcessId, NORMAL_PRIORITY_CLASS,
                PROCESS_INFORMATION, STARTF_FORCEOFFFEEDBACK, STARTUPINFOW,
            },
        },
        UI::Shell::CommandLineToArgvW,
    },
    core::{BOOL, PCSTR, PCWSTR, PWSTR, w},
};

// Written once by Windhawk before callbacks. Same pointer-sized ABI as its SDK.
#[unsafe(export_name = "InternalWhModPtr")]
static mut MOD_PTR: *mut c_void = std::ptr::null_mut();
static ROLE: AtomicU8 = AtomicU8::new(0); // 0 excluded/uninitialized, 1 launcher, 2 our tool
static REQUEST: Mutex<Option<Vec<u8>>> = Mutex::new(None);
static WORKER: Mutex<Option<thread::JoinHandle<()>>> = Mutex::new(None);
type Setting = unsafe extern "C" fn(*mut c_void, PCWSTR, *const c_void) -> PCWSTR;
type FreeSetting = unsafe extern "C" fn(*mut c_void, PCWSTR);
type Hook =
    unsafe extern "C" fn(*mut c_void, *const c_void, *const c_void, *mut *mut c_void) -> BOOL;
type Log = unsafe extern "C" fn(*mut c_void, PCWSTR, *const c_void);

unsafe fn symbol(name: &'static std::ffi::CStr) -> Option<unsafe extern "system" fn() -> isize> {
    let engine = unsafe { GetModuleHandleW(w!("windhawk.dll")) }.ok()?;
    unsafe { GetProcAddress(engine, PCSTR(name.as_ptr().cast())) }
}
fn log(message: &str) {
    // No printf placeholders: null va_list is never dereferenced by the formatter.
    if message.contains('%') || unsafe { MOD_PTR.is_null() } {
        return;
    }
    let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    unsafe {
        if let Some(ptr) = symbol(c"InternalWh_Log") {
            let call: Log = std::mem::transmute(ptr);
            call(MOD_PTR, PCWSTR(text.as_ptr()), std::ptr::null());
        }
    }
}
fn setting(key: &str) -> Option<String> {
    let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let get: Setting = std::mem::transmute(symbol(c"InternalWh_GetStringSetting")?);
        let free: FreeSetting = std::mem::transmute(symbol(c"InternalWh_FreeStringSetting")?);
        let value = get(MOD_PTR, PCWSTR(key.as_ptr()), std::ptr::null());
        if value.is_null() {
            return Some(String::new());
        }
        let result = (0..32768)
            .find(|&n| *value.0.add(n) == 0)
            .and_then(|len| String::from_utf16(std::slice::from_raw_parts(value.0, len)).ok());
        free(MOD_PTR, value);
        result
    }
}
fn expand(value: &str) -> Option<String> {
    let input: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    let size = unsafe { ExpandEnvironmentStringsW(PCWSTR(input.as_ptr()), None) };
    if size == 0 || size > 32768 {
        return None;
    }
    let mut output = vec![0; size as usize];
    let copied = unsafe { ExpandEnvironmentStringsW(PCWSTR(input.as_ptr()), Some(&mut output)) };
    if copied == 0 || copied > size {
        return None;
    }
    String::from_utf16(&output[..copied as usize - 1]).ok()
}
fn executable() -> Option<String> {
    let mut buf = vec![0; 32768];
    let len = unsafe { GetModuleFileNameW(None, &mut buf) } as usize;
    if len == 0 || len >= buf.len() {
        return None;
    }
    String::from_utf16(&buf[..len]).ok()
}
#[derive(Debug, PartialEq)]
enum Role {
    Excluded,
    Launcher,
    Tool,
}
fn classify(exe: &str, args: &[String], session: u32) -> Role {
    if session == 0
        || !exe
            .rsplit(['\\', '/'])
            .next()
            .is_some_and(|s| s.eq_ignore_ascii_case("windhawk.exe"))
    {
        return Role::Excluded;
    }
    if args
        .iter()
        .any(|s| matches!(s.as_str(), "-service" | "-service-start" | "-service-stop"))
    {
        return Role::Excluded;
    }
    let flags: Vec<_> = args
        .iter()
        .enumerate()
        .filter(|(_, s)| s.as_str() == "-tool-mod")
        .collect();
    if flags.is_empty() {
        return Role::Launcher;
    }
    if flags.len() == 1 && args.get(flags[0].0 + 1).is_some_and(|s| s == "termielle") {
        Role::Tool
    } else {
        Role::Excluded
    }
}
fn current_role() -> Role {
    let Some(exe) = executable() else {
        return Role::Excluded;
    };
    let mut session = 0;
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) }.is_err() {
        return Role::Excluded;
    }
    unsafe {
        let mut count = 0;
        let argv = CommandLineToArgvW(GetCommandLineW(), &mut count);
        if argv.is_null() {
            return Role::Excluded;
        }
        let args: Vec<_> = (1..count)
            .map(|i| (*argv.add(i as usize)).to_string().unwrap_or_default())
            .collect();
        LocalFree(Some(HLOCAL(argv.cast())));
        classify(&exe, &args, session)
    }
}
fn request() -> Option<Vec<u8>> {
    let directory = expand(&setting("PayloadDirectory")?)?;
    let profile = expand(&setting("Profile")?)?;
    let surface = setting("Surface")?;
    make_request(&directory, &profile, &surface)
}
fn make_request(directory: &str, profile: &str, surface: &str) -> Option<Vec<u8>> {
    if !std::path::Path::new(directory).is_absolute()
        || !std::path::Path::new(directory).is_dir()
        || (!profile.is_empty() && !std::path::Path::new(profile).is_absolute())
    {
        return None;
    }
    let mut value = serde_json::json!({"abi_version":1,"payload_dir":directory});
    if !profile.is_empty() {
        value["config_path"] = profile.into();
    }
    match surface {
        "" | "Profile" => (),
        "Bar" => value["surface"] = "bar".into(),
        "Island" => value["surface"] = "island".into(),
        "Notch" => value["surface"] = "notch".into(),
        _ => return None,
    }
    let bytes = serde_json::to_vec(&value).ok()?;
    (bytes.len() <= 16384).then_some(bytes)
}
unsafe extern "system" fn tool_entry() {
    unsafe { ExitThread(0) }
}
fn hook_entry() -> bool {
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return false;
        };
        let base = module.0.cast::<u8>();
        if base.cast::<u16>().read_unaligned() != 0x5a4d {
            return false;
        }
        let offset = base.add(0x3c).cast::<u32>().read_unaligned() as usize;
        if !(64..=1024 * 1024).contains(&offset) {
            return false;
        }
        let pe = base.add(offset);
        if pe.cast::<u32>().read_unaligned() != 0x4550
            || pe.add(4).cast::<u16>().read_unaligned() != 0x14c
            || pe.add(24).cast::<u16>().read_unaligned() != 0x10b
        {
            return false;
        }
        let rva = pe.add(24 + 16).cast::<u32>().read_unaligned() as usize;
        let image_size = pe.add(24 + 56).cast::<u32>().read_unaligned() as usize;
        if rva < 64 || rva >= image_size {
            return false;
        }
        let Some(ptr) = symbol(c"InternalWh_SetFunctionHook") else {
            return false;
        };
        let hook: Hook = std::mem::transmute(ptr);
        hook(
            MOD_PTR,
            base.add(rva).cast(),
            tool_entry as *const c_void,
            std::ptr::null_mut(),
        )
        .as_bool()
    }
}
fn initialize() -> bool {
    let role = current_role();
    if role == Role::Excluded {
        return false;
    }
    if unsafe {
        MOD_PTR.is_null()
            || symbol(c"InternalWh_GetStringSetting").is_none()
            || symbol(c"InternalWh_FreeStringSetting").is_none()
            || symbol(c"InternalWh_SetFunctionHook").is_none()
    } {
        return false;
    }
    if role == Role::Launcher {
        ROLE.store(1, Ordering::Release);
        return true;
    }
    // Every process-exit path below is reachable only in our verified tool.
    ROLE.store(2, Ordering::Release);
    let mutex = unsafe { CreateMutexW(None, true, w!("windhawk-tool-mod_termielle")) };
    if mutex.is_err() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        std::process::exit(1);
    }
    // Intentionally keep the mutex handle until dedicated-process exit.
    let Some(bytes) = request() else {
        log("Invalid Termielle payload/profile/surface");
        std::process::exit(1);
    };
    if !hook_entry() {
        log("Cannot install dedicated-host entry-point hook");
        std::process::exit(1);
    }
    *REQUEST.lock().unwrap_or_else(|e| e.into_inner()) = Some(bytes);
    true
}
fn launch() {
    let Some(exe) = executable() else {
        return;
    };
    let mut command: Vec<u16> = format!("\"{exe}\" -tool-mod \"termielle\"")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let path: Vec<u16> = exe.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let kernel = GetModuleHandleW(w!("kernelbase.dll"))
            .or_else(|_| GetModuleHandleW(w!("kernel32.dll")));
        let Some(ptr) = kernel
            .ok()
            .and_then(|m| GetProcAddress(m, PCSTR(c"CreateProcessInternalW".as_ptr().cast())))
        else {
            log("Cannot resolve tool launcher");
            return;
        };
        type Create = unsafe extern "system" fn(
            HANDLE,
            PCWSTR,
            PWSTR,
            *const c_void,
            *const c_void,
            BOOL,
            u32,
            *const c_void,
            PCWSTR,
            *const STARTUPINFOW,
            *mut PROCESS_INFORMATION,
            *mut HANDLE,
        ) -> BOOL;
        let create: Create = std::mem::transmute(ptr);
        let startup = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            dwFlags: STARTF_FORCEOFFFEEDBACK,
            ..Default::default()
        };
        let mut process = PROCESS_INFORMATION::default();
        if create(
            HANDLE::default(),
            PCWSTR(path.as_ptr()),
            PWSTR(command.as_mut_ptr()),
            std::ptr::null(),
            std::ptr::null(),
            BOOL(0),
            NORMAL_PRIORITY_CLASS.0,
            std::ptr::null(),
            PCWSTR::null(),
            &startup,
            &mut process,
            std::ptr::null_mut(),
        )
        .as_bool()
        {
            let _ = CloseHandle(process.hProcess);
            let _ = CloseHandle(process.hThread);
        } else {
            log("Cannot start dedicated Windhawk tool process");
        }
    }
}
#[unsafe(export_name = "_Z10Wh_ModInitv")]
extern "C" fn mod_init() -> i32 {
    i32::from(std::panic::catch_unwind(initialize).unwrap_or(false))
}
#[unsafe(export_name = "_Z15Wh_ModAfterInitv")]
extern "C" fn mod_after_init() {
    if std::panic::catch_unwind(|| match ROLE.load(Ordering::Acquire) {
        1 => launch(),
        2 => {
            let bytes = REQUEST
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .expect("missing tool request");
            let worker = thread::spawn(move || {
                let result =
                    unsafe { super::termielle_runtime_run_v1(bytes.as_ptr(), bytes.len() as u32) };
                log("Termielle runtime ended; a fresh tool process is needed to restart");
                std::process::exit(if matches!(result, 1 | 2) { 1 } else { 0 });
            });
            *WORKER.lock().unwrap_or_else(|e| e.into_inner()) = Some(worker);
        }
        _ => (),
    })
    .is_err()
        && ROLE.load(Ordering::Acquire) == 2
    {
        std::process::exit(1);
    }
}
#[unsafe(export_name = "_Z21Wh_ModSettingsChangedv")]
extern "C" fn mod_settings_changed() {
    if ROLE.load(Ordering::Acquire) == 2 {
        let _ = std::panic::catch_unwind(|| {
            log("Payload/profile/surface changes need disable/re-enable")
        });
    }
}
#[unsafe(export_name = "_Z12Wh_ModUninitv")]
extern "C" fn mod_uninit() {
    if ROLE.load(Ordering::Acquire) != 2 {
        return;
    }
    super::termielle_runtime_stop_v1();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if WORKER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_none_or(|w| w.is_finished())
        {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    // Never unload an active one-shot Rust runtime; end ONLY our verified tool.
    std::process::exit(0);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_our_flagged_windhawk_tool_can_own_runtime_exit() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            classify(
                "C:\\Windhawk\\windhawk.exe",
                &args(&["-tool-mod", "termielle"]),
                1
            ),
            Role::Tool
        );
        assert_eq!(
            classify("windhawk.exe", &args(&["-tray-only"]), 1),
            Role::Launcher
        );
        for (exe, a, session) in [
            ("explorer.exe", vec!["-tool-mod", "termielle"], 1),
            ("windhawk.exe", vec!["-tool-mod", "termielle"], 0),
            (
                "windhawk.exe",
                vec!["-service", "-tool-mod", "termielle"],
                1,
            ),
            ("windhawk.exe", vec!["-tool-mod", "other"], 1),
            ("windhawk.exe", vec!["-tool-mod"], 1),
            (
                "windhawk.exe",
                vec!["-tool-mod", "termielle", "-tool-mod", "other"],
                1,
            ),
        ] {
            assert_eq!(classify(exe, &args(&a), session), Role::Excluded);
        }
    }
    #[test]
    fn requests_escape_paths_and_accept_only_supported_surfaces() {
        let dir = tempfile::tempdir().unwrap();
        for surface in ["Profile", "Bar", "Island", "Notch"] {
            let bytes = make_request(dir.path().to_str().unwrap(), "", surface).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["payload_dir"], dir.path().to_str().unwrap());
            assert_eq!(value["abi_version"], 1);
        }
        assert!(make_request("relative", "", "Bar").is_none());
        assert!(make_request(dir.path().to_str().unwrap(), "relative", "Bar").is_none());
        assert!(make_request(dir.path().to_str().unwrap(), "", "Classic").is_none());
    }
}

//! Versioned, one-shot runtime ABI for a dedicated Windhawk tool process.
//! Never load this in Explorer. The module stays loaded until its host exits:
//! some existing OS/decoder workers are process-owned, not safe for DLL unload.
use crate::window::WakeHandle;
use std::{
    cell::RefCell,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

const MAX_REQUEST: usize = 16 * 1024;
static USED: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static WINDOW: Mutex<Option<WakeHandle>> = Mutex::new(None);
thread_local! {
    static CONTEXT: RefCell<Option<HostRequest>> = const { RefCell::new(None) };
    static PROFILE_SURFACE: RefCell<Option<(termielle_core::IslandLayout, bool, bool)>> = const { RefCell::new(None) };
}

pub(crate) fn remember_profile_surface(config: &termielle_core::IslandConfig) {
    if is_hosted() {
        PROFILE_SURFACE.with(|p| {
            *p.borrow_mut() = Some((
                config.layout,
                config.bar.replace_taskbar,
                config.bar.reserve_space,
            ))
        });
    }
}
pub(crate) fn restore_profile_surface(config: &mut termielle_core::IslandConfig) {
    if is_hosted() {
        PROFILE_SURFACE.with(|p| {
            if let Some((layout, replace, reserve)) = *p.borrow() {
                config.layout = layout;
                config.bar.replace_taskbar = replace;
                config.bar.reserve_space = reserve;
            }
        });
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostRequest {
    pub abi_version: u32,
    pub payload_dir: PathBuf,
    #[serde(default)]
    pub config_path: Option<PathBuf>,
    /// Explicit host-only surface override; saved standalone layout is retained.
    #[serde(default)]
    pub surface: Option<termielle_core::IslandLayout>,
    /// Only for isolated diagnostics; never set by the shipped mod.
    #[serde(default)]
    pub data_dir: Option<PathBuf>,
    #[serde(default)]
    pub pipe: Option<String>,
    #[serde(default)]
    pub smoke_test: bool,
    #[serde(default)]
    pub review_hidden: bool,
}
impl HostRequest {
    fn parse(bytes: &[u8]) -> Result<Self, ()> {
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ())?;
        if value.abi_version != 1
            || !value.payload_dir.is_absolute()
            || !value.payload_dir.is_dir()
            || value.config_path.as_ref().is_some_and(|p| !p.is_absolute())
            || value.surface.is_some_and(|s| {
                !matches!(
                    s,
                    termielle_core::IslandLayout::Bar
                        | termielle_core::IslandLayout::Island
                        | termielle_core::IslandLayout::Notch
                )
            })
        {
            return Err(());
        }
        if value.smoke_test
            || value.review_hidden
            || value.data_dir.is_some()
            || value.pipe.is_some()
        {
            // Test overrides must be an explicitly isolated hidden smoke run.
            if !(value.smoke_test || value.review_hidden)
                || (value.smoke_test && value.review_hidden)
                || !value.data_dir.as_ref().is_some_and(|p| p.is_absolute())
                || !value.pipe.as_ref().is_some_and(|p| {
                    p.starts_with(r"\\.\pipe\termielle-host-test-") && p.len() <= 200
                })
            {
                return Err(());
            }
        }
        Ok(value)
    }
}

pub(crate) fn set_thread_context(request: HostRequest) {
    CONTEXT.with(|c| *c.borrow_mut() = Some(request));
}
pub(crate) fn clear_thread_context() {
    CONTEXT.with(|c| *c.borrow_mut() = None);
    PROFILE_SURFACE.with(|p| *p.borrow_mut() = None);
}
pub(crate) fn is_hosted() -> bool {
    CONTEXT.with(|c| c.borrow().is_some())
}
pub(crate) fn data_directory() -> Option<PathBuf> {
    CONTEXT.with(|c| c.borrow().as_ref().and_then(|r| r.data_dir.clone()))
}
pub(crate) fn review_hidden() -> bool {
    CONTEXT.with(|c| c.borrow().as_ref().is_some_and(|r| r.review_hidden))
}
pub(crate) fn payload_directory() -> Option<PathBuf> {
    CONTEXT.with(|c| c.borrow().as_ref().map(|r| r.payload_dir.clone()))
}
pub(crate) fn stop_requested() -> bool {
    is_hosted() && STOP.load(Ordering::Acquire)
}
pub(crate) fn publish_window(wake: WakeHandle) {
    if is_hosted() {
        *WINDOW.lock().unwrap_or_else(|e| e.into_inner()) = Some(wake);
        if stop_requested() {
            let _ = wake.post_power_off();
        }
    }
}
pub(crate) fn clear_window() {
    if is_hosted() {
        *WINDOW.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Runtime-only policy: tool hosting never becomes taskbar replacement.
pub(crate) fn apply_surface_policy(config: &mut termielle_core::IslandConfig) {
    if is_hosted() {
        if let Some(surface) = CONTEXT.with(|c| c.borrow().as_ref().and_then(|r| r.surface)) {
            config.layout = surface;
        }
        if config.layout == termielle_core::IslandLayout::Classic {
            config.layout = termielle_core::IslandLayout::Island;
        }
        config.bar.replace_taskbar = false;
        // A live hosted bar honors the saved AppBar preference, just like native.
        // Hidden diagnostics must never reserve real desktop space; pill surfaces
        // likewise cannot inherit a standalone bar's reservation.
        let diagnostic = CONTEXT.with(|c| {
            c.borrow()
                .as_ref()
                .is_some_and(|r| r.smoke_test || r.review_hidden)
        });
        if !config.is_bar() || diagnostic {
            config.bar.reserve_space = false;
        }
    }
}

pub extern "system" fn termielle_runtime_abi_v1() -> u32 {
    1
}

/// Runs once on a host-owned GUI thread; returns when UI teardown is complete.
/// Codes: 0 normal, 1 runtime failure, 2 bad ABI/request, 3 another frontend,
/// 4 persisted Off, 5 already used in this process, 11 user-requested Restart.
///
/// # Safety
/// `bytes` must address `len` readable bytes for the duration of this call. The
/// caller must use a dedicated tool process, keep this DLL loaded until process
/// exit, and never attempt to unload/restart it in-process.
pub unsafe extern "system" fn termielle_runtime_run_v1(bytes: *const u8, len: u32) -> u32 {
    if bytes.is_null() || len == 0 || len as usize > MAX_REQUEST {
        return 2;
    }
    let input = unsafe { std::slice::from_raw_parts(bytes, len as usize) };
    let Ok(request) = HostRequest::parse(input) else {
        return 2;
    };
    if USED.swap(true, Ordering::AcqRel) {
        return 5;
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::runtime::run_hosted(&request)
    }));
    clear_window();
    clear_thread_context();
    result.map_or(1, |code| code as u32)
}

/// Cross-thread graceful stop. Does not change persistent Off or profile bytes.
pub extern "system" fn termielle_runtime_stop_v1() {
    STOP.store(true, Ordering::Release);
    if let Some(wake) = *WINDOW.lock().unwrap_or_else(|e| e.into_inner()) {
        let _ = wake.post_power_off();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hosted_policy_is_ephemeral_and_keeps_native_taskbar() {
        let dir = tempfile::tempdir().unwrap();
        let request = HostRequest {
            abi_version: 1,
            payload_dir: dir.path().into(),
            config_path: None,
            surface: Some(termielle_core::IslandLayout::Island),
            data_dir: None,
            pipe: None,
            smoke_test: false,
            review_hidden: false,
        };
        set_thread_context(request);
        let original = termielle_core::IslandConfig::default();
        let mut view = original.clone();
        view.bar.replace_taskbar = true;
        view.bar.reserve_space = true;
        let profile = view.clone();
        remember_profile_surface(&profile);
        apply_surface_policy(&mut view);
        assert_eq!(view.layout, termielle_core::IslandLayout::Island);
        assert!(!view.bar.replace_taskbar && !view.bar.reserve_space);
        view.layout = termielle_core::IslandLayout::Notch;
        apply_surface_policy(&mut view);
        assert_eq!(
            view.layout,
            termielle_core::IslandLayout::Island,
            "explicit host override wins without rewriting saved layout"
        );
        view.show_name = false;
        restore_profile_surface(&mut view);
        assert_eq!(view.layout, profile.layout);
        assert!(view.bar.replace_taskbar && view.bar.reserve_space);
        assert!(
            !view.show_name,
            "edited settings must survive surface restoration"
        );
        clear_thread_context();
        assert_eq!(original, termielle_core::IslandConfig::default());
    }
    #[test]
    fn hosted_bar_profile_override_and_saves_preserve_native_policy() {
        use termielle_core::IslandLayout::{Bar, Classic, Island, Notch};
        let dir = tempfile::tempdir().unwrap();
        for layout in [Bar, Island, Notch, Classic] {
            for surface in [None, Some(Bar), Some(Island), Some(Notch)] {
                for reserve in [false, true] {
                    for replace in [false, true] {
                        let request = HostRequest {
                            abi_version: 1,
                            payload_dir: dir.path().into(),
                            config_path: None,
                            surface,
                            data_dir: None,
                            pipe: None,
                            smoke_test: false,
                            review_hidden: false,
                        };
                        set_thread_context(request);
                        let mut profile = termielle_core::IslandConfig {
                            layout,
                            ..Default::default()
                        };
                        profile.bar.reserve_space = reserve;
                        profile.bar.replace_taskbar = replace;
                        profile.bar.position = termielle_core::BarPosition::Bottom;
                        profile.bar.height = 42;
                        profile.glass.tint = [19, 32, 47, 186];
                        remember_profile_surface(&profile);
                        let mut view = profile.clone();
                        apply_surface_policy(&mut view);
                        let effective =
                            surface.unwrap_or(if layout == Classic { Island } else { layout });
                        assert_eq!(view.layout, effective);
                        assert!(!view.bar.replace_taskbar);
                        assert_eq!(view.bar.reserve_space, effective == Bar && reserve);
                        let applied = view.clone();
                        apply_surface_policy(&mut view);
                        assert_eq!(view, applied, "policy must be idempotent");
                        assert_eq!(view.bar.position, profile.bar.position);
                        assert_eq!(view.bar.height, profile.bar.height);
                        assert_eq!(view.glass, profile.glass);
                        // Simulate a hosted preferences save: preserve independent edits,
                        // not an override layout or the forced replacement protection.
                        view.show_name = false;
                        view.bar.height = 46;
                        restore_profile_surface(&mut view);
                        profile.show_name = false;
                        profile.bar.height = 46;
                        assert_eq!(view, profile);
                        clear_thread_context();
                    }
                }
            }
        }
    }

    #[test]
    fn hidden_bar_diagnostics_never_reserve_the_real_desktop() {
        let dir = tempfile::tempdir().unwrap();
        for smoke in [false, true] {
            let json = serde_json::json!({
                "abi_version":1, "payload_dir":dir.path(), "surface":"bar",
                "data_dir":dir.path(), "pipe":r"\\.\pipe\termielle-host-test-policy",
                "smoke_test":smoke, "review_hidden":!smoke,
            });
            set_thread_context(HostRequest::parse(&serde_json::to_vec(&json).unwrap()).unwrap());
            let mut view = termielle_core::IslandConfig::default();
            view.bar.reserve_space = true;
            view.bar.replace_taskbar = true;
            apply_surface_policy(&mut view);
            assert!(view.is_bar());
            assert!(!view.bar.reserve_space && !view.bar.replace_taskbar);
            clear_thread_context();
        }
    }

    #[test]
    fn bad_abi_and_nonisolated_diagnostic_overrides_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let good = serde_json::json!({"abi_version":1,"payload_dir":dir.path()});
        assert!(HostRequest::parse(&serde_json::to_vec(&good).unwrap()).is_ok());
        for surface in ["bar", "island", "notch"] {
            let good =
                serde_json::json!({"abi_version":1,"payload_dir":dir.path(),"surface":surface});
            assert!(HostRequest::parse(&serde_json::to_vec(&good).unwrap()).is_ok());
        }
        for bad in [
            serde_json::json!({"abi_version":2,"payload_dir":dir.path()}),
            serde_json::json!({"abi_version":1,"payload_dir":dir.path(),"surface":"classic"}),
            serde_json::json!({"abi_version":1,"payload_dir":"relative"}),
            serde_json::json!({"abi_version":1,"payload_dir":dir.path(),"smoke_test":true}),
            serde_json::json!({"abi_version":1,"payload_dir":dir.path(),"data_dir":dir.path()}),
            serde_json::json!({"abi_version":1,"payload_dir":dir.path(),"unknown":true}),
        ] {
            assert!(HostRequest::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
        }
    }
}

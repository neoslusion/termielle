//! DLL packaging boundary and optional Windhawk adapter. UI/runtime behavior
//! lives in termielle-app; the x86 feature integrates only host lifecycle.
//! Dedicated, one-shot tool host; do not unload this library in-process.

#[cfg(all(feature = "windhawk-x86", not(target_arch = "x86")))]
compile_error!("windhawk-x86 requires --target i686-pc-windows-msvc");
#[cfg(all(feature = "windhawk-x86", target_arch = "x86"))]
mod windhawk;

#[unsafe(no_mangle)]
pub extern "system" fn termielle_runtime_abi_v1() -> u32 {
    termielle_app::hosted::termielle_runtime_abi_v1()
}

/// # Safety
/// See `termielle_app::hosted::termielle_runtime_run_v1`: readable request bytes,
/// a dedicated process, and DLL residency until that process exits are required.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn termielle_runtime_run_v1(bytes: *const u8, len: u32) -> u32 {
    unsafe { termielle_app::hosted::termielle_runtime_run_v1(bytes, len) }
}

#[unsafe(no_mangle)]
pub extern "system" fn termielle_runtime_stop_v1() {
    termielle_app::hosted::termielle_runtime_stop_v1();
}

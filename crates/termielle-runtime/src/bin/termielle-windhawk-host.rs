//! Minimal x64 bootstrap. Windhawk injects the adapter which loads the runtime;
//! no renderer, protocol consumer or duplicate engine lives in this executable.
#![windows_subsystem = "windows"]
use std::{thread, time::Duration};
use windows::{Win32::System::LibraryLoader::GetModuleHandleW, core::w};
fn main() {
    // Fail closed if Windhawk/mod activation is missing. Startup delay permits
    // existing-process injection as well as interception before our entry point.
    for _ in 0..80 {
        if unsafe { GetModuleHandleW(w!("termielle_runtime.dll")) }.is_ok() {
            loop {
                thread::park();
            } // runtime owns dedicated-process shutdown
        }
        thread::sleep(Duration::from_millis(250));
    }
    std::process::exit(2);
}

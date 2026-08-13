//! Minimal entry point for the fail-open hook emitter.
//!
//! All behavior lives in the library so the hook semantics are unit-testable
//! without spawning a process; this file only connects the real stdin/stdout
//! and maps the exit code.
//!
//! The binary is a GUI-subsystem executable so a hook firing outside a shared
//! console never flashes a terminal window. Redirected stdout (how agents
//! read the `{}` response) still works, and a missing handle is fail-open.

#![windows_subsystem = "windows"]

use std::io;

fn main() -> std::process::ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let code = termielle_emit::run(args, &mut stdin, &mut stdout);
    std::process::ExitCode::from(code as u8)
}

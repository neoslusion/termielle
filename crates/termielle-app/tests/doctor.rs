//! Runs the PowerShell doctor against a staged copy of the binaries: it must
//! exit 0 on a healthy install and nonzero when a required binary is missing.
//! The staged copy keeps the checks honest without touching the built tree.
//!
//! These tests spawn the overlay on the default pipe, so a live overlay on
//! the host makes them fail loudly rather than pass falsely.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn repo_root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn stage_dir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "termielle-doctor-stage-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn stage_binaries(staging: &Path) {
    let source_dir = Path::new(env!("CARGO_BIN_EXE_termielle-app"))
        .parent()
        .expect("app binary has a parent");
    std::fs::create_dir_all(staging).expect("staging dir");
    for name in ["termielle-app.exe", "termielle-emit.exe"] {
        let source = source_dir.join(name);
        assert!(source.exists(), "missing binary for staging: {source:?}");
        std::fs::copy(&source, staging.join(name)).expect("staged copy");
    }
}

fn run_doctor(bin_dir: &Path) -> std::process::Output {
    Command::new("pwsh")
        .args(["-NoProfile", "-File"])
        .arg(repo_root().join("scripts/doctor.ps1"))
        .arg("-BinDir")
        .arg(bin_dir)
        .output()
        .expect("pwsh runs")
}

#[test]
fn doctor_passes_on_a_healthy_install() {
    let staging = stage_dir();
    stage_binaries(&staging);
    let out = run_doctor(&staging);
    let _ = std::fs::remove_dir_all(&staging);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "doctor must pass on a healthy install:\n{stdout}"
    );
    assert!(
        stdout.contains("ALL") && stdout.contains("PASSED"),
        "doctor must report a clean bill:\n{stdout}"
    );
}

#[test]
fn doctor_fails_when_the_emitter_is_missing() {
    let staging = stage_dir();
    stage_binaries(&staging);
    std::fs::remove_file(staging.join("termielle-emit.exe")).expect("remove emitter");
    let out = run_doctor(&staging);
    let _ = std::fs::remove_dir_all(&staging);

    assert!(
        !out.status.success(),
        "doctor must fail when the emitter is missing"
    );
}

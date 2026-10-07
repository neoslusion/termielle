# Termielle — Native or Windhawk

**0.4.0 consolidated x86 preview installed and live-tested on Windhawk 1.7.3,
October 7, 2026.** Native remains x64. Both editions share Rust rendering,
sessions, integrations and settings; only one frontend should run at a time.
Physical fullscreen/DPI/lid/recording acceptance and resource benchmarks remain pending.

## Choose an edition

Start-menu shortcuts under **Termielle**:

- **Termielle - Native:** disable only Termielle's mod, stop its owned tool
  gracefully and restart the existing standalone scheduled task.
- **Termielle - Windhawk:** gracefully close standalone, enable only Termielle's
  mod and launch its dedicated Windhawk tool process if needed.

```powershell
pwsh -NoProfile -File scripts/switch-termielle-edition.ps1 -Edition Native
pwsh -NoProfile -File scripts/switch-termielle-edition.ps1 -Edition Windhawk
```

The installed switcher reads `%LOCALAPPDATA%\Termielle\windhawk\edition.json`.
It supports both the previous x64 host and consolidated x86 hosting mode. It
validates the owned registration and identifies tools by **executable path plus
`-tool-mod termielle`**; it never closes the manager, services or other tools.
Profile, persistent Off, other mods and scheduled-task definition are preserved.
No force termination. Failed hosted startup falls back to native. Persistent Off
must explicitly be cleared with `termielle-app.exe --enable-only` before switching.

Login policy is unchanged: whichever configured frontend claims startup first
wins. Enabling the mod doesn't silently clear persistent Off or override ownership.

## Bar, Island and Notch

Windhawk's **Surface** setting offers **Bar**, Island, Notch and Follow profile.
The local preview is set to **Bar**. Follow profile retains saved Bar/Island/Notch;
Classic maps to Island. Disable/re-enable after payload/profile/surface changes.
Preferences shows Windhawk edition; layout remains host-controlled.

The hosted Bar uses saved position, size, modules, pins, theme and material. It
honors saved `reserve_space`, so maximized windows leave room. Island/Notch do not
reserve space, and hidden diagnostics cannot register an AppBar. **Native taskbar
hiding/replacement is always blocked**, even if the native profile requests it.
Hosted saves retain native layout/reservation/replacement and independent edits.

Shared Preferences > Behavior / display offers monitor choice, optional fullscreen
hiding for Island/Notch, and Custom/Smooth/Balanced/Snappy/Bouncy animation feel.
See [desktop improvements](../docs/desktop-improvements.md).

## Current architecture — one main DLL

```text
Windhawk manager (x86; launcher callback only, no Termielle UI workers)
  └─ windhawk.exe -tool-mod "termielle" (dedicated x86 process)
       ├─ Windhawk's x86 engine
       └─ Termielle mod DLL: adapter + shared Rust runtime + bar/pill/notch
```

The manager and tool map the same consolidated DLL; only the verified tool starts
Termielle's runtime. No Termielle host EXE or separate runtime DLL is required by
this edition. Windows/system DLL dependencies and assets/themes still apply.
The native x64 executable links the same Rust app implementation directly.

`crates/termielle-runtime/src/windhawk.rs` binds the installed SDK's C ABI and
exports the cdecl callback names used by Windhawk 1.7.3's engine, including its
mod-context data export. The compatibility hook targets **only the entry point
of our flag-validated disposable Windhawk tool**. No Explorer renderer, global
input hook, UIAccess, elevation or security-setting changes.

This does not mean no process exists, nor prove less memory/CPU. The manager also
maps the consolidated DLL; comparisons require measurements, not file counts.

### Previous 0.3.0 x64 preview

The earlier working version used x86 launcher + x64 adapter +
`termielle-windhawk-host.exe` + `termielle_runtime.dll`. Its payload/adapters are
retained locally as fallback, not running alongside 0.4.0. The x86 port resolved
Windows LongPtr API signature differences without creating a separate UI fork.
`windhawk/termielle.wh.cpp` and the SDK compiler check still describe that fallback.

## Build and review

```powershell
rustup target add i686-pc-windows-msvc
cargo build -p termielle-runtime --release --target i686-pc-windows-msvc --features windhawk-x86 --lib
cargo build -p termielle-app --release --target i686-pc-windows-msvc --example windhawk_host_review
pwsh -NoProfile -File scripts/review-windhawk-runtime.ps1 -BinDir target/i686-pc-windows-msvc/release -Consolidated
cargo test -p termielle-runtime --target i686-pc-windows-msvc --features windhawk-x86
```

Output: `target/i686-pc-windows-msvc/release/termielle_runtime.dll`. For installation
it is assigned a versioned Termielle mod filename in Windhawk's **32** mod directory.
It is one consolidated DLL, despite the build artifact's historical runtime name.
The release archive stages it as `windhawk/termielle-x86.dll` alongside native and
legacy payload. Packaging doesn't enable mods or change task/profile/security policy.

**Important:** `termielle-x86.wh.cpp` is metadata/settings/readme for the Cargo-built
Rust DLL, not a C++ implementation. Windhawk's editor cannot rebuild Rust by itself;
the metadata deliberately fails C++ compilation rather than produce an empty mod.
Use Cargo; do not overwrite the known-good registration via editor recompilation.

Validation: **519 passed / 0 failed / 2 existing manual tests ignored** on each
x64 and x86 workspace run, plus two x86 adapter tests. Eight private DLL-host modes
cover hidden pill/bar rendering, stop, Off, ABI rejection, collision, pre-start
cancellation and refusal/no-op callbacks in an ordinary process. A pointer-bit test
covers userdata storage; native placement tests cover available work areas rather
than assuming all displays are right of primary. Exact PE32 exports and required
APIs in the installed x86 engine were inspected before activation.

Live review verified WOW64/x86 process identity, actual x86 Windhawk engine and
**one matching Termielle DLL**, one visible full-width Bar, no old bootstrap/native
process, reservation release on normal Close, and Native→Windhawk switching.
Profile/task/native/styler bytes were preserved; no added panic-log bytes. This is
initial lifecycle evidence, not blanket Windows/alpha compatibility certification.

## Installation outside the local preview

1. Build the consolidated DLL; preserve native as fallback. Place assets/themes in
   a trusted absolute payload directory (existing Termielle `bin` is suitable).
2. Register the versioned DLL in Windhawk's **32** mod directory with Include
   `windhawk.exe`, Architecture `x86`, and the accompanying 0.4.0 source metadata.
   This preview targets the verified 32-bit Windhawk 1.7.3 installation only.
3. Configure PayloadDirectory, optional absolute Profile, and Surface. Exit native
   temporarily, not persistent Off. Enable only Termielle's mod. A portable setup
   can start `windhawk.exe -tool-mod "termielle"` if the manager doesn't launch it.
4. For the switcher, use a validated manifest with `Hosting: WindhawkX86`,
   HostExecutable equal to WindhawkExecutable, matching mod filename and paths.
   It refuses mismatched registrations and never installs unrelated mods.

Do not download/load untrusted DLLs. Follow endpoint-security alerts; a successful
build/test doesn't constitute antivirus clearance. An HP alert was reported during
this experiment; after the user's clarification and ordinary rebuild they reported
no recurrence and authorized deployment. No exclusions/restoration/security changes
were made. HP Sure Sense was observed stopped beforehand, so lack of alerts is not
claimed as a clean scan.

## Lifetime and privacy

ABI v1 is one-shot per **dedicated process**. Never load the runtime into the main
manager/Explorer, `FreeLibrary` an active runtime, or rerun in-place. The manager's
callbacks start no UI/decoder/IPC workers. Tool GUI/pipe stop gracefully; remaining
process-owned OS/decoder workers are released by tool exit. Restart needs a fresh
host. Exit paths are restricted to the exact owned tool; service/other-tool/foreign
executable/duplicate-flag cases are excluded before starting anything.

Runtime codes: 0 normal, 1 failure, 2 bad request, 3 another frontend, 4 Off, 5 already
used, 11 Restart. Bounded UTF-8 JSON validates paths/surfaces. Agent messages remain
content-free. Optional existing toast forwarding retains profile/Windows consent;
no reference clipboard/weather/notification scraping was imported. Window metadata
uses handles, rectangles/classes, not text/content.

The tool lifecycle is adapted from MIT Dynamic Island snapshot
`0ce97cdd4e1d98c79ae2946d419004c35c6ffe12` and official Windhawk guidance. Retained
notices: `LICENSES/Dynamic-Island-MIT.txt`, `THIRD_PARTY_NOTICES.md`. True native-
taskbar embedding, arbitrary-unload safety and performance certification remain
separate work.

// ==WindhawkMod==
// @id              termielle
// @name            Termielle
// @description     Full Termielle bar/pill/notch in Windhawk's own 32-bit tool process; one consolidated Rust DLL.
// @version         0.4.0
// @author          Termielle contributors
// @include         windhawk.exe
// @architecture    x86
// @license         Apache-2.0
// ==/WindhawkMod==

// ==WindhawkModReadme==
/*
# Termielle — consolidated x86 preview

This metadata accompanies a Cargo-built Rust DLL; it is not a C++ implementation
that Windhawk's editor can recompile. Build from the Termielle repository:

    cargo build -p termielle-runtime --release --target i686-pc-windows-msvc --features windhawk-x86 --lib

The resulting DLL includes the Windhawk adapter and shared Rust UI/runtime.
It runs in Windhawk 1.7.3's own disposable `windhawk.exe -tool-mod "termielle"`
process, not in the manager or Explorer. No Termielle host EXE or second runtime
DLL is needed. PayloadDirectory still supplies assets/themes. Native remains x64.

Bar honors saved reserved space; Windows' taskbar is never replaced/hidden.
Follow profile retains Bar/Island/Notch; Classic becomes Island. Explicit Surface
choices are host-only; hosted saves preserve native surface policy and custom
material. Persistent Off and the common single-instance/event-pipe claim apply.
Disable/re-enable after payload/profile/surface changes; Restart needs a fresh
process. At login, whichever configured frontend starts first wins.

One-shot process-owned runtime: never load into an arbitrary long-lived process
or unload/restart in-place. Tool shutdown ends only its verified dedicated process.
No clipboard/weather/notification scraping, security exclusions, elevation or
UIAccess added. Hosting/32-bit compilation do not prove resource savings.

The tool lifecycle is adapted from the MIT Dynamic Island reference and official
Windhawk guidance. See LICENSES/Dynamic-Island-MIT.txt and THIRD_PARTY_NOTICES.md.
See windhawk/README.md for build, review, installation and acceptance limits.
*/
// ==/WindhawkModReadme==

// ==WindhawkModSettings==
/*
- PayloadDirectory: '%LOCALAPPDATA%\Termielle\bin'
  $name: Termielle assets/themes directory
  $description: Trusted absolute payload directory. The runtime is already inside this mod DLL. Disable/re-enable after changing.
- Profile: ''
  $name: Optional profile path
  $description: Absolute config.json path. Empty uses the shared user profile. Disable/re-enable after changing.
- Surface: Profile
  $name: Windhawk surface
  $description: Profile retains Bar/Island/Notch. Bar honors saved reserved space; native taskbar replacement is always blocked. Does not rewrite native layout. Disable/re-enable after changing.
  $options:
  - Profile: Follow profile
  - Bar: Full bar
  - Island: Floating Island
  - Notch: Attached Notch
*/
// ==/WindhawkModSettings==

#error This is metadata for the consolidated Rust DLL. Build with Cargo as described above; keep the working mod installed until the replacement has passed review.

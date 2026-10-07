# Windhawk integration: upstream reference review

## Status

The subsequent [experimental shared-runtime/tool edition](../windhawk/README.md)
is implemented and was installed/live-exercised at user request on October 7.
Initial lifecycle switching passed with an owned x64 host. A later user-authorized
0.4.0 port consolidated the adapter/runtime into one x86 DLL in Windhawk's own
dedicated tool process, with Bar, ownership and graceful reservation cleanup
verified. No Explorer renderer was injected. See the edition guide for evidence/limits. The findings below describe original research.

Original stage: source inspection only. No Windhawk mod was installed, enabled, compiled or
loaded into Explorer, and no upstream implementation has been imported into
Termielle. The standalone app remains the fallback. This is not a completed
Windhawk port, security audit or resource benchmark.

User-provided reference:
[devcode90/Dynamic-Island-for-Windows](https://github.com/devcode90/Dynamic-Island-for-Windows).
Reviewed snapshot: **`0ce97cdd4e1d98c79ae2946d419004c35c6ffe12`**, source metadata
version **1.3.1**. Clone and documentation downloads are ignored under
`target/research/`, not shipped with Termielle.

## Additional visual/scheduling review — October 7

User asked to continue learning from the same reference after consolidated x86
deployment. Rechecked pinned HEAD (still0ce97cdd4e1d98c79ae2946d419004c35c6ffe12),
without installing upstream or capturing desktop/app contents. Useful next lessons:

- **Content-aware compact sizing:** `IdleStripLayout` (610–655) and
  `Renderer::MeasureIdleStrip` (6856–6911) use shared layout constants, measured
  text, bounded min/max width and two-pixel quantization. Sizer/painter agree;
  cached metrics invalidate on font/DPI changes, and widest-digit substitution
  avoids clock-width jitter. Apply the principle to our idle/media/session chips
  and bar module allocation, not their weather/privacy data collection.
- **Quiet scheduling:** render loop (11906–11999) separates manual/automatic hiding,
  blocks on stop/messages when parked, releases high-resolution timer requests and
  resets timing on restoration. Fullscreen parking still rechecks at1500ms; that
  is not literally zero wakeups. Termielle already parks hidden visual deadlines;
  remaining media/decoder/OS workers and always-visible idle animation need measured
  review before claiming whole-process savings.
- **Stable transitions:** `SpringValue::Step` (1194–1217) bounds substeps and snaps
  settled position/velocity. Learn convergence/restoration rules; do not blindly
  copy its0.5ms integration step or500Hz claims. Our existing spring/clock and
  reduced-motion contract should remain the shared implementation.

Already delivered related shared features: display selection/fallback, optional
Island/Notch fullscreen hiding and animation feel presets. Existing notch/lid
repairs predate this reference review. At this review stage, compact sizing and
idle-worker optimization were not implemented. The user-approved follow-up now
implements shared measured compact layout and subsequently deployed it to Windhawk
x86 at explicit user request (19:07 +07:00); native fallback remains unchanged. Idle tuning
was prototyped, measured and withdrawn because it showed no whole-process CPU win;
no resource savings are claimed. See [compact layout and measured idle review](compact-layout-idle.md)
for functional evidence, higher pilot CPU readings and remaining profiling gates.
No clipboard/network/weather/notification-body modules or global input hooks to
import. Retain MIT notices for any adapted code and preserve custom profiles.

## Important architecture finding

This is a genuine Windhawk mod, but it is **not an embedded taskbar widget**.
It uses Windhawk's **mods-as-tools** pattern:

- Metadata targets `windhawk.exe`, not `explorer.exe`.
- The loader starts a dedicated tool process using `-tool-mod <id>`.
- `WhTool_ModInit`, `WhTool_ModSettingsChanged` and `WhTool_ModUninit` own the
  actual UI/workers. The compatibility loader hooks the dedicated host's entry
  point; this is not a hook into Explorer's taskbar functions.
- `RenderThreadProc` creates its own top-level, layered, no-activate overlay via
  `CreateWindowExW`, and initializes a Direct2D renderer.
- The implementation owns media, audio, weather, notification, Bluetooth and
  input worker threads. Windhawk hosting alone does not remove their cost.

Evidence in the pinned `.wh.cpp`: metadata lines 1–11; overlay creation around
11753–11824; worker lifecycle around 12694–12791; tool loader around 12793–12972.
The repository's top-level README includes older Explorer-inclusion advice for
notifications, while the source's embedded README and actual metadata describe
the dedicated-host implementation. Use the reviewed source and official docs,
not that older instruction, to choose an architecture.

Official reference:
[Mods as tools: Running mods in a dedicated process](https://github.com/ramensoftware/windhawk/wiki/Mods-as-tools:-Running-mods-in-a-dedicated-process).
The fetched documentation describes the compatibility snippet for Windhawk
1.7.3 and separate first-class support in Windhawk 2.0 alpha. The alpha's header
is not available in 1.7.3, and its host executable/privilege choices differ.
Check the user's installed Windhawk version before selecting a build target;
do not require an alpha upgrade or elevated/UIAccess host without a reason.

## Reuse and privacy boundaries

Upstream is **MIT licensed**, copyright 2026 devcode90. Reusing substantial
source requires retaining the copyright and MIT permission notice, and crediting
upstream/contributors. Termielle's Apache-2.0 project license does not replace
that notice. Check the provenance/licensing of any separately borrowed loader
or dependency code before distributing it.

Useful candidates include mod metadata/settings, lifecycle structure, rendering,
shape/layout, monitor/DPI handling, fullscreen visibility and animation pacing.
These require independent review and testing; the README's zero-CPU/high-refresh
claims are not measurements of our future port.

Do not copy the complete implementation blindly. It includes clipboard text/
image handling, file references, weather requests and notification-body extraction
via UI Automation. Those features are not prerequisites for session activity and
must not become implicit collection behavior in Termielle. The agent lifecycle
must stay content-free: no prompts, outputs or tool payloads. Source review has
not established unload/cancellation guarantees for every detached worker or
network operation; those paths need auditing if reused.

## Recommended direction

Build a **Termielle mod compatible with Windhawk**, not a Windhawk replacement.
For the pill/notch, prefer an isolated **tool mod** over injecting a renderer
into Explorer. This coexists with native taskbar styling without claiming that
the pill is part of the taskbar. Actual embedding remains a separate, more
Windows-build-sensitive feature.

There are distinct implementation choices:

1. A thin lifecycle adapter managing the existing executable is an integration
   prototype, not a port or demonstrated memory saving.
2. A C++ tool frontend plus a real headless Rust engine reuses session logic but
   introduces another process and a separately authenticated, bounded UI bridge.
3. A tool host loading an extracted Rust runtime DLL could reuse the existing
   renderer/reducer in one dedicated process, but needs a carefully defined C ABI,
   GUI-thread ownership, panic boundaries and complete stop/unload guarantees.

Choose after a minimal lifecycle prototype and resource measurements, rather than
assuming any option is faster. Do not rewrite the Rust reducer in the upstream
monolithic C++ file simply to obtain Windhawk compatibility.

The existing persistent Off preference, explicit profiles and `show_name` choice
must remain authoritative. Mod activation must not silently undo Off or enable
native-taskbar replacement. Define which component owns startup/settings/stop so
there are no duplicate overlays, abandoned headless workers, or hidden pollers
left behind on disable. Retain standalone operation and native taskbar/tray access.

First acceptance gates: known Windhawk version/architecture, isolated host test,
repeat enable/disable with no residual workers, settings and Off behavior,
coexistence with installed styling mods, Explorer restart and fullscreen review,
then physical DPI/monitor/wake and recording tests. Nothing here proves those
human/live-shell acceptance gates have passed.

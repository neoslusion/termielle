# Shared desktop improvements — October 7, 2026

These improvements run in both native and Windhawk Termielle. They selectively
learn from the Dynamic Island reference's fullscreen/placement/animation ideas,
without importing its monolithic renderer, clipboard/weather collectors or
notification scraping. Defaults preserve existing profiles/material.

Later follow-up: [content-fit compact layout and idle review](compact-layout-idle.md).
Functionally tested and deployed to Windhawk x86 at user request at 19:07 +07:00;
the native fallback was not replaced. Measured pilots do **not** support a CPU/
memory savings claim, and experimental idle scheduling was withdrawn.

## Preferences > Behavior / display

- **Display:** Automatic preserves existing behavior. Primary pins an attached
  surface to primary; Follow pointer rechecks at 500ms; named choices resolve the
  chosen GDI display. Missing displays fall back to primary without rewriting the
  saved choice. Bar, Island and Notch use it; Classic remains freely draggable.
  Windows can renumber `DISPLAYn` after hardware changes; these are explicit GDI
  names, not a guarantee of permanent physical-monitor identity.
- **Animation feel:** Smooth, Balanced, Snappy and Bouncy set existing spring
  timing/bounce fields. Custom is a no-op and preserves hand tuning. Theme/glass,
  reduced-motion preference and unrelated values are untouched.
- **Hide pill in fullscreen:** optional, default Off, Island/Notch only. A visible
  foreground application must cover the chosen monitor; ordinary work-area
  maximization, desktop/shell windows, other-monitor fullscreen and own UI are not
  treated as fullscreen. No taskbar reservation/replacement changes are made.

Preview is runtime-only; Apply validates/stale-checks/atomically saves the selected
profile; Revert restores committed state. Windhawk's original standalone layout,
reservation and native-taskbar policy remain preserved in saved copies.

## Scheduling/privacy

Fullscreen observation is out-of-context WinEvent metadata, not DLL injection or
keyboard monitoring. Foreground/location/minimize events are filtered/coalesced;
callbacks are GUI-thread-owned and unhooked on disable/drop. Only handles, window
class and rectangles are inspected—no titles, text, pixels or agent payloads.

While hidden, the pill parks visual animation deadlines and clears backdrop
requests/cache epochs. Session events still drain; timed states catch up when
restored. Explicit pointer-following, or failed native hook registration, uses a
500ms recheck. This does not claim all OS/media/background workers are parked or
that the whole process consumes zero CPU. Memory/CPU comparisons remain pending.

Display enumeration temporarily sets/restores per-monitor thread DPI so selector
metadata is physical even before an overlay exists. It never changes display
modes, topology, cursor or global foreground focus.

## Validation and deployment

**519 passed, 0 failed, 2 existing manual tests ignored** on both x64/x86 workspace runs after the pointer-width port; two additional adapter tests passed. Regressions cover old
profile defaults, config/material/shell round trips, preset/custom preservation,
native draft Preview/Apply/Revert, unavailable monitors, explicit monitor geometry,
physical-DPI metadata, hidden-window recovery and WinEvent registration cleanup.
Release build, fmt/diff, established-allowance Clippy and seven doctor checks passed.

The hidden preferences review renders page 4 with WM_PRINT. Selected combo text
may be absent in WM_PRINT, so native-value assertions separately verify controls.
The image is not a desktop capture or live interactive-acceptance substitute.

Standalone was updated and the Windhawk tool edition installed/live-exercised at
user request. The x86-manager/x64-runtime mismatch discovered in initial live
attempts was corrected with x86 launcher/x64 adapter and an owned x64 bootstrap.
Repeated switching passed with one visible frontend, preserved profile/task/styler
and no new panic bytes. Windhawk 0.3.0 adds the shared full Bar surface, including
saved reserved-space behavior while always retaining the Windows taskbar. Saved
native surface settings remain unchanged. The later 0.4.0 x86 preview combines
adapter/runtime in one DLL inside Windhawk's own tool process, with live ownership,
bar reservation cleanup and Native→Windhawk switching verified. Native stays x64.
See [Windhawk guide](../windhawk/README.md).

Remaining acceptance: real fullscreen games/video, moving between differing DPI
monitors, reconnect/Explorer restart, lid/wake, recording, interaction/focus and
resource measurements. These were not simulated as proof of physical behavior.

# Launcher and capsule motion — 2026-10-16

This follows the [fast document launcher](document-search.md), borrowing the
short slide/fade and hover vocabulary of Linux desktop ricing without replacing
the native Windows renderer or adding continuous decorative effects.

## Delivered motion

- **Launcher entrance:** the launcher grows out of the bar's pill over 160 ms
  and collapses back into it over 120 ms. It was first delivered as a 160 ms fade-and-rise with a
  100 ms exit, which the
  [motion pass](motion-and-performance.md#second-pass-what-the-skills-said-about-frequency)
  removed (a window summoned from the keyboard is opened hundreds of times a
  day, and a fade there is latency, not polish). It came back as a connection to
  the pill, not a fade: see
  [the launcher grows out of the pill](motion-and-performance.md#the-launcher-grows-out-of-the-pill).
  With no bar to grow from, or under reduced motion, it appears and
  disappears instantly. Every way of closing it, including losing focus and
  launching something, collapses it; focus is not held for the collapse.
- **Selection highlight:** Up/Down moves the highlight over 60 ms. Selection
  itself changes immediately: Enter acts on the selected item, not wherever
  the animated background happens to be. Text, icons, and row hit regions do
  not move. Typing snaps the highlight to the newly ranked results rather than
  animating every query; unchanged asynchronous updates preserve a running glide.
- **Macchiato status capsules:** right-side capsule backgrounds and borders
  crossfade over 120 ms on pointer entry/exit. Icons and hit geometry do not
  scale or move. This includes passive readouts, not just clickable controls.
  The left rail's existing interactions and the notch's independent hover
  behaviour remain unchanged.

## Scheduling and accessibility

The launcher runs a display-synced clock (the same `AnimationClock` the overlay
uses, not a `WM_TIMER`), armed only while the selection transition is active
and parked at rest, on dismissal, or on reduced motion. Native EDIT input/IME
is available the instant the window appears, including while it is still
growing.

Capsule transitions use the controller's existing display-paced deadline path,
invalidate only the right zone cache, and keep the left zone cached. They stop
scheduling after settling even while the cursor remains over a capsule. No
permanent glow, particle loop, or spinning border is introduced. Existing media,
metrics, indexing, and other application workers retain their own scheduling;
this is not a claim of zero whole-process CPU.

Both features use the controller's resolved reduced-motion preference. Reduced
motion switches to immediate presentation/selection/hover, without an animation
timer. No saved timing preset, theme, material, taskbar, or connectivity setting
is rewritten. Notch/Control Center ownership and Clear All readiness are unchanged.

## Validation

```powershell
rtk cargo fmt --all -- --check
rtk cargo test --workspace -j 2
rtk cargo build -p termielle-app --release --bin termielle-app -j 2
```

Regressions cover interruption without a value jump, unchanged-target updates,
timer parking, typing during selection motion, reduced-motion cancellation,
capsule background changes, unchanged hit regions, retained left-zone cache,
and top/bottom bar placement at 100%/150% scale with Control Center open.

Validation passes **539 workspace tests**, with two pre-existing manual launcher
checks ignored, plus formatting and the release native-app build. The native
executable was backed up and redeployed; installed/build hashes match and saved
configuration/DLLs are untouched.

A scoped installed-window probe observed entrance opacity from 0 to 255 and an
eight-pixel movement, then a completed fade-out/hidden window. The settled
launcher screenshot was visually inspected at the machine's current DPI. The
probe used only Termielle-owned window messages, not global input injection,
and opened no applications/documents. Raw local artifacts are ignored
`target/motion-installed-probe.json` and `target/launcher-motion-review.png`.

Physical feel and mixed-monitor/DPI acceptance still require real interaction.
An app-only native deployment does not replace the installed Windhawk runtime.

Implementation references: [layered window opacity](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setlayeredwindowattributes)
and [window timers](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-settimer).

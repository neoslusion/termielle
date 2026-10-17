# Launcher and capsule motion — 2026-10-16

This follows the [fast document launcher](document-search.md), borrowing the
short slide/fade and hover vocabulary of Linux desktop ricing without replacing
the native Windows renderer or adding continuous decorative effects.

## Delivered motion

- **Launcher entrance:** a 160 ms ease-out fade and eight-logical-pixel rise.
  Escape/Alt+Space dismissal uses a shorter 100 ms exit. Activation loss and
  successful launch hide immediately so another application is never held up by
  a closing animation. Reopening during dismissal reverses from the current
  presentation instead of waiting for the old exit.
- **Selection highlight:** Up/Down moves the highlight over 110 ms. Selection
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

The launcher has one 16 ms native window timer, armed only while a reveal or
selection transition is active and killed at rest, on immediate dismissal, or
destruction. Entrance/exit use native layered-window opacity/position; reveal
ticks do not rerasterize the result canvas just to change opacity. Native EDIT
input/IME remains available during entrance.

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

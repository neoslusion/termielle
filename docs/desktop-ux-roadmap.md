# Desktop UX follow-up — approved scope

The user approved all four follow-up areas after installing the navigation /
background build on 2026-10-04. This document records the implementation plan;
it is NOT evidence that these additional features are delivered.

## Safety and scope

- Preserve all pre-existing working-tree changes and the installed user profile.
- Keep the Windows taskbar and native notification area available during review.
- Do not hide native shell affordances automatically or bypass app protections.
- Use explicit, owner-validated app/window identities; no guessed terminal links.
- No automatic approval, force termination, lifecycle protocol/content capture,
  sleep/wake injection, display-mode changes or monitor-topology changes.
- Separate implementation, synthetic/native tests and physical human acceptance.
- No replacement deployment until the added implementation is built and tested.

## 1. Application navigation polish

- Delayed hover tooltips: app name, running-window count and launch/switch action.
  Cancel on leave, press, popup opening, source removal or DPI/layout change.
- Drag reorder for existing explicit pins: stable identity, motion threshold,
  capture cancellation, insertion marker and one atomic save only on release.
  Escape, lost capture or missing app cancels without changing configuration.
- Clear and consistent hover, pressed, active and keyboard-focus indicators.
- Window chooser: icon, readable/ellipsized title, active/minimized state and
  owner-checked normal Close with keyboard access and explicit feedback.
- Launch feedback tied to app identity; suppress repeated dispatch while the
  request is pending and show failures without taking foreground focus.
- Regression coverage for reordered live snapshots, DPI, top/bottom, cancelled
  drags, stale HWND/PID, failed save rollback and launch completion.

## 2. Preferences window

- One nonmodal preferences window using native, accessible controls; singleton
  show/focus behavior, keyboard tab order, labels and DPI-aware layout.
- Layout, edge, density/height, theme, modules/order and pin management.
- Draft settings separate from the committed profile. Preview changes only
  runtime state; Apply validates and saves atomically; Revert/Cancel restores
  committed state. Failed saves retain the existing profile and show an error.
- Preserve arbitrary existing theme/tint overrides and unedited preferences.
- Honor the active --config path, including Restart/custom-profile behavior.
- Explain taskbar-replacement limitations and provide an immediate native-shell
  fallback. No preview silently enables replacement or changes native settings.
- Route Bar settings here; retain the existing tray menu as a fallback.

## 3. System-control flow

- Volume popover: slider, mute, current output and explicit output selection.
  Enumerate/operate audio on the COM worker; no GUI-thread device queries.
- Network popover: live connection details and Windows network-settings route;
  report unavailable/permission-limited information honestly.
- Battery popover: percentage, charging/power state and power-settings route.
- Clock: date/calendar entry separate from recent-notification access; explicitly
  distinguish Termielle's in-memory notifications from Windows' notification area.
- Shared popup anchoring, readable density, keyboard navigation, Escape/outside
  dismissal, focus ownership and clipped mid-morph hit targets.
- Keep activity and Control Center independent; test stale-action prevention.

## 4. Replacement readiness

- Native notification-area access: supported shell fallback, NOT Quick Settings
  mislabeled as tray overflow; no foreign-process toolbar scraping or injection.
- Deliberate monitor policy, clear selection and fallback when a chosen monitor
  disappears. Do not imply that one bar supplies secondary-monitor taskbars.
- UI Automation/accessibility for custom bar actions as well as native preferences:
  accessible names, roles, bounds, invocation and focus/navigation. Standard
  controls alone do not make the custom-drawn bar fully screen-reader accessible.
- Validate wake, DPI, Explorer restart and fullscreen behavior without changing
  the user's monitor topology or putting the host to sleep.
- Recording acceptance for scoped glass capture: concurrent recording frames can
  omit the overlay. Test explicitly; do not declare recording parity from a
  single screenshot or solve it by permanently excluding the bar from capture.
- Physical lid, mixed-monitor and recording acceptance require user-assisted
  checks. Retain native taskbar fallback until these gates are accepted.

## Current status

The command runner recovered on retry. The first implementation increment is
complete in source: navigation tooltips/drag reorder/launch feedback and a native
preferences window with Preview/Apply/Revert, pin/module ordering and recording-
safe glass. See [implementation and validation](preferences-navigation-polish.md).
489 workspace tests passed, 0 failed, 2 existing manual launcher tests ignored;
release build and Clippy with the pre-existing allowances passed. No replacement
deployment or real-profile edits were made for this increment.

Dedicated system popovers/device selection, distinct notification access,
custom-bar UI Automation, monitor policy and replacement-readiness checks remain
pending. Native OS routes in preferences are not those dedicated popovers, and
standard preferences controls do not provide accessibility to every bar action.
The physical/recording acceptance gates still require user-assisted checks.

## Historical command-runner blocker

After reviewing navigation state/painters, GUI routing, core configuration,
Windows events and the audio/metrics worker, the command runner stopped launching
Git/MSYS Bash. Two attempts failed before executing commands with:

```
fatal error - add_item ("\\??\\C:\\Program Files\\Git", "/", ...) failed, errno 1
```

A subsequent `echo ready` timed out. File tools still work, but builds/tests and
shell diagnostics cannot currently run. No additional implementation or installed
binary changes had been made when the blocker was recorded. It has since been
resolved and the first increment above implemented; continue the remaining
sequence with testable increments and evidence.

Related: [app navigation](app-navigation.md), [notch background](notch-background.md),
[development handoff](development-handoff.md).

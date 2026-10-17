# Notification drawer and Clear All repair

## Reproduced failure

The earlier click-readiness fix removed a 98%-morph input gate, but did not
stabilize the Clear All button's position. The footer was laid out immediately
after the current history rows. A notification arriving between mouse-down and
mouse-up moved the button by 104 logical pixels, so release no longer hit it.
This is a confirmed failure path, not proof that it explains every user report.

The new regression first failed with Clear All moving from y=212 to y=316.
It now passes without changing the notification-clear scope.

## Interaction changes

- Nonempty history uses a bounded 424-logical-pixel drawer. The full-width Clear
  All footer stays in place as notifications arrive, update, or scroll.
- Three readable rows are visible at once. The existing four-entry retention
  limit remains unchanged; wheel/touchpad scrolling reveals the older entry.
- The subtitle shows the visible range and a scrolling hint when needed.
- Clear All clears the complete retained history, including off-screen entries,
  resets scrolling/unread state, and updates the unread clock indicator.
- Panel-local hover coordinates now drive Clear All's visible hover feedback.
  Moving within the same button does not add a continuous repaint timer.
- Empty history retains the existing 180-pixel caught-up card. Closing/reopening
  history starts at the newest entries. Control Center remains independent.

Clear All affects **Termielle's recent history**, not Windows Notification Center,
live agent state, or the active/queued notch banners. A later genuinely new
notification can populate history again. No Windows notification permissions or
system settings are changed.

## Caelestia reference

Assuming “Celestia Shell” means [Caelestia Shell](https://github.com/caelestia-dots/shell),
the useful pattern is a scrollable notification region with independent action
chrome. Its [notification dock](https://github.com/caelestia-dots/shell/blob/main/modules/sidebar/NotifDock.qml)
uses a list viewport and a separately anchored clear action. This repair adopts
that separation in Termielle's existing Rust renderer; it does not port QML,
install Hyprland/Quickshell, copy GPL implementation code, or replace the macOS
palette. Source grouping and per-item dismissal remain possible follow-ups.

Caelestia owns notification-server objects; Termielle currently presents its own
recent history. Their [notification service](https://github.com/caelestia-dots/shell/blob/main/services/Notifs.qml)
therefore has different clear semantics. Adopting the layout does not silently
grant Termielle control over Windows' notification store.

## Validation

The 2026-10-16 workspace run reports **582 passed, 0 failed, 2 ignored** (existing
manual launcher checks). Formatting, whitespace checks, and the native release
build pass. Dark/light fixture renders verify the latest and older history views.

- Failing-before/passing-after arrival-during-press regression.
- Native hidden-window regression sends mouse-down/up only to app-owned HWNDs,
  pumps the real window events, introduces an arrival during the press, and
  checks clearing for top/bottom placement at 100% and 150% rendering scale.
- Scroll bounds, outside-panel wheel handling, Clear All after scrolling, and
  unchanged action geometry are checked at 100%, 150%, and 200% scale.
- Hover feedback, old morph-readiness checks, full notification lifecycle tests,
  and outside-popup containment remain covered.

These checks do not generate Windows toasts, manipulate another application's
windows, or inject global keyboard/mouse input. Actual pointer use on the user's
desktop remains the final acceptance check.

# Multi-session activity and window links

The first agent-cockpit milestone adds a content-free session list to the
expanded activity card in Bar, Island, and Notch layouts. Classic remains the
animated companion. The `agents` widget enables the list; no new configuration
or hook/protocol fields are required.

## Use it

1. Start one or more supported agents, then click the Termielle pill. Bar hover
   also opens the card when `expand_on_hover` is enabled and sessions are tracked.
2. Each row shows the source, a shortened session identifier, status, and elapsed
   time. Choose **Link** (or click an unlinked row) to open the window picker.
3. Select the open window containing that session. The card returns to the list;
   selecting a target does not immediately switch focus.
4. Click the linked row to restore/focus its window. **Change** reopens the picker;
   **Unlink** removes the association. **Back** leaves it unchanged.

Lists show four entries per page with **Prev** and **Next**. The session count
includes all tracked sessions, not only those on the current page. The window
picker uses a lightweight catalog of up to 64 eligible open application windows,
independent of the six decoded icons used by the app rail. An agents-only layout
polls metadata without extracting task artwork. Minimized windows are
eligible; Termielle's own windows, cloaked windows, and tool windows are excluded.

Clicking a control in a hover-opened activity card pins it open while you choose
a target. Escape, outside click, or clicking the pill closes it as before.
Control Center and notification history retain their independent right popover.
Fresh alert banners temporarily replace the activity view under the existing
alert rules. When media is available alongside agents, the activity list remains
visible and its header offers Play/Pause; the full media card is used when no
agent sessions are tracked or the `agents` widget is disabled.

## Status and lifetime

- Ordering follows the reducer's existing attention priority: input needed,
  failed, ready, thinking/working, then idle. Within a priority, newer activity
  wins; source and session identifier resolve ties deterministically.
- Elapsed time is time in the current state, based on event/scheduled-transition
  timestamps, not the moment a delayed repaint happened. Repeated same-state
  signals do not restart the state age.
- A completed turn is labeled **Finished**, including after the Ready hold has
  decayed to Idle. Its age is time since completion. An explicitly opened card
  stays open across completion; it does not disappear when the Ready hold ends.
- A busy-stall decay is **Idle**, not a fabricated completion. Sessions leave on
  `session_ended` or the reducer's four-hour stale horizon.
- Non-primary events and scheduled transitions repaint the list even when the
  primary face has not changed. Ages refresh approximately once per second only
  while the list is painted; the picker and closed card add no age repaint loop.

## Privacy, safety, and limits

Associations are **explicit, local, and in-memory**. No foreground-window guesses,
terminal-output parsing, prompt capture, command execution, or automatic approval
are added. Window titles come from the existing desktop window enumeration and
are not sent through lifecycle IPC or written into the event journal. Links do
not survive a restart, even when journal replay restores the sessions.

A link is keyed by source plus full session identifier. It is removed when the
session disappears or its window is absent from the catalog. Changing a terminal
window title does not break a link. Focus checks the owning process both against
the latest snapshot and again through Win32, rejecting a handle reused by another
process. This is not a persistent window identity or a guarantee against all
same-process HWND/PID reuse races.

This milestone focuses **windows, not individual terminal tabs or panes**. If
several sessions share one terminal window, they may all link to that window.
Windows foreground policy can refuse a focus request; this does not execute a
fallback command. Friendly labels, persisted workspace launch actions, attention
hotkeys, and terminal-specific tab navigation are follow-up work, not delivered
features.

## Validation and isolated review

```powershell
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo run -p termielle-app --example session_activity_review -- target/session-activity.bmp --bar --media
cargo run -p termielle-app --example session_activity_review -- target/session-picker.bmp --bar --bottom --picker
cargo run -p termielle-app --example session_activity_review -- target/session-light.bmp --light
```

The example renders synthetic sessions/windows into a BMP. It does not activate
live windows, install hooks, or change installed configuration. Controller tests
cover explicit linking/focus outcomes, source isolation, pagination, secondary
updates, window disappearance/process changes, title changes, DPI, visible
mid-morph controls, and independence from Control Center. Core tests cover
snapshots, state ages, completion retention, and secondary revisions.

The implementation validation recorded **430 passing workspace tests and two
ignored manual launcher tests**, plus synthetic dark/light and bottom-picker
visual review. Formatting and the release app/review-example build also pass. Strict Clippy
remains blocked by pre-existing warnings in the Control Center painter, launcher,
existing review example, and island tests (`unused_mut`). Live terminal activation and human pointer confirmation remain
separate from these tests. That initial validation did not redeploy the installed
app. The subsequent [lid/display-recovery update](display-recovery.md) includes
this activity implementation and has now been installed locally; its runtime
checks do not replace real session-focus or physical lid-cycle acceptance.

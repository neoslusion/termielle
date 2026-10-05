# Desktop bar development handoff

Snapshot: **2026-10-02**. This records the desktop-bar refinement work through
the working Alt+Space launcher. It describes the current working tree and local
installation, not a claim that these changes have been committed or published
as a GitHub release.

## Documentation map

- [README](../README.md): installation, agent integrations, privacy, and workspace.
- [Island and bar guide](island.md): detailed interaction, rendering, and configuration.
- [Reported behaviours](reported-behaviours.md): root causes, fixes, and verification caveats.
- [Performance follow-up](performance.md): later idle/caching improvements, measured results, and the Clear All morph fix.
- [Session activity](session-activity.md): later multi-session list, explicit window links, validation, and limits.
- [Display recovery](display-recovery.md): lid/Modern Standby crash fix and subsequent local deployment of the activity/recovery build.
- [App navigation](app-navigation.md): newer pinned/running rail, chooser, overflow, app actions and settings access; installed at user request on 2026-10-04.
- [Notch background](notch-background.md): confirmed self-capture repair, scoped capture policy, edge math and stale-background rejection; included in the 2026-10-04 installed build.
- [Desktop UX roadmap](desktop-ux-roadmap.md): approved follow-up scope and acceptance gates.
- [Whole-application power](application-power.md): persistent tray/preferences Off and external On, isolated tests, no resident app while disabled; not yet deployed.
- [Preferences/navigation polish](preferences-navigation-polish.md): first follow-up increment in source, not installed; 489 tests passed, with system popovers/replacement readiness still pending.
- [Event protocol](protocol.md): the content-free agent lifecycle contract.
- This document: agreed direction, delivered work, source map, validation, and handoff.

## Agreed direction

Termielle is a **macOS/SketchyBar-inspired native Windows menu bar**, with an
iOS-inspired activity pill. It does not run SketchyBar or Waybar, and it is not
a complete replacement for the Windows shell.

User-selected visual references:

- [SketchyBar](https://github.com/felixkratz/sketchybar).
- [The top-sorted showcase discussion](https://github.com/FelixKratz/SketchyBar/discussions/47?sort=top).
- [FelixKratz's referenced dotfiles snapshot](https://github.com/FelixKratz/dotfiles/tree/e6288b3f4220ca1ac64a68e60fced2d4c3e3e20b).

These are design references, not dependencies or a promise of pixel-identical
macOS behaviour. Asset provenance remains in the repository's third-party notices.

| Decision | Current contract |
| --- | --- |
| Notch and Control Center | Independent open/close and hover state; both cards can be visible together in Bar layout. |
| Empty pill hover | Stay collapsed; never open a filler Idle card. |
| Empty pill click | Open Today in Bar layout; standalone Island/Notch retain their system dashboard. |
| Left rail | Open applications and focused-window context, not virtual desktops by default. |
| Notification arrival | Grow from the resting pill; do not leave a duplicate pill underneath. |
| Native taskbar | Keep it and its notification-area icons visible: `replace_taskbar: false`. |
| Connectivity controls | Live status where available, with safe Windows Settings handoff. |
| Search | Local app launcher, not Windows Search; Alt+Space opens it. |
| Styling | Catppuccin Macchiato, rounded grouped capsules, readable icons, restrained motion. |

Independence does not mean that hover pins a card forever: leaving a
hover-opened notch still starts its own close grace. Opening Control Center
must not clear, freeze, or re-arm the notch's hover state.

## Delivered experience

### Bar styling and left-side refinement

- The original six-window rail is superseded in the working tree by stable
  pinned/running app groups, count/active indicators, a multi-window chooser,
  paginated overflow and explicit app actions. Bar settings no longer requires
  tray access. A focused-window label remains lower-priority context;
  `workspaces` is optional. See [app navigation](app-navigation.md) for current
  validation and acceptance boundaries.
- Macchiato groups the rail and right-side status items into rounded capsules.
  Capsule padding was increased so boundaries do not crowd the glyphs.
- Macchiato status glyphs use an 18-logical-pixel size. Tabler path-based icons
  are drawn at render scale rather than enlarged low-resolution bitmaps.
- Native window-icon selection prefers the largest available artwork so a
  small per-window icon does not displace a better class icon. App artwork
  quality still depends on what that app supplies.
- Left-zone overflow drops lower-priority content rather than painting or
  leaving invisible click targets underneath the center pill.
- The default right strip is Network, Volume, Battery, Control Center, Clock.
  CPU and Memory live in Control Center unless explicitly pinned.

### Pill, hover, and Today

- Empty-state hover stays quiet. In Bar layout, clicking opens **Today**:
  current date, latest observed notification or “All caught up”, Search, and
  Notifications shortcuts. No expanded Idle placeholder.
- With live content and `expand_on_hover: true`, the Bar pill opens its detail
  card after a **300 ms dwell**. A **500 ms exit grace** lets the pointer cross
  the transparent gap into the card. Standalone layouts retain their smaller
  hover presentation.
- Click dismissal suppresses immediate hover reopening until the pointer
  leaves and returns. Hover detection uses the notch's own geometry, not all
  opaque pixels or the entire enlarged overlay window.
- Agent activity, media, and notifications remain content-driven. Paused media
  stays available so playback can be resumed; its equalizer stops animating.
- Geometry springs preserve velocity when interrupted; content swaps blend
  without restarting the container. Reduced motion is respected.
- Standalone Island/Notch layouts, split agent/media blobs, and the Classic pet
  remain available; the full-width Bar is the current daily layout.

### Notification presentation and recent history

- Alerts have a source/avatar, a clear headline, optional detail, and a subtle
  lifetime line. Agent copy uses “Needs your input” and “Turn failed”.
- In Bar layout, an arriving alert takes over the center pill's presentation
  and grows into its card. The resting pill returns after dismissal/expiry;
  it is not painted as a second notch behind the alert.
- Agent input/failure alerts last **3.5 seconds**; forwarded Windows toasts
  last **6 seconds**. Only the visible alert consumes its timeout.
- The queue holds one visible alert and up to three deferred alerts. An
  actionable agent alert can preempt a passive toast; the deferred toast
  resumes with a fresh lifetime. Stable event identities update existing alerts.
- The lifetime line repaints every **16 ms** while visible, rather than waiting
  for the slower metrics refresh. Countdown repaint requests stop on exit.
- Clicking a transient banner dismisses it; this does not activate the
  originating Windows notification or launch its application.
- Clicking the **clock** opens recent notifications. The latest **four**
  observed alerts show source and local arrival time; a clock dot marks unread
  arrivals. The panel has a close button and, when populated, **Clear all
  notifications**.
- Clear All clears Termielle's recent list and unread indicator, not Windows
  Notification Center. This history is in-memory only and disappears on restart.
- Outside-click checks use the notification panel and its own entry controls,
  not the full transparent window rectangle. Escape also dismisses popovers.

Windows toast forwarding requires notification-listener access. It does not
replay existing Action Center notifications at startup and is not a complete
mirror of Windows notification history.

### Control Center

- A right-zone menu-bar icon toggles its own detached popover. The notch can
  remain open beside it, including when a fresh alert arrives.
- Network reports Online, Limited, Offline, or an unavailable status.
  Bluetooth reports On, Off, Disabled, or unavailable. Unknown readings are
  not presented as a falsely definitive Off state.
- Network, Bluetooth, Focus, and Display tiles open Windows Settings pages.
  Focus and Display are Settings links, not live toggles. Direct radio
  switching was deliberately not implemented for this unpackaged app.
- The Sound card supports mute, absolute volume selection via the slider, and
  wheel adjustment. Now Playing shows play/pause only when a session exists.
- CPU and memory remain available as readouts; the footer opens Settings home.
- The tray's **Menu Bar Items** submenu pins/hides Network, Volume, Battery,
  CPU, and Memory. Preferences stay in Termielle's tray menu.

Control Center and recent notifications share the **right-side popover slot**:
opening one replaces the other, but neither takes ownership of the notch.
Shared Escape/outside dismissal closes a right popover before a manually
opened notch when both are open; it is not an unconditional close-all action.

### Volume feedback

- A system output-volume or mute change temporarily replaces resting pill
  content with a speaker, level track, and percentage or “Muted”.
- Feedback lasts **1.5 seconds after the last change**. The startup snapshot
  does not create feedback.
- Core Audio endpoint notifications wake the metrics worker, so media keys
  and Windows sound controls are observed without waiting for a periodic poll.
- Alerts and expanded notch cards take priority. Feedback does not resize the
  pill, change hover/manual expansion, or close Control Center.
- The metrics worker owns the COM apartment and endpoint watcher. Callbacks
  enqueue work rather than render or move COM proxies to the GUI thread.

### Spotlight-style app launcher

| Action | Input |
| --- | --- |
| Open/toggle | Alt+Space, Today → Search, or Find in optional taskbar-replacement mode. |
| Search | Type an application name in the native Unicode text field. |
| Select | Up/Down; selection wraps through visible results. |
| Launch | Enter or click a result. |
| Select query | Ctrl+A; native editing and clipboard shortcuts also work. |
| Dismiss | Escape, Alt+Space again, or activation loss after clicking outside. |

- Apps come from Windows' registered **AppsFolder** catalog, including
  registered desktop and Store applications. It is not a filesystem scan.
- Discovery runs on a background STA worker at startup. Opening after at least
  one minute requests a refresh; there is no continuous disk indexing.
- Matching is in-memory: exact/prefix, word/substring, word initials such as
  `vsc`, and bounded fuzzy subsequences. At most **six** results are shown.
  An empty query shows alphabetically ordered entries.
- Names and shell targets become available before icons. Only visible results
  request icons; extraction is asynchronous and cached.
- The launcher is a separate focusable Win32 window with a real EDIT control,
  IME-aware keyboard handling, theme colours, and DPI-scaled rendering. It
  does not set notch or Control Center state.
- Launch uses the registered shell target on a background STA worker. Failures
  appear in the footer; stale launch completions cannot hide a reopened window.
- Alt+Space is fixed in this version. If another app owns it, Termielle does
  not steal it; use Today → Search or remove the other binding. Win+S is
  untouched. File/web search, arbitrary commands, and query logging are absent.

## Important fixes and lessons

The [behaviour log](reported-behaviours.md) preserves the original click,
flicker, state-coupling, and alpha-hit-target investigations, plus later reports.
Two launcher issues deserve explicit handoff notes:

1. **Catalog discovery was blocked by icon extraction.** The initial prototype
   read all app icons before publishing names. On this machine, 317 entries
   took about 68 seconds. Separating catalog publication from visible-only icon
   loading reduced name discovery to roughly half a second in local checks.
   These are historical observations, not cross-machine latency guarantees.
2. **Alt+Space did nothing after the first deployment.** The overlay's
   `GetMessageW` filtered messages to the bar HWND, excluding the launcher's
   separate HWND and EDIT child. Removing the HWND filter makes the GUI pump
   dispatch messages for the entire thread. Direct synchronous-message tests
   had missed this production integration bug.

The regression `production_message_pump_dispatches_launcher_hotkey_and_edit_input`
posts launcher and edit messages through the real `OverlayWindow::next_event`
pump. It failed with the old filter and passes with the fix. Local verification
also checked queued open, typing, and Escape against the installed executable;
the user subsequently confirmed Alt+Space works.

Distinguish controller tests, native-window dispatch, installed-binary checks,
and actual human pointer/keyboard confirmation. A running process or registered
hotkey alone does not prove a usable feature.

## Architecture and source map

Paths in this table are relative to `crates/termielle-app/` unless stated otherwise.

| Area | Primary source |
| --- | --- |
| Protocol, session priorities, config, journal | `crates/termielle-core/src/` (repository-relative) |
| GUI loop, worker updates, Settings/launcher routing, outside dismissal | `src/main.rs` |
| Layered overlay, alpha hit map, thread-wide GUI message pump | `src/window.rs` |
| State, alerts/history, deadlines, hover ownership | `src/app/controller.rs`, `src/app/island.rs` |
| Bar geometry, zone painting, typed module registry, partial damage | `src/app/bar/` |
| Today, Control Center, notification-history painters | `src/app/cards/` |
| Transient banner content | `src/app/paint/alert.rs` |
| Audio/connectivity snapshots and serialized commands | `src/bar/metrics.rs`, `src/bar/volume.rs`, `src/bar/connectivity.rs` |
| Shell affordances and optional taskbar reservation/replacement | `src/bar/shell.rs`, `src/bar/appbar.rs` |
| Media, task/window icon extraction, backdrop worker | `src/media.rs`, `src/tasks.rs` |
| Launcher search model, shell catalog, painter, native window | `src/launcher/model.rs`, `catalog.rs`, `paint.rs`, `window.rs` in the same directory |
| Vector status icons | `src/animation/icons.rs` |
| Controller interaction regressions | `tests/island_controller.rs` |

The GUI thread owns presentation state and renders cached data. Workers own
shell/COM work and publish snapshots or messages. Bar side layers are cached;
metric damage can repaint one zone without rebuilding the opposite zone.
Transparent gaps remain click-through, while declared controls get explicit
hit coverage even where their glyph contains blank pixels.

Agent integrations retain the content-free pipeline: hook/plugin →
`termielle-emit` → owner-only IPC → reducer → controller → native presentation.
No prompts or terminal output are captured. See the README for Claude Code,
Codex, opencode 1/2, and Antigravity details. The event journal survives restarts;
it is separate from the nonpersistent notification-history list.

## Current local configuration

Config: `%USERPROFILE%\.termielle\config.json`. These relevant values were
verified on 2026-10-02. This is a **partial reference**, not a replacement for
the user's full file; preserve unrelated settings and explicit glass overrides.

```json
{
  "scale": 1.0,
  "render": "per_pixel",
  "frame_rate": 60,
  "reduced_motion": "system",
  "island": {
    "layout": "bar",
    "theme": "catppuccin-macchiato",
    "collapsed_width": 180,
    "expanded_width": 420,
    "minimal_width": 80,
    "height": 40,
    "widgets": ["agents", "music", "face"],
    "face_animated": true,
    "expand_on_hover": true,
    "auto_hide": false,
    "forward_toasts": true,
    "show_tasks": false,
    "bar": {
      "position": "top",
      "height": 36,
      "reserve_space": true,
      "replace_taskbar": false,
      "follow_active_monitor": false,
      "modules_left": ["apps", "window"],
      "modules_center": ["termielle"],
      "modules_right": ["network", "volume", "battery", "control_center", "clock"]
    }
  }
}
```

`follow_active_monitor: false` pins this installation's Bar to the primary
monitor; it is not the configuration type's default. `island.height` and
`island.bar.height` are distinct. Tray changes persist, and validated island
configuration changes reload live.

## Build, review, and redeploy

Run from the repository root on Windows with Rust and PowerShell 7. Commands
use the RTK prefix required by this workspace.

```powershell
rtk cargo fmt --all -- --check
rtk cargo test --workspace
rtk cargo build -p termielle-app --release
```

Focused regressions and isolated visual review:

```powershell
rtk cargo test -p termielle-app --test island_controller
rtk cargo test -p termielle-app launcher::
rtk cargo run -p termielle-app --example render_review -- target/bar-review.bmp idle --bar --compact --macchiato
rtk cargo run -p termielle-app --example render_review -- target/notification-review.bmp notification --bar --macchiato
rtk cargo run -p termielle-app --example render_review -- target/volume-review.bmp idle --bar --compact --macchiato --volume-feedback --volume=65
```

These ignored tests are **manual** checks: the first writes
`target/launcher-review.bmp`; the second actually opens Calculator. Do not
include them blindly in an unattended run.

```powershell
rtk cargo test -p termielle-app live_launcher_visual_review -- --ignored --nocapture
rtk cargo test -p termielle-app registered_calculator_can_be_launched_without_windows_search -- --ignored --nocapture
```

Installation and restart ownership:

- Installed binary: `%LOCALAPPDATA%\Termielle\bin\termielle-app.exe`.
- Startup/watchdog scheduled task: **Termielle**.
- Build output: `target\release\termielle-app.exe`.
- For a local redeploy, finish validation first; stop the scheduled task and
  installed instance before replacing a locked executable. Identify the process
  by its full executable path, not every process sharing its name.
- Save a timestamped executable backup, copy the new app binary, and verify
  source/destination SHA256 hashes match. Do not replace configuration, assets,
  integration hooks, or DLLs for an app-only update.
- Restart through the existing scheduled task and confirm one installed
  instance. Recheck native taskbar visibility and the real Alt+Space workflow.
- Rollback uses the same stopped-instance procedure with the executable backup;
  config remains untouched.

No build or redeploy was performed merely to write this documentation.

## Verified baseline and remaining limits

The final launcher-fix run on **2026-09-30** recorded **403 passing workspace
tests**, no failures, and two normally ignored manual tests. Both manual checks
were also exercised during launcher implementation. Formatting and the release
build passed. Existing `unused_mut` warnings in `tests/island_controller.rs`
were not part of this work and remain unrelated.

On **2026-10-02**, the saved full-workspace log was checked, the Termielle task
was Running with one installed app process, and the installed executable's
SHA256 matched `target/release/termielle-app.exe`. This does not replace a new
test run after future source changes. The latest fix backup is
`termielle-app.exe.bak-20260930-145228` beside the installed executable.

Manual acceptance checklist for the next change:

- Empty pill hover stays collapsed; click opens Today.
- With live content, hover opens after dwell and survives travel into the card.
- Control Center coexists with the notch/alert without flicker or state resets.
- Notifications replace the resting pill; the clock panel closes easily, and
  Clear All affects only Termielle's list.
- Volume changes show temporary feedback without overriding an expanded card.
- Real Alt+Space opens; typing filters; navigation and app launch work; Escape
  and outside activation dismiss. Also check Today → Search.
- Native taskbar/tray remain visible; inspect scaling after a monitor/DPI change.

Known boundaries and possible follow-up work, **not already implemented**:

- No direct Wi-Fi/Bluetooth, Focus, or brightness switches in Control Center.
- No persistent history or full Windows Notification Center mirror.
- Launcher coverage depends on registered AppsFolder entries; arbitrary portable
  executables may not appear. Hotkey customization, recents/favourites, files,
  calculations, and broader search providers are future possibilities.
- Native tray icons, jump lists, taskbar previews, and all Windows shell surfaces
  are not replaced. Optional taskbar replacement remains a separate mode.
- The original padded Control Center target fix has regression coverage, but
  its historical exact blank-space physical-click check was not explicitly
  closed out. General positive UI feedback is not that specific measurement.
- Media/toast snapshot polling, mixed-DPI regression capture, and cancellable
  IPC shutdown are further engineering work listed in the guide.
- The desktop UI is Windows-native; portable core/IPC/emitter support does not
  imply a macOS or Linux bar backend.

This is a working baseline. Further work should preserve surface independence,
content-driven hover, safe Settings handoff, and the thread-wide message pump
rather than trading them for more decoration.

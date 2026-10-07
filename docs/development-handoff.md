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
- [Windhawk tool edition](../windhawk/README.md): shared Rust executable/DLL runtime plus C++ adapter; initial live preview installed on October 7; one-shot owned-host lifetime.
- [Windhawk reference review](windhawk-integration-research.md): MIT upstream tool-mod architecture and original research evidence.
- [Content-fit compact layout / idle review](compact-layout-idle.md): shared measured geometry, 522/0/2 on both architectures; user-authorized Windhawk x86 deployment at 19:07 +07:00, PID4984. Native fallback preserved; idle tuning withdrawn after unsuccessful CPU measurements.
- [Whole-application power](application-power.md): persistent tray/preferences Off and external On; executable controls deployed on 2026-10-06, with no installer/shortcut changes.
- [Preferences/navigation polish](preferences-navigation-polish.md): deployed in the 2026-10-06 standalone binary; manual UI acceptance and system popovers/replacement readiness remain pending.
- [Event protocol](protocol.md): the content-free agent lifecycle contract.
- This document: agreed direction, delivered work, source map, validation, and handoff.

## Follow-up publication scope

After compact-layout deployment, the user requested committing and pushing the
completed follow-up on `main`: branding preference, shared executable/DLL runtime,
desktop behavior controls, consolidated x86 Windhawk tool, compact layout, review/
switching scripts, CI/release packaging and documentation. See Git history for the
publication commit. Local research, diagnostic binaries/logs and private deployment/
profile/task backups remain excluded from Git. Historical "uncommitted" statements
below describe their earlier stages, not the publication status of this checkout.

Latest deployment evidence is in [compact layout / idle review](compact-layout-idle.md).
The installed native fallback was not replaced; higher pilot CPU results are
reported, and experimental idle tuning was withdrawn rather than claiming savings.

## Consolidated x86 Windhawk deployment — 2026-10-07

User clarified the endpoint alert was HP, reported no recurrence on an ordinary
rebuild and explicitly authorized deployment/continuation. Implemented the x86
adapter in `crates/termielle-runtime/src/windhawk.rs`, feature `windhawk-x86`, with
adapter/runtime in **one DLL**. Metadata is `windhawk/termielle-x86.wh.cpp` (Cargo-
built Rust, deliberately not editor-recompilable C++). Native remains x64. The old
0.3.0 payload and bootstrap are retained unchanged as rollback, not in active use.

Adapter mirrors Windhawk1.7.3's SDK C ABI and exports exact cdecl callback/data
names verified in engine source/PE metadata. Launcher starts Windhawk's own
`windhawk.exe -tool-mod termielle`; only that verified tool may start UI workers
or take process-exit paths. Manager/service/other-tool/foreign-exe/duplicate flags
are excluded or launcher-only. Own entry-point hook is the official disposable
host compatibility pattern; no Explorer/input hooks. One-shot lifetime remains.

Validation: **519/0/2 on each x64 and x86 workspace run**, two x86 adapter unit tests,
eight isolated x86 DLL-host checks, both established-allowance Clippy passes,
release/fmt/diff and PE32 export/installed-engine API checks. Pointer aliases were
ported; style-bit tests use architecture-neutral Long. An existing smoke test
assumed all monitors were right of primary; the user's current left-of-primary
setup exposed that assumption. Corrected the test to require placement inside an
available work area, not identical primary-origin placement. No display topology
or modes changed by the agent.

Installed at **2026-10-07T17:06:58.7701268+07:00**, mod0.4.0 SurfaceBar,
HostingWindhawkX86. DLL `termielle_0.4.0_x86_20261007-170641.dll` in Windhawk's **32**
mod directory; hash `B460EF69F4EC2D6DAEF0F93F7DEE152EEB5BEF29EB01D39A16FE9DA5C6CFD84F`.
Final expanded callback diagnostics found that settings logging without an SDK
mod-context pointer could fault when a Windhawk engine was resident in an ordinary
review process. Added null-context and tool-role guards; all eight isolated modes,
including the ordinary init/settings/after-init/uninit callbacks, then passed.
Final reviewed DLL `termielle_0.4.0_x86_reviewed_20261007-172621.dll`, hash
`75ED9ABC5B14EED2E57F6257D62428E8EBF188FCD554D4F7020119DFE3932357`, was activated.
At17:26:54+07, fresh owned x86 tool PID**36140** passed module/window/profile/task/
styler/panic checks and normal Close/Native→Windhawk review. This supersedes first
activation PID26032 below. Seven doctor checks and final fmt/diff passed as well.

Backup `%LOCALAPPDATA%\Termielle\backups\windhawk-x86-20261007-170641` stores old
manifest/INI/source/switcher/guide and private profile/task snapshots. Native hash
stayed `97DA38D8393038092BCABBFD6176FDA4FA7CEED0659633205FD80BC39859A3EC`.
Preflight profile now `B9CF3658528B1D3436D6F113D6D0ABE635464A3C4196C333B7E2331955A5E89A`
(user changed since earlier review, preserved rather than overwritten). Styler/task
unchanged; panic2202bytes, Off absent. No security settings/exclusions/restoration.

At **2026-10-07T17:11:51.9280530+07:00**, owned tool PID**26032**, WOW64 machine14C/
native8664; one matching Termielle DLL and real x86 engine loaded. No native or old
bootstrap process. Visible bar **(0,0)-(2560,36)**, primary work area top36/bottom1392,
both Windows taskbars visible; secondary now left of primary in the user's current
setup. Normal Close stopped first tool PID36320, releasing top36→0. Public Native→
Windhawk switch succeeded and restored one bar/reservation. Task naturally Ready.
Launchers use updated manifest-aware switcher; logon policy left unchanged.

Evidence: ignored `target/windhawk-x86-{workspace-tests,all-tests,mod-tests,clippy,
mod-clippy,mod-build,consolidated-review,deployment,live,lifecycle}.log/json`,
`windhawk-x86-exports.json`. CI/release now build/review/package the x86 DLL alongside
native/legacy fallback. Do not claim arbitrary unload, antivirus clearance, lower
resources or physical fullscreen/lid/recording acceptance. HP Sure Sense had been
observed stopped before the rebuild; no-alert report is not a clean scan. Source
changes remain uncommitted; no push. Latest live edition is **consolidated x86 Bar**.

## Earlier 32-bit experiment/security pause — 2026-10-07

User authorized trying a consolidated x86 DLL in Windhawk's own dedicated 32-bit
`windhawk.exe -tool-mod termielle` process. Installed i686-pc-windows-msvc Rust
standard library. First build exposed windows-rs Long/LongPtr signature differences;
added private pointer-width wrappers and an owned hidden userdata-bit regression.
The x86 runtime DLL then built successfully and all seven isolated x86 host modes
passed. **At that pause, consolidation/adapter integration was not implemented or deployed; the later deployment above supersedes this stage.**
The previously installed x64 hosted Bar remains the active edition; native executable,
installed payload/registration/profile/task were not changed by this experiment.

User reported a detected/quarantined built binary, then clarified it was **HP
security, not Windows Defender**. Initial passive Defender queries returned no
record. The particular flagged file/threat is still unknown; both tested x86 outputs
existed when inspected. Existing x64 hosted PID31492 was still running at16:10.

At the user's request, performed **one ordinary same-source/same-path rebuild** of
the x86 runtime and review executable to reproduce the alert. No renaming, detection-
avoidance changes, exclusions, quarantine restoration or security-setting changes.
Both builds succeeded; both outputs remained present after15 seconds, but **neither
rebuilt binary was executed or deployed**. Re-link hashes changed; they were logged,
not presented as proof of safety. No new HP-provider Application event was observed.
HP Sure Sense Antimalware Service (BrAmSvc) was already **Stopped** before the rebuild
and remained stopped afterward; Sure Click services were running. This is not a
clean antivirus scan or proof of false positive. Ask whether HP alerted again and
request its threat/path details before resuming execution or live port activation.
Passing earlier isolated checks does not establish malware safety.

Evidence under ignored target: `windhawk-x86-{toolchain,first-build,runtime-build,
review-build,runtime-review,hp-rebuild}.log`, `windhawk-x86-defender-observation.json`,
`windhawk-x86-hp-{before,after}.json`.
All source changes remain uncommitted. Do not replace/inject the one-shot runtime
into the Windhawk manager or Explorer; any future x86 runtime still needs its own
flag-validated disposable tool process and fresh-host lifecycle tests.

## Windhawk full Bar follow-up — 2026-10-07

User explicitly requested the bar in Windhawk after seeing the pill. Version
**0.3.0** adds ABI/mod Surface **Bar**; Follow profile now retains saved Bar,
Island and Notch (Classic still maps to Island). Live Bar honors saved reserved
space; replacement remains forced false and hosted taskbar suppression is still
blocked. Hidden reviews forcibly disable reservation, so tests cannot change the
real desktop work area. Hosted saves retain native layout/reservation/replacement,
independent edits and custom material. Layout selection stays host-controlled.

**518/0/2** workspace tests, release/fmt/diff, established-allowance Clippy,
seven isolated DLL modes (including hidden bar override geometry), both SDK
architectures and seven doctor checks passed. Policy matrix covers saved/override
layouts, both reservation/replacement flags, idempotence, saves and diagnostic
isolation. No new agent protocol/content collection or separate rendering fork.

Deployed only owned runtime/adapters/source/manifest; **native executable was not
replaced**. Termielle mod Surface is **Bar**. Profile/task XML/taskbar-styler bytes
were preserved. Backup: `%LOCALAPPDATA%\Termielle\backups\windhawk-bar-20261007-131951`.
Runtime SHA256 `7B66B8B425C567019914825B08D8EA6A1160C1273D62E5C7FDBC22CF61FDE434`;
native SHA256 stayed `97DA38D8393038092BCABBFD6176FDA4FA7CEED0659633205FD80BC39859A3EC`.
Existing bootstrap reused; adapters `termielle_0.3.0_bar_20261007-131951.dll` in
Windhawk's 32/64 mod directories. No other mod/startup/task changes.

At **2026-10-07T13:22:55.7009413+07:00**, hosted PID **31492** owned one visible
full-width top bar **(0,0)-(2560,45)**. Primary work area **(0,45)-(2560,1380)**;
both native Windows taskbars visible. Real x64 engine, matching adapter and runtime
were observed loaded; native process count zero, task naturally Ready. Normal
owner-validated WM_CLOSE stopped first hosted PID12576; reservation released
(work-area top **45→0**), then fresh tool activation restored it to45. No force
termination, foreground/input injection, topology or sleep changes. Profile hash
remained `7E859C926404B8446854AEE402E3157F6B2CE93A59D1464D074DF124483F41C9`, panic
log2202 bytes, persistent Off absent. Work remains uncommitted.

Evidence: ignored `target/windhawk-bar-{tests,clippy,build,host-review,mod-build,doctor,
deployment,restart}.log`, `windhawk-bar-{deployment,live}.json`; scripts under target
are local deployment/review, not automatic installation. Fullscreen/DPI/lid/recording,
human interaction and resource measurements remain pending; fullscreen hiding is
still Island/Notch-only. Both editions retain their Start-menu switchers.

## Shared desktop improvements and live Windhawk preview — 2026-10-07

User requested shared improvements and visible editions. Delivered display policy
(legacy Automatic, Primary, Pointer, named GDI display with primary fallback),
optional Island/Notch fullscreen suppression, animation-clock parking and custom-
preserving feel presets through native Preferences > Behavior / display. No prompts,
agent payloads, clipboard reader, guessed associations or system display changes.

**516/0/2** workspace tests; release/fmt/diff, established-allowance Clippy, all seven
doctor checks, six isolated host modes and x86/x64 SDK compilation passed. Named-
monitor native testing caught DPI-virtualized pre-window metadata; enumeration now
uses/restores physical-thread DPI. Native executable updated with profile/assets/
task definition preserved; its release hash matches installed.

Live testing caught that this portable Windhawk 1.7.3 manager is x86. The x64-only
original adapter could not start; the switcher automatically restored standalone.
Corrected to an x86 launcher plus injected x64 adapter in our **own disposable
`termielle-windhawk-host.exe`**, not Explorer. The bootstrap is now built by Cargo,
contains no renderer/event consumer, and parks after DLL activation. DLL unload
is still process-owned/one-shot, not a general unload-safe SDK.

Only Termielle mod/source/DLL/manifest and edition launchers were installed;
existing Windows 11 taskbar-styler configuration remained hash-identical. Profile
hash at preview installation `7E859C926404B8446854AEE402E3157F6B2CE93A59D1464D074DF124483F41C9`
matched during live checks. Repeated Native→Windhawk→Native→Windhawk switches
produced one visible frontend; inspected host modules included real x64 Windhawk
engine, Termielle adapter and shared runtime DLL. Panic log stayed 2202 bytes.
Task definition retained; task naturally Ready while native is stopped. Power Off
was not cleared/toggled. No Explorer restart, global-input injection, physical
sleep/wake, recording or display-topology test was performed. No memory claim.

Owned Start-menu shortcuts: **Termielle - Native**, **Termielle - Windhawk**.
Switcher/manifest under `%LOCALAPPDATA%\Termielle\windhawk`; the switcher only
changes Termielle's enable bit and preserves profile/task/other mods. Windhawk
preview initially left active as a floating pill; the later 0.3.0 follow-up above
left the full bar active instead. Native remains the fallback.
Autostart preference was not changed: whichever frontend starts first at login
claims ownership. Surface overrides are host-only; shared saves preserve native
layout/reservation/replacement. User-assisted interactions/fullscreen/DPI/lid and
full unload/resource certification remain pending. Work remains uncommitted.

Evidence: ignored `target/shared-improvements-*` logs; see
[edition guide](../windhawk/README.md) and [improvements](desktop-improvements.md).

## Standalone deployment — 2026-10-06

Authorized executable-only deployment at **2026-10-06T15:05:06.6813021+07:00**.
The installed `termielle-app.exe` now includes preferences/navigation polish,
persistent power controls, name visibility and shared-runtime startup. Windhawk
mod/DLL activation and new Start-menu shortcut installation were not performed.

- Installed/release SHA-256: `DDD3379C386A5FC90786D955961977F5F0B993072573E629C40F49DEF67EC5AB`.
- Previous executable SHA-256: `0EA521210C77FF33099A4BBAD8C28395EC4A7B95B7083147384BAF49A6F101E3`.
- Preflight/current profile SHA-256: `D0EAF6084DA079EA6847CB0452E413D6639708B74C1C70356CBE88825DAEAF24`;
  deployment preserved its bytes, including custom glass and native taskbar policy.
  This is the current user profile, not the older October 4 profile snapshot.
- Backup executable/profile/task XML under
  `%LOCALAPPDATA%\Termielle\backups\standalone-20261006-150450`.
- One owner-validated WM_CLOSE gracefully stopped the previous installed process.
  No force termination, task definition edit or power-marker change.
- Existing task restarted **Running**, one installed process observed **PID 62372**,
  visible **(0,0) 2560×45** top bar. Primary and secondary native taskbars visible.
  No display-topology change or synthetic recovery messages were sent.
- Installed hash matched release; 14 preserved asset/theme/integration/emitter
  files matched preflight hashes; panic log stayed **2202 bytes**. A subsequent
  passive check observed the same PID/hash/profile/task state.
- Fresh release build and all seven doctor checks passed before deployment.
  Workspace validation remains **508/0/2**. Physical lid/recording and interactive
  preference, tooltip and drag acceptance remain outstanding.

Ignored evidence: `target/standalone-deployment.{json,log}`, deployment script,
release/doctor logs. Source changes remain uncommitted.

## Experimental shared-runtime/Windhawk increment — 2026-10-06

Originally user-approved source implementation; standalone later deployed above,
but no Windhawk mod was installed or activated. The executable is now a
thin wrapper around `termielle-app/src/runtime.rs`; `termielle-runtime` packages
that same implementation as a versioned C-ABI DLL. The Windhawk C++ adapter is a
dedicated x64 tool, not Explorer/taskbar UI injection. Both frontends share the
single-instance/event-pipe claim, profile, branding setting and persistent Off.
Hosted Bar/Classic become ephemeral Island views; no native taskbar hiding or
AppBar reservation is allowed, and standalone surface choices survive saved
hosted settings. Assets/themes resolve from the DLL payload; explicit profiles
now participate in file reload. `--enable-only` clears Off without choosing or
launching a frontend.

Lifetime is deliberately **one-shot/dedicated-process**. Graceful GUI stop and
cancellable/joined synchronous pipe I/O are implemented, but remaining process-
owned decoder/OS workers make arbitrary `FreeLibrary`/in-process restart unsafe.
The mod holds the DLL until its own host exits. No foreign process/thread is
force-terminated. Runtime Restart returns 11 for fresh-host handling.

Validation: **508 passed, 0 failed, 2 existing manual tests ignored**; release
build, fmt/diff and Clippy with established unrelated-lint allowances passed.
The adapter compiled/linked using local Windhawk 1.7.3 x64 headers/import library.
Six private DLL-host modes passed: hidden rendering/event delivery, GUI stop,
persistent Off, invalid ABI, existing frontend claim and pre-start cancellation;
profile bytes were preserved and the review processes exited. All seven doctor
checks passed. Compiler/isolated-host success is not live Windhawk integration,
full unload-safety, lower-memory evidence or physical lid/display acceptance.

Release packaging now includes the DLL, mod source, guide and upstream MIT
notice. CI has a separate isolated-DLL review step (existing strict-Clippy
warnings are not silently waived in CI). No mod was installed/enabled, no
scheduled task/native-taskbar preference was changed, and work is uncommitted.
Fullscreen-aware visibility, monitor-picker improvements and measurements remain
subsequent work; see the [edition guide](../windhawk/README.md).

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

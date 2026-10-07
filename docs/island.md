# Dynamic Island / Notch — Windows

Fork of Termielle's overlay into a top-center notch or floating island with **custom layered glass** (per-pixel alpha, no DWM acrylic dependency).

For completed desktop refinements, agreed decisions, validation history, and
local redeployment, see the [development handoff](development-handoff.md).
Root-cause investigations live in [reported behaviours](reported-behaviours.md).

## Features (current)

- **macOS- and iOS-inspired presentation lifecycle**:
  - **Idle stays quiet**: Without agent activity, media, or a notification, hovering leaves the pill collapsed. Clicking the bar pill opens a compact Today card with the date, latest notification, Search, and Notifications; standalone Island keeps its system dashboard. With `auto_hide`, the standalone island can rest as a top-edge sensor strip.
  - **Live Activities Ride in Compact** — an active agent session (`Thinking`, `Working`, `Ready`, ...) keeps the **Compact pill** up, hugging its live content, until the turn ends. Needs-input and failure get concise **notification alert banners** (96px) that spring back after 3.5s. An alert grows from the resting pill rather than leaving a second pill behind.
  - **Hover for live content**: Hover reveals available agent, media, or notification content with organic spring physics; an empty pill does not pop out or show an Idle card.
  - **Click to Extend**: Clicking the pill extends it vertically and horizontally into the **Expanded tall card**; session lists size to at most four rows per page. Clicking it again collapses it back to the pill or hidden. Tapping the media blob toggles playback.
  - **Press Swell**: Holding the pointer down swells the pill ~3% and it settles back on release — the Dynamic Island under the fingertip.
- **Liquid Blob Split & Merge** — the signature Dynamic Island morphology. When an agent session and media playback are live at once, the compact pill **splits in two**: the agent blob narrows while the media blob pulls out, connected by a liquid bridge (a smooth-min fillet over both signed-distance fields) that thins and **snaps** as the separation spring extends. When one activity ends, the blobs **flow back and merge** into one pill.
- **3-Axis Spring Morphing** — Apple's WWDC23 spring physics (`stiffness = (2π/duration)²`, `damping = (1-bounce)·4π/duration`) drives width, height, **and corner radius** through one integrator (semi-implicit Euler, sub-stepped at ~0.35/ω), so the silhouette never snaps: pill ends stay near-semicircular deep into an expansion, and **interrupted morphs inherit their velocity** and settle naturally.
- **Content rides the morph** — expanded content fades in as the container reaches 80–98% of its target dimensions. Same-size state, media, and alert swaps use a 140 ms source/destination crossfade; the previous content is shared rather than copied. Content rises slightly into place and never restarts opacity during an interrupted geometry spring.
- **Custom layered glass** — premultiplied PBGRA is composited through `UpdateLayeredWindow`; a bounded worker captures and blurs the desktop behind the surface. Optional DWM backdrop APIs are not required.
  - Top parabolic specular sheen across the upper surface.
  - Optical refraction groove and chamfered outer specular rim.
  - Subtle dither texture that avoids visible gradient banding.
  - A color-key renderer remains available for display drivers that cannot present layered DIB redirection.
- **Dynamic Content-Based Sizing** — no rigid fixed widths; compact mode automatically scales to hug active content (scaling smoothly as multiple agent sessions or live media sessions activate).
- **Media Player Card** — expanded mode features 64×64 rounded album art, track/artist typography, a bottom-aligned transport row with evenly spaced previous/play-pause/next controls, and a centered 4-bar equalizer. No timeline is shown because playback-position data is not available.
- **Transient alert ownership** — fresh `needs_input`/`turn_failed` events and Windows toasts use a bounded queue. A user-action agent alert can temporarily preempt a passive toast; the toast resumes with a fresh visible lifetime. Repeated stable event IDs update rather than multiply banners. Each banner owns 3.5 s (agent) or 6 s (toast) only while it is actually in front. Clicking anywhere dismisses it immediately.
- **Live timeout countdown** — a visible banner's subtle one-pixel lifetime line drains on its own repaint tick (16 ms) instead of riding the bar's two-second metrics refresh. The tick stops when the banner leaves, and the countdown repaints only the card.
- **Windows toast forwarding** (`forward_toasts`, default on) — app notifications from Action Center ride the island as transient alert banners (app name plus the first text lines, 6 s). Needs notification-listener access (Settings → Privacy → Notifications); without it the watcher exits silently. Local-only: nothing leaves the machine. Pre-existing toasts never flood on startup — only new arrivals fire.
- **Motion signatures** — motion stays purposeful: thinking dots bounce, Ready gets a brief 600 ms sparkle burst, failure shakes damped over 300 ms, and needs-input breathes. The animated face carries the working state without an additional orbit competing around it. Procedural motion follows the overlay monitor’s vertical blank through a dedicated DXGI clock. Idle frames do not continuously redraw, and reduced motion disables procedural effects.
- **Triple-spring morphs** — expands bounce (`animation_ms`), collapses settle critically damped and quicker (`collapse_ms`), alert banners drop in fast (`alert_ms`). Three unseen alerts may wait behind the visible alert without consuming their timeout early.
- **Attached notch** (`layout: notch`) flush with the top edge — flat top, rounded bottom; the pill is drawn *from* `y=0` **in every presentation** (the anchor never moves mid-morph, so expansions grow downward from the bezel like the real thing).
- **True-black notch material** — in notch layout the body renders as opaque `#000` (`glass.notch_black`, default on), fusing with the display bezel like real hardware instead of reading as a floating glass widget. Floating islands keep the translucent glass.
- **Floating island** (`layout: island`) with `y_offset` 0-500.
- **Active display** — the island anchors to the monitor under the cursor, picked on startup and re-picked on display/DPI changes (`WM_DISPLAYCHANGE`/`WM_DPICHANGED`); morphs never move it between screens mid-animation.
- **Auto theme** (`theme: "auto"`) — follows the Windows light/dark setting and updates on `WM_SETTINGCHANGE("ImmersiveColorSet")`. Dark mode uses cool-neutral glass at roughly 75% opacity, or opaque glass when transparency is disabled. System-accent tinting is applied only when "show accent color on Start and taskbar" is enabled (40% blend). Explicit `glass` overrides still win.
- **Animated Termielle face** — the active state's first frame is decoded synchronously, then the remaining GIF frames stream into a bounded 96 px cache on face deadlines. Standalone idle faces run at 10 fps; the full-width native bar streams one idle loop, settles on visible artwork, and releases the other frames. It resumes animation for active agent/media states. `face_animated: false` or reduced motion freezes the first frame.
- **Agent states** — lifecycle events morph the pill; a short 1px marker along the top edge uses a restrained violet/green/amber/teal/red state palette.
- **Click to expand/collapse** — `WM_LBUTTONUP` toggles manual expansion, including when idle; hovering expands only when `expand_on_hover` is set and content is available. Clicking the media element toggles play/pause.

## Multi-session activity

The `agents` widget now shows a paginated session list in the expanded card,
with per-session status, elapsed time, and explicit local window links. Clicking
**Link** selects a target; clicking the linked row returns to its window.
Completed sessions remain accessible after the Ready hold. The list takes
priority over the media body while sessions are tracked, with Play/Pause in its
header. Right-side popovers remain independent.

See [the activity guide](session-activity.md) for lifetime rules, privacy,
window-versus-tab limits, and synthetic visual-review commands.

## Modes

### Isolated visual review

Render the real controller without starting the desktop overlay or modifying
your installed configuration:

```powershell
cargo run -p termielle-app --example render_review -- review.bmp media
cargo run -p termielle-app --example render_review -- review-light.bmp media --light
cargo run -p termielle-app --example render_review -- review-motion.bmp thinking --elapsed=160
```

States: `idle`, `thinking`, `working`, `ready`, `waiting`, `failed`, `media`,
`notification`, and `tasks`. Add `--compact` for the compact presentation. The default freezes
motion; `--elapsed=N` samples the real spring/controller in 16 ms steps.
Media and window data are synthetic. BMP output is composited over neutral gray;
it does not simulate live wallpaper blur or prove real-time presentation cadence.

### Waybar-style system bar

Bar mode is the primary desktop layout: a persistent top or bottom strip with
left, center, and right module zones. Clicking the center `termielle` module
opens the focused Termielle popup, and clicking it again closes the popup.
Escape or an outside click also closes it.

With `expand_on_hover` on and live content available, dwelling over the pill
opens that same popup: the bar's pill *is* the island, so hover reaches the
state a click reaches. Empty-state hover leaves it collapsed. Both
edges are deferred - 300 ms to open, so a cursor crossing the strip does not
throw the card open, and 500 ms to close, so the pointer can travel down into
the card it just opened. A click that dismissed the card is not undone by the
pointer still resting on the pill; leaving and returning re-arms it. Standalone
Island keeps its own smaller hover step, so the two layouts differ here.

The module shows the face when enabled, a consistent agent state label, and
media metadata when a playing or paused media session is available. Media
artwork and transport controls remain available after Pause so the user can
resume playback. The default left rail provides pinned/running app navigation,
a multi-window chooser, overflow and preferences access;
`workspaces` can be configured instead for virtual desktops. The speaker
toggles mute and the wheel adjusts volume. Network, battery, and the current
window title remain passive readouts. CPU and memory are shown in Control
Center by default and can be pinned to the strip from the tray menu.

Changing the system output volume or mute state briefly replaces the resting
center pill's label with a speaker, level track, and percentage (or “Muted”).
Core Audio change notifications wake the metrics worker immediately, including
changes made with media keys or Windows sound controls. The feedback clears
1.5 seconds after the last change; startup does not show it. Alerts and expanded
island cards take priority, and Control Center keeps its independent open state.

Right-zone modules are a glyph plus its number and no label word: the icon
names the metric, the number is the thing being read. The glyphs are
[Tabler Icons](https://github.com/tabler/tabler-icons) (MIT, © 2020-2024
Pawel Kuna), embedded as their upstream SVG path data and stroked at draw time
from a distance field, so they stay sharp at any scale and a new icon is one
line of path data. `draw_icon` lives in `animation/icons.rs`.

The bar keeps a fixed strip height while the focused Termielle popup opens
below or above it. This is a macOS-inspired status strip on Windows rather than
a replacement for Windows' native taskbar. The popup is a focused detail view.

### Control panel

Control Center is a menu-bar item in the right zone, between the status icons
and the clock, as macOS has it. Its panel drops from that icon independently
of the centered notch/pill card. Either popover can be open while the other
stays open, and each icon toggles only its own popover. An incoming island
alert can expand beside an open Control Center without hiding its controls.

It used to be a glyph inside the pill, ahead of the pill's own open/close hit.
That made it unreachable the moment hovering the pill opened the card, and it
could only ever mean "open", never close.

- **Quick-control tiles** — Network and Bluetooth show live Windows status
  when available; Focus and Display remain clearly labeled Settings links.
  Clicking any tile opens its Windows Settings page. This unpackaged app does
  not pretend to control radio state without permission.
- **Sound card** — the speaker mutes, clicking the slider sets an absolute
  level, and the wheel over it adjusts volume.
- **Now Playing card** — shows the current media session with play/pause when
  one exists; otherwise it says `Nothing playing` without an inactive button.
- **System readouts** — CPU and memory remain available in the panel while
  staying off the default strip.
- **Windows Settings** — footer opens the Settings home page. The native
  Windows taskbar and notification-area icons remain visible; this panel does
  not replace the system tray. Termielle's own preferences are available in
  the tray and the rail's Bar settings menu.

The tray menu's **Menu Bar Items** submenu pins or hides Network, Volume,
Battery, CPU, and Memory. The clock and Control Center stay on the strip so
their popovers remain reachable.

### App launcher

**Alt+Space** opens Termielle's Spotlight-style app launcher. The **Search**
shortcut in Today and the rail's Apps button open the same window;
neither invokes Windows Search. Type an app name, use **Up/Down** to select,
then **Enter** to launch. Clicking a result also launches it. **Esc**, clicking
outside, or pressing Alt+Space again dismisses it. Ctrl+A selects the query.

Desktop and Store apps come from Windows' registered AppsFolder catalog,
indexed on a background worker at startup and refreshed on opening after a
minute. Filtering is entirely in-memory, with exact/prefix matches before fuzzy
abbreviations; no file indexing, web search, query logging, or arbitrary command
execution. Native text input supports normal editing and IME composition.
Only visible results request icons, asynchronously; icon extraction never
holds up app discovery, typing, or launching. A 32-entry worker cache shares
bitmaps with the visible results instead of copying them; hidden results release
their artwork, and catalog refresh clears the cache.

The launcher is a separate focusable window, not another notch card. It follows
the bar's colours and current monitor DPI without changing the island or Control
Center's open/hover state. If another program owns Alt+Space, Termielle doesn't
take it over: open the launcher from Today instead, or disable the conflicting
program's shortcut. Windows' normal Win+S shortcut remains unchanged.

### Recent notifications

Clicking the clock opens a separate recent-notifications popover anchored to
the clock. It keeps the latest four alerts observed by Termielle in memory,
shows their source and local arrival time, and marks new arrivals with a small
dot on the clock. A close button remains visible, and a full-width Clear All
action appears when the list has items. Clear All affects only Termielle's
recent list. It is
not a mirror of the entire Windows Notification Center, and its history does
not persist across restarts. The notification popover and Control Center
replace each other; neither opens or closes the centered notch card.

Outside dismissal checks the popup's own visible rectangle and its entry
controls, not the full transparent overlay window. When a right popover and a
manually opened notch are both open, shared Escape/outside dismissal closes the
right popover first; a later dismissal can close the notch.

The panel is a deliberate body, never a takeover: media, agent activity, and
the task switcher keep their own card, while the Control Center keeps its own
panel and hit targets.


### Taskbar replacement mode

`bar.replace_taskbar` turns the bar into a taskbar replacement: the native
taskbar is hidden, the desktop work area is reserved through the AppBar API,
and the bar gains the affordances the shell used to own.

**Provided.** The left rail uses the same [app navigation](app-navigation.md)
in ordinary and replacement modes: launcher, explicit pinned/running groups,
active/count indicators, multi-window chooser, overflow, app actions and Bar
settings. This replaces the earlier five-button shell shortcut row. Native
Start, Task View, Quick Settings and Show Desktop remain available through
`Win`, `Win+Tab`, `Win+A` and `Win+D`; the clock opens Termielle's recent
notifications. Keep replacement off while validating this milestone.

**Not replaced — Windows shell surfaces Termielle does not reimplement.** The
Start menu's own pin list, jump lists, and per-app "recent" entries; taskbar
thumbnails and window previews on hover; full native task-button menus
(basic Pin/Unpin, New window and Close now exist, but not jump lists,
workspace rename or cascade windows);
dragging windows onto or between desktops; the notification-area icons
themselves — `Win+A` opens Quick Settings, not the icon overflow; per-monitor secondary taskbars beyond the one bar this
process draws; taskbar auto-hide and peek behavior; and live badge counts on
taskbar buttons. Virtual desktops are addressed by number, because the shell
does not expose desktop names to a bar process.

**Layout rules.** The left zone is anchored to the same place in both modes,
so nothing existing moves when replacement is turned on. Content is laid out
from that anchor. The window title yields space before app navigation; app
entries that do not fit move into More rather than creating hidden controls.
Optional workspace entries may still be dropped under space pressure. Anything dropped is not painted and gets
no hit target — a narrow bar never leaves an invisible live control under the
pill, and never silently shifts the pill or the right zone.

**Lifecycle.** Turning the flag on hides the taskbar, turns it off hands it
back, and a one-shot watchdog process restores it if the overlay dies without
running its own teardown. Explorer re-shows the taskbar on its own after a
shell restart, and offers no notification that can be registered for on every
Windows build, so a 2 s tick re-asserts the hidden state.

Use the tray or the rail's Bar settings menu to switch layouts, themes,
widget visibility, and hover behavior.
The menu marks the active layout, theme, position, and widget state. Classic,
Notch, and Island remain available as explicit alternate layouts.

Termielle does not invoke or configure Waybar. “Waybar-style” describes the
left/center/right visual language of the native Windows bar only.

To inspect isolated bar frames, add `--bar` to `render_review`; use `--compact`
for the resting state. To measure actual render/present costs and display-clock
intervals without saving configuration or reserving desktop space:

```powershell
cargo run --release -p termielle-app --example bar_motion_review -- --primary
```

The probe prints the active display mode and median/p95 timings. These measure
application submissions, not guaranteed physical scanout; load can still cause
missed refreshes. The clock follows the display's configured refresh rate and
does not change Windows display settings.

### Layout selection


`~/.termielle/config.json` → `island.layout`:

| `layout` | Shape | Position | Use |
|----------|-------|----------|-----|
| `bar` | full-width modular strip | top or bottom | default Waybar-style system bar |
| `classic` | 360×360 square, free-drag, clamped to work area | anywhere | legacy desktop pet |
| `notch` | flat top, rounded bottom (18px), `width × 36` | top-center, `y=0` | macOS-like notch |
| `island` | pill, all corners rounded | top-center, `y = y_offset` | floating Dynamic Island |

The tray switches layouts live. Classic, Notch, and Island are explicit
alternate surfaces; live-content hover may use a smaller compact step than Bar.

## Customization

### Geometry — `island.*` in `config.json`

```json
"island": {
  "layout": "bar",
  "minimal_width": 72,
  "collapsed_width": 180,
  "expanded_width": 320,
  "height": 36,
  "corner_radius": 18,
  "y_offset": 0,
  "animation_ms": 350,
  "spring_bounce": 0.18,
  "theme": "auto",
  "glass": { "tint": [30,22,18,190], "blur_radius": 12, "border_alpha": 24, "highlight_alpha": 42, "shadow_alpha": 48 },
  "face_animated": true,
  "show_name": true,
  "widgets": ["face","agents","music"],
  "expand_on_hover": true
}
```

### Widgets — configurable Termielle content


`widgets` selects the content inside the Termielle module and its focused
popup. Unknown names are dropped on load.

| name | shows |
|------|-------|
| `face` | optional animated Termielle face |
| `agents` | live session dots, expanded session list, and explicit window links |
| `music` | media artwork, metadata, and transport controls; paused sessions remain available |

The `ring` and `ring_metric` fields are compatibility-only and are no longer
part of the default Waybar module. `tasks` remains available in the focused
popup when explicitly enabled by configuration.

Remove a name to hide it, e.g. `"widgets": ["face","music"]` for a minimal
module. The tray toggles media, hover expansion, the face, and **Show Termielle
name**; layout changes remain explicit choices.

`show_name` (default `true` for existing profiles) controls decorative branding
in the idle pill, the bar's center pill and the standalone dashboard header.
Set it to `false`, uncheck **Show Termielle name** under tray → Widgets, or use
Preferences → Appearance → **Show Termielle name** with Preview/Apply/Revert.
This does not hide agent status, media content, the face or real source names;
an unnamed agent alert uses "Agent" instead of the branding fallback. Geometry
and click targets remain unchanged. Tray tooltips/window accessibility identities
remain named. This setting is in the new source/build, not the installed October
4 binary; do not add the field to an older binary's profile (older versions reject
unknown configuration fields).

### Interactivity

- **Bar module**: the persistent bar is always visible; the Termielle module
  opens its focused popup on click. Empty-state hover leaves the pill collapsed.
- **Left apps rail**: stable pinned/running groups; click to launch, focus or
  choose a window. More reaches hidden groups; right-click opens app actions.
  See [app navigation](app-navigation.md).
  Replace `"apps"` with `"workspaces"` in `bar.modules_left` to use virtual desktops instead.
- **Standalone Island/Notch**: hover reveals live content when enabled; empty
  hover stays collapsed, and clicking opens the dashboard.
- **Alerts**: fresh agent input/failure events and Windows toasts appear as
  dismissible transient banners. Clicking anywhere on the banner dismisses it
  immediately. The banner shows the source, a clear headline, and a subtle lifetime indicator.
- **Controls**: transparent pixels pass through, while visible module controls
  use explicit hit targets. Media transport hit targets are inset from their
  painted circles and share the same DPI-scaled geometry as the glyphs.

The default visual language is intentionally sparse: the bar shows the face,
one state label, and optional media title. Focused cards add only the context
needed to act on the current activity. Session IDs, generic explanations, and
duplicate status prose are intentionally omitted.

### Rendering pipeline (no-freeze design)

Media queries (1 s), optional task enumeration (1.5 s), artwork decoding, and
backdrop captures run on a background worker. Disabled widgets are not polled,
unchanged snapshots are not republished, and CPU sampling is capped at 2.5 Hz.
The GUI thread only composites cached frames.

Presentations size to their content. Standalone cards use 154/175/180/210 px
according to media/task presence; the agent-only native bar popup is 112 px
including its 36 px strip instead of reserving empty media space. Frames are
authored at device pixels and presented 1:1 (`render_scale` = monitor DPI × user
zoom, unless `scale_with_dpi: false`). `AnimationClock` waits for the target
monitor's vertical blank and falls back to 16 ms when DXGI is unavailable.
Remote/secure desktops may lose live blur and present the flat material.

All values clamp like user `scale: 0.5..2.0`; a bad value never discards the file.

### Glass — live frosted glass

The pill composites over a **live capture of the desktop behind it**: a
desktop copy fills the pill rect, a three-pass box blur at `glass.blur_radius`
frosts it, and the theme tint goes over the top. Plain BitBlt does **not** reliably
exclude layered windows on modern DWM: the worker now scopes capture exclusion
to its own HWND for the copy and restores the original policy before blur.
See [notch background repair](notch-background.md), including the short
concurrent-recording limitation and unsupported-platform fallback.
Capture bursts stop after five quiet seconds; geometry, foreground-window
switches/moves, and display/material invalidation re-arm them. A 250 ms minimum
interval bounds changing-scene copies. Failed capture retains flat material;
missing samples preserve translucency, and old-generation captures are rejected.

On top of the blurred backdrop, baked into the `FrameBuffer` PBGRA before
`UpdateLayeredWindow(ULW_ALPHA)`:

1. shadow — black, `shadow_alpha` (60), 2px offset
2. body — `tint` BGRA at `tint[3]` (180 ~ 0.70) with AA rounded-rect coverage (2×2 supersample)
3. border — 1px inner stroke `border_alpha` (38 ~ 0.15), tint lightened toward white
4. highlight — top 1px `highlight_alpha` (70 ~ 0.28)
5. content — animated face, live agent activity, media artwork, transport controls, and theme-aware text

`RenderMode::ColorKey` is the compatibility path for drivers whose layered DIB
redirection is broken; it intentionally omits the live blurred backdrop.

### Themes — `themes/*.json` and `auto`

Search order: `~/.island/themes/<name>.json` (user shadows) → `~/.termielle/themes/` → `exe_dir/themes/` → `themes/` (repo). Missing file falls back to builtin. `theme: "auto"` maps to `light` or `liquid-dark` from the Windows personalization setting and re-resolves live when it changes.

Preset files ship in `themes/`:

- `liquid-dark` — `[30,22,18,190]` (BGRA), border 24, highlight 42, shadow 48 (cool neutral dark default)
- `midnight` — navy `[12,18,32,200]`, stronger shadow
- `catppuccin-macchiato` — dark plum glass with a grouped apps rail, focused-window context, and padded mauve status capsules with 18px vector glyphs in Bar layout; inspired by SketchyBar showcase layouts. A workspace rail remains optional.
- `light` — `[245,245,245,160]` for dark wallpapers
- `transparent` — `[30,30,30,70]`, blur 0
- `auto` — follows Windows light/dark

**Explicit `glass` fields in `config.json` win over any theme preset** — a user-set `tint` survives a theme switch (fields differing from the glass default are treated as overrides).

```json
{ "name":"liquid-dark", "tint":[30,22,18,190], "blur_radius":12, "border_alpha":24, "highlight_alpha":42, "shadow_alpha":48, "corner_radius":18 }
```

At startup `theme::resolve_theme(&mut config.island)` applies the built-in, then
the file overlay, then clamps the result. The tray theme picker updates the
material and persisted config live.

Add your own: copy `themes/liquid-dark.json` to `~/.island/themes/my.json`, tweak `tint`/`border_alpha`/etc, set `"theme":"my"` in `config.json`.

## Current architecture

`agent hook/plugin → termielle-emit → owner-only named pipe → protocol decode
→ SessionReducer → Controller → presentation/layout → spring/procedural
motion → layered Win32 present`.

- `termielle-core` owns the versioned content-free protocol, per-session
  priority/deadlines, validated/clamped config, and the bounded event journal.
- `termielle-app::Controller` is the single GUI-thread state owner. It combines
  semantic state, transient alert ownership, media/task snapshots, pointer
  state, target geometry, spring velocity, cached face layers, and hit targets.
- `tasks.rs` owns SMTC/task polling and screen capture. `toast.rs` owns a
  separate STA notification listener. Neither performs layout or Win32 present.
- `window.rs` owns the no-activate layered HWND, physical alpha hit map,
  monitor/DPI selection, and `UpdateLayeredWindow`/color-key presentation.
- `bar/appbar.rs` reserves work area through the Windows AppBar API and restores
  the native taskbar on graceful, restart, panic, and early-failure paths.
- Taskbar replacement (`bar.replace_taskbar`) has a single owner:
  `sync_taskbar_mode` hides the taskbar while the bar owns it, hands it back
  the moment the mode ends, and starts a one-shot watchdog process. The
  watchdog restores the taskbar if the overlay dies without running its own
  teardown; every teardown path (`leave_bar_shell`, the panic hook, `Drop`)
  restores it in-process. Explorer re-shows the taskbar on its own after a
  shell restart or taskbar re-creation and offers no notification this app can
  rely on, so a 2 s tick re-asserts the hidden state. The in-process restore is
  gated on "we hid it", so an auto-hide taskbar is never forced open by a clean
  exit.
- Bar modules are declared in one typed registry with explicit zone ownership. Left/right strip content is cached as transparent device-resolution layers, and metric damage repaints only the invalidated side while preserving the other side, center content, and hit regions. Full damage still rebuilds the bar for animation, popup, DPI, and layout changes; no user scripts are executed.
- Hidden metrics remain cached without repainting the strip. Device-size vector coverage masks use a bounded 512 KiB cache. See the [performance follow-up](performance.md) for measurements and repeatable probes.
- Bar popup geometry is a separate island surface: a six logical pixel transparent gap and independently rounded card corners keep it visually detached from the persistent strip. The gap is click-through and preserved for both top and bottom bars.
- The default config path is watched with a debounced, validated reload. Same-layout reloads retarget live springs; a layout cutover resets only surface-specific state.

## Try it

```powershell
cargo run -p termielle-app
'{"session_id":"demo"}' |
  cargo run -p termielle-emit -- --source codex --event prompt_submitted --input stdin
cargo test -p termielle-app --test island_controller -- --nocapture
pwsh -NoProfile -File scripts/doctor.ps1 -BinDir target/debug
```

The default layout is the native Bar. Use the tray to switch to Notch, Island,
or Classic; tray theme and layout changes persist immediately.

## Platform limits

- This repository has no Waybar, GTK, Wayland layer-shell, DBus, or MPRIS UI
  backend. Linux CI covers the portable core, IPC, and emitter crates only.
- Per-pixel alpha, live desktop capture, DXGI vertical blank, AppBar reservation,
  and color-key fallback are Windows/Desktop Window Manager behaviors.
- Color-key mode cannot provide the invisible auto-hide sensor used by the
  per-pixel path; use per-pixel rendering for hover-to-reveal floating islands.
- Secure/remote desktops may block capture or DXGI output lookup. The island
  remains functional with flat material and 16 ms pacing.

## Further enhancements

- Replace SMTC and notification 1–3 s snapshots with event-driven WinRT
  callbacks, retaining the existing poll as a reconnect fallback.
- Add a cancellable server receive operation so Restart can join the IPC thread
  rather than relying on the bounded endpoint-bind retry.
- Add deterministic frame-sequence capture for display-change and mixed-DPI
  regression review across physical monitors.

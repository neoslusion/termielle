# Dynamic Island / Notch — Windows

Fork of Termielle's overlay into a top-center notch or floating island with **custom layered glass** (per-pixel alpha, no DWM acrylic dependency).

## Features (current)

- **Authentic macOS & iOS Presentation Lifecycle**:
  - **Idle is Hidden**: When idle and unhovered, the island completely hides from view, leaving your workspace unobstructed. An invisible 2px sensor strip along the top screen bezel detects incoming hover gestures.
  - **Live Activities Ride in Compact** — the real island never auto-expands: an active agent session (`Thinking`, `Working`, `Ready`, ...) keeps the **Compact pill** up, hugging its live content, until the turn ends. Needs-input and failure still get prominent treatment as auto-expanding **notification alert banners** (124px) that spring back after 3.5s.
  - **Hover to Pop Out**: Bumping or hovering the top bezel pops the island down into the **Compact pill format** with organic Apple spring physics, showing the character face, liquid glass, and status. Leaving smoothly retracts it back into hidden.
  - **Click to Extend**: Clicking the pill extends it vertically and horizontally into the full **Expanded tall card** (320×154px). Clicking it again collapses it back to the pill or hidden. Tapping the media blob toggles playback.
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
- **Windows toast forwarding** (`forward_toasts`, default on) — app notifications from Action Center ride the island as transient alert banners (app name plus the first text lines, 6 s). Needs notification-listener access (Settings → Privacy → Notifications); without it the watcher exits silently. Local-only: nothing leaves the machine. Pre-existing toasts never flood on startup — only new arrivals fire.
- **Motion signatures** — motion stays purposeful: thinking dots bounce, Ready gets a brief 600 ms sparkle burst, failure shakes damped over 300 ms, and needs-input breathes. The animated face carries the working state without an additional orbit competing around it. Procedural motion follows the overlay monitor’s vertical blank through a dedicated DXGI clock. Idle frames do not continuously redraw, and reduced motion disables procedural effects.
- **Triple-spring morphs** — expands bounce (`animation_ms`), collapses settle critically damped and quicker (`collapse_ms`), alert banners drop in fast (`alert_ms`). Three unseen alerts may wait behind the visible alert without consuming their timeout early.
- **Attached notch** (`layout: notch`) flush with the top edge — flat top, rounded bottom; the pill is drawn *from* `y=0` **in every presentation** (the anchor never moves mid-morph, so expansions grow downward from the bezel like the real thing).
- **True-black notch material** — in notch layout the body renders as opaque `#000` (`glass.notch_black`, default on), fusing with the display bezel like real hardware instead of reading as a floating glass widget. Floating islands keep the translucent glass.
- **Floating island** (`layout: island`) with `y_offset` 0-500.
- **Active display** — the island anchors to the monitor under the cursor, picked on startup and re-picked on display/DPI changes (`WM_DISPLAYCHANGE`/`WM_DPICHANGED`); morphs never move it between screens mid-animation.
- **Auto theme** (`theme: "auto"`) — follows the Windows light/dark setting and updates on `WM_SETTINGCHANGE("ImmersiveColorSet")`. Dark mode uses cool-neutral glass at roughly 75% opacity, or opaque glass when transparency is disabled. System-accent tinting is applied only when "show accent color on Start and taskbar" is enabled (40% blend). Explicit `glass` overrides still win.
- **Animated Termielle face** — the active state's first frame is decoded synchronously, then the remaining GIF frames stream into a bounded 96 px cache on face deadlines. Standalone idle faces run at 10 fps; the full-width native bar keeps its resting face still and animates only during active agent/media states. `face_animated: false` or reduced motion freezes the first frame.
- **Agent states** — lifecycle events morph the pill; a short 1px marker along the top edge uses a restrained violet/green/amber/teal/red state palette.
- **Click to expand/collapse** — `WM_LBUTTONUP` toggles manual expansion (idle only); hovering expands when `expand_on_hover` is set. Clicking the media element toggles play/pause.

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
and `tasks`. Add `--compact` for the compact presentation. The default freezes
motion; `--elapsed=N` samples the real spring/controller in 16 ms steps.
Media and window data are synthetic. BMP output is composited over neutral gray;
it does not simulate live wallpaper blur or prove real-time presentation cadence.

### Waybar-style system bar

Bar mode is the primary desktop layout: a persistent top or bottom strip with
left, center, and right module zones. The center `termielle` module is compact
and click-only; hovering it does not expand the bar. Clicking the module opens
the focused Termielle popup, and clicking it again closes the popup. Escape or
an outside click also closes it.

The module shows the face when enabled, a consistent agent state label, and
media metadata when a playing or paused media session is available. Media
artwork and transport controls remain available after Pause so the user can
resume playback. Workspace buttons select virtual desktops; the speaker
toggles mute and the wheel adjusts volume. CPU, memory, battery, clock, and
window title remain passive status modules.

The bar keeps a fixed strip height while the focused Termielle popup opens
below or above it. This is a persistent Waybar-style surface, not a macOS menu
bar clone. The popup is a focused detail view; it does not duplicate the
system telemetry already present in the bar.

Use the tray to switch layouts, themes, widget visibility, and hover behavior.
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
alternate surfaces; they do not inherit the Bar's click-only popup behavior.

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
| `agents` | live agent session dots and state beacon |
| `music` | media artwork, metadata, and transport controls; paused sessions remain available |

The `ring` and `ring_metric` fields are compatibility-only and are no longer
part of the default Waybar module. `tasks` remains available in the focused
popup when explicitly enabled by configuration.

Remove a name to hide it, e.g. `"widgets": ["face","music"]` for a minimal
module. The tray toggles media, hover expansion, and the face; layout changes
remain explicit choices.

### Interactivity

- **Bar module**: the persistent bar is always visible; the Termielle module
  opens its focused popup only on click.
- **Standalone Island/Notch**: hover and click retain their compact-to-expanded
  behavior when enabled.
- **Alerts**: fresh agent input/failure events and Windows toasts appear as
  dismissible transient banners. Clicking anywhere on the banner dismisses it
  immediately; the `×` is a visual affordance, not a separate-only action.
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
plain `BitBlt` (which excludes layered windows, so the glass never feeds
back into itself) fills the pill rect, a three-pass box blur at
`glass.blur_radius` (the standard gaussian approximation — no banding or
ringing) frosts it, and the theme tint goes over the top. The worker thread
recaptures at most every ~140 ms while geometry is changing. An unchanged
surface stops requesting capture after five seconds, avoiding a permanent 7 Hz
screen-capture loop. If capture fails, the flat material remains available.

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
- Bar modules are declared in one typed registry with explicit zone ownership. Left/right strip content is cached as transparent device-resolution layers, and metric damage repaints only the invalidated side while preserving the other side, center content, and hit regions. Full damage still rebuilds the bar for animation, popup, DPI, and layout changes; no user scripts are executed.
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

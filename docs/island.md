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
- **Content Rides the Morph** — content is drawn on its own canvas and composited with a mid-spring opacity dip and a slight rise-into-place, so it fades and settles with the container instead of popping. The face animation keeps ticking **during** morphs, and worker updates (media, tasks) apply mid-flight instead of waiting for the morph to end.
- **Windows 11 Taskbar Fluent Acrylic & Liquid Glass** — uses native Windows 11 Taskbar Acrylic composition (`SetWindowCompositionAttribute` with `ACCENT_ENABLE_ACRYLICBLURBEHIND` state 4, `DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE, 3)`, and `DwmEnableBlurBehindWindow`). Solves desktop double-blending with opaque internal compositing and renders:
  - Top parabolic specular sheen across the upper surface.
  - Optical refraction groove mimicking curved glass refraction.
  - Chamfered outer specular rim catching overhead lighting.
  - Subtle acrylic dither texture eliminating banding on dark gradients.
- **Dynamic Content-Based Sizing** — no rigid fixed widths; compact mode automatically scales to hug active content (scaling smoothly as multiple agent sessions or live media sessions activate).
- **Prominent Tall Media Player Card** — expanded mode features a 56×56 album art card with squircle rounded corners, supersampled grayscale typography (track title, artist, app source — rasterized at 2x, blended at 1x), an animated 4-bar equalizer wave, interactive play/pause hit-rect click toggle, and a sleek timeline progress bar.
- **Window Notification Alerts** — transient auto-expanding notification banners (124px tall dropdown) when agent events arrive (`needs_input`, `turn_failed`) or songs change. Displays prominent titles, session context, and accent beacon badges for 3.5s before gracefully springing back. These are the only auto-expanding surfaces — the island itself never springs open on its own.
- **Windows toast forwarding** (`forward_toasts`, default on) — app notifications from Action Center ride the island as transient alert banners (app name plus the first text lines, 6 s). Needs notification-listener access (Settings → Privacy → Notifications); without it the watcher exits silently. Local-only: nothing leaves the machine. Pre-existing toasts never flood on startup — only new arrivals fire.
- **Motion signatures** — each agent state moves: thinking dots bounce, a three-dot orbit circles the face while tools run, Ready pops a 600 ms sparkle burst, failure shakes damped over 300 ms, and the needs-input strip breathes. All ride a 50 ms motion tick (idle-silent, reduced-motion stills everything); settled frames are pixel-stable by construction.
- **Triple-spring morphs** — expands bounce (`animation_ms`), collapses settle critically damped and quicker (`collapse_ms`), alert banners drop in fast (`alert_ms`). Alert banners queue (3 deep) with a timeout hairline instead of overwriting each other.
- **Attached notch** (`layout: notch`) flush with the top edge — flat top, rounded bottom; the pill is drawn *from* `y=0` **in every presentation** (the anchor never moves mid-morph, so expansions grow downward from the bezel like the real thing).
- **True-black notch material** — in notch layout the body renders as opaque `#000` (`glass.notch_black`, default on), fusing with the display bezel like real hardware instead of reading as a floating glass widget. Floating islands keep the translucent glass.
- **Floating island** (`layout: island`) with `y_offset` 0-500.
- **Active display** — the island anchors to the monitor under the cursor, picked on startup and re-picked on display/DPI changes (`WM_DISPLAYCHANGE`/`WM_DPICHANGED`); morphs never move it between screens mid-animation.
- **Auto theme** (`theme: "auto"`) — follows the Windows light/dark setting (`AppsUseLightTheme`), hot-swaps live on `WM_SETTINGCHANGE("ImmersiveColorSet")`. With "show accent color on Start and taskbar" on, the glass also tints 40% toward the system accent and re-resolves live on accent changes; explicit themes stay exact and explicit `glass` fields still win. The auto dark material is sampled from a transparency-on neutral taskbar (cool luminous veil, ~73% opacity) rather than the generic dark preset.
- **Animated termielle inside the notch** — each agent state's GIF is pre-decoded once per state change (downscaled to a 96px cache, shown at ≤32px) and cycled on its own deadline. `face_animated: false` or reduced-motion freezes it on the first frame.
- **Agent states** — lifecycle events morph the pill; a 2px accent strip along the top edge colors by state (amber thinking, green working, blue needs-input, teal ready, red failed).
- **Click to expand/collapse** — `WM_LBUTTONUP` toggles manual expansion (idle only); hovering expands when `expand_on_hover` is set. Clicking the media element toggles play/pause.

## Modes

`~/.termielle/config.json` → `island.layout`:

| `layout` | Shape | Position | Use |
|----------|-------|----------|-----|
| `classic` | 360×360 square, free-drag, clamped to work area | anywhere (legacy pet) | default, preserves exact pre-fork behaviour |
| `notch` | flat top, rounded bottom (18px), `width × 36` | top-center, `y=0` flush with bezel | macOS-like notch |
| `island` | pill, all corners rounded | top-center, `y = y_offset` (8-12) floating | iPhone Dynamic Island |

Switchable is just a config toggle — set `notch` or `island` and restart (tray → Exit, then `termielle-app.exe` or task restart).

## Customization

### Geometry — `island.*` in `config.json`

```json
"island": {
  "layout": "notch",
  "minimal_width": 72,      // nothing live  48..800 (>= height+8)
  "collapsed_width": 180,   // agent/media live (compact)  80..1200
  "expanded_width": 320,    // hovered/pinned (expanded)  120..1600
  "height": 36,             // 28..200
  "corner_radius": 18,      // 0..120, clamped to height/2
  "y_offset": 0,            // island only, 0..500
  "animation_ms": 350,      // spring perceptual duration 100..800
  "spring_bounce": 0.18,    // 0 = no overshoot, 0.5 = very springy
  "theme": "auto",          // liquid-dark | midnight | light | transparent | auto
  "glass": { "tint": [26,26,26,180], "blur_radius": 0, "border_alpha": 38, "highlight_alpha": 70, "shadow_alpha": 60 },
  "face_animated": true,    // cycle the termielle GIF inside the notch
  "widgets": ["face","agents","music","ring"],
  "expand_on_hover": true
}
```

### Widgets — configurable island content

`widgets` selects what the pill shows; unknown names are dropped on load:

| name | shows | where |
|------|-------|-------|
| `face` | animated termielle face (state GIF, 44px) | leading / left of the pill, all states |
| `agents` | live agent session dots + state beacon | center (expanded) or trailing (compact) |
| `music` | media artwork + animated equalizer bars; click toggles play/pause (SMTC) | trailing / right side; teal strip on top edge when idle |
| `ring` (`ring_metric`) | progress ring around the face: battery/CPU/mem % | around the face (config kept for compat) |


> **On agent "quota/usage":** the event protocol is content-free by design
> (session ids plus lifecycle kinds only — no prompts, no tokens, no
> credentials ever touch the overlay), so true API quota is not visible to
> the notch. The `agents` widget shows live session activity instead: one dot
> per connected session, in the agent state's accent color.

Remove a name to hide it, e.g. `"widgets": ["face","music"]` for a
minimal island. Tray → `Widgets` toggles live media / hover-expand / face live.

### Interactivity

- **Idle is Hidden**: By default (`auto_hide: true`), the island stays hidden when idle, resting as an invisible 2px sensor along the top bezel.
- **Hover to Pop Out**: Moving the cursor to the top edge pops out the compact pill (140×36px). Leaving smoothly retracts it back into hidden.
- **Click to Extend**: Clicking the popped-out pill extends it vertically and horizontally into the full tall card (320×154px). Clicking again collapses it back.
- **Live Activity**: Active agent turns (`Thinking`, `Working`, `NeedsInput`, `Ready`) stay in extended mode live until the turn is completely done.
- **Hand cursor** over the visible pill signals clickability; when hidden, the cursor remains a standard arrow. Transparent corners stay click-through via the per-pixel alpha map.

### Rendering pipeline (no-freeze design)

Media queries, artwork decoding, and backdrop captures run on a **background worker thread** (`tasks.rs:
spawn_worker`, every 1.5s), which posts batches to the GUI thread. The GUI
thread only composites the cached frames — hovering and morphing never
stall. The face GIF is decoded once per agent state, never on clock ticks.

Presentations size to their content: the compact pill hugs face/dots/media, the expanded card uses `expanded_width` with a content-dependent height (154/180/210). Frames are authored at device pixels and presented 1:1 with no filtering (`render_scale` = monitor DPI × user `scale`, honored unless `scale_with_dpi: false`); layout reads logical throughout while clicks/hover map back to frame space. Morphs are spring-integrated (width, height, corner radius, blob separation) at `frame_interval_ms` (default 16ms) while `AnimationClock` (dedicated thread, not `WM_TIMER`) paces and the measured present cost is subtracted from each interval.

All values clamp like `scale:0.5..2.0` — a bad value never discards the file.

### Glass — live frosted glass

The pill composites over a **live capture of the desktop behind it**: a
plain `BitBlt` (which excludes layered windows, so the glass never feeds
back into itself) fills the pill rect, a three-pass box blur at
`glass.blur_radius` (the standard gaussian approximation — no banding or
ringing) frosts it, and the theme tint goes over the top. The worker thread
recaptures every ~140 ms, fast enough that the glass tracks windows moving
behind the island instead of showing a seconds-stale snapshot. If capture
fails (locked/secure desktop) it falls back to flat.

On top of the blurred backdrop, baked into the `FrameBuffer` PBGRA before
`UpdateLayeredWindow(ULW_ALPHA)`:

1. shadow — black, `shadow_alpha` (60), 2px offset
2. body — `tint` BGRA at `tint[3]` (180 ~ 0.70) with AA rounded-rect coverage (2×2 supersample)
3. border — 1px inner stroke `border_alpha` (38 ~ 0.15), tint lightened toward white
4. highlight — top 1px `highlight_alpha` (70 ~ 0.28)
5. content — animated face, live agent activity, media artwork + equalizer bars (no text anywhere)

Uses `SetWindowCompositionAttribute(ACCENT_ENABLE_ACRYLICBLURBEHIND)` with Windows 11 `DWMWA_SYSTEMBACKDROP_TYPE` and `DwmEnableBlurBehindWindow` for genuine hardware taskbar blur, with software backdrop compositing fallback for remote desktop, and `RenderMode::ColorKey` (magenta `#FF00FF` threshold 128) for broken DIB drivers.

### Themes — `themes/*.json` and `auto`

Search order: `~/.island/themes/<name>.json` (user shadows) → `~/.termielle/themes/` → `exe_dir/themes/` → `themes/` (repo). Missing file falls back to builtin. `theme: "auto"` maps to `light` or `liquid-dark` from the Windows personalization setting and re-resolves live when it changes.

Preset files ship in `themes/`:

- `liquid-dark` — `[26,26,26,180]`, border 38, highlight 70, shadow 60 (dark default)
- `midnight` — navy `[12,18,32,200]`, stronger shadow
- `light` — `[245,245,245,160]` for dark wallpapers
- `transparent` — `[30,30,30,90]` barely-there
- `auto` — follows Windows light/dark

**Explicit `glass` fields in `config.json` win over any theme preset** — a user-set `tint` survives a theme switch (fields differing from the glass default are treated as overrides).

```json
{ "name":"liquid-dark", "tint":[26,26,26,180], "blur_radius":0, "border_alpha":38, "highlight_alpha":70, "shadow_alpha":60, "corner_radius":18 }
```

At startup `theme::resolve_theme(&mut config.island)` applies builtin then file overlay, then `glass.clamp()` + `island.clamp()`. Tray picker is TODO — edit `config.json` and restart for now.

Add your own: copy `themes/liquid-dark.json` to `~/.island/themes/my.json`, tweak `tint`/`border_alpha`/etc, set `"theme":"my"` in `config.json`.

## Architecture reuse

| Termielle | Island fork |
|-----------|-------------|
| `termielle-ipc` pipe `termielle-v1`, owner-only SDDL `D:P(A;;GA;;;SID)` + `SECURITY_IDENTIFICATION` | rename to `island-v1` (or keep) — `Source` already accepts any `[a-z0-9_-]{1,32}` so no emitter change |
| `protocol.rs` 8 `EventKind` content-free | 1:1 pill states `Idle→collapsed`, `Thinking/Working/NeedsInput/Ready→expanded` |
| `reducer.rs` `priority()` + `THINKING_HOLD_MS=1000` | clone, retune `busy_stall_ms` 300s→10s for island auto-collapse (currently keeps 300s; set `busy_stall_ms` in config to tune) |
| `window.rs:572` `UpdateLayeredWindow` + `alpha_hit_test:172` `threshold 16` | keep, add `island_anchored_position()` + `present_with_anchor()` + force `WS_EX_TOPMOST` when island enabled |
| `animation/gif.rs:90` `AnimationSource::Gif|Still` + `fallback.rs:33` square | add `notch.rs:island_frame()` + `theme.rs`, keep Gif for `classic` |
| `app.rs:41` `Controller` | dual-mode: `classic` → Gif, `island` → `island_frame` + `Morph{from_w,to_w,start,duration}` at 60fps |
| `install.ps1:359` atomic `MoveFileExW(WRITE_THROUGH)` + `Backup-Once` | add `themes/` copy |

## Try it

```powershell
cargo build
# notch (macOS) — attached, 180→320, liquid-dark glass
cat ~/.termielle/config.json  # island.layout notch already
cargo run -p termielle-app               # shows notch top-center
# trigger expand:
cargo run -p termielle-emit -- --source codex --event prompt_submitted --input stdin <<< '{"session_id":"demo"}'
# check morph
cargo test -p termielle-app --test island_controller -- --nocapture
pwsh -File scripts/doctor.ps1 -BinDir target/debug
```

Switch to floating island:

```powershell
# edit ~/.termielle/config.json -> "layout":"island","y_offset":12
taskkill /IM termielle-app.exe /F; cargo run -p termielle-app
```

Revert to pet:

```powershell
# "layout":"classic"
```

## Files added

- `crates/termielle-core/src/island.rs` — `IslandConfig`, `GlassConfig`, `IslandLayout`, `island_anchored_position`, eases
- `crates/termielle-app/src/animation/notch.rs` — `island_frame`, rounded-rect SDF + AA, glass layers
- `crates/termielle-app/src/theme.rs` — builtin presets + `resolve_theme` file overlay
- `crates/termielle-app/tests/island_controller.rs` — morph, reduced_motion, state widths
- `themes/{liquid-dark,midnight,light,transparent}.json`
- patches to `config.rs`, `window.rs`, `app.rs`, `main.rs`, `animation/mod.rs`

## Next

- Tray menu: `Notch / Island / Classic` + `Theme →` submenu (calls `set_island_config` live without restart)
- File watcher on `config.json` for hot reload
- True liquid blur via `PrintWindow` capture + `Direct2D` `GaussianBlur` + displacement (measure `present_cost_ms`, target <5ms at 320×36)
- `install.ps1`/`release.yml` copy `themes/` next to `bin/assets`

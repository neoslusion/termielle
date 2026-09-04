# Dynamic Island / Notch — Windows

Fork of Termielle's overlay into a top-center notch or floating island with **custom layered glass** (per-pixel alpha, no DWM acrylic dependency).

## Features (current)

- **iOS presentation model** — the pill moves through the same three size classes as the Dynamic Island: **Minimal** (small resting dot when nothing is live), **Compact** (resting pill while an agent session or media is live), **Expanded** (hovered/pinned dashboard). Transitions are driven by a real **spring** (Apple's model: `stiffness = (2π/duration)²`, `damping = (1-bounce)·4π/duration`), so morphs overshoot slightly and interrupted morphs inherit velocity — the "living organism" feel.
- **Attached notch** (`layout: notch`) flush with the top edge — flat top, rounded bottom; the pill is drawn *from* `y=0`.
- **Floating island** (`layout: island`) with `y_offset` 0-500.
- **Auto theme** (`theme: "auto"`) — follows the Windows light/dark setting (`AppsUseLightTheme`), hot-swaps live on `WM_SETTINGCHANGE("ImmersiveColorSet")`.
- **Animated termielle inside the notch** — each agent state's GIF is pre-decoded once per state change (downscaled to 44px) and cycled on its own deadline. `face_animated: false` or reduced-motion freezes it on the first frame.
- **Visual-only mini dashboard** — no text anywhere: the animated face, one app **icon** per running task (window icons, not screenshots), one dot per live agent session (`agents`), and a **live media element** (`music` widget): the SMTC artwork thumbnail when the source exposes one (YouTube shows the video thumbnail), else the source app icon, with three equalizer bars pulsing while playback runs. Clicking the media element toggles play/pause via the multimedia key.
- **Agent states** — lifecycle events morph the pill; a 2px accent strip along the top edge colors by state (amber thinking, green working, blue needs-input, teal ready, red failed).
- **Click to expand/collapse** — `WM_LBUTTONUP` toggles manual expansion (idle only); hovering expands when `expand_on_hover` is set.
- **Idle clock + stats** — collapsed shows `HH:MM`; expanded adds `XX% MEM`, `XX% CPU`, battery `+` when charging, and the focused app name.

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
  "show_tasks": true,
  "max_thumbnails": 4,      // 0..6 app icons in the expanded pill
  "face_animated": true,    // cycle the termielle GIF inside the notch
  "widgets": ["face","tasks","agents","music","ring"],
  "expand_on_hover": true
}
```

### Widgets — configurable island content

`widgets` selects what the pill shows; unknown names are dropped on load:

| name | shows | where |
|------|-------|-------|
| `face` | animated termielle face (state GIF, 44px) | left of the pill, all states |
| `tasks` | running-task app icons (with `show_tasks`) | expanded pill |
| `agents` | one dot per live agent session (accent color) | after the icons |
| `music` | media artwork + animated equalizer bars; click toggles play/pause (SMTC) | expanded pill; teal strip on top edge when idle |
| `ring` (`ring_metric`) | macOS-style arc around the face: battery/CPU/mem % | around the face (config kept for compat; widget currently omitted from the compact layout) |


> **On agent "quota/usage":** the event protocol is content-free by design
> (session ids plus lifecycle kinds only — no prompts, no tokens, no
> credentials ever touch the overlay), so true API quota is not visible to
> the notch. The `agents` widget shows live session activity instead: one dot
> per connected session, in the agent state's accent color.

Remove a name to hide it, e.g. `"widgets": ["face","tasks","ring"]` for a
minimal island. Tray → `Widgets` toggles tasks / hover-expand / face live.

### Interactivity

- **Hover to expand** (`expand_on_hover`, default on): entering the pill
  morphs it open when idle; leaving collapses it unless toggled or an agent
  is active. Hover is edge-triggered (`TrackMouseEvent(TME_LEAVE)`), so no
  polling.
- **Click to toggle**: a click takes control from hover — the pill stays as
  toggled until the next click, agent event, or config change.
- **Hand cursor** over the pill signals clickability; transparent corners
  stay click-through via the per-pixel alpha map.

### Rendering pipeline (no-freeze design)

Icon reads, media queries, and face pre-decoding run on a **background worker thread** (`tasks.rs:
spawn_worker`, every 1.5s), which posts batches to the GUI thread. The GUI
thread only composites the cached frames — hovering and morphing never
stall. The face GIF is decoded once per agent state, never on clock ticks.

`collapsed_width` is Idle/Failed; `expanded_width` is all other states. Morph interpolates width with `easeInOutCubic(progress)` at `frame_interval_ms` (default 16ms) while `AnimationClock` (dedicated thread, not `WM_TIMER`) paces.

All values clamp like `scale:0.5..2.0` — a bad value never discards the file.

### Glass — live frosted glass

The pill composites over a **live capture of the wallpaper behind it**: a
plain `BitBlt` (which excludes layered windows, so the glass never feeds
back into itself) fills the pill rect, a separable box blur at
`glass.blur_radius` (default 12, 0 = flat tint) frosts it, and the theme
tint goes over the top — the same frosted look as Windows acrylic, tracking
the wallpaper and the light/dark `auto` theme. The capture is cached and
only refreshed when the pill moves or every second, so animated faces stay
smooth. If capture fails (locked/secure desktop) it falls back to flat.

On top of the blurred backdrop, baked into the `FrameBuffer` PBGRA before
`UpdateLayeredWindow(ULW_ALPHA)`:

1. shadow — black, `shadow_alpha` (60), 2px offset
2. body — `tint` BGRA at `tint[3]` (180 ~ 0.70) with AA rounded-rect coverage (2×2 supersample)
3. border — 1px inner stroke `border_alpha` (38 ~ 0.15), tint lightened toward white
4. highlight — top 1px `highlight_alpha` (70 ~ 0.28)
5. content — animated face, ring, app icons, session dots (no text anywhere)

No `SetWindowCompositionAttribute(ACCENT_ENABLE_ACRYLICBLURBEHIND)` — deterministic tint/highlight/shadow, works on remote desktop, falls back to `RenderMode::ColorKey` (magenta `#FF00FF` threshold 128) for broken DIB drivers.

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

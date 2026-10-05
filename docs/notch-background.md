# Notch / activity-pill background repair

The user requested a notch-background fix during the app-navigation milestone.
A direct, narrow capture of the installed bar's center confirmed a concrete bug:
**plain desktop BitBlt includes Termielle's own layered pill on this Windows
build**, despite the old comments claiming SRCCOPY excluded layered windows.
Using that image for glass makes the pill blur/tint itself, not the scene behind
it. This finding does not identify every possible visual defect without a user
screenshot.

## Repair

- The worker excludes **only its own process-owned HWND** during the desktop
  copy using Windows 10 2004+ `WDA_EXCLUDEFROMCAPTURE`. A scoped guard restores
  the previous policy before blurring/publishing the image, including failure
  paths. The physically displayed window is never hidden or moved for capture.
- A real-version check avoids older Windows interpreting exclusion as an opaque
  black redaction. Failed/unsupported exclusion uses flat translucent material
  rather than knowingly sampling itself or bypassing another app's protection.
- There is **no permanent screenshot/recording exclusion**. A capture made
  concurrently with the short internal copy can omit the widget for that frame;
  ordinary captures after restoration include it again. This tradeoff must be
  reviewed for recording-heavy use before complete taskbar replacement.
- Desktop copying uses a scoped physical-DPI context and restores its caller's
  context. Screen-anchored sampling remains unchanged.
- The actual capture DIB is initialized, not just the returned vector, so
  partially off-screen captures do not introduce uninitialized black fringes.
  A failed in-screen copy returns no image rather than a fabricated capture.
- Frosted-edge composition separates silhouette coverage from material opacity.
  Already-premultiplied edge colours are not multiplied twice; a missing sample
  preserves the original translucent pixel rather than forcing opaque tint.
  Fully transparent pixels remain transparent; text/art is not retinted.
- Foreground-window switches/moves re-arm a short capture burst even when the
  notch itself stays still. The five-second quiet cutoff remains, with a 250 ms
  minimum capture interval during changes; this is not continuous video blur.
- Ownership/generation stamps reject pre-resume or pre-theme captures. Display
  invalidation re-arms capture even if the monitor dimensions did not change.
  Intentional opaque-black hardware-notch material does not consume old glass.

The implementation and tests did not change the user's JSON, theme/tint
preferences or native taskbar. The executable was subsequently replaced with
this build at the user's explicit request on 2026-10-04; see
[installation evidence](app-navigation.md#local-installation--2026-10-04).

## Evidence

Combined workspace run: **476 passed, 0 failed, 2 existing manual launcher tests
ignored**. Tests cover premultiplied edges/all alpha values, transparent and
missing-sample handling, DIB fringes, DPI restoration, owner validation, capture
policy restoration and background-generation invalidation.

A narrow live diagnostic additionally verified an owned 64x32 nonactivating
marker: the old path captures the marker, scoped exclusion sees the existing
scene behind it (not a black rectangle), and ordinary capture includes the marker
again after restoration. Foreground ownership was unchanged and the marker was
destroyed. This is a capture-path check, not physical lid-cycle or full UI
acceptance.

```powershell
# Read only the installed bar's center strip using the old raw path.
cargo run -p termielle-app --example notch_backdrop_probe -- target/notch-backdrop-before.bmp
# Briefly show an isolated marker; verify background-worker / GUI-owner flow.
cargo run -p termielle-app --example notch_backdrop_probe -- target/notch-backdrop-excluded.bmp --owned
```

Review the actual activity pill/standalone Notch and Island against light/dark
wallpapers and app windows, expanded/morphing states, top/bottom bars, monitor
edges, DPI changes and a physical lid cycle. The updated build is installed;
full visual acceptance still remains. If the reported problem instead
means the desired material colour or another shape defect, get a screenshot
before claiming that appearance is resolved.

Related: [app navigation](app-navigation.md), [bar guide](island.md),
[display recovery](display-recovery.md).

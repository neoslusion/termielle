# Bar performance and Clear All follow-up

Measured on **2026-10-02**, after the desktop-bar/launcher baseline documented
in [the development handoff](development-handoff.md). These changes are a local
follow-up, not a published release or cold-boot performance guarantee.

## Changes

- **Quiet idle bar:** the resting face streams its first GIF loop, selects the
  frame with the most visible artwork, then stops ticking and releases the other
  frames. This avoids freezing on the almost-empty opening frame. Active
  agent/media animation, standalone idle animation, and reduced motion retain
  their existing behaviour; starting playback reloads a settled face.
- **Visible damage only:** hidden CPU/RAM and Bluetooth changes do not repaint
  the strip when Control Center is closed. Snapshots are still cached so opening
  the panel shows fresh data. Worker-driven metrics replace the unconditional
  two-second repaint when a cached snapshot is available.
- **Reusable vector masks:** path parsing and distance-field coverage are cached
  per icon/device size, independently of colour, with a 512 KiB coverage-buffer
  budget and least-recently-used eviction. Larger masks are drawn but not kept.
- **Bounded launcher artwork:** the worker retains at most 32 icon results,
  including failed extractions. Shared bitmaps avoid worker/model deep copies;
  the model releases artwork outside its six visible results. Catalog refresh
  clears the worker cache.
- **First frame before tray work:** production presents its initial overlay
  before decoding/registering the notification-area icon.
- **Responsive Clear All:** the right panel previously disabled every control
  until its height reached 98% of the opening morph, even when Clear All was
  already painted. Hit regions now follow the visible, sufficiently opaque
  content and are clipped to the card. Clearing also invalidates the unread
  clock indicator immediately. Its scope remains Termielle's recent list, not
  Windows Notification Center, live agent state, or queued banners.

## Before/after measurements

Same host, primary 2560×1440 display at 100 Hz, Macchiato bar, five warm startup
samples and approximately 20 seconds of idle sampling after a six-second
warm-up. The motion probe opens/closes Control Center over 240 frames.

| Metric | Before | After |
| --- | ---: | ---: |
| Idle CPU, percentage of one core | 8.56% | 0.70% |
| Idle overlay presentations | 50.68/s | 0/s |
| Mean private bytes | 17.09 MiB | 15.46 MiB |
| Mean working set | 49.94 MiB | 50.42 MiB |
| Warm first-frame median | 99.11 ms | 74.08 ms |
| Warm first-frame maximum | 122.98 ms | 179.28 ms |
| Motion render + present median | 4.87 ms | 4.20 ms |
| Motion render + present p95 | 22.18 ms | 18.37 ms |
| Motion paint p95 | 15.05 ms | 12.49 ms |

Idle CPU fell about **92%** in this sample. Zero idle presentations means no
visible change arrived during that sampling window, not that clocks, volume
feedback, alerts, or other real updates are disabled. Private bytes fell, but
the total working set did **not** improve in this run. Motion p95 remains above
the display's 10 ms frame budget; this is not a claim of locked 100 fps.

Startup results are variable and file/process caches are warm. The resource
probe uses a custom pipe and isolated config/data, disables toast forwarding
and AppBar reservation, and skips the production tray/journal paths. It does
not measure a Windows login, production hotkey readiness, or prove the tray
reordering's startup benefit. Host media activity and background load can
change the results. The timing probe itself adds acknowledgement overhead.

Raw local results are in ignored `target/bar-performance-before.json`,
`target/bar-performance-after.json`, `target/bar-motion-before.log`, and
`target/bar-motion-after.log`; the resource JSON includes binary hash, config,
individual startup samples, and artifact paths.

## Repeat the checks

Run from the repository root on Windows with Rust and PowerShell 7:

```powershell
rtk cargo fmt --all -- --check
rtk cargo test --workspace
rtk cargo build -p termielle-app --release --bin termielle-app --example bar_motion_review
rtk proxy pwsh -NoProfile -File scripts/bar-performance.ps1 -Samples 5 -DurationSeconds 20 -OutputPath target/bar-performance.json
rtk proxy target/release/examples/bar_motion_review.exe --primary --macchiato --controls
```

The resource script starts and stops only its isolated instances, leaves the
installed app/config/task untouched, and retains artifacts under `target/`.
The motion probe briefly shows a separate overlay without taskbar reservation
or global input injection. Close unrelated workload and avoid active media for
comparable idle results.

## Verification

- Rebuilt and redeployed the installed executable on 2026-10-02; the existing
  `Termielle` scheduled task is running with one installed process. Installed
  and profiled release hashes match. The previous executable is backed up as
  `termielle-app.exe.bak-20261002-152719`; config is unchanged and native taskbar
  replacement remains disabled. No DLLs, assets, or hooks were replaced.
- Workspace: **414 passing tests, two manual launcher tests ignored**; formatting
  check and release app/motion-probe build pass. Existing unrelated `unused_mut`
  warnings in `tests/island_controller.rs` remain.
- Idle cadence regressions failed before the change and pass after it. Tests
  also preserve active animation and verify playback restarts a settled face.
- Vector-cache tests check bounded storage, device-size keys, and identical
  pixels across colours/scales. Launcher tests check shared ownership, negative
  caching, eviction/refresh release, and hidden-result artwork release.
- `visible_notification_clear_button_works_before_the_panel_finishes_resizing`
  failed with the 98% input gate and passes without it for top and bottom bars.
  A real pointer check after deployment remains distinct from this regression;
  see [report 11](reported-behaviours.md#11-clear-all-was-visible-but-not-clickable).

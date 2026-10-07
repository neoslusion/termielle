# Content-fit compact layout and idle review — October 7, 2026

## Status

Implemented in the **shared Rust app** for native x64 and Windhawk x86; built and
functionally tested. Subsequently **deployed and running in the Windhawk x86
edition at user request**, October 7 at 19:07 +07:00. The installed native fallback
executable remains unchanged. Source publication is separate from deployment;
see Git history for the user-requested commit/push. Experimental idle tuning was
measured and withdrawn, not enabled in the final source.

## Geometry

- `src/app/compact_layout.rs` supplies compact target widths, face/label spacing
  and the bar's actual content label. Existing `compact_primary_width()` and
  `bar_pill_rect()` route through it. The bar rectangle remains the authority
  for glass, popup morph origins, painting, hover and click targets.
- Idle/hover Island/Notch uses measured branding for the **stock 140px** compact
  width. Other configured compact widths remain explicit. Minimal/expanded
  widths, zoom, custom glass/theme, reduced motion and spring settings are not
  rewritten. The stock-width interpretation is a rendering change, not a new
  saved preference; an explicitly chosen 140px is indistinguishable from stock.
- Media-only compact pills no longer reserve an unused idle-text column. With
  the standard 36px-height/face fixture, this changes 186px to 106px. No-face
  media removes the primary reservation; the configured minimal width still
  bounds the complete clickable surface. Paused media remains visible/clickable.
- Agent face/dot spacing is accounted for together. The one-agent split fixture
  grows slightly, 129px to 133px, to preserve a clear face/dot gap; the transparent
  separation and media hit targets are tested through interrupted/paused morphs.
- Resting Bar is content-fit, quantized to two logical pixels, bounded by 112–180px
  (and the available monitor width). The lower bound preserves room for transient
  volume icon/meter/value **without moving its hit targets**. Long media titles
  remain ellipsized; empty titles use the existing generic Media fallback.
- `show_name` remains decoration-only: hiding it does not change geometry or
  accessibility/click targets. Side-zone cache keys include the actual center
  width, so a longer title cannot leave stale painted or clickable side limits.

`src/animation/text_metrics.rs` measures the same Segoe UI Variable Text font,
weight and supersampled physical font height used by text paint. Cache keys
include logical font size, bold and render scale (DPI/zoom); the hot path borrows
text without allocation. The per-render-thread cache is bounded at 128 labels,
each at most 512 Unicode characters. Font/DC handles are released on misses;
failures use a bounded conservative estimate. No desktop capture, additional
foreign text collection or agent payload is involved. Clock layout is unchanged;
this does not implement the reference's entire idle-strip or widest-digit clock.

The measured/quantized-layout principle comes from the pinned MIT reference
review, not a copied C++ renderer. See [reference research](windhawk-integration-research.md).

## Idle review: measured, not assumed

Reviewed the existing 120ms task-worker loop, independent media/task/CPU cadences,
foreground/glass probes, bar metrics worker and GUI deadlines. A prototype used
bounded 400/500/1000ms quiet waits, suppressed unused flat-glass scene probes and
requested CPU samples only for the open system dashboard. Live glass retained
120ms movement checks. These changes **are absent from final production source**:
the before/after measurements did not demonstrate a whole-process CPU benefit.
This is not proof that changing one interval caused the difference; layout and
worker changes initially shared a candidate, and OS/worker noise was not traced.

Added `perf` to the existing disposable `windhawk_host_review` and
`scripts/review-idle-resources.ps1`. Both DLLs use the same matching-bitness host,
a fresh private profile/pipe, hidden center-only Bar, no side modules/media/tasks/
face/toasts, flat glass and five-second warm-up. The host reports process CPU
and stops itself normally; the script samples private/working-set memory,
alternates baseline/current order, verifies normal exit, and never force-kills
or unloads a running runtime. Hidden diagnostics do not reserve space/replace
native taskbars. No existing installation/profile/task/mod registration is edited.

Final **layout-only** pilot: two fresh 30-second samples per DLL, October 7 at
18:49:40 +07:00; unweighted means:

| Metric | Installed-code baseline | Layout-only candidate |
| --- | ---: | ---: |
| CPU, percentage of one core | 0.16% | 0.34% |
| Private memory | 6.52 MiB | 6.33 MiB |
| Working set | 18.67 MiB | 18.77 MiB |

CPU was higher, not lower. Short samples, coarse CPU accounting, fresh-process
variation and a reduced hidden fixture do not certify memory savings or establish
a single cause. **No CPU/GPU/memory improvement is claimed.** Trace worker/render
costs before performance-driven deployment; live glass, native-vs-hosted resources,
transition latency, GPU, fullscreen/DPI/lid/recording acceptance remain unmeasured.

Ignored raw evidence: `target/compact-layout-resources.{json,log}`. Earlier
combined-prototype evidence is retained in `target/compact-idle-resources-prototype.json`;
its three 30-second pairs averaged 0.12% baseline versus 0.21% candidate CPU.
Baseline SHA256 is `75ED9ABC5B14EED2E57F6257D62428E8EBF188FCD554D4F7020119DFE3932357`;
layout-only candidate is `B97689F9CD59502A5451C68BCF009571219D4709F24A2315254451F7697524E3`.

Reproduce (paths must match host/DLL bitness):

```powershell
pwsh -NoProfile -File scripts/review-idle-resources.ps1 `
  -HostExecutable target/i686-pc-windows-msvc/release/examples/windhawk_host_review.exe `
  -BaselineDll target/compact-idle-baseline/termielle_runtime.dll `
  -CurrentDll target/i686-pc-windows-msvc/release/termielle_runtime.dll `
  -DurationSeconds 30 -Runs 3 -OutputPath target/compact-layout-resources.json
```

The ignored baseline directory is a local snapshot, not a bundled release asset.
Supply a separately preserved reviewed DLL when reproducing elsewhere.

## Validation

Final workspace tests: **522 passed / 0 failed / 2 existing manual tests ignored**
on **each** x64 and x86 run, plus the two x86 adapter unit tests. Eight consolidated
isolated-host checks passed. Native x64 release and consolidated x86 release/host
builds passed, as did both workspace Clippy checks and feature-enabled adapter
Clippy with the project's established allowances. Fmt/diff checks and all seven
doctor checks passed. The previous reviewed DLL is retained with its original hash.
New regressions cover bounded/
scale-keyed native metrics, actual paint/hit geometry, side-cache invalidation,
long/narrow labels, custom-width preservation and no empty media reservation.
Existing brand/volume-target, paused-media, interrupted-morph, custom-material,
profile/power/recovery and ordinary-process callback tests remain passing.

These are builds/hidden tests, not live interaction or physical acceptance.

## Authorized Windhawk deployment and live observation

User explicitly requested deployment and running after the higher CPU pilot was
reported. Deployed the exact tested DLL; no rebuild, performance tuning, antivirus
exclusions or security changes. Normal mod disable waited for the old disposable
tool to exit; a fresh tool started using the existing edition switcher. No
manager/Explorer termination or in-process runtime restart/unload.

- Active file: `termielle_0.4.0_x86_compact_20261007-190621.dll` in the registered
  Windhawk `AppData/Engine/Mods/32` directory; hash is the layout-only candidate
  above. Manifest's Mod/Runtime/Launcher paths/hashes all identify this one DLL.
- Observed at **19:07:33** and again **19:09:17 +07:00**: owned x86 Windhawk tool
  **PID 4984**, actual `Engine/1.7.3/32/windhawk.dll`, exactly one Termielle DLL,
  exactly one visible bar `(0,0)-(2560,36)`. No native/legacy bootstrap frontend.
- Primary work area `(0,36)-(2560,1392)` matches reservation. Both Windows taskbars
  remain visible. Profile, native executable, task definition and taskbar-styler
  hashes match the current pre-deployment snapshot; persistent Off remains absent.
  Panic log remains 2202 bytes; task naturally Ready while hosted.
- Final resumed check at **21:10:33 +07:00**: current tool **PID 25188**, same
  tested DLL/hash, one visible bar `(0,0)-(1920,45)` on the currently enumerated
  1920×1200 monitor; work area `(0,45)-(1920,1140)`, native taskbar visible. All
  preservation/no-new-panic checks still pass. The host/display differ from the
  earlier observation; this agent did not restart processes or change topology
  during the resumed check. No continuous-PID or physical recovery claim.
- Old DLL remains intact. Rollback registration/manifest and private profile/task
  snapshots: `%LOCALAPPDATA%/Termielle/backups/compact-layout-20261007-190621`.
  No profile/task snapshots were restored over user data. Older legacy rollback
  and Native/Windhawk shortcuts are unchanged.

Ignored evidence: `target/compact-layout-{deployment,live}.json`, deployment and
follow-up logs; local deployment/observer scripts are also ignored. Live ownership,
modules/rectangle/reservation observations are **not** human compact-layout,
fullscreen, DPI, lid/wake, recording or performance acceptance. No savings or
antivirus-clearance claim.

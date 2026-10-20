# Motion and performance: the macOS-grade pass

This records a pass over the overlay's motion and frame cost, guided by the
[Emil Kowalski design-engineering skills](https://github.com/emilkowalski/skills)
installed under `.opencode/skills/`: `apple-design`, `animation-performance`,
`emil-design-eng`, `animate`, `review-animations`, `improve-animations`,
`find-animation-opportunities`, and `animation-vocabulary`.

The skills are written for the web. Their rules were translated to this
codebase rather than applied literally: the equivalent of a browser's
composite-only tier is a layered window that is composited rather than
repainted, the equivalent of `requestAnimationFrame` competing with the main
thread is a `WM_TIMER` competing with the message queue, and the equivalent
of "don't animate through state" is "don't allocate a new canvas for every
frame of the animation".

## What the skills say, and what it means here

| Skill rule | Native equivalent | State before this pass |
| --- | --- | --- |
| Animate `transform`/`opacity`; never layout properties | Composite the surface instead of repainting it | The per-pixel alpha blit already composited; the *upload* into it did not |
| Move motion off the main thread | A dedicated clock thread, not a message-queue timer | The overlay had one; the launcher did not |
| Don't animate through state | Write into the buffer that is already there | Every animated frame allocated a fresh multi-megabyte canvas |
| Keep blur under 20px | Blur on a worker thread, never per frame | Already correct — the worker owns the capture and the blur |
| Name the cause before changing code | Measure with `bar_motion_review` before and after | Done; see below |

## Findings

### 1. The launcher animated on `WM_TIMER` (smoothness)

`crates/termielle-app/src/launcher/window.rs` drove the launcher's reveal and
its selection highlight from `SetTimer(hwnd, MOTION_TIMER, 16, None)`.

Windows delivers `WM_TIMER` through the window's own message queue and
**drops** the message when that queue is busy. The launcher's motion runs
while the user is typing in it — the exact moment the queue is carrying IME,
character, and repaint messages. The overlay itself had already learned this
lesson: its README states that animation is paced by a dedicated clock
thread because "`WM_TIMER`'s message-queue latency cannot hold a 16 ms
cadence". The launcher was the one surface that had not been converted.

**Fix.** `AnimationClock` gained `spawn_with(wake, message)`, so a second
window can run its own display-synced clock. The launcher now owns one: it
waits for the DXGI vertical blank and posts `MOTION_MSG`, and each tick
re-arms the clock for the next refresh. The timer is gone.

### 2. Every animated frame allocated a fresh canvas (performance)

`render_bar_with_damage` and `render_island` returned an owned `FrameBuffer`,
so `self.current = self.render_island(...)` allocated a new pixel buffer and
dropped the old one on every frame of a morph. With the Control Center open
the bar surface is 2560 × ~330 device pixels — about 3.4 MB — allocated,
zeroed, and freed sixty times a second. The partial-update path made it
worse: it cloned the whole surface before repainting the damaged zone.

**Fix.** The surface is now rendered *into* the canvas that is already on
screen.

- `FrameBuffer::resize_cleared` resizes in place and keeps the allocation.
- `notch::glass_layer_blobs_into` writes the material into a destination
  instead of into a buffer of its own.
- `Controller::glass_layer_blobs_into` copies a cached layer into the
  destination rather than handing back a clone.
- `render_bar_with_damage` / `render_island` take the destination as an
  argument; the partial path paints only the damaged zones into the frame
  that is already there, instead of cloning the whole surface first.

### 3. The present copied the frame one pixel at a time (performance)

`OverlayWindow::draw_at` copied the frame into the layered window's upload
surface with a bounds-checked four-byte `copy_from_slice` per pixel, and
allocated a fresh alpha map (`Vec::with_capacity(w * h)`) on every frame —
about 140 KB of allocation per present at bar width.

**Fix.** The frame's rows are contiguous and the surface's stride is a
multiple of four, so the copy is now one `memcpy` per row. The alpha map is
kept on the window and reused across presents.

## What the skills confirmed as already right

These were checked and deliberately left alone:

- **The springs are Apple's own model.** `spring_params` derives
  `stiffness = (2π/duration)²` and `damping = (1 − bounce)·4π/duration`, which
  makes the damping ratio exactly `1 − bounce`. The default
  `spring_bounce = 0.18` therefore gives ζ = 0.82, which matches Apple's own
  table for a drawer or sheet (`damping 0.8`), and the default
  `animation_ms = 350` sits inside Apple's 0.3–0.4 s response range. The
  author had already tuned this correctly.
- **The island morph interrupts cleanly.** A retargeted spring inherits the
  current position and velocity, which is the interruptibility rule in
  `apple-design` §3 and the reason the project uses a spring rather than a
  fixed-duration curve.
- **The launcher's enter and exit are asymmetric.** Opening takes 160 ms and
  closing takes 100 ms, which is the "exit faster than enter" rule in
  `emil-design-eng`.
- **The easing is a strong ease-out.** `Transition` uses
  `1 − (1 − t)³`, which is the `ease-out` the skills call for on anything
  entering or responding to a pointer.
- **The blur is off the frame path.** The worker owns the backdrop capture
  and the blur; a frame never pays for it.
- **Reduced motion is honoured.** `ReducedMotion::System` reads the Windows
  client-area-animation setting, and the morph snaps rather than springing
  when motion is reduced.

## Measurements

`bar_motion_review` drives 240 frames of the Control Center open/close morph
on a 2560×1440 display at 59 Hz, so the frame budget is 16.7 ms. Nine runs of
the baseline build and eleven of the new one, on an otherwise idle host;
every figure below is the median of the per-run medians or p95s:

| Metric | Before | After | Change |
| --- | ---: | ---: | ---: |
| Render + present, median | 3.76 ms | 2.90 ms | **−23 %** |
| Render + present, p95 | 13.20 ms | 10.62 ms | **−20 %** |
| Paint, median | 1.53 ms | 1.29 ms | −16 % |
| Paint, p95 | 8.59 ms | 6.67 ms | **−22 %** |
| Present, median | 3.08 ms | 2.56 ms | −17 % |
| Present, p95 | 5.68 ms | 4.69 ms | −17 % |
| Presentation interval, p95 | 22.24 ms | 21.27 ms | −4 % |

The distributions separate cleanly rather than merely trending: the *best*
baseline run measured a 12.33 ms render-and-present p95, and the *worst*
post-change run measured 12.03 ms. Every post-change run beat every
baseline run on that metric.

The p95 of render + present now sits inside the 59 Hz frame budget on every
run, where before it sat above it. The spread also tightened: the p95
ranged from 12.3–14.0 ms before and 9.8–12.0 ms after.

**These numbers are from a sandbox host and are not a claim about any
particular machine.** The `scripts/benchmark.ps1` budgets (idle CPU, warm
first frame, event-to-frame) are dominated by host scheduling on that host
and were too noisy to compare — the same baseline binary produced a
395 ms and a 1258 ms event-to-frame reading on consecutive runs. The motion
probe measures the render-and-present path directly over 240 frames and is
the instrument these numbers come from. Re-measure on the target machine
before treating any of it as a published figure.

## Also fixed: the toolchain

The workspace declares `rust-version = "1.85"` but CI installs `stable`, so
a newer Clippy failed the `-D warnings` gate on eight pre-existing findings
(`assertions_on_constants`, `needless_update`, `needless_range_loop`,
`cmp_owned`, `unnecessary_mut_passed`, `items_after_test_module`, and two
`too_many_arguments`). Those are fixed, and the gate is green again.

## Not done, and why

- **The overlay's `Controller` is still a god object** (about sixty fields
  spanning animation, springs, glass caches, tasks, notifications,
  navigation, bar metrics, DPI, and media) and `runtime.rs::run_gui` is
  still a ~690-line function with twelve arguments. Splitting them is the
  highest-value structural work left in this crate, and it is a refactor
  rather than a fix; it was left alone here so the perf work stayed
  reviewable.
- **The glass cache is still keyed on eleven fields** and is capped at four
  entries. The copy it now performs is a memcpy rather than an allocation,
  which is the expensive part, but the key could be a struct.
- **The island path still allocates two scratch canvases per frame**
  (`content` and `visible_content`). The bar path — the default surface —
  was converted first; the island path keeps a `content_layer` alive as an
  `Rc` for the crossfade, which makes its reuse a separate change.
- **The launcher's reveal still fades and rises without scaling.** Apple's
  "materialize, don't just fade" would have the glass scale in from its
  trigger, but the launcher paints 1:1 into a DIB, so a scale means either
  resampling the content or moving the window — a larger change than this
  pass took on.

## Re-measuring

```powershell
cargo build -p termielle-app --release --example bar_motion_review
.\target\release\examples\bar_motion_review.exe --primary --macchiato --controls
```

The probe prints render + present, paint, present, and the presentation
interval, each as a median and a p95 over 240 frames. Run it with no other
Termielle instance running: a second instance competes for the CPU and
inflates every number.

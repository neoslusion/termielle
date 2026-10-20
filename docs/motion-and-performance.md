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

## Second pass: what the skills said about frequency

The first pass took the skills' performance and spring rules and confirmed
what was already right. It did not run the `review-animations` gate
(`.opencode/skills/review-animations/STANDARDS.md`, *Should it animate?*),
which is the first question the skills ask and the one that matters most for
a launcher.

| Surface | Gate result | Change |
| --- | --- | --- |
| Alt+Space launcher open/close | Keyboard-initiated, 100+ times a day: **no animation, ever** (the skills cite Raycast). The 160/100 ms reveal was praised in the first pass as asymmetric timing; the right answer was to delete it. | The launcher appears and disappears instantly. The fade, the eight-pixel rise, the exit, and the `reveal`/`origin` state behind them are gone. |
| Arrow-key selection highlight | List navigation, tens of times a day: near-imperceptible or nothing. | 110 ms → 60 ms. Typing and mouse selection still snap. |
| Status chips in the bar (clock, Control Center, volume, …) | Had a hover crossfade but no press state; `apple-design` §1 says feedback lands on the press, not the release. | A pressed chip takes its fill on the frame the pointer goes down and eases back over 140 ms on release. Reduced motion releases at once. The notch's existing press swell is unchanged; the bar had no press feedback at all. |
| A click on a status chip | `apple-design` §10: a tap commits on release and can be cancelled by dragging away. A press that began on a chip used to activate whatever was under the pointer at release, which contradicted the new press fill. | A press that began on a chip only clicks if it is released on that chip. Other controls are unchanged. |

The launcher's display-synced clock from the first pass stays: it now drives
only the 60 ms highlight glide, which is where a dropped `WM_TIMER` while the
user is typing would still show.

### Also fixed: the alpha map was borrowed across the present

The first pass moved the hit-test alpha map into `OverlayWindow::draw_at` as a
`RefCell` borrow held across `UpdateLayeredWindow` and `SetWindowPos`. The
window procedure reads that map for `WM_NCHITTEST` and `WM_SETCURSOR`, so a
synchronous dispatch from inside the present would have hit an already-borrowed
`RefCell` and aborted the process. The map is now built in a scratch buffer on
the window and swapped into place only after the present succeeds, which keeps
the allocation reuse, keeps the previous map readable throughout the present,
and means a failed present no longer leaves the hit test describing a frame that
never reached the screen.

## Third pass: accessibility, shared motion tokens, and the surfaces not yet audited

### Reduced transparency and high contrast

`apple-design` §14 asks for three independent signals. Reduced motion was
already honoured. Windows "transparency effects" off was honoured only by the
themes that spelled out an opaque variant (`light`, `macos-dark`, the default),
so `catppuccin-macchiato`, `midnight` and `transparent` stayed translucent. High
contrast was not read at all.

`theme::accessible_material` now applies both settings to every theme: a
solid tint when transparency effects are off or a high-contrast theme is
active, and, for high contrast, a defined border (never weaker than the theme
already had). `system::high_contrast_enabled` reads `SPI_GETHIGHCONTRAST`, and a
`WM_SETTINGCHANGE` for `SPI_SETHIGHCONTRAST` re-resolves the theme live the same
way a light/dark flip does. The blur radius is left alone: preferences derives
its recording-safe toggle from it, and an opaque tint already hides the blur.

### One motion vocabulary

The durations were hand-typed in four files (60, 120, 140 ms) and the one easing
was a plain cubic ease-out, which the skills call too weak for UI. They now live
in `animation/motion.rs`: `HOVER_MS`, `PRESS_RELEASE_MS`, `SELECTION_GLIDE_MS`,
`CONTENT_SWAP_MS`, and `ease_out`, the skills' exact
`cubic-bezier(0.23, 1, 0.32, 1)`. `Transition` uses it, so hover, press release
and the launcher highlight share one curve. It reaches 68 % of the distance by
20 % of the time where the cubic reached 49 %, so those motions start faster.
The island morph is a spring driven by the user's `animation_ms` and
`spring_bounce`, and the content crossfade keeps its smoothstep (an
ease-in-out, which is right for a state change that is not entering).

### Audit: toasts, alerts, tooltips

| Surface | Result |
| --- | --- |
| Alert drop-in and collapse | Conforms. Arrival is a 220 ms spring, growth 350 ms, collapse 300 ms critically damped with no overshoot, so the system response snaps and the exit never bounces. The same morph spring is interruptible, and reduced motion snaps it. |
| Queued alerts | Conforms. A second banner takes the first's place through the existing content crossfade rather than stacking or teleporting. |
| Alert countdown hairline | Conforms. It is constant motion (linear), ticks only while a banner with a timeout is visible, and stops with it. |
| Tooltips | Native Windows control with a 600 ms initial delay. It keeps its own shorter reshow delay while moving between tools, which is the "skip the delay after the first" rule. Not ours to restyle. |
| Notification list scrolling | One row per wheel notch, with no glide. This is the only opportunity found: a 60-100 ms glide would remove a small teleport. It was left alone because the list is short and Windows list controls row-snap the same way. |

## Re-measuring

```powershell
cargo build -p termielle-app --release --example bar_motion_review
.\target\release\examples\bar_motion_review.exe --primary --macchiato --controls
```

The probe prints render + present, paint, present, and the presentation
interval, each as a median and a p95 over 240 frames. Run it with no other
Termielle instance running: a second instance competes for the CPU and
inflates every number.

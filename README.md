# Termielle

Termielle is a low-overhead animated desktop companion for the terminal. A
transparent overlay character follows Claude Code, Codex CLI, opencode (1.x
and 2.x), and Antigravity CLI (`agy`) lifecycle events, received over a local
named pipe (Windows) or Unix domain socket (macOS/Linux). Version 1 targets
Windows, including hooks launched from WSL through a bridge executable; the
event protocol, transport, and emitter are platform-neutral and covered by a
Linux CI job. The events are small and content-free: no prompts or terminal
output are ever captured.

The bar is a native Windows surface inspired by Waybar's information layout;
Termielle does not run Waybar, parse Waybar JSON/CSS, or use GTK/Wayland.

The current desktop refinements add a Macchiato menu bar, independent Control
Center and activity-pill cards, recent notifications, volume feedback, and an
**Alt+Space app launcher** that does not invoke Windows Search. See the
[development handoff](docs/development-handoff.md) for delivered features,
decisions, validation, and local redeployment; the
[island and bar guide](docs/island.md) covers detailed configuration and usage.

Any other agent CLI works too: the emitter's `--source` accepts any short
lowercase identifier and its session extraction recognizes every common
identifier key, so wiring up a new agent is pure configuration, following the
scaffold in `integrations/_template/`.

## Install

One-shot installer — downloads the latest release, verifies the checksum,
installs the binaries to `%LOCALAPPDATA%\Termielle\bin`, registers the
crash-watchdog startup task (logon trigger with a one-minute delay past
boot races, plus restarts on failure), and wires up whichever agent
integrations are present:

```powershell
pwsh -NoProfile -Command "irm https://github.com/neoslusion/termielle/releases/latest/download/install.ps1 | iex"
```

The installer is safe to re-run: it upgrades the binaries in place and
refreshes only the Termielle-owned configuration. Before its first config
change it saves the original once to `%LOCALAPPDATA%\Termielle\backups`;
every write is atomic and validated. Ownership flags carry across upgrades.

```powershell
pwsh -File "$env:LOCALAPPDATA\Termielle\bin\uninstall.ps1"
pwsh -File "$env:LOCALAPPDATA\Termielle\bin\uninstall.ps1" -RemoveData
```

Uninstall surgically removes Termielle-owned entries so newer user edits
survive, falling back to the first-run backup only if surgical cleanup fails.
It stops the overlay, restores the taskbar, removes only resources recorded as
Termielle-owned, and optionally deletes `~\.termielle`.

Manual install: unpack the `termielle-windows-x64.zip` release asset into
`%LOCALAPPDATA%\Termielle\bin`, add that directory to `PATH`, and run
`termielle-app.exe`. It reads `~\.termielle\config.json` (the same dot-directory
convention as `~\.claude`) and sits in the notification area.

## Integrations

Each agent translates its own lifecycle events into one of the protocol events
and invokes `termielle-emit.exe` with the session identifier and nothing else.
The wire contract is versioned and documented in [docs/protocol.md](docs/protocol.md);
new agents start from the scaffold in `integrations/_template/`.

The integrations are packaged the way each agent expects. Installing one means
placing a file in your agent's own configuration directory — the installer
wires them automatically when it finds the agents, or you can do it by hand:

| Event | Overlay face |
| --- | --- |
| `session_started` | idle |
| `prompt_submitted` | thinking -> working (after the local one-second transition) |
| `thinking_started` | thinking, held while the agent reasons |
| `thinking_ended` | working |
| `needs_input` | waiting |
| `turn_completed` | ready |
| `turn_failed` | failed |
| `session_ended` | gone |

The emitter is fail-open: with no overlay running it prints `{}` and exits 0,
so a hook never blocks an agent. It must be reachable on `PATH` (or via the
`TERMIELLE_EMIT` environment variable pointing at the absolute path). A custom
overlay endpoint is supported with `--pipe <name>` on both the app and the
emitter.

- **Claude Code** — the native plugin package is
  `integrations/claude/.claude-plugin/plugin.json` (name `termielle`, with all
  six hooks). Installing it the classic way means merging the `hooks` object
  from `integrations/claude/settings.fragment.json` into
  `~/.claude/settings.json`. Covers all six events, including
  `StopFailure` -> `turn_failed`.
- **Codex** — merge the `hooks` table from `integrations/codex/hooks.toml` into
  `~/.codex/config.toml`. The legacy `notify` fallback is only honored from the
  user-level config, so keep it there. Codex exposes no failure event, so
  `turn_failed` is not mapped.
- **opencode** — the native plugin is
  `integrations/opencode/termielle.plugin.ts`; copy it into
  `~/.config/opencode/plugins/` (all projects) and restart. The plugin maps
  prompts, real reasoning (`thinking_started`/`thinking_ended` from the
  assistant message stream), permission prompts, step failures, and status to
  emitter calls.
- **opencode2 (OpenCode 2)** — OpenCode 2 reads the same plugin directories,
  and its plugin is `integrations/opencode2/termielle.plugin.ts`; copy it next
  to the V1 file and restart. It emits under the distinct `opencode2` source
  word, so sessions from both runtimes stay apart when they run side by side.
- **Antigravity CLI (`agy`)** — merge the handler from
  `integrations/agy/hooks.fragment.json` into `~/.gemini/config/hooks.json`
  (the installer substitutes the absolute emitter path that agy requires).
  Each model invocation shows as Thinking and tool work as Working;
  `Stop` completes the turn. agy exposes no failure or approval-prompt event,
  so `needs_input` and `turn_failed` are not mapped. See
  `integrations/agy/README.md`.

Claude and Codex expose no reasoning boundary, so their `Thinking -> Working`
transition is the local one-second heuristic; opencode's thinking face is
driven by its actual reasoning step, and agy's by its model invocations.
Neither Claude nor Codex emits a universal
"resumed after approval" event, so an approval prompt may stay `NeedsInput`
until the next event. opencode publishes no session-end event, so a finished
conversation leaves via the Ready hold, an explicit `session.deleted`, or the
busy-stall idle decay (`busy_stall_ms`, default 5 minutes).

## Assets

The overlay artwork is the Gemielle asset set, redistributed under the Apache
License 2.0 (copy at `assets/LICENSE.Gemielle`). Every file is listed with
source, SHA256, and modifications in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
The shipped GIFs are re-encodings produced by `crates/termielle-asset` from
the unmodified originals in `assets/sources/`: composited with the GIF
specification's disposal semantics, cropped to the shared character box
(329x294, so the window never resizes between states), and re-encoded with
one global palette and full-canvas Background-disposal frames so motion never
traces. The `refine` subcommand regenerates them, and `verify` proves a
regenerated set renders pixel-identical to the sources through WIC, the
overlay's own decoder.

To install a custom set, copy GIFs into `~\.termielle\assets\`;
each visual state resolves to one file, and a procedural ring is drawn when an
asset is missing or undecodable.

## Rendering and tray

The default surface is a Waybar-style persistent system bar with a compact
Termielle module. Clicking it opens a focused popup for agent and media detail;
hovering opens it only when content is available. The default apps rail switches
open windows, and the speaker is a control. The default right side keeps
Network, Volume, Battery, Control Center, and the clock; CPU and Memory live
in Control Center unless pinned to the strip.

The overlay composites its layered window with per-pixel alpha via
`UpdateLayeredWindow` by default. `--render color-key` (or
`{"render": "color_key"}` in the config file) falls back to GDI color-keying
for display drivers whose layered DIB redirection renders black. The tray menu
switches layout, theme, position, widget state, and pinned menu-bar items; it marks the active choices
and offers `Restart` and `Exit`. `Exit` terminates with code 0 so the external
watchdog does not relaunch it.

Animation playback is paced by a dedicated clock thread (not `WM_TIMER`, whose
message-queue latency cannot hold a 16 ms cadence), with the measured present
cost subtracted from each frame interval. Set `{"frame_rate": 60}` in the
config file to play every animation at that frame rate instead of the GIF's
own delays — the loop then takes frame count / rate seconds (the shipped
60-frame assets loop in one second).

## Diagnostics

Two PowerShell 7 scripts exercise an install without any agent running:

- `scripts/doctor.ps1` — validates the host, locates the binaries, runs the
  overlay smoke test on the default pipe, then drives a unique-pipe smoke with
  the real emitter. One `PASS`/`FAIL` line per check; exits 0 only when every
  check passes.
- `scripts/benchmark.ps1` — measures against the performance budgets: warm
  emitter median/p95 (20/50 ms), event-to-frame (< 50 ms), warm first frame
  (< 250 ms), overlay working set while the largest asset animates (< 50 MiB),
  and long-run idle CPU (< 0.5% of one core). Exits nonzero when a budget
  fails; `-ReportOnly` prints without gating (CI).

Both scripts launch the overlay in an isolated instance (unique pipe, unique
mutex), so they never disturb a live companion.

## Persistence

The overlay journals every accepted event to `events.log` next to its
config and replays it at startup, so a tray `Restart` or a crash-watchdog
relaunch picks up exactly where the session was instead of starting over at
Idle. The journal is bounded (1 MiB, oldest lines dropped) and replay is
absolute-time driven: a restart after the ready-hold or busy-stall expired
comes back as Idle anyway. Diagnostics runs on custom pipes neither journal
nor replay.

## Workspace

The Rust workspace (edition 2024) is split into five crates:

- `crates/termielle-core` — event protocol, session reducer, configuration,
  and the replayed event journal.
- `crates/termielle-ipc` — the platform-abstracted transport: an owner-only
  named pipe on Windows, an owner-only Unix domain socket elsewhere. Both
  share one one-connection-per-event contract and one test suite.
- `crates/termielle-emit` — the fail-open hook emitter executable.
- `crates/termielle-asset` — the artwork toolkit: analyze, refine, verify,
  preview. Its LZW encoder is a faithful port of GifLib's, whose streams WIC,
  .NET, and the gif crate all read identically.
- `crates/termielle-app` — the GIF overlay, single-instance guard, tray, and
  diagnostics (`--smoke-test`, `--ack-file`). Its assets test validates the
  shipped artwork frame by frame through WIC, the overlay's own decoder.

`scripts/` holds the PowerShell 7 harnesses, and `.github/workflows/` runs
fmt/clippy/tests in CI on Windows, packages checksummed release builds, and
keeps core/ipc/emit green on Linux.

## Inspiration

Termielle was inspired by [Gemielle by Rainan1010](https://github.com/Rainan1010/Gemielle).
Its artwork is redistributed under the Apache License 2.0 with full provenance
in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md); no Gemielle source code is
included.

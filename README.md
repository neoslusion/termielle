# Termielle

Termielle is a low-overhead animated desktop companion for the terminal. A
transparent overlay character follows Claude Code, Codex CLI, and opencode
lifecycle events, received over a local named pipe. Version 1 targets Windows,
including hooks launched from WSL through a bridge executable. The events are
small and content-free: no prompts or terminal output are ever captured.

## Install

One-shot installer — downloads the latest release, verifies the checksum,
installs to `%LOCALAPPDATA%\Termielle`, registers the crash-watchdog startup
task, and wires up whichever agent integrations are present:

```powershell
irm https://github.com/neoslusion/termielle/releases/latest/download/install.ps1 | iex
```

Manual install: unpack the `termielle-windows-x64.zip` release asset into
`%LOCALAPPDATA%\Termielle\bin`, add that directory to `PATH`, and run
`termielle-app.exe`. It reads `%LOCALAPPDATA%\Termielle\config.json` and sits
in the notification area. `scripts/doctor.ps1` validates an install end to end.

## Integrations

Each agent translates its own lifecycle events into one of six protocol events
and invokes `termielle-emit.exe` with the session identifier and nothing else:

| Event | Overlay face |
| --- | --- |
| `session_started` | idle |
| `prompt_submitted` | thinking -> working |
| `needs_input` | waiting |
| `turn_completed` | ready |
| `turn_failed` | failed |
| `session_ended` | gone |

The emitter is fail-open: with no overlay running it prints `{}` and exits 0,
so a hook never blocks an agent. It must be reachable on `PATH` (or via the
`TERMIELLE_EMIT` environment variable pointing at the absolute path). A custom
overlay pipe is supported with `--pipe \\.\pipe\<name>` on both the app and the
emitter.

- **Claude Code** — merge the `hooks` object from
  `integrations/claude/settings.fragment.json` into `~/.claude/settings.json`.
  Covers all six events, including `StopFailure` -> `turn_failed`.
- **Codex** — merge the `hooks` table from `integrations/codex/hooks.toml` into
  `~/.codex/config.toml`. The legacy `notify` fallback is only honored from the
  user-level config, so keep it there. Codex exposes no failure event, so
  `turn_failed` is not mapped.
- **opencode** — copy `integrations/opencode/termielle.plugin.ts` into
  `~/.config/opencode/plugins/` (all projects) or `.opencode/plugins/` (one
  project) and restart; plugins load at startup, so no hooks config is needed.
  The plugin maps prompts, permission prompts, step failures, and status to
  emitter calls.

Neither Claude nor Codex emits a universal "resumed after approval" event, so
the one-second `Thinking -> Working` transition happens locally and an approval
prompt may stay `NeedsInput` until the next event. opencode publishes no
session-end event, so a finished conversation leaves via the Ready hold, an
explicit `session.deleted`, or the busy-stall idle decay (`busy_stall_ms`,
default 5 minutes).

## Assets

The overlay artwork is the Gemielle asset set, redistributed under the Apache
License 2.0 (copy at `assets/LICENSE.Gemielle`). Every file is listed with
source, SHA256, and modifications in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
To install it, copy the GIFs into `%LOCALAPPDATA%\Termielle\assets\`; each
visual state resolves to one file, and a procedural ring is drawn when an asset
is missing or undecodable.

## Rendering and tray

The overlay composites its layered window with per-pixel alpha via
`UpdateLayeredWindow` by default. `--render color-key` (or
`{"render": "color_key"}` in the config file) falls back to GDI color-keying
for display drivers whose layered DIB redirection renders black. The tray icon
shows the current face and offers `Restart` and `Exit`; `Exit` terminates with
code 0 so the external watchdog does not relaunch it.

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

## Workspace

The Rust workspace (edition 2024) is split into four crates:

- `crates/termielle-core` — event protocol, session reducer, configuration.
- `crates/termielle-ipc` — user-scoped named-pipe transport.
- `crates/termielle-emit` — the fail-open hook emitter executable.
- `crates/termielle-app` — the GIF overlay, single-instance guard, tray, and
  diagnostics (`--smoke-test`, `--ack-file`).

`scripts/` holds the PowerShell 7 harnesses, and `.github/workflows/` runs
fmt/clippy/tests in CI and packages checksummed release builds.

## Inspiration

Termielle was inspired by [Gemielle by Rainan1010](https://github.com/Rainan1010/Gemielle).
Its artwork is redistributed under the Apache License 2.0 with full provenance
in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md); no Gemielle source code is
included.

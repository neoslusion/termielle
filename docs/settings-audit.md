# Settings audit — 2026-10-16

Scope: every control in Termielle Preferences, its draft/commit path, and the
corresponding renderer/runtime behavior. Tests use hidden app-owned windows,
temporary profiles, and scoped native messages. They do not inject global input,
toggle radios, open Windows Settings, or turn off the installed app.

## Confirmed defects and fixes

- **Theme:** the picker changed the theme name but retained the old material.
  Selecting a preset now resolves its material, including when selecting another
  theme and returning to the original. An untouched Apply preserves custom glass.
- **Auto:** previously resolved colors could be mistaken for explicit overrides
  on later resolution. Picker-selected themes now retain ownership of their
  material instead of freezing the previous resolved colors.
- **Glass / recording:** Frosted on Transparent could revert to unblurred at
  resolution/restart because 12px was indistinguishable from an omitted default.
  An explicit backdrop mode now survives serialization and subsequent theme
  changes. Recording-safe remains unblurred until deliberately changed.
- **Animation:** returning to Custom after previewing a preset kept that
  preset's physics. Custom now starts from the saved baseline, not the last
  preview's mutated draft.
- **Bar items:** the UI promised ordering but painters used fixed order. Both
  zones now follow their lists left-to-right. Apps no longer suppresses the
  optional Workspaces module; earlier items take available space first.
- **Character:** enabling animation after a still-only load did not reopen its
  decoder until a state change. It now reloads immediately when the setting or
  Face-widget presence changes.
- **Availability:** controls for other layouts, fullscreen hiding in Bar/Classic,
  and character animation without a Face widget are disabled rather than looking
  effective. Pin movement/removal is disabled without a valid selection and at
  list boundaries.
- **Apply:** the resolved material is saved, so the acknowledged runtime settings
  and persisted profile agree. Save errors keep the committed profile unchanged;
  an existing temporary preview remains reversible.

## Coverage

| Controls | Automated checks |
| --- | --- |
| Layout, position, height | All four layout choices, both edges, all stock heights; existing rendering/geometry regressions |
| Theme | Every built-in picker entry, stale tint removal, reselection, no-op preservation, derived Auto refresh |
| Glass / recording | Frosted and recording-safe on Transparent/macOS, resolution and JSON restart, sticky explicit recording-safe |
| Hover, character, name | Both checkbox values independently; existing hover/branding render tests; live GIF decoder restart |
| Left/right items | Text parsing, order, invalid/duplicate/missing required modules, rendered order across three palettes, Apps plus Workspaces |
| Pins | Move up/down, remove, bounds, selection readiness, draft isolation and Revert; existing persistence tests |
| Display | Automatic/Primary/Pointer draft choices; existing unavailable named-display preservation and monitor-selection tests |
| Fullscreen | Checkbox round-trip and layout availability; existing visibility-policy tests |
| Animation feel | Every preset and restoration of custom physics |
| Preview / Apply / Revert / Close | Real queued Preview through the shared GUI pump, temporary profile unchanged until Apply, restart equivalence, Close rollback, save failure, stale-draft rejection |
| System buttons | Sound, Network, Power, Clock and Task View emit the correct shell requests without executing them during tests |
| Turn Off | Request routing plus existing isolated power preference/quit tests, never the installed profile |

Physical multi-monitor placement, real exclusive fullscreen, Windows light/dark
and accent transitions, perceptual animation
feel, screen-reader operation, recording behavior, and opening shell-owned
Settings surfaces still require manual acceptance. Automated coverage does not
prove those physical interactions on every machine. Character animation also
respects reduced motion; the idle Bar deliberately settles its face rather than
running a permanent loop.

## Material persistence

`island.glass.use_theme_material: true` is written by the theme picker. It tells
resolution to refresh preset colors instead of treating resolved values as custom
overrides. Existing hand-edited profiles default to `false` and retain the previous
explicit-field behavior. Set it to `false` when deliberately overriding a preset's
glass fields by hand.

`island.glass.backdrop` is `theme`, `frosted`, or `recording_safe`. Explicit Frosted
and recording-safe choices persist independently of theme selection. The default
`theme` retains existing preset/radius resolution. Preset JSON search roots and
palette selection are unchanged.

## Validation

```powershell
cargo test -p termielle-app --test settings_audit
cargo test --workspace -j 2
cargo fmt --all -- --check
cargo build -p termielle-app --release --bin termielle-app -j 2
```

Before fixing, the new theme, backdrop-restart, custom-animation, and combined
Apps/Workspaces regressions failed. Detailed local logs are under
`target/settings-*.log`; these generated artifacts are not committed.

Final result: **559 passed, 0 failed, 2 existing manual launcher tests ignored**.
Formatting and the native release build pass. The 11 new native audit tests run
alongside controller, theme, persistence, display, power, and launcher regressions.

The native EXE was backed up and redeployed through the existing Termielle task;
no DLLs, system themes, security agents, or task definitions were changed. Profile
bytes and the Windows taskbar preference were preserved.

A live, app-owned-window check previewed Light from the installed macOS Dark bar
and reverted it. Bright-pixel samples in the center bar region changed **0 → 526
→ 0**, with the same installed process, unchanged profile hash, and no global
input injection. This verifies rendered material, not just the combo-box value.
It does not replace the manual acceptance limits above.

Installed SHA256:
`28E03CBCDC1B9A1C18A97DBBB7DC68AF8ADB5EA78F0A6BA913C863A7952574C4`.
Local deployment/probe evidence: `target/settings-deployment.json` and
`target/settings-live-probe.json`. Rollback copies use suffixes
`termielle-app.exe.bak-20261016-160135` and
`config.json.bak-settings-20261016-160135` beside their installed originals.

# Preferences and navigation polish — first follow-up increment

The command runner recovered after the Git/MSYS launch failure. This increment
resumes the approved [desktop UX roadmap](desktop-ux-roadmap.md); it does not
claim that all four roadmap areas are delivered.

## Implemented

### Navigation

- Standard Windows app-name/window-count tooltips with a 600 ms hover delay.
  Tooltip tools own their UTF-16 buffers, update only when labels/geometry
  change, and disappear for presses, drags and navigation popups.
- Existing explicit pins can be dragged to reorder. A six-logical-pixel motion
  threshold avoids turning ordinary clicks into drags. Insertion feedback and
  pressed-state feedback follow identity, not mutable group indexes.
- Drops save once through the existing atomic pin path. Escape, lost capture,
  an outside drop, source removal or stale pin order cancels; a cancelled drag
  cannot accidentally launch an app on release. Save failure restores pins.
- Keyboard Tab navigation reaches normal window Close and popup Dismiss as well
  as row actions. Invisible clipped controls and background shields remain
  excluded from keyboard actions.
- Per-app pending-launch feedback remains after successful shell dispatch until
  a new window appears or the ten-second deadline expires. A second click on
  that pending app is not another launch. Failures clear pending state.

### Native preferences

Bar settings opens one nonmodal preferences window with standard Windows
labels, combo boxes, check boxes, edit fields, lists and buttons. Native dialog
navigation handles Tab, access keys and Escape. Pages include:

1. Appearance: layout, top/bottom edge, height/density, theme, hover, animation,
   and frosted/recording-safe glass.
2. Bar items: left/right module order via validated comma-separated lists.
3. Pins: keyboard-accessible Move up, Move down and Remove in a draft list.
4. System/readiness: native Windows sound/output, network, power, clock/calendar
   and Task View routes, plus explicit taskbar-replacement limitations.

**Preview** changes only runtime rendering/sampling/reservation. **Apply**
validates, checks that the committed profile has not changed externally, and
saves atomically to the active profile. **Revert**, Close and Escape restore the
current committed profile. A failed save leaves the old profile intact. Existing
custom material overrides and unedited values are preserved. Bar pin writes are
blocked while preview is active so preview-only pin order cannot leak into disk.
Restart now forwards the original command-line arguments, including --config.

Recording-safe glass uses normal per-pixel translucency without desktop copying;
it therefore does not need temporary capture exclusion. Choosing it is explicit,
not a change made to the installed profile during development. Frosted glass
retains the scoped-exclusion tradeoff documented in [background repair](notch-background.md).

Preferences cannot enable native-taskbar replacement from a nonreplacement
baseline in this review build. Apps, Control Center and Clock remain required
entry points. The tray menu remains a fallback if preferences creation fails.

## Validation

- 489 workspace tests passed, 0 failed, 2 existing manual launcher tests ignored.
- Release app/review builds, formatting/diff checks, Clippy with pre-existing
  allowances and all 7 doctor checks passed.
- Installed executable SHA256 remained
  `0EA521210C77FF33099A4BBAD8C28395EC4A7B95B7083147384BAF49A6F101E3`;
  config SHA256 remained
  `783AC6914D528B38E964581A74C0D12406922C9EB07A671E2BB548B007043FC6`,
  with one existing installed-path process running.
- Native hidden-window tests exercise request separation, native controls,
  selected values, Escape/Close reversion and footer fit without showing windows,
  changing taskbar state or writing profiles.
- Model/controller tests cover scaled top/bottom drag order, Escape/capture
  cancellation, outside/stale drops, tooltips, keyboard Close and launch
  deduplication/deadlines.
- `preferences_review` paints an isolated hidden native window into BMP via
  WM_PRINT. It does not display/focus windows or capture desktop/terminal content.
  Hidden combo-box selected text may be omitted from WM_PRINT; native text/value
  assertions verify selection independently. This is not live UI acceptance.

```powershell
cargo run -p termielle-app --example preferences_review -- target/preferences-appearance.bmp 0
# Pages: 0 Appearance, 1 Bar items, 2 Pins, 3 System/readiness
```

No installed executable or real user profile was replaced for this increment.
Before deployment, manually exercise mouse/keyboard, Preview/Apply/Revert,
save failure and custom profiles, top/bottom and DPI, native tooltip visibility,
drag threshold/capture loss and foreground restoration.

## Still pending — do not call complete

Dedicated volume/network/battery/calendar popovers, in-app output-device
selection and a distinct notification action remain pending; the System page's
native routes are not substitutes claimed as those completed features.
Custom-bar UI Automation, deliberate monitor selection/policy, fullscreen and
Explorer-restart review, native tray access during replacement, physical lid/
multi-monitor/recording acceptance also remain pending. Native preferences do
not make the custom-drawn bar fully screen-reader accessible.

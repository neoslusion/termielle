# Lid / display-resume recovery and local deployment

Follow-up on **2026-10-02** to the multi-session activity milestone. The updated
activity build plus display-recovery fix is now installed locally; this is not
a published release or a claim of physical lid-cycle acceptance.

## Report and evidence

Reported: the bar disappears after closing and reopening the laptop lid.
Inspection found **no installed app process**, the `Termielle` scheduled task in
Ready state, and last task result **0xC0000409**. Battery-start/stop restrictions
were already disabled; they were not changed.

The installed `~/.termielle/panic.log` recorded:

```text
panicked at crates\termielle-app\src\window.rs:198:20:
min > max. min = 0, max = -640
panic in a function that cannot unwind
```

Windows Application events identified the installed executable and the same
0xC0000409 termination. Kernel-Power events confirmed Modern Standby entry
because of **Lid** at 17:01:54 and exit because of **Lid** at 21:22:26. The latest
panic was at approximately 17:01:58 local time. The configured crash policy has
three one-minute retries; a failed task is not an always-running service.

`WM_DISPLAYCHANGE` and `WM_DPICHANGED` previously clamped every overlay inside
the current monitor's work area in the native callback. A 2560-pixel bar against
a 1920-pixel work area computed an upper X bound of -640, then called
`clamp(0, -640)`. That panicked inside the non-unwinding Win32 callback, aborting
the process before the GUI-loop panic boundary could help. The focused regression
reproduced that **exact** error before the fix.

## Changes

- Clamp bounds stay valid for oversized surfaces, negative monitor origins,
  zero/inverted transient work areas, and negative sizes. An oversized surface
  keeps its origin visible until the GUI owner resizes it.
- Island/Notch/Bar callbacks no longer reposition anchored surfaces as free
  floating pets. They enqueue display invalidation; the GUI owner resolves the
  monitor, DPI, width, and placement.
- Resume broadcasts and session-local display-on notifications request recovery.
  The latter covers Modern Standby/display wake; registration is production-only
  and failure is non-fatal. The power notification handle is released on teardown.
- Recovery discards the old layered DIB and blurred backdrop, re-picks the primary
  or active monitor under the existing configuration, refreshes scale, advances
  overdue state deadlines, restores non-activating visibility, and re-presents.
- A native one-shot timer schedules bounded retries after **250 ms, then 1 s,
  then 3 s**. It is independent of the DXGI worker, so recovery can repaint even
  while a vertical-blank wait is stalled or no output is available. The output
  cache is explicitly invalidated for the worker to re-resolve when it can run.
- AppBar registration is refreshed even when dimensions did not change. Shell
  work-area changes are distinguished from topology/resume events so our own
  reservation broadcasts do not continually restart recovery.

No power plan, lid behavior, wake-to-run policy, agent integration, layout,
configuration, native taskbar preference, asset, or DLL was changed.

## Validation

- Before the fix, three focused math regressions failed, including
  `a_2560px_bar_survives_a_temporary_1920px_monitor` with the installed panic's
  `min = 0, max = -640` values. They pass after the fix.
- **440 passing workspace tests**, two existing manual launcher tests ignored.
  Native regressions exercise posted resume messages through the real GUI pump,
  the recovery timer without an animation clock, deferred anchored DPI placement,
  separate AppBar work-area events, and oversized Classic display callbacks.
- Formatting check and release build pass. Clippy passes with the previously
  documented unrelated lint categories allowed; strict Clippy still encounters
  the existing repository warnings.
- Release doctor: **all seven checks passed** before installation.

Raw local logs are ignored under `target/display-recovery-before.log`,
`target/display-recovery-after.log`, `target/lid-resume-workspace-tests.log`,
`target/lid-resume-release-build.log`, and `target/lid-resume-doctor.log`.

## Deployment and runtime check

Installed executable:
`%LOCALAPPDATA%\Termielle\bin\termielle-app.exe`.

The existing scheduled task was stopped, only installed-path processes were
stopped, the original executable and task XML were backed up, a checksum-verified
staged binary replaced the app, and the existing task restarted it. No task
settings or other installed files were replaced.

- Executable backup: `termielle-app.exe.bak-20261002-222310` beside the app.
- Task snapshot: `Termielle-task-20261002-222310.xml` beside the app.
- Installed/release SHA256:
  `01F8ED122B57C76EC14C648573B04869039A1A9C6C602FF6BF0A7CF118AC9654`.
- Config SHA256 before/after:
  `783AC6914D528B38E964581A74C0D12406922C9EB07A671E2BB548B007043FC6`.
- One installed process (PID 35792 at validation), scheduled task Running.
- Targeted **synthetic** resume/topology messages to that HWND left the same
  process alive, with a visible **1920×45 physical-pixel** bar at (0, 0), native
  taskbar visible, no added panic bytes, and unchanged config. The observer was
  DPI-aware; logical height is 36 pixels at 125% scaling.
- A bar-strip-only capture was visually checked. It does not capture the terminal
  body or rest of the desktop.

Deployment/runtime artifacts are in ignored `target/termielle-deployment.json`,
`target/termielle-resume-verification.json`, and
`target/installed-bar-after-resume.png`. Local deployment/probe scripts also stay
in ignored `target/`; they are not release installer replacements.

**Remaining acceptance:** a human should close and reopen the physical laptop
lid and confirm that the bar returns. Synthetic messages do not exercise the
actual GPU/panel sleep transition. Live session-to-terminal focusing and other
manual activity interactions remain separate checks as described in
[the activity guide](session-activity.md).

Rollback: stop the existing task/installed process, restore the executable
backup, and restart the same task. Keep config unchanged. The backup contains
the prior display-clamp bug, so rollback is a recovery option, not the preferred
lid-resume solution.

# Whole-application On / Off

The experimental [Windhawk edition](../windhawk/README.md) shares the same Off
marker. New `--enable-only` explicitly clears Off without starting a frontend,
for switching to a fresh Windhawk tool host. `--enable` retains its original
standalone launch behavior. Mod activation never clears Off implicitly.

Termielle now has a persistent, per-user power switch. This is distinct from
hiding the surface, changing layout, disabling a widget, or exiting temporarily.

Executable controls were deployed in the standalone build on **2026-10-06**.
The binary-only deployment did not rerun the installer or add new Start-menu
shortcuts; external On remains available through the installed executable's
`--enable` command. Windhawk activation remains separate and opt-in.

## Controls

- Tray menu: **Turn Off Termielle…**, with confirmation (default answer No).
- Bar settings → System/readiness: **Turn Off Termielle…**, same confirmation.
- New-release installer: Start → **Turn Termielle On** / **Turn Termielle Off**.
  Shortcuts are only created for releases carrying `integrations/termielle-power-v1`;
  older release binaries do not understand the new command-line switches.
- Command line, using the new executable:

  ```powershell
  # Fully exit and stay off across subsequent launches/logins:
  Start-Process "$env:LOCALAPPDATA\Termielle\bin\termielle-app.exe" -ArgumentList '--disable' -Wait

  # Clear the off preference and launch with saved settings:
  Start-Process "$env:LOCALAPPDATA\Termielle\bin\termielle-app.exe" -ArgumentList '--enable'

  # A custom profile is supported; power is global for this user, not per-profile:
  Start-Process '.\target\release\termielle-app.exe' -ArgumentList '--enable', '--config', 'custom.json'
  ```

The ordinary **Exit (until next launch)** action still exits without changing the
persistent on/off preference. Restart does not turn a deliberately disabled app
back on. No on/off UI remains resident while Off; use a shortcut or command to
turn it back on.

## Behavior and resource use

The empty `%USERPROFILE%\.termielle\disabled` marker means Off. Missing marker
means On, preserving the behavior of existing installations. The switch never
rewrites `config.json`, including custom glass overrides and explicit profiles.
Settings, pins, assets, integrations and scheduled-task settings are retained.
Unapplied runtime previews are discarded on exit.

Off signals cooperating instances in the current Windows session through a
per-user-directory named Windows event. The persistent preference also applies
to subsequent launches in other sessions; already-running instances in a separate
Windows logon session are not remotely stopped. Each GUI owner takes normal Quit teardown, removing its AppBar reservation
and restoring the Windows taskbar if replacement mode owned it. There is no
Explorer injection, force termination, guessed HWND association, new agent-event
protocol or content capture. Switching off is asynchronous while normal GUI
teardown completes; a stuck instance is not force-killed.

While On, a kernel-event waiter blocks without polling and exits with the app.
While Off, no Termielle GUI/worker/controller stays running. The logon task itself
is deliberately not modified: if it starts the new binary, that binary exits 0
before window, renderer, event pipe, media/task polling, config loading or journal
replay. Hooks remain installed and can still spawn short-lived fail-open emitter
processes; Off does not uninstall integrations or eliminate those hook launches.

Taskbar-recovery helper mode is allowed to run even while Off so a previous
abnormal shutdown can still give Windows its taskbar back. Smoke diagnostics
remain isolated from the user's off preference and cannot change it.

## Validation and deployment boundary

Unit tests cover persistent marker transitions, repeated calls, profile-byte
preservation, failed changes, multiple cooperating instances, directory isolation,
late registration and rearming after On. Hidden native tests cover clean disabled
startup without a pipe/journal/profile write, and Quit routing while a chooser
owns focus. CLI parsing tests cover custom profiles and contradictory switches.

Final validation: **498 workspace tests passed, 0 failed, 2 existing manual
launcher tests ignored**. Release build, Clippy with the established allowances,
format/diff checks and all **7 doctor checks** passed. An isolated temporary
Start-menu test verified capability gating, shortcut targets/arguments and
preservation of unrelated shortcuts during install/uninstall. Passive checks
confirmed the installed binary and config hashes unchanged, no real disabled
marker, and one existing installed process still running.

The On retry path also repairs the existing single-instance mutex's missing NUL
terminator and collision-handle cleanup; a repeated-claim regression verifies
that collision attempts cannot keep the previous claim alive after shutdown.

The installed October 4 binary does **not** yet support these controls. This is
implemented in the working tree/new build, not automatically deployed. Actual
tray/preferences confirmation and Start-menu interaction still need manual review
on the new installation. No real-user on/off changes are made by automated tests.

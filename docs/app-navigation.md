# Bar app navigation

This working-tree milestone makes application switching the left zone's main
job. It does **not** enable native-taskbar replacement or change the installed
user profile. This build was installed at the user's request on 2026-10-04. The activity pill and Control Center remain independent.

## Everyday flow

- **Apps** opens the existing local launcher (also available with Alt+Space).
- Explicitly pinned apps stay first, in saved order. Running apps keep their
  relative positions when foreground state or window titles change.
- A running dot, active underline, and multiple-window count distinguish states.
- Click an app with one window to focus it, restoring it if minimized.
- Click an app with several windows to open a title-based window chooser.
- Click a closed pin to launch it. No saved command lines are executed.
- **More** reaches entries that do not fit the rail, including iconless apps;
  four-row pages keep the popup bounded. Selecting a group opens its chooser.
- Right-click an app for **Pin/Unpin**, **New window**, normal **Close** actions,
  and **Bar settings**. A fresh window inventory validates each action.
- The rail settings button, overflow footer, and app menu open the existing
  Termielle preferences menu in the installed 2026-10-04 build. The newer
  [preferences/navigation follow-up](preferences-navigation-polish.md), not yet
  installed, replaces this with a native Preview/Apply/Revert preferences window.

The popup supports arrows, Home/End, Enter, Tab/Shift-Tab, Page Up/Down and
Escape. Clicking outside dismisses it. It receives focus only after an explicit
user opening, restores the previous foreground window only while it still owns
focus, and does not take focus just to show a failure notice. Windows can deny
activation or elevated-window commands; errors are shown rather than silently
claiming success. Close is a normal application close request: save prompts
remain the application's responsibility, and processes are never killed.

Painted actions carry app/window identities, not transient list indexes.
Background shielding and clipped hit targets prevent a mid-morph click from
activating an invisible row or a card underneath. The chooser is not an agent
card and does not take over Control Center state.

## Identity and saved pins

Groups use OS-reported executable paths or registered AppUserModelIds, not
window-title guesses. Windows without an eligible identity stay separate and
cannot be pinned. Terminal switching remains **window-level**, not tab/pane
selection. Host executables and command-line launch targets are deliberately
excluded from pin launch targets.

`island.bar.pinned_apps` defaults to an empty list. Up to 32 valid, deduplicated
pins survive restart. For example (replace the placeholder with a real path):

```json
"pinned_apps": [
  {
    "name": "Editor",
    "target": { "kind": "executable", "value": "C:\\Apps\\Editor.exe" }
  }
]
```

Pin changes use the existing atomic config writer; a save failure rolls the
change back and shows feedback. Preferences and pins honor `--config` instead
of writing to the default profile during an isolated/custom-profile run.
No lifecycle protocol or hook payload changes, automatic approvals, prompt
capture, or persisted session/window links are introduced.

Window metadata is collected off the GUI thread, separately from expensive
artwork. The defensive catalog limit is 4096 eligible windows, not the six-icon
worker rail limit; navigation's additional icon cache is bounded to 96.
Fallback initials and window titles keep iconless groups reachable.

## Validation and acceptance

Combined navigation/background validation: **476 workspace tests passed,
0 failed, 2 existing manual launcher tests ignored**. Release app/review builds
and Clippy passed with the previously documented unrelated lint allowances.
Isolated native tests cover key routing, focus-style restoration, owner-checked
Close, launch failure, capture policy restoration, and stale background epochs.
They do not prove human interaction with arbitrary installed applications.

Synthetic previews (no real apps launched or profile writes):

```powershell
cargo run -p termielle-app --example app_navigation_review -- target/app-navigation-chooser.bmp --chooser
cargo run -p termielle-app --example app_navigation_review -- target/app-navigation-overflow.bmp --overflow --narrow --bottom
cargo run -p termielle-app --example app_navigation_review -- target/app-navigation-resting.bmp
cargo run -p termielle-app --example app_navigation_review -- target/app-navigation-light.bmp --chooser --light
```

## Local installation — 2026-10-04

Authorized executable-only deployment completed at 12:22:44 +07:00 through the
existing Termielle scheduled task. Release/installed SHA256:
`0EA521210C77FF33099A4BBAD8C28395EC4A7B95B7083147384BAF49A6F101E3`.
The previous executable was backed up beside it as
`termielle-app.exe.bak-20261004-122222`; task XML was exported alongside it.
No hooks, assets, profile values or scheduled-task settings were replaced.

A passive installed-window check confirmed one installed-path process, task
Running, a visible 1920x45 physical-pixel top bar, native taskbar visible,
unchanged config checksum and no new panic bytes during observation. A bar-only
capture was reviewed. No synthetic system messages or global input were sent.
Live source self-capture checks do not prove every installed interaction or
physical lid-cycle acceptance.

Before replacement acceptance, manually check real pin/unpin persistence, launch,
single/multiple/minimized-window switching, right-click menus, keyboard focus,
Escape/outside dismissal, top/bottom bars, mixed DPI and long/iconless entries.
The native taskbar must stay available throughout this review.

This is not complete shell parity: native tray icons, jump lists, drag-to-pin,
live window previews, multi-monitor secondary taskbars and full UI Automation
accessibility remain future work. Quick Settings is not tray overflow.

Related: [bar guide](island.md), [notch background](notch-background.md),
[development handoff](development-handoff.md).

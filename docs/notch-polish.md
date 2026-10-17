# Notch polish — media, notifications, and Today

This pass makes the existing notch more useful and readable, rather than adding
another launcher, a plugin host, or an always-animated idle dashboard. It works
with the existing palettes, including macOS Dark, and preserves the user's profile.

## Media

- The compact bar pill shows album artwork, the track title, and a small playback
  indicator. The indicator conveys playing/paused state, not measured audio levels.
- The expanded media card uses an artwork-led layout with a restrained cover-derived
  accent, a two-line title, artist, source app, and larger playback targets.
- Artwork decodes to 160 × 160 pixels for the 80-logical-pixel cover at up to 2×
  rendering scale. Missing artwork gets a vector music glyph, not a broken image.
- Play/pause has a 48 × 48 logical-pixel target; previous/next use 42 × 42 targets.
  Very narrow cards retain play/pause without squeezing the skip buttons together.
- Transport glyphs fill device-pixel shapes with antialiased edges. The old
  renderer scaled individual pixel positions without filling the gaps between
  them, breaking pause bars and skip/play icons at fractional and higher DPI.
  Solid-bar, continuous-row, and premultiplied-edge regressions cover the repair.
- Playback position and duration come from the active Windows SMTC session. Invalid
  or unavailable timelines show an honest unavailable label. The line is read-only;
  there is no fake scrubber or seeking support.
- Position is the application's published snapshot, polled by the existing media
  worker. Some applications publish it infrequently, so the displayed position may
  not advance every second. There is no wall-clock extrapolation.
- Timeline-only changes repaint an expanded card without restarting its content
  crossfade or resizing it. They do not repaint a collapsed pill by themselves.
- Paused playback and reduced motion hold the indicator still. No new timer or
  background service is introduced.
- Playback buttons now send direct SMTC requests to the displayed source app on
  the existing background service, not synthetic global multimedia keys. An
  explicit play or pause request is selected from the session's supported
  controls; toggle-only sources use their supported toggle command.
- A missing/ambiguous source, unsupported skip, rejected command, or Windows
  failure shows an error banner instead of silently doing nothing. Commands do
  not fall through to a different player. Completed commands request an immediate
  metadata refresh so the icon follows the player's reported state.

## Notifications

- A 148-logical-pixel banner separates the source, headline, and preview. Both the
  headline and body allow two measured lines, with bounded Unicode-safe wrapping
  and ellipsis rather than text spilling into controls.
- An explicit notification source stays separate from the body, including when
  the message itself contains a colon.
- A source initial identifies system banners; agent banners retain the character
  when the face widget is enabled. No additional app-icon extraction is needed.
- The close affordance and the banner body dismiss the current banner. In bar
  mode, **View history** opens the right-side notification history instead.
- The queue count and lifetime indicator remain visible. Existing deduplication,
  critical-agent priority, and queued-banner lifetimes are unchanged.
- History rows allow two title lines and keep their source, received time, body,
  close button, and full-width **Clear all notifications** action.
- The [notification drawer repair](notification-drawer.md) keeps that action
  fixed while arrivals change the list, with three visible rows and scrolling
  for older retained entries, range hints, and panel-local hover feedback.
- Clear All still clears Termielle's recent history and unread count only. It does
  not clear Windows Notification Center, live agent activity, or queued banners.

## Today and interaction

- Empty-pill hover stays collapsed. Clicking the bar pill opens a 224-pixel Today
  card with the date, a large app/document-search shortcut, and the latest recent
  notification. Standalone Island retains its existing system dashboard.
- Media and Today have an explicit close button. Closing them does not close
  Control Center, change its hover state, or change its settings.
- Visible notch controls follow the painted geometry during a morph instead of
  waiting until the spring reaches 98% completion. Hits are clipped to the visible
  card and suppressed while its content is mostly transparent.
- An alert replaces the resting pill; Control Center remains an independent
  surface. Notification history and Control Center still share the right popover.

## Validation

Verified on 2026-10-16: `cargo test --workspace -j 2` reports **582 passed,
0 failed, 2 ignored** (the existing manual launcher checks). Formatting,
`git diff --check`, and the native release build pass. The release executable is
redeployed through the existing Termielle scheduled task, with matching source
and installed hashes, one running instance, an unchanged profile, and Windows
taskbar replacement still disabled. No DLLs or system settings are deployed.
The transport repair is regression-tested at 100–300% scale, including fractional
steps, and playing/paused fixture renders are reviewed at 125%, 150%, 175%, and 200%.

Automated coverage includes timeline conversion/clamping, invalid durations,
artwork accents, paused indicators, text wrapping at multiple scales, media
target bounds, timeline-only repaint behavior, Control Center independence,
notification action priority, and visible controls during top/bottom-bar morphs.
Existing integration tests also cover alert expiry, queue priority, no duplicate
pill, outside-click dismissal, and Clear All during resizing.

The isolated `render_review` example supports `--macos`, `--paused`, `--no-art`, and `--scale=1.5`
for repeatable media fixtures, alongside its existing bar, notification, light,
and history modes. It does not read private documents or generate desktop input.
Dark/light media, missing artwork, paused media, compact media, standalone media,
Today, banners, and history are visually reviewed with these fixtures.
`cargo run -p termielle-app --example media_control_review` reads the current
source and playback state without changing it. Adding `-- --exercise` sends
play/pause and restores the original state, provided the same clip remains
available; use that option only when a brief playback change is acceptable.

Real application playback, live toast delivery, pointer feel, and physical-monitor
DPI transitions still need hands-on acceptance. Fixture renders do not prove those.

## Suggested next additions

1. **Temporary file shelf:** drop a file on the notch, then drag it into another
   application; explicit remove/clear, no automatic upload or permanent indexing.
2. **Focus timer:** a compact remaining-time glance, with start/pause/reset inside
   the expanded card. Keep idle quiet and use the existing reduced-motion policy.
3. **Activity chooser:** when agents and music are both active, offer a small
   deliberate switcher rather than replacing whichever activity the user opened.

These are suggestions, not features implemented in this pass. Start with the file
shelf as one end-to-end capability rather than building a general extension SDK.

## Design references

- [Apple: Design dynamic Live Activities](https://developer.apple.com/videos/play/wwdc2023/10194/)
  informs glanceable identity, artwork, restrained color, and essential controls.
- [Apple: Notifications](https://developer.apple.com/design/human-interface-guidelines/notifications)
  informs the source/headline/body hierarchy.
- [SketchyBar item properties](https://felixkratz.github.io/SketchyBar/config/items)
  provides a ricing reference for artwork, padding, and shaped item backgrounds;
  this is a native Termielle renderer, not a SketchyBar port.
- [Microsoft: SMTC timeline properties](https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessiontimelineproperties)
  documents the application-reported timestamps used by the progress display.
- [Microsoft: SMTC playback requests](https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssession.trytoggleplaypauseasync)
  and [playback capabilities](https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionplaybackcontrols)
  document the direct transport commands and their success/capability checks.

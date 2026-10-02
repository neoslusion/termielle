# Reported behaviours — desktop bar, notch, and launcher

A log of the problems reported from the desktop, in the order they were raised,
what turned out to be causing each one, and what state each is in now.

This file is a record, not a plan. It is written from the outside in: what was
seen, what it was expected to do, and what the code turned out to be doing.

## Current status — 2026-10-02

Reports 1–4 preserve the original investigation and its verification state.
The implementation subsequently moved beyond report 3's one-card geometry:
**Bar layout now renders the notch and a right popover concurrently**, with
independent state. The earlier mutual-exclusion rule is historical, not the
current contract. Control Center and recent notifications share only the
right-popover slot; neither claims the notch.

Reports 5–10 record later desktop refinements and the launcher integration fix.
See the [development handoff](development-handoff.md) for the complete feature
inventory, configuration, source map, and build/redeployment checklist.

## Standing constraints

Two rules were given alongside these reports and still apply:

- **Do not modify or ship any DLL.** Everything so far has been Rust source and
  its tests. `git status` has been checked for binary artifacts on each commit.
- **Do not drive the app with synthetic input.** Repeated `mouse_event` probing
  destabilised the running app and produced misleading results. The
  reproductions below that needed a real pointer were driven by hand.

## Original report summary

| # | Reported | Cause | Fix | State |
|---|---|---|---|---|
| 1 | Closing the Control Center pops the notch out, then it collapses | One physical click delivered two `WM_LBUTTONUP` messages; both reached the click arm and toggled twice | `c0c91a8` | Fixed, measured |
| 2 | The pill blinks as the panel closes | The pill's opacity was keyed to the window's morph, which is still tall for the first frames of the shrink | `b4cfe22` | Fixed, measured |
| 3 | Interacting with the Control Center affects the notch | Six cross-writes between the two surfaces' state | `6e073ca` | Fixed, tested |
| 4 | The Control Center's target is tiny — you must click the icon exactly | The alpha map is the frame's own alpha, so only painted ink was a mouse target | `fdde437` | Fixed in code, **not yet seen working on screen** |

---

## 1. One click was two clicks

> If I clicked to the control center and click again, its collapsed the control
> center which is correct, but it also suddenly pop the notch out and then
> collapsed immediately

> Some how interacting with the command center cause the pill notch to blink
> popped out and in, very weird

**Expected.** Pressing the panel's icon closes the panel. The notch is not
involved.

**Actual.** The surface toggled twice from one press, so it opened and shut in
quick succession.

**Cause.** A single physical press was arriving at the overlay as two
`WM_LBUTTONUP` messages, one frame apart. The first was usually swallowed
because `is_island` read `false` on it — but that flag tracks the current
presentation, not the message, so it was luck rather than a rule. When it read
`true` on both, both releases reached the click arm.

The trace that found it, from one run:

```
w LBUTTONDOWN
w LBUTTONUP                    <- is_island false, swallowed
w LBUTTONUP hit=Caption ...    <- is_island true
w sending ClickAt
click: panel CLOSE             <- both handled at the same millisecond
click: panel OPEN
```

**Fix.** `press_active` in the window state. A release with no press
outstanding is not the end of a click, so it is dropped in the message arm
rather than in each branch that might act on it. A lost capture drops the
armed press with it, or the next release would be eaten and a click would take
two presses.

**How it was found.** Not by the automated tests — those were all clean, because
each one clicked once and the double needed the `is_island` race. It took
repeated fast clicking, by hand, to make it near-certain.

**Verification.** 22 presses produced 22 clicks. Twelve open/close rounds held
parity every time. No unit test: the pairing lives in the window proc and needs
a real window handle, which the repo has no harness for.

---

## 2. The pill blinked as the panel closed

**Expected.** The pill is not part of the panel's collapse, so it should not
change at all while the window shrinks.

**Actual.** The pill's own pixels dipped and recovered over about 90 ms.

```
before:  pill-lit = 5408 4692 4692 4692 5459 5414 ...   (a 716-pixel dip)
after:   pill-lit = 5416 5416 5410 5406 5409 5407 ...   (flat)
```

**Cause.** The pill faded for the island's own morph, but the test for "is the
island's own morph running" was the window's height. Both surfaces make the
window tall, so the height cannot answer it — and on the way *down* the window
is still tall for the first frames, so the pill faded during a collapse that
was not its own.

**Fix.** `island_card_open` — `manually_expanded || hover_expanded` — is the
answer, and the card carries it. The pill fades only for the island's morph.

**This one was missed by measuring the wrong thing.** The window height is
monotonic through the whole close (`338, 255, 174, 106, 45`), so three separate
runs concluded the close was clean while the notch flickered underneath it.
Counting lit pixels inside the pill's own rect is what showed it.

---

## 3. The two surfaces were not independent

> Please focus on the changes on the control center expand and the notch expand,
> they shall be independent, without interference with each other.

**Expected.** Opening or closing either surface leaves the other alone.

**Actual.** Six places where one surface's arm wrote the other's flags:

| Interference | Site |
|---|---|
| Opening the panel cleared `manually_expanded`, `hover_expanded`, `hover_deadline` | `island.rs` panel open arm |
| Opening the pill cleared `panel_open` | `toggle_expand` |
| The pill's click arm dismissed the panel outright | `island.rs` pill arm |
| Closing the panel cleared the notch's hover deadline and set its suppression | `island.rs` panel close branch |
| `set_hover` returned early while the panel was up, freezing the notch's dwell mid-countdown | `set_hover` |
| `point_over_bar_surface` widened to the whole window for whichever surface was showing | `bar/geometry.rs` |

The fifth is the one that matters most: a dwell armed just before the panel
opened stayed frozen, then fired on the frame the panel closed, opening the card
behind a dismissal. That is report 1's "pops out, then collapses" by a second
route.

**Original fix.** `close_panel` and `close_notch` each tear down one surface and nothing
else. At this stage, `claim_window` was the single place that said only one card could be drawn —
one window, one geometry, so mutual exclusivity is real — and it dismisses the
other surface *through that surface's own close*. `collapse_if_expanded` stays
shared, because Escape and click-outside both mean "dismiss what I am looking
at", and it dispatches to whichever close applies.

The pill no longer dismisses the panel. `set_hover` runs regardless, so the
notch's dwell and grace resolve on their own rules during the panel's open
period instead of being frozen and fired later. Hover asks
`point_over_notch_surface`, scoped to the notch.

**Verification.** `surfaces_are_independent` holds four properties. Each was
confirmed to fail when its coupling was put back — restored one at a time, not
assumed. Two pre-existing tests pinned the coupling outright ("the pill
dismisses the panel") and were rewritten to the independence contract.

---

## 4. The Control Center's target was its ink, not its slot

> Somehow the interacting area is so small, I mean I have to click exactly to the
> icon to take effect, is it a thing?

Yes. It is a real bug, and the hit region was never the problem.

**Expected.** The renderer declares a target — the icon's box plus 4px of
padding, centred — and clicking anywhere in it should work.

**Actual.** Only the painted pixels responded.

**Cause.** The alpha map is built from the frame's own alpha
(`alpha_map.push(source[3])`). The window is a mouse target only where something
was painted. A bar module is drawn on transparent glass, so the gaps between the
icon's two slider strokes were click-through and Windows routed those clicks to
whatever was behind the bar. The declared rect existed and did nothing. The
real target was a few thin lines.

**Fix.** `mark_hit_targets` raises the declared regions in the alpha map before
it is kept, in both present paths. The surface alpha goes to 1 — 1/255,
invisible — which is what makes DWM route the pointer there at all. This is the
same trick the top sensor strip already used.

Deliberately no wider than that: each module's own slot, with the padding it
already carries, becomes clickable, and the clock and the tray beside it keep
their own clicks. A test asserts the marking does not spill one pixel past the
region — swallowing the neighbours' clicks would be a worse bug than the small
target.

**Verification.** Three tests, each confirmed to fail with the marking disabled:
a blank gap inside a region becomes a target, a stroke keeps its real alpha so
nothing is dimmed, and a region running off the frame edge marks only the part
that fits.

**State: not yet seen working.** The fix is covered by tests that provably fail
without it, but it has not been confirmed against the real window. The attempt
to locate the icon on screen for a live check failed because the measurement
looked for dark ink and the bar's ink is light on a light background — the
measurement was wrong, not the app. It needs one click in the blank space beside
the icon to close out.

---

## Notes on how these were found

Three things cost this round of work more than the fixes did, and are worth
recording:

- **Measuring the wrong signal.** Window height is monotonic through the panel
  close while the pill flickers underneath it. Two of the four reports were
  missed this way, and one was reported as "clean" three separate times.
- **Stale coordinates.** Hard-coded click positions were derived early in the
  session and were wrong by the time they were used, twice — once after a
  monitor change, once after the bar's right-zone contents changed. Every test
  that survived reads its target from the frame instead; the ad-hoc probing
  scripts never did, and those are what misled.
- **Tests that pass for the wrong reason.** Three were written and then checked
  by reverting the fix; two passed regardless and were deleted rather than
  kept as guards that prove nothing.

Synthetic input probing was the fourth, and it is now a standing constraint
rather than a note. It destabilised the app more than once, and one probe
launched the app inside a job that later timed out and killed it.

---

## Later report summary

| # | Reported or requested | Current resolution | Verification |
| --- | --- | --- | --- |
| 5 | Notch and Control Center must be fully independent | Concurrent centered and right-side cards with separate state/hit geometry | Controller regressions, including top/bottom bar placement |
| 6 | A default-position notch remains visible when a notification pops out | Alert grows from and replaces the resting pill until dismissal/expiry | Render/controller regression |
| 7 | Capsules crowd icons; icons are small/pixelated | More padding, 18px Macchiato vector status glyphs, larger native app-icon selection | User accepted the spacing refinement; artwork quality remains app-dependent |
| 8 | Pill hover does not work; clicking shows an unhelpful Idle view | Scoped hover detection/deadlines; empty hover stays collapsed; click opens Today | Hover/controller tests and subsequent user acceptance |
| 9 | Expanded notifications are hard to dismiss; a clear action is needed | Popup-scoped outside dismissal, visible close button, Clear All | Geometry/interaction regressions and subsequent user acceptance |
| 10 | Alt+Space does nothing | Thread-wide GUI message pump dispatches launcher/EDIT messages | Failing-before/passing-after regression, installed-binary checks, explicit user confirmation |

## 5. Concurrent cards instead of mutual exclusion

**Expected.** Opening or closing Control Center must not collapse a manually
opened notch, cause a notch morph, or alter its hover machine.

**Change.** The single overlay frame can paint both surfaces. Its height fits
the taller card, but each card has separate geometry, content, and open state.
The pill's visibility belongs to its own card, not the window's height.
Hover checks the notch alone; Control Center anchors to its own right-zone icon.
This supersedes report 3's original `claim_window` restriction in Bar layout.

**Verification.** `opening_and_closing_the_panel_preserves_the_island_card`,
`bottom_bar_keeps_both_popovers_attached_to_the_strip`, and
`bar_alert_and_control_center_keep_separate_surfaces` cover preservation,
placement, and incoming alerts. A hover-opened notch still follows its own exit
grace; that ordinary hover close is not a Control Center state write.

## 6. Notifications left a second resting pill visible

**Expected.** The notification should feel like the pill expanding, not a
second unrelated card with the original notch still sitting above it.

**Fix.** Bar alert rendering takes ownership of the center presentation during
the morph, then returns it after dismissal/expiry. It does not hide the separate
Control Center. The banner also gained source/avatar hierarchy, clearer copy,
and a visible lifetime line rather than generic status prose.

**Verification.** `bar_alert_grows_from_the_resting_pill_and_returns` checks
the resting pill's replacement and return. Countdown tests check intermediate
updates and that repaint deadlines stop after the banner leaves.

## 7. Capsule spacing and icon quality

**Expected.** Status capsules should have breathing room and readable glyphs;
left-side app icons should not look like tiny bitmaps enlarged to fill a slot.

**Fix.** Macchiato status items gained padding and 18-logical-pixel vector
glyphs. Native window-icon selection prefers the largest available artwork,
avoiding an early 16px per-window choice when better class artwork exists.

**Verification.** The user explicitly accepted the improved capsule spacing.
Vector status glyphs scale with the render resolution; native app artwork
still cannot be sharper than the source provided by the application.

## 8. Hover reachability and the Idle placeholder

**Expected.** Live content should be reachable by hover, while an empty pill
should stay collapsed. Clicking an empty pill should offer useful shortcuts.

**Fix.** Hover input is scoped to the pill/card's logical geometry rather than
the overlay alpha map or unrelated status items. Dwell/grace deadlines are
scheduled even when arming them does not immediately repaint. The Bar's empty
click opens Today with date, recent-notification preview, Search, and
Notifications; standalone layouts keep their dashboard.

**Verification.** Controller tests cover dwell, grace, card traversal,
suppression after click dismissal, and empty hover. User feedback after these
refinements was positive. The chosen empty-hover rule remains an explicit
requirement, not evidence that hover is broken when no content is available.

## 9. Notification dismissal and clearing

**Expected.** Clicking outside the visible notification panel should close it
without requiring a click beyond the entire overlay's enlarged rectangle.
Clearing the list should be easy and have an unambiguous scope.

**Cause.** The broad overlay-surface check treated transparent space inside
the full-width frame as inside the open panel.

**Fix.** `point_over_open_popup` uses the panel's own visible bounds and entry
controls. The panel has a close button and a full-width Clear All action when
populated. Its list is bounded to four recent observations, with source,
arrival time, and an unread clock dot. Clear All affects this in-memory list,
not Windows Notification Center.

**Verification.**
`notification_center_only_claims_its_popup_and_bar_entries_for_outside_clicks`
checks the narrowed inside region. `clock_opens_recent_notifications_and_clear_all`
checks clock opening and clearing.
`control_center_and_notifications_switch_without_closing_the_notch` checks
right-popover switching without taking over the notch.

## 10. Alt+Space did nothing after launcher deployment

**Expected.** The registered global shortcut opens the app launcher and its
native text field accepts typing. Today → Search reaches the same window.

**Cause.** `OverlayWindow::next_event` called
`GetMessageW(&mut message, Some(self.hwnd), 0, 0)`. This retrieved only bar-window
messages; the separate launcher window, EDIT child, and worker update messages
were excluded. Creating the launcher and registering the shortcut were not
enough. Direct synchronous-message tests had bypassed the failing pump.

**Fix.** `GetMessageW(&mut message, None, 0, 0)` processes the GUI thread's
whole queue, dispatching messages to their own windows as usual. No input hook,
DLL modification, or injected system keystroke was needed.

**Verification.**
`production_message_pump_dispatches_launcher_hotkey_and_edit_input` posts an
open message, characters, and Escape through the actual shared overlay pump.
It failed before the filter change and passed after it. The rebuilt installed
binary was checked with queued messages to its own launcher/EDIT handles:
open, typing, and Escape all worked. These are scoped native-message checks,
not the global synthetic pointer probing prohibited above. The user then
explicitly confirmed the real Alt+Space experience works.

The final fix run recorded 403 passing workspace tests, with the two manual
launcher review/launch tests normally ignored. That is the 2026-09-30 baseline,
not a claim of a new full test run during this documentation update.

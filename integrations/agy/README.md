# Termielle x Antigravity CLI (agy)

`agy` is Google's terminal coding agent. Its hook system reads a JSON file —
the workspace-level `.agents/hooks.json`, or the global `~/.gemini/config/
hooks.json` — mapping a handler name to the events it listens for. Merge the
object from `hooks.fragment.json` into one of those files and every agy
conversation drives the overlay.

The installer does this merge automatically when it finds a `.gemini`
directory or an `agy` on PATH, substituting the placeholder below with the
installed emitter's absolute path.

## Absolute paths

agy resolves hook commands as absolute paths only; relative paths resolve
against wherever the session was launched and fail with exit 127. That is why
this fragment ships `{{TERMIELLE_EMIT}}` instead of a bare `termielle-emit.exe`:
replace it with the absolute path of your installed emitter (for the standard
install, `%LOCALAPPDATA%\Termielle\bin\termielle-emit.exe`) before merging by
hand. The installer performs the substitution itself.

## Event mapping

| agy hook event | Protocol event | Overlay face |
| --- | --- | --- |
| `PreInvocation` | `thinking_started` | thinking |
| `PostInvocation` | `thinking_ended` | working |
| `Stop` | `turn_completed` | ready -> idle |

A model invocation is bracketed by `PreInvocation`/`PostInvocation`, so each
model call shows as Thinking and the tool work between calls shows as Working.
`Stop` fires when the execution loop terminates, completing the turn.

## Boundary

agy exposes invocation boundaries, not reasoning boundaries: Thinking here
means "a model call is in flight", which is honest but coarser than
opencode's real reasoning step. There is no failure event (`turn_failed` is
never sent) and no approval-prompt event that can be observed without
answering it — `PreToolUse` matchers such as `ask_permission` would require
the hook to print a tool decision, which the emitter deliberately never does,
so a permission prompt stays Working until the next invocation or turn end.
Sessions are identified by the document's `session_id`, falling back to
`conversationId` on `Stop`.

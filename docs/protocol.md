# Termielle event protocol

**Version 1.** This document is the stable contract between an agent
integration (the *emitter* side) and the overlay (the *server* side). Anything
not specified here is out of contract: the overlay is free to change how it
interprets events, and an integration must not rely on undocumented fields.

The reference implementation of every rule here lives in
`crates/termielle-core/src/protocol.rs`; when code and prose disagree, code
wins and this document should be fixed.

## Shape of a conversation

A conversation is a *session*: one interactive run of an agent in one terminal.
An integration identifies its sessions and maps its own lifecycle events onto
the eight event kinds below. The overlay tracks every session it hears about,
reduces the whole set to one visual state (the "busiest" session wins), and
renders it.

Events are **content-free**. The only information an event may carry is which
session it concerns and what lifecycle moment it marks. No prompts, assistant
text, tool payloads, paths, or other agent data may be transmitted. The emitter
enforces this by extracting only a session identifier from the hook document.

## Transport

- **One event per connection.** Each event is delivered over its own
  connection, which is then torn down. There is no persistent stream, no
  multiplexing, and no server push.
- **One line per connection.** The event is a single newline-terminated JSON
  object, at most **4096 bytes** (`MAX_EVENT_BYTES`). A longer line is rejected
  and never delivered.
- **Endpoints.** Windows: the named pipe `\\.\pipe\termielle-v1`. Unix: a
  per-user Unix domain socket under `$XDG_RUNTIME_DIR` when set, else under the
  user's data directory. Both are owner-only: a different user cannot connect.
  A custom endpoint is supported with `--pipe <name>` on both the overlay and
  the emitter.
- **Delivery is bounded and fail-open.** The emitter never blocks longer than
  its 20 ms budget. A missing overlay, a wedged overlay, or a full pipe is not
  an error to the agent: the emitter writes its neutral `{}` response and exits
  0 either way.
- **Session identifier rules.** A session identifier is a nonempty string of
  at most 128 bytes (`MAX_SESSION_ID_BYTES`) containing no control characters.
  It is opaque: integrations may use any stable ID their agent exposes.

## Message schema

```json
{
  "version": 1,
  "source": "claude",
  "session_id": "abc123",
  "event": "prompt_submitted",
  "timestamp_ms": 1750000000000
}
```

| Field | Type | Rules |
| --- | --- | --- |
| `version` | integer | Must be `1`. A different version is rejected; the overlay reports it as an unsupported-version error rather than guessing. |
| `source` | string | Any short lowercase agent identifier: `[a-z0-9_-]+`, at most 32 bytes — `claude`, `codex`, `opencode`, and `agy` are conventions, not an exhaustive list. The overlay uses it only to distinguish sessions from different agents; a well-formed but unknown source is accepted, and a malformed one is rejected (`invalid source`). |
| `session_id` | string | See the rules above. Duplicate delivery detection and the visual reducer both key on `(source, session_id)`. |
| `event` | string | One of the eight kinds below. |
| `timestamp_ms` | integer | Unix epoch milliseconds, nonzero. The overlay additionally rejects timestamps more than four hours old or over 60 seconds in the future when applying them to live state; this keeps old journal lines and malformed future clocks from creating immortal sessions. |

Unknown fields are rejected (`deny_unknown_fields`), so a newer integration
cannot silently smuggle meaning past an older overlay.

## Events and what they mean

| Event | Overlay face | Notes |
| --- | --- | --- |
| `session_started` | idle | A conversation began. |
| `prompt_submitted` | thinking, then working after the local one-second hold | The user asked something. The one-second transition is the *default* reasoning boundary for agents that expose none. |
| `thinking_started` | thinking | The agent began a real reasoning step. Overrides the one-second heuristic: the overlay holds thinking until `thinking_ended`. |
| `thinking_ended` | working | Reasoning finished; the agent is generating or running tools. |
| `needs_input` | waiting | The agent is blocked on the user (a permission prompt, a question). |
| `turn_completed` | ready, then idle after the ready-hold | An answer is on screen. |
| `turn_failed` | failed | The turn ended in an error. |
| `session_ended` | gone | The conversation ended; its state is forgotten. |

### Semantics the overlay relies on

- **Ordering.** The overlay accepts an event only if it is not older than the
  last accepted event for the same session, and rejects exact duplicates. An
  integration must not reorder or fabricate timestamps.
- **Priority.** When several sessions are busy at once, the most attention-
  demanding state wins: needs_input > failed > ready > thinking/working > idle.
  Ties go to the most recently active session.
- **Staleness and clock bounds.**
  - An event for that session only arrives once and its timestamp is not older than four hours.
  - The timestamp is no more than 60 seconds ahead of the overlay clock.
  - A `thinking`/`working` session that goes silent for the busy-stall window
    (default 5 minutes, configurable 1–60) decays back to idle. An integration
    does not need to send `session_ended`; the overlay retires a dead session
    on its own.
- **Missing reasoning boundary.** Agents that expose no reasoning step (Claude
  Code, Codex) can only offer `prompt_submitted`, and the one-second hold is
  their best approximation. Agents whose only boundary is a model invocation
  (`agy`: `PreInvocation`/`PostInvocation`) may bracket each invocation with
  `thinking_started`/`thinking_ended`, which shows tool work as Working and
  model time as Thinking. Agents that expose one should send
  `thinking_started`/`thinking_ended` and skip nothing else.

## Versioning

`PROTOCOL_VERSION` is bumped only on a change that makes an old emitter or
overlay misinterpret a new peer: a new event kind or field, a changed
semantic, a new validation rule. An overlay that receives a newer version
rejects the line (unsupported version, code `100 + version`) and keeps running;
it never guesses. Backward-compatible additions (a new `source` value that the
overlay only keys sessions on) do not bump the version.

## Integration checklist

To add an agent, follow the scaffold in `integrations/_template/`. Adding one
is a configuration exercise only: pick a source word, map lifecycle moments,
and ship hooks — no overlay or emitter change is needed.

1. **Pick the source word.** Any short lowercase identifier (`[a-z0-9_-]+`,
   at most 32 bytes). Use the agent's own command name where possible
   (`claude`, `codex`, `opencode`, `agy`).
2. **Identify sessions.** Find the stable per-conversation identifier your
   agent exposes to hooks (Claude Code: `session_id`; Codex: `thread-id`;
   opencode: `sessionID`; agy: `session_id`, falling back to
   `conversationId`). The emitter recognizes all of those spellings.
3. **Map lifecycle moments.** Decide which of the eight events each agent
   signal maps to, and — critically — what your agent offers for the reasoning
   boundary and the "waiting" state.
4. **Emit.** Have each hook invoke `termielle-emit` with `--source`,
   `--event`, `--input`, and the session document. Emitter flags and wire
   names must stay the same word.
5. **Document the boundary.** Write down the approximations in your
   integration's comment block, exactly as the existing integrations do, so
   users know what to expect.

The wire name of every event and the emitter's flag grammar are exercised by
`crates/termielle-emit/tests/` against the real binary.

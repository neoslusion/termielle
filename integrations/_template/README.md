# Integration template

Scaffold for wiring a new agent into Termielle. The contract everything here
must satisfy is `docs/protocol.md`; read it first.

Copy this directory's fragments into your new `integrations/<agent>/` folder,
then follow the checklist. Keep the comment block at the top of every file:
it is the integration's contract with its users, and the boundary
approximations it documents are the parts most likely to be wrong.

## Checklist

1. **Find the session identifier.** What stable per-conversation ID does the
   agent hand its hooks? The emitter extracts these document keys, in order:
   `session_id`, `sessionID`, `sessionId`, `thread-id`, `thread_id`,
   `threadId`, `conversation_id`, `conversationId`. If your agent uses
   another key, extend `session_id_from_document` in
   `crates/termielle-emit/src/lib.rs` and add a test.
2. **Map lifecycle moments to events.** Go through the eight event kinds in
   `docs/protocol.md` and decide, for each agent signal you can observe, which
   event it maps to. The two hard questions:
   - *Reasoning boundary* — can you tell thinking from working? If yes, emit
     `thinking_started`/`thinking_ended`; if no, emit only `prompt_submitted`
     and say so (the overlay falls back to the one-second heuristic).
   - *Waiting* — is there a signal for "blocked on the user" and, crucially,
     a signal for "unblocked"? If unblocking is silent, say so: the overlay
     may sit on `needs_input` until the next event.
3. **Emit.** Each hook runs `termielle-emit` with `--source <agent>`,
   `--event <wire name>`, `--input stdin|argv`, and nothing else. The source
   word may be any short lowercase identifier (`[a-z0-9_-]+`, at most 32
   bytes) — a new agent needs no emitter change, just a new word. The wire
   names must match `docs/protocol.md` exactly.
4. **Wire the fixtures.** `crates/termielle-emit/tests/fixture_commands.rs`
   parses every checked-in `command_windows` line and runs it against the real
   emitter: add your hooks table to that test or its fixture readers break
   the count assertion.
5. **Document the boundary.** Keep the "Boundary:" note in your comment block
   honest about every approximation.

## Files

- `settings.fragment.json` — for agents with a Claude-Code-style JSON hooks
  configuration. Replace `--source <agent>`; the `command` must be reachable
  on `PATH` (or point at `TERMIELLE_EMIT`).
- `hooks.toml` — for agents with a Codex-style TOML hooks configuration.
- `termielle.plugin.ts` — for agents with an opencode-style plugin API. The
  skeleton below maps the opencode event stream; rewrite the cases for your
  agent's own event types.

## Placeholders

Every fragment uses `<agent>` and `<label>`; replace them with the agent's
source name (`--source` value) and a descriptive label.

## Verification

```powershell
cargo test -p termielle-emit          # unit + fixture tests
cargo test -p termielle-core          # protocol + reducer
```

Both suites must pass before the integration is complete, and the fixture
count assertion in `fixture_commands.rs` must be updated with your new hook
count.

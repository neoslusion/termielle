// Termielle companion for opencode.
//
// Install: copy this file into `~/.config/opencode/plugins/` (global) or
// `.opencode/plugins/` (one project). Plugins load at opencode startup; no
// hooks configuration is needed.
//
// The overlay follows the opencode event stream:
//   message.updated (role=user) -> prompt submitted
//   permission.asked             -> waiting for input
//   permission.replied           -> back to work
//   session.idle                 -> answer ready
//   session.status (type=error)  -> step failed
//   session.deleted              -> conversation ended
//
// Every event is one neutral `termielle-emit` invocation with only the
// session identifier in the argv document; failures are swallowed so a
// missing overlay never disturbs opencode. The emitter is resolved from
// TERMIELLE_EMIT, then the standard install location
// (%LOCALAPPDATA%\Termielle\bin on Windows,
// ~/.local/share/termielle/bin elsewhere), then PATH.

import { existsSync } from "node:fs"

export const TermiellePlugin = async ({ $ }) => {
  const sessionIDOf = (event) =>
    event?.properties?.sessionID ??
    event?.durable?.aggregateID ??
    event?.data?.sessionID ??
    "opencode"

  const emitter = () => {
    const candidates = []
    if (process.env.TERMIELLE_EMIT) candidates.push(process.env.TERMIELLE_EMIT)
    if (process.platform === "win32") {
      if (process.env.LOCALAPPDATA)
        candidates.push(
          `${process.env.LOCALAPPDATA}\\Termielle\\bin\\termielle-emit.exe`,
        )
    } else if (process.env.HOME) {
      candidates.push(`${process.env.HOME}/.local/share/termielle/bin/termielle-emit`)
    }
    return candidates.find((path) => existsSync(path)) ?? "termielle-emit.exe"
  }

  const emit = (kind, sessionID) =>
    $`${emitter()} --source opencode --event ${kind} --input argv ${JSON.stringify({ session_id: sessionID })}`
      .quiet()
      .catch(() => {})

  return {
    event: async ({ event }) => {
      const sessionID = sessionIDOf(event)
      switch (event.type) {
        case "message.updated":
          // A user-role message landing is the prompt being submitted.
          if (event?.properties?.info?.role === "user") {
            await emit("prompt_submitted", sessionID)
          }
          return
        case "permission.asked":
          await emit("needs_input", sessionID)
          return
        case "permission.replied":
          // An approval was granted; the agent resumes working.
          await emit("prompt_submitted", sessionID)
          return
        case "session.idle":
          await emit("turn_completed", sessionID)
          return
        case "session.status": {
          const status = event?.properties?.status?.type
          if (status === "idle") {
            await emit("turn_completed", sessionID)
          } else if (status === "error") {
            await emit("turn_failed", sessionID)
          }
          return
        }
        case "session.deleted":
          // A deleted conversation ends at once: the overlay must not hold a
          // busy state until its stall timeout.
          await emit("session_ended", sessionID)
          return
        default:
          return
      }
    },
  }
}

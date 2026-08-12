// Termielle companion for <Agent Label>.
//
// Install: copy this file into the plugin directory and restart the agent.
// Plugins load at startup; no hooks configuration is needed.
//
// The overlay follows the agent event stream:
//   <event type 1>                           -> prompt submitted
//   <event type 2>                           -> thinking started
//   <event type 3>                           -> thinking ended, working
//   <event type 4>                           -> waiting for input
//   <event type 5>                           -> back to work
//   <event type 6>                           -> answer ready
//   <event type 7>                           -> step failed
//   <event type 8>                           -> conversation ended
//
// Boundary: <describe the approximations: reasoning boundary, waiting state,
// failure detection — anything the agent's events cannot observe exactly>.
//
// Every event is one neutral `termielle-emit` invocation with only the
// session identifier in the argv document; failures are swallowed so a
// missing overlay never disturbs the agent. The emitter is resolved from
// TERMIELLE_EMIT, then the standard install location
// (%LOCALAPPDATA%\Termielle\bin on Windows,
// ~/.local/share/termielle/bin elsewhere), then PATH.

import { existsSync } from "node:fs"

export const TermiellePlugin = async ({ $ }) => {
  // Replace with the property path that names the current conversation in the
  // agent's events, with the fallback that matches how the agent identifies a
  // session when the property is absent.
  const sessionIDOf = (event) =>
    event?.properties?.sessionID ??
    event?.durable?.aggregateID ??
    event?.data?.sessionID ??
    "<agent>"

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
    $`${emitter()} --source <agent> --event ${kind} --input argv ${JSON.stringify({ session_id: sessionID })}`
      .quiet()
      .catch(() => {})

  return {
    event: async ({ event }) => {
      const sessionID = sessionIDOf(event)
      switch (event.type) {
        // One case per mapped event type. Every `emit` argument must be a wire
        // name from docs/protocol.md.
        default:
          return
      }
    },
  }
}

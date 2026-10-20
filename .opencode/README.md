# Installed skills

These are the [Emil Kowalski design-engineering
skills](https://github.com/emilkowalski/skills), installed **for this
repository only** so that the agent working on Termielle gets the same
motion and interface guidance the author does.

They are MIT licensed, copyright 2026 Emil Kowalski. The notice is kept at
`licenses/emilkowalski-skills-MIT.txt` and must travel with the files.

## What was installed, and what was not

Installed — the eight skills that bear on a native, custom-rendered desktop
surface:

| Skill | ID |
| --- | --- |
| Design Engineering | `emil-design-eng` |
| Apple Design | `apple-design` |
| Animation Performance | `animation-performance` |
| Animate | `animate` |
| Review Animations | `review-animations` |
| Improve Animations | `improve-animations` |
| Find Animation Opportunities | `find-animation-opportunities` |
| Animation Vocabulary | `animation-vocabulary` |

Deliberately **not** installed, because they describe platforms this
repository does not target and would only add noise to every prompt:

- `animate-expo` — React Native and Expo
- `ask-sonner` — the Sonner React toast library
- `pick-ui-library` — web component libraries
- `prototype` — a web UI version switcher
- `break-ui` — web layout stress testing
- `mobile-native` — web and mobile browser quirks
- `write-swift` — Swift and iOS

## How they are applied here

The skills are written for the web. Their rules are translated rather than
applied literally — the web's composite-only tier is a layered window that is
composited rather than repainted, and the web's "main thread" is the
window's message queue. The translation, the findings it produced, and the
measurements are recorded in
[docs/motion-and-performance.md](../docs/motion-and-performance.md).

## Updating

```powershell
git clone --depth 1 https://github.com/emilkowalski/skills.git "$env:TEMP\emil-skills"
Copy-Item "$env:TEMP\emil-skills\skills\animate\*" ".opencode\skills\animate" -Recurse -Force
# ...and the same for each skill above
```

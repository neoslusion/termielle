# Third-party notices

## Gemielle

Termielle was inspired by Gemielle by Rainan1010:
https://github.com/Rainan1010/Gemielle

Gemielle is published under the Apache License 2.0; a copy of the license is
included as `assets/LICENSE.Gemielle`. Termielle's implementation is
independent, but its artwork derives from Gemielle's `assets/` directory,
fetched from `main` on 2026-08-06. The unmodified originals are kept in
`assets/sources/` and are the ground truth the shipped re-encodings are
validated against (see `crates/termielle-app/tests/assets.rs`, which compares
every frame through the overlay's own WIC decoder).

The files in `assets/` are re-encodings produced by `crates/termielle-asset`:
each animation is composited with the GIF specification's rect-scoped
Background disposal (matching WIC), cropped to the shared character box used
by all five states, and re-encoded with a shared global palette, a dedicated
transparent index, and full-canvas Background-disposal frames so a moving
character never leaves traces of earlier frames. A trailing wrap frame that
nearly duplicates the first is dropped, so the loop closes with a
normal-sized step instead of a hiccup. Every animation is resampled to
exactly 60 frames at a uniform delay, preserving the total loop duration,
so all five states share one cadence. The listed SHA256 digests are of the

| Included file | Gemielle source path | SHA256 | Modification |
| --- | --- | --- | --- |
| `assets/standby.gif` | `assets/sources/waiting_user_input.gif` | `69BA3089E27BA987191CA29886340CEB7D6E330E5F4DAF17185A94F979A8FF03` | renamed, re-encoded: cropped to the shared character box, palette-normalized, Background-disposal frames |
| `assets/ai_thingking.gif` | `assets/sources/ai_thingking.gif` | `17EAC33692E76E5CD1F01853AC1EE8653BC9BF921B5ACD957021A38AC0664C15` | re-encoded: cropped to the shared character box, palette-normalized, Background-disposal frames |
| `assets/ai_working.gif` | `assets/sources/ai_typing.gif` | `DCFB7F92646BBF8E0E6455E0FC59AFAFEF9C633F7A3EAC8EF691260BA389D75E` | renamed, re-encoded: cropped to the shared character box, palette-normalized, Background-disposal frames |
| `assets/user_typing.gif` | `assets/sources/user_typing.gif` | `2121D0CEEBAA93B1A3D471F6658F8206CAFDF0E7BB7615E165871D9E16040A80` | re-encoded: cropped to the shared character box, palette-normalized, Background-disposal frames |
| `assets/ai_complete_answer.gif` | `assets/sources/ai_complete_answer.gif` | `052CC559D8EB223D1712CF9AC5196A65E93BEBFB8EF0AF5FA36C00F968E9F672` | re-encoded: cropped to the shared character box, palette-normalized, Background-disposal frames |

The crop box is the union of the opaque pixels across all five source
animations plus a 4-pixel margin: 329x294 within each 360x360 source. The
overlay window is sized to the frames, so every state presents at the same
window size and the character keeps its pixel size — only the empty transparent
margin is trimmed.

# Fast document lookup

Added **2026-10-16**. Alt+Space now finds applications and documents in the same
native launcher. This is **filename/path lookup**, not searching inside files,
OCR, semantic search, Windows Search, or a local AI model.

## Usage

- Open Alt+Space (or Today → Search), type a name, then use Up/Down and Enter.
- `proposal` finds matching filenames; `dsgn pdf` can find `Design Proposal.pdf`.
- Add folder hints to disambiguate: `acme proposal pdf` favours the matching
  document under an Acme folder. Terms are case-insensitive; Unicode names work.
- Six combined app/document results are shown. Documents have a file glyph and
  their parent path; applications retain their native artwork.
- Enter/click opens a document with its existing Windows file association.
  Missing, nonlocal, linked, or unsupported file targets fail without executing
  a command string. Windows security/protected-view behaviour is unchanged.
- Reopening matching files through Termielle boosts their ranking using bounded
  frequency/recency, inspired by [zoxide](https://github.com/ajeetdsouza/zoxide/wiki/Algorithm).
  This is not an integration with zoxide or an import of shell history.

Supported extensions: PDF, DOC/DOCX, RTF, TXT, MD/Markdown, RST, ORG, ODT/ODS/ODP,
XLS/XLSX, CSV, PPT/PPTX, EPUB, and TEX. Scanned/encrypted PDFs can be found by
filename too: the contents are never opened or parsed during indexing/search.

## Scope and exclusions

The first opening of Search starts a low-priority index worker, separate from
the app catalog/icon STA worker. The user's profile is visited first, then the
local fixed drives detected when the launcher was created. No administrative
permissions are requested and inaccessible directories are skipped.

Excluded paths include Windows/Program Files/ProgramData, AppData, hidden and
system entries, dot-folders, reparse points/junctions/symlinks, offline/cloud
placeholders, and common generated trees (`node_modules`, `target`, `build`,
`dist`, `vendor`, `venv`, `__pycache__`). Ordinary user folders named Windows,
away from the drive root or real system directory, are not excluded by name.
Network shares, mapped network drives, removable drives, and cloud-only files
are not enumerated. A OneDrive tree exposed as a reparse point is also skipped;
this version does not try to hydrate or traverse it.

These conservative exclusions are deliberate: this is not a complete inventory
of every file visible to Explorer. Only filenames with supported extensions are
indexed; executables, scripts, arbitrary source files, and directories are not
document results.

## Responsiveness and resource bounds

- App search remains available while documents are indexing. Native editing
  never walks the filesystem or waits for the index worker.
- File queries search prepared lowercase paths off the UI thread. Only the best
  six candidates are returned; old query generations cannot replace newer ones.
  Existing file matches that still fit the next query remain visible while the
  next reply is pending. Index progress updates preserve the selected target.
- Cached names are usable before the background rebuild completes. Later
  queries/openings request another rebuild after at least five minutes. There
  is no continuous watcher or repeated full scan on every keystroke.
- The index stops at **50,000 documents**, **8 MiB of prepared path accounting**,
  or **1,000,000 visited entries**; directory depth is limited to 64. The footer
  reports when an index limit is reached. This is not a total-process RAM cap;
  rebuild/cache serialization temporarily uses additional memory.
- Only up to 512 successful file-open usage records are retained. File rows use
  vector icons instead of extracting thumbnails or reading documents.
- Deleted files disappear on a completed refresh; a stale cached row can remain
  until then, but opening it checks availability and reports failure.
- First indexing can take longer on a busy or large disk. There is no claim that
  every unindexed file can be found instantly before that scan finishes.

## Local data

`~/.termielle/launcher-files.json` stores the bounded filename/path cache and
successful-open counts/timestamps. Hosted profiles use their own data directory.
It contains no document text, query history, shell history, prompts, or terminal
output, and is never uploaded. Paths themselves may be sensitive; the cache
inherits its directory's access permissions.

To reset the cache and learned ranking, first exit the active Termielle frontend,
delete only this JSON file, then restart. Removing it while the app is running
is not a persistent opt-out: the worker can write it again.

## Checks

```powershell
rtk cargo test -p termielle-app launcher:: --lib -- --nocapture
rtk cargo fmt --all -- --check
rtk cargo test --workspace -j 2
rtk cargo build -p termielle-app --release --bin termielle-app -j 2
```

Regression coverage includes fuzzy/path/extension and Unicode queries, bounded
ranking/history, cache round-trip and invalid/system-entry rejection, scanner
exclusions, worker query/disconnect behaviour, app/document merging, retained
selection, native stale-query suppression, and unsafe/missing launch rejection.
The worker fixture uses invalid PDF bytes to prove lookup needs only metadata,
not a PDF parser. The native tests use app-owned messages, not injected global
keyboard or mouse input.

The 2026-10-16 validation run passes **534 workspace tests** with two manual
launcher checks ignored; formatting and the release native-app build pass.

The native executable was rebuilt and locally redeployed with matching
source/installed hashes, unchanged configuration, and no DLL replacement.
An installed-window message-pump check indexed **5,959 documents** without
reaching a limit and populated document result rows. It opened/dismissed only
Termielle's own launcher, injected no global input, and opened no documents;
it is not a claim of physical shortcut or associated-reader acceptance.

A release-mode synthetic query over 50,000 prepared document paths took about
**6.5 ms** on the development machine (4,477,780 bytes of path accounting).
That is one query-engine sample, not initial filesystem indexing time, total
process memory, or a guarantee for every query/disk. Raw output is in ignored
`target/document-search-timing.log`.

Physical pointer/keyboard and associated-reader acceptance remain separate from
these checks. App-only binary deployments do not update an installed Windhawk
runtime; rebuilding that edition must be an explicit separate deployment.

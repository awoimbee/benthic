# Architecture

## Goals and constraints

* **Runs in a browser and on the desktop from one codebase.** Dioxus gives us
  a single component tree over `wasm32` and native targets.
* **No platform logic in the domain.** `benthic-core` must compile for both
  targets with no `cfg(target_arch)` branches.
* **Data integrity over cleverness.** All measurements are integers in
  well-defined base units. Every format has round-trip tests.
* **Local-first.** Storage is a swappable, narrow interface.

## Crate graph

```
benthic (binary)                apps, UI, platform glue
   │  depends on
   ▼
benthic-core (lib)              model + formats + math, no UI/platform
```

`benthic-core` has no dependency on Dioxus. This keeps it fast to test, easy to
reuse (e.g. a future CLI or a fullstack server), and free of UI lifetimes.

## Layers in the app crate

```
src/main.rs        launch(App)
src/app.rs         root component: load-once, autosave effect, shortcuts, layout
src/state.rs       AppState { log, selected, selection, status, history, filter, prefs, show_prefs }
src/actions.rs     high-level user actions -> commands (new/duplicate/delete/...)
src/storage.rs     autosave backend (localStorage | data dir), JSON
src/platform.rs    export: Blob download (web) | file write (desktop), clock
src/components/    Toolbar, ImportExport, DiveList, DiveDetail, DiveProfile
src/format.rs      presentation helpers (titles, subtitles)
```

`benthic-core` additionally provides `history` (the undo/redo command stack)
and `filter` (the `DiveFilter` matching model), both heavily unit-tested.

### Data flow

```
 importer ──► benthic_core::io::{ssrf,json} ──► DiveLog
                                                  │
                       storage::load() ◄───────────┤ (startup)
                                                  ▼
                                         Signal<DiveLog>  ──► DiveList / DiveDetail
                                                  │
                       storage::save() ◄───────────┘ (use_effect on change)
```

* The **single source of truth** is `Signal<DiveLog>`.
* Components read the log signal; mutations replace the whole log (or, later,
  go through commands). This makes autosave trivial and keeps rendering
  predictable.
* `Signal` is `Copy`, so `AppState` is a small `Copy` struct shared through
  Dioxus context.

### Mutations go through commands

User-visible edits are expressed as `benthic_core::history::Command` values.
`AppState::dispatch` applies a command to a clone of the log, records it on the
undo stack, and stores the new log. Reverting a command is an exact inverse, so
undo/redo come for free and the edit history is auditable.

Commands are fine-grained where that is easy (`AddDive`, `UpdateDive`, ...) and
coarse (`Snapshot { before, after }`) for bulk operations like import and
autogroup, where a precise diff would be awkward. The history is bounded
(`History::DEFAULT_LIMIT`). `AppState::dispatch_all` groups several commands
into a single undo step (for example "edit dive" that also creates a site).

Autosave persists the whole log on every change. Because the log is the single
source of truth this stays simple and correct; incremental persistence is
future work.

## Persistence

`storage.rs` exposes `load() -> Option<String>` and `save(&str) -> Result<(), String>`,
implemented per target:

| Target | Log backend | Preferences | Notes |
| --- | --- | --- | --- |
| web | `localStorage` (`benthic.log`) | `benthic.prefs` | ~5 MB limit; IndexedDB planned |
| desktop | `ProjectDirs::data_dir()/log.benthic.json` | `prefs.json` | created on first save |

The stored payload is the **native JSON** format, which is lossless. SSRF is an
interchange format, not the autosave format, precisely because it cannot
represent everything (e.g. internal ids, some metadata) without ambiguity.
Preferences are stored separately so they survive replacing the log; they are
**display-only** (the model and all file formats remain canonical metric).

Autosave runs in a `use_effect` keyed on the log signal. It is intentionally
synchronous and cheap; debouncing and incremental writes are future work.

## Platform abstraction

The only genuinely platform-specific operations are:

1. **Reading an import** — handled uniformly by Dioxus' file input and
   `FileData::read_string()`, which works on both web and desktop.
2. **Writing an export** — `platform::save_file` downloads a `Blob` on web and
   writes to the working directory on desktop.

Everything else goes through Dioxus' cross-platform API.

## Error handling

* `benthic-core` defines one `Error` enum (via `thiserror`) with variants for
  XML, JSON, parse, date/time and I/O failures.
* The UI never panics on bad input: parse errors are surfaced in the status
  message, and optional fields degrade gracefully.
* The SSRF reader is deliberately tolerant: unknown elements are ignored, and
  missing optional fields fall back to sensible defaults.

## Testing strategy

* **Unit tests** for units, gas naming and helpers.
* **Round-trip tests** for JSON (lossless) and SSRF (stable counts and fields),
  using a checked-in demo log.
* **Real-world validation** via `cargo run -p benthic-core --example parse_ssrf`
  against large Subsurface sample logs.
* CI type-checks the web target and runs clippy with `-D warnings`.

Future: golden-file tests per importer, `proptest` round-trips, and parser
fuzzing.

## Performance notes

* Parsing builds a minimal DOM then maps it; this is simple and fast enough for
  the 26k-sample sample logs used during development.
* The profile is rendered as a single SVG polyline, so it is O(samples) but
  allocation-light.
* Lists will need virtualization before logs grow to tens of thousands of
  dives.

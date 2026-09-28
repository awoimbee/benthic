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

benthic-divecomputer (lib)      native-only libdivecomputer wrapper;
   │  depends on                 an empty crate on wasm32
   ▼
benthic-core
```

`benthic-core` has no dependency on Dioxus. This keeps it fast to test, easy to
reuse (e.g. a future CLI or a fullstack server), and free of UI lifetimes.
`benthic-divecomputer` is kept separate so the C library and its transports
never leak into the domain or the web build; the app will depend on it for
device download.

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
| web | IndexedDB (`benthic.log`), `localStorage` fallback | `localStorage` (`benthic.prefs`) | synchronous cache plus an ordered, coalescing write queue |
| desktop | `ProjectDirs::data_dir()/benthic.log` | `prefs.json` | created on first save |

The stored payload is the **native JSON** format, which is lossless. SSRF is an
interchange format, not the autosave format, precisely because it cannot
represent everything (e.g. internal ids, some metadata) without ambiguity.
Preferences are stored separately so they survive replacing the log; they are
**display-only** (the model and all file formats remain canonical metric).

An automatic backup of the previous log is kept, refreshed at most once an
hour. If the primary log fails to parse at startup, the app loads the backup
and reports the recovery; the unreadable primary is left untouched. The
preferences dialog also offers a manual "Restore last backup".

Autosave runs in a `use_effect` keyed on the log signal. On the web it updates
an in-memory cache synchronously and flushes to IndexedDB in the background, so
the effect stays cheap and writes never reorder.

## Remote sync

`benthic-core::sync` is the pure, tested decision logic: given the local log,
the last-synced bookkeeping and the remote copy, it returns `UpToDate`, `Push`,
`Pull` or `Conflict`. A push happens only when the remote is unchanged since the
last sync, a pull only when the local copy is unchanged, and otherwise the user
chooses — nothing is overwritten silently.

The transport lives in the app (`src/sync/`) and stores the whole log as one
file. Backends implement the `Backend` trait (`src/sync/backend.rs`), which
owns the settings fields the dialog renders, the optional interactive sign-in,
and fetch/push. They are listed in one registry, so adding a service (OneDrive,
FTP, ...) is a new module plus a registry entry — nothing else changes. HTTP
goes through a single cross-platform shim: `gloo-net` on the web, `reqwest`
natively. Settings are stored per backend as JSON, and old flat configs are
migrated on load.

GitHub uses the repository-contents API (the base URL is configurable, so Gitea
and enterprise hosts work). Google Drive stores the log in the app's private
`appDataFolder` (scope `drive.appdata`) and signs in differently per target:

* **Web** uses Google Identity Services' browser token flow
  (`public/google-auth.js` + `src/sync/oauth_web.rs`), which needs only a public
  OAuth *client ID* — no secret and no backend.
* **Desktop** runs the installed-app loopback flow with PKCE
  (`src/sync/oauth_desktop.rs`): it opens the system browser, receives the code
  on a temporary `127.0.0.1` listener, exchanges it, and keeps the refresh token
  so later syncs need no interaction.

Both need a one-time Google Cloud setup. For the web, enable the Drive API and
register a **Web application** OAuth client, adding the app's origins as
Authorized JavaScript origins. For the desktop, register a **Desktop app**
client and set its ID and (non-confidential) secret in `src/sync/gdrive.rs`.

## Dive computer integration

`benthic-divecomputer` wraps [libdivecomputer](https://libdivecomputer.org/).
The C library is vendored as a git submodule (`vendor/libdivecomputer`, pinned
to the Subsurface fork that backs the reference implementation) and built by
the crate's `build.rs` with its own autotools build system, so nothing has to be
installed system-wide. The crate exposes:

* `descriptors()` — the full table of supported models and their transports;
* `parse_dump(vendor, product, data)` — turn a raw memory dump into a
  `benthic_core::Dive` (samples, events, gases, tanks, setpoints, CNS/NDL).

The C build is gated behind the crate's `native` feature and the whole crate
compiles to nothing on `wasm32`, so a plain workspace `cargo test`/`clippy`
stays pure Rust on machines without the toolchain. The native path is built in
the devcontainer and in CI (`just check-native`). Parsing is covered offline by
the three raw dumps shipped with libdivecomputer; live download over
serial/USB/BLE is the next layer.

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

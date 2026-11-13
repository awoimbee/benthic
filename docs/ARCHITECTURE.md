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
src/state.rs       AppState { log, selected, selection, status, history, filter, prefs, dialog, ... }
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
hour; a stored log that no longer parses never replaces it. If the primary log
fails to parse at startup, the app first copies it to `benthic.unreadable.<ts>`
and waits for that write to be persisted, then loads the backup and reports
both. If the copy cannot be written, autosave stays off. Without a usable
backup, autosave stays off until the first edit. The preferences dialog also
offers a manual "Restore last backup".

Autosave runs in a `use_effect` keyed on the log signal. On the web it updates
an in-memory cache synchronously and flushes to IndexedDB in the background, so
the effect stays cheap and writes never reorder. A failed flush stays queued
for retry and is reported in the UI until a later flush succeeds. On the
desktop each file is written to a temporary sibling, synced, and renamed into
place, so a crash leaves either the old or the new file, never a truncated one.

## Cloud storage

> **Temporary.** The backends described in this section are a stopgap until a
> long-term cloud-sync design is settled. The cloud storage dialog says so in
> the UI.
> Treat the transport and its stored configuration as replaceable rather than
> as a stable contract.

The remote copy is used like a file. **Open from cloud storage** replaces the
local log with it (one undoable step) and **Save to cloud storage** replaces it
with the local log. With **Autosave** on, the app saves a few seconds after the
last change (`src/app.rs`). `benthic-core::sync` is the pure, tested decision
logic. It remembers the remote revision and local fingerprint from the last
open or save. Saving only replaces the remote copy it last saw, and opening only
discards local changes that were saved to it. Otherwise the user confirms, so
nothing is overwritten silently. Autosave never overwrites: it turns itself off
when the remote changed elsewhere.

The transport lives in the app (`src/sync/`) and stores the whole log as one
file. Backends implement the `Backend` trait (`src/sync/backend.rs`), which
owns the settings fields the dialog renders, the optional interactive sign-in,
and fetch/push. They are listed in one registry, so adding a service (OneDrive,
FTP, ...) is a new module plus a registry entry — nothing else changes. HTTP
goes through a single cross-platform shim: `gloo-net` on the web, `reqwest`
natively. Settings are stored per backend as JSON.

GitHub uses the repository-contents API (the base URL is configurable, so Gitea
and enterprise hosts work) and needs only a personal access token.

Subsurface cloud is a normal bare git repository per account, served over smart
HTTP with Basic auth. The user's email is both the repository and branch name.
`src/sync/git.rs` implements the small part of the git protocol needed to fetch
a shallow tip and push one commit (pure Rust, so it runs in the browser too);
`benthic_core::io::subsurface_git` translates Subsurface's text-blob tree to
the model. Files benthic does not understand (such as filter presets) are
carried over unchanged on push. The cloud server sends no CORS headers, so the
web build goes through a small CORS proxy (`tools/subsurface-proxy/`) by
default, while desktop builds talk to the server directly. The proxy only
forwards `/git/<account>/...` and only for allowed origins; the server URL is
an advanced setting.

Google Drive stores the log in the app's private `appDataFolder` (scope
`drive.appdata`). It is **only offered in local/development builds**: publishing
an OAuth app that uses it requires verifying ownership of the registrable
domain behind the home page and privacy policy, and `github.io` is a public
suffix that cannot be owned. The registry in `src/sync/backend.rs` therefore
lists Drive only when `cfg!(debug_assertions)`. Sign-in differs per target:

* **Web** uses Google Identity Services' browser token flow
  (`public/google-auth.js` + `src/sync/oauth_web.rs`), which needs only a public
  OAuth *client ID* — no secret and no backend.
* **Desktop** runs the installed-app loopback flow with PKCE
  (`src/sync/oauth_desktop.rs`): it opens the system browser, receives the code
  on a temporary `127.0.0.1` listener, exchanges it, and keeps the refresh token
  so later syncs need no interaction.

For development builds that expose Google Drive, both target flows need a
one-time Google Cloud setup. For the web, enable the Drive API and register a
**Web application** OAuth client, adding the app's origins as Authorized
JavaScript origins. For the desktop, register a **Desktop app** client and set
its ID and (non-confidential) secret in `src/sync/gdrive.rs`.

## Dive computer integration

`benthic-divecomputer` wraps [libdivecomputer](https://libdivecomputer.org/).
The C library is vendored as a git submodule (`vendor/libdivecomputer`, pinned
to the Subsurface fork that backs the reference implementation) and built by
the crate's `build.rs` with its own autotools build system, so nothing has to be
installed system-wide. The crate exposes:

* `descriptors()` — the full table of supported models and their transports;
* `parse_dump(vendor, product, data)` — turn a raw memory dump into a
  `benthic_core::Dive` (samples, events, gases, tanks, setpoints, CNS/NDL);
* `scan(descriptor, transport)` — enumerate matching serial, USB, USB-HID or
  Bluetooth devices;
* `download(descriptor, id, fingerprint, cancel, on_event)` — open a device,
  download every unseen dive and parse it, reporting progress, device info and
  supporting cooperative cancellation. The returned `latest_fingerprint` is
  registered on the next run so already-downloaded dives are skipped.

The C build is gated behind the crate's `native` feature and the whole crate
compiles to nothing on `wasm32`, so a plain workspace `cargo test`/`clippy`
stays pure Rust on machines without the toolchain. The native path is built in
the devcontainer and in CI (`cargo xtask check-native`). Parsing and the download
plumbing are covered by the three raw dumps shipped with libdivecomputer and by
error-path tests; real device protocols need real hardware. BLE has no native
transport in this libdivecomputer fork (it expects the application to provide
an I/O stream), so it is not wired up yet.

On the web the same Rust mapping is reused. `cargo xtask build-web-shim`
builds libdivecomputer with Emscripten plus a small C shim
(`web/divecomputer/shim.c`) that emits the neutral `RawDive` JSON (see
`benthic-core::divecomputer`); the app deserialises it and builds the model, so
there is exactly one implementation of the mapping. Device I/O runs through
`dc_custom_open` with Asyncify, so the synchronous C iostream can drive async
WebSerial/WebBluetooth.

On the web the shim is loaded on demand: `index.html` defines
`window.benthicLoadDc`, the app calls it when the download dialog opens, and
`public/divecomputer/api.mjs` dynamically imports the Emscripten module and
installs `globalThis.benthicWeb` (a JSON-in/JSON-out surface) plus
`globalThis.benthicHost` (the WebSerial implementation in `host.mjs`).
These files are shipped with a `.mjs` extension so the Dioxus bundler copies
them verbatim instead of running esbuild over the prebuilt Emscripten glue.
`src/divecomputer_web.rs` is the thin `js-sys` bridge; the web download dialog
merges into the log through the same `actions::merge_downloaded` and
fingerprint storage as the desktop one.

BLE is the exception to "libdivecomputer owns the transport": the library has no
native BLE support, so benthic supplies a `dc_custom_open` iostream. The desktop
build uses BlueZ through `bluer` on a dedicated thread with a current-thread
tokio runtime; the synchronous C callbacks post a request and block on the
reply. The web build does the same in `host.mjs` over Web Bluetooth. Both pick a
known GATT serial service (`SERIAL_SERVICE_UUIDS`, mirrored from Subsurface) and
stream over its write/notify characteristics, handling the `DC_IOCTL_BLE_*`
operations the few backends need.

The app's **Download** dialog (desktop only, behind the `divecomputer` feature)
drives this: pick a model, scan the bus, choose a device, watch progress and
merge the new dives into the log. libdivecomputer is blocking, so scans and
downloads run on a worker thread and report back through a Dioxus coroutine.
Downloaded dives are deduplicated against the log by start time and model, and
the device fingerprint is remembered so later downloads only fetch new dives.

## Map view

`src/components/map.rs` shows dive sites on an interactive [Leaflet](https://leafletjs.com/)
map: the **Map** toolbar button opens a modal with a site list and a clustered
map, and the dive detail embeds a small map for the selected site. Leaflet and
its marker-cluster plugin are vendored under `assets/map/` and loaded as asset
bundles, so only the tiles are fetched from the network: Esri World Imagery
(satellite, the default) and OpenStreetMap (streets), switchable in the map.

Drawing is delegated to a script injected with `document::eval`. Rust serialises
the sites as JSON, the script builds the map and sends marker clicks back over
the eval channel, and Rust turns a click into a dive selection. This keeps the
same code path on web and desktop and leaves the domain crate untouched.

## AI assistant

The ✨ toolbar button toggles `AssistantPanel` (`src/components/assistant.rs`),
a chat column beside the dive panes (full screen on phones), so a dive stays
visible while the diver asks about it. `src/ai.rs` builds a [rig](https://rig.rs)
agent on the provider the user configured (OpenRouter, Groq, Gemini or
DeepSeek), sends requests directly from the device, and streams the reply.

The model never receives the log wholesale. It gets a short preamble (dive
count, the open dive, language and units) and read-only tools that run locally
against a snapshot of the log: `search_dives` (text, site, buddy, date range,
depth, freedive filters), `get_dive` (one dive's full details), `log_stats`
(totals, records, per-year counts, top sites and buddies) and `list_sites`. It
is told to cite dives as `[dive #N](dive:ID)`; the panel renders those as links
that open the dive.

A thread keeps both what the panel shows (questions, tool calls, answers,
errors) and rig's message history, which is replayed with the next question.
The last 20 threads and the API key live in local storage (`benthic.ai_threads`
is bulk storage on the web) and are never exported or synced. An undocumented
`base_url` in the stored config points the OpenRouter client at an
OpenAI-compatible mock for manual testing.

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

# benthic roadmap

This roadmap is intentionally ambitious: the goal is to cover Subsurface's
feature set while taking advantage of Rust and a web-first architecture. Items
are grouped into phases; within a phase, work can proceed in parallel. Checked
boxes are implemented in this repository today.

Guiding principles:

1. **Never lose a dive.** Import/export and local persistence are core, not
   add-ons. Every write path is covered by round-trip tests.
2. **Local-first, sync later.** No account is required. Cloud/git sync is an
   opt-in layer on top of the local log.
3. **One model, many front-ends.** All logic lives in `benthic-core`, free of
   UI and platform code.
4. **Correct units, always.** Internal storage uses integer base units
   (mm, mbar, mkelvin, ml, permille) to avoid floating-point drift.

---

## Phase 0 — Foundations ✅

- [x] Cargo workspace; `benthic-core` separated from the Dioxus app
- [x] Strongly-typed integer units (`Depth`, `Pressure`, `Temperature`,
      `Duration`, `Volume`, `Weight`, `Fraction`, `Location`)
- [x] Gas-mix model with air/nitrox/trimix naming
- [x] Domain model: `DiveLog`, `Dive`, `DiveComputer`, `Sample`, `Event`,
      `Cylinder`, `WeightSystem`, `DiveSite`, `DiveTrip`, `Device`, `Picture`
- [x] Native lossless JSON format (`serde`)
- [x] Subsurface-compatible SSRF reader/writer (samples are delta-decoded on
      read, fully written on write)
- [x] Format auto-detection and log merging with id renumbering
- [x] Dioxus app shell: toolbar, dive list, dive detail, SVG depth profile
- [x] Local persistence: `localStorage` (web) / platform data dir (desktop)
- [x] Import (auto-detected SSRF/XML/JSON) and export (SSRF) in the UI
- [x] Round-trip tests against a real Subsurface sample log
- [x] GitHub Actions: CI (fmt, clippy, tests, wasm check) and Pages deploy

## Phase 1 — A log you can actually live in

- [x] Add, edit, duplicate and delete dives
- [x] Editable dive detail: notes, buddy, divemaster, suit, ratings, tags
- [x] Undo/redo via a command stack (bounded history, groupable commands)
- [x] Trip assignment plus automatic trip grouping with an on/off toggle
- [x] Dive-site create/edit (name + GPS) from the dive form
- [x] Filter model with full-text search and rating/tag/depth constraints
- [x] Keyboard shortcuts for undo/redo (`Ctrl/Cmd+Z`, `Shift+Z`, `Y`)
- [ ] Multi-select and bulk edit/delete
- [ ] Cylinder & weight editing UI (presets: AL80, LP85, steel 12/15, ...)
- [ ] Trip create/rename/merge/split UI
- [ ] Dive-site merge, map picker, and deduplication of inline locations
- [ ] Filter UI for tags/rating/depth, plus saveable filter presets
- [ ] Preferences: metric/imperial units, date/time formats, salinity, defaults
- [ ] Command palette
- [ ] Unsaved-change tracking and manual save/load of log files
- [ ] Data safety: rotating local backups, crash recovery, "undo delete"

## Phase 2 — Profiles, decompression and planning

- [ ] Interactive profile: hover/crosshair readout, pan & zoom
- [ ] Overlays: temperature, cylinder pressure, NDL, TTS, CNS, SAC, heart rate
- [ ] Deco ceiling / stop visualization and gas-switch markers
- [ ] Multiple dive computers per dive, switchable, with difference view
- [ ] Dive planner: Bühlmann ZH-L16 (GF) and VPM-B
- [ ] Open-circuit, CCR and pSCR planning; bailout and gas needs
- [ ] Plan ↔ actual comparison; save plans as dives
- [ ] Gas calculations: MOD/END/EAD, best mix, ICD warnings, SAC/RMV

## Phase 3 — The import ecosystem

- [ ] Dive computer download via `libdivecomputer`
      (desktop: serial/USB/Bluetooth; web: WebSerial + Web Bluetooth where available)
- [ ] Device management, firmware/settings, dive computer nicknames
- [ ] CSV import with user-defined column mapping (Subsurface templates)
- [ ] Additional importers: UDDF, DL7, GPX, Cobalt, Shearwater, Suunto,
      Diving Log, SeaBear, Cochran, ...
- [ ] Subsurface cloud storage and git-backed logs
- [ ] CSV/HTML/JSON bulk export; Subsurface-compatible export options

## Phase 4 — Visualization, media and statistics

- [ ] Statistics view: histograms, box plots, scatter plots, regression
- [ ] Charts of consumption, temperature, depth, SAC over time
- [ ] Map view of dive sites (MapLibre/Leaflet) with clustering
- [ ] Photo/video gallery; EXIF time sync to dive timeline
- [ ] Printable logs and export to HTML/PDF (Subsurface templates)
- [ ] Year-in-review / summary dashboards
- [ ] World-map (visited countries) export

## Phase 5 — Platform polish

- [ ] IndexedDB storage for large logs; quota handling; migration from v1
- [ ] PWA: offline install, update prompts, file-handling API for `.ssrf`
- [ ] Mobile targets (Dioxus iOS/Android)
- [ ] Internationalization (gettext/fluent) and translations
- [ ] Theming (light/dark/high-contrast) and accessibility (a11y) audit
- [ ] Import/export through the File System Access API on supported browsers
- [ ] Performance: virtualized lists, streaming parse for very large logs

## Cross-cutting / continuous

- [ ] Golden-file tests for every importer/exporter
- [ ] Property-based round-trip tests (`proptest`) for model ↔ formats
- [ ] Fuzzing of the SSRF/JSON parsers
- [ ] Benchmark suite for parse/write and UI responsiveness
- [ ] `cargo-deny` for license and advisory checks; SBOM on release
- [ ] Contributing guide, issue/PR templates, release automation
- [ ] Documentation: user guide and a format reference

---

## Milestones

| Milestone | Definition of done |
| --- | --- |
| **M0 — Skeleton** ✅ | App builds for web + desktop, imports/exports SSRF, persists locally, deploys to Pages |
| **M1 — Usable log** | Create/edit/delete dives, sites and trips; search/filter; undo/redo |
| **M2 — Real dives** | Import from common dive computers and CSV; full profile rendering |
| **M3 — Planner** | OC/CCR dive planner with plan-vs-actual |
| **M4 — Insight** | Statistics, maps, media, printing |
| **M5 — Everywhere** | Mobile builds, PWA, translations, accessibility |

## Non-goals (for now)

* A server-side component or mandatory cloud account.
* Byte-for-byte identical Subsurface output; we target *compatibility*, not
  cosmetic parity.
* Bundling GPL-incompatible dive-computer libraries into the web build.

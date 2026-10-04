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
- [x] Responsive layout: compact header and separate list/detail screens on phones
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
- [x] Cylinder & weight editing with presets (AL80, LP85, steel 12/15, ...)
- [x] Multi-select and bulk delete (one undo step)
- [x] Filter UI for tags, rating and depth
- [x] Preferences: metric/imperial units, persisted separately from the log
- [x] Saveable filter presets
- [x] Trip create (from selection), rename and delete
- [x] Trip merge and split (split via 'New trip' from a selection)
- [x] Dive-site merge and de-duplication (by name + GPS)
- [x] Dive-site map link (a full map picker comes with the Phase 4 map view)
- [x] Preferences: date and time formats
- [x] Preferences: dark and light themes
- [x] Preferences: default salinity and default cylinder
- [x] Manual load/save via import/export; autosave means there are no unsaved changes
- [x] Data safety: hourly automatic backup, crash recovery, undoable deletes

## Phase 2 — Profiles, decompression and planning

Complete except VPM-B, which is deliberately deferred (see the note in the
list below).

- [x] Profile scrubber with a time/depth/temperature/pressure readout
- [x] Pan & zoom on the profile (time window sliders)
- [x] Hover crosshair (pointer readout snapped to the nearest sample)
- [x] Overlays: temperature and cylinder pressure
- [x] Overlays: NDL, TTS, heart rate and CNS (SAC is dive-level, shown as a fact)
- [x] Deco ceiling visualization and gas-switch markers
- [x] Per-metric y-axis scales (depth plus each active overlay) with depth gridlines
- [x] Multiple dive computers per dive, switchable, with an overlay comparison
- [x] Bühlmann tissue model with gradient factors (ceiling, NDL),
      validated against Subsurface's planner: identical first ceilings and
      matching stop tables
- [x] Ascent planner with decompression stops and a saveable profile
- [x] Multi-waypoint planner: add a dive planner point per segment, each with
      depth, duration, gas and dive mode; per-point run time and gas used
- [x] VPM-B algorithm, validated against Subsurface's planner CLI: identical
      first ceilings and stop tables (conservatism 0-4, nitrox and trimix);
      35 of 37 reference plans match exactly, the rest differ by a single
      one-minute stop boundary
- [x] Open-circuit gas needs (RMV-based)
- [x] CCR planning (diluent + setpoint, loop gas, CCR schedule)
- [x] Bailout / ascent gas requirement
- [x] pSCR planning (Subsurface loop model, breathing-mode selector)
- [x] Dive comparison: overlay two dives (a saved plan vs the actual dive)
- [x] Save plans as dives
- [x] Gas calculations: MOD, END, EAD, best mix and ICD warnings
- [x] Gas calculations: SAC/RMV

## Phase 3 — The import ecosystem

- [ ] Dive computer download via `libdivecomputer`
      (desktop: serial/USB/Bluetooth; web: WebSerial + Web Bluetooth where available).
      Done on desktop (serial/USB/USB-HID/classic Bluetooth + BLE via BlueZ)
      and on the web (Web Serial + Web Bluetooth via wasm + Asyncify)
- [ ] Device management, firmware/settings, dive computer nicknames
- [x] CSV import (one dive per row, header-based column mapping)
- [ ] CSV import with user-defined column mapping (Subsurface templates)
- [x] GPX import (dive sites from waypoints and tracks)
- [ ] Additional importers: UDDF, DL7, Cobalt, Shearwater, Suunto,
      Diving Log, SeaBear, Cochran, ...
- [ ] Subsurface cloud storage and git-backed logs
- [ ] CSV/HTML/JSON bulk export; Subsurface-compatible export options

## Phase 4 — Visualization, media and statistics

- [ ] Statistics view: histograms, box plots, scatter plots, regression
- [ ] Charts of consumption, temperature, depth, SAC over time
- [x] Map view of dive sites (Leaflet) with clustering, satellite/street layers
- [ ] Photo/video gallery; EXIF time sync to dive timeline
- [ ] Printable logs and export to HTML/PDF (Subsurface templates)
- [ ] Year-in-review / summary dashboards
- [ ] World-map (visited countries) export

## Phase 5 — Platform polish

- [x] IndexedDB storage for large logs and migration from the old localStorage log
- [x] Opt-in remote sync: mirror the log to a Git repository (GitHub, or any
      compatible API) or Google Drive, with push/pull/conflict handling. Both
      run on the web and desktop, and the backend registry makes new services
      (OneDrive, FTP, ...) additive
- [ ] PWA: offline install, update prompts, file-handling API for `.ssrf`
- [ ] Mobile targets (Dioxus iOS/Android)
- [x] Internationalization with a typed string table and a French translation
      (a light in-house layer: one `Strings` table per language, persisted in
      preferences and applied to the `<html lang>` attribute)
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

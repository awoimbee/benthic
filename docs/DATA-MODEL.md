# Data model

`benthic-core` models a dive log as plain, serializable data. There are no
pointers or ownership tricks: dives reference trips and sites by id. This keeps
the model easy to serialize, diff and (eventually) apply commands to.

## Core entities

| benthic type | Subsurface equivalent | Notes |
| --- | --- | --- |
| `DiveLog` | `divelog` | Root document: dives, trips, sites, devices, settings |
| `Dive` | `struct dive` | One dive; references `trip_id` / `site_id` |
| `DiveComputer` | `struct divecomputer` | One recording device; owns samples/events |
| `Sample` | `struct sample` | A single profile point |
| `Event` | `struct event` | Gas switch, mode change, marker, alarm |
| `Cylinder` | `struct cylinder_t` | Tank + gas + start/end pressure |
| `WeightSystem` | `struct weightsystem_t` | Weight + description |
| `DiveSite` | `struct dive_site` | Named location with optional GPS |
| `DiveTrip` | `struct dive_trip` | Group of dives |
| `Device` | `struct device` | Known dive computer (nickname/serial) |
| `Picture` | `struct picture` | Photo/video + time offset |
| `GasMix` | `struct gasmix` | O2/He in permille |
| `SensorPressure` | `sample.pressure[]` + `sample.sensor[]` | Per-sensor tank pressure |

## Units

All physical quantities are integer newtypes. Conversions to and from
human-readable values live in `units.rs`.

| Type | Base unit | Constructors | Accessors |
| --- | --- | --- | --- |
| `Duration` | seconds | `from_minutes` | `hours` |
| `Depth` | millimetres | `from_meters`, `from_feet` | `meters`, `feet` |
| `Pressure` | millibar | `from_bar`, `from_psi` | `bar`, `psi` |
| `O2Pressure` | millibar (u16) | `from_bar` | `bar` |
| `Temperature` | millikelvin (u32) | `from_celsius`, `from_fahrenheit` | `celsius`, `fahrenheit` |
| `Volume` | millilitres | `from_liters`, `from_cubic_feet` | `liters` |
| `Fraction` | permille | `from_percent` | `percent` |
| `Weight` | grams | `from_kg`, `from_lbs` | `kg` |
| `Bearing` | degrees | `new` | `degrees` |
| `Location` | decimal degrees (f64) | `new` | `lat`, `lon` |
| `Timestamp` | Unix seconds (i64) | — | — |

Rationale: integer base units avoid floating-point drift (the reason
Subsurface does the same), while the constructor/accessor API keeps call sites
readable and hard to mix up.

## Identifiers

* `Dive::id`, `DiveTrip::id`, `DiveSite::uuid` and `Device::device_id` are
  `u32`.
* Import/merge renumbers ids so logs can be combined without collisions
  (`DiveLog::merge`).
* Subsurface cross-references dive sites with a uuid and dive computers with
  hashed device/dive ids; we preserve those where the format provides them.

## Relationships

```
DiveLog
├── dives:   Vec<Dive>       ── site_id ──┐
├── trips:   Vec<DiveTrip>   ◄─ trip_id ──┤
├── sites:   Vec<DiveSite>   ◄────────────┘
└── devices: Vec<Device>

Dive
├── cylinders: Vec<Cylinder>
├── weights:   Vec<WeightSystem>
├── computers: Vec<DiveComputer>  (one or more)
│   ├── samples: Vec<Sample>
│   └── events:  Vec<Event>
└── pictures:  Vec<Picture>
```

## Derived values

`Dive::duration()` and `Dive::max_depth()` fall back to the best value across
all dive computers when the dive-level value is absent. `DiveLog::fixup_all()`
recomputes these after import. Aggregates such as OTU/CNS/SAC and deco
information are not yet stored; they belong to the deco engine (Phase 2).

## Compatibility notes

The SSRF reader synthesizes a `DiveSite` from an inline `<location>` element
when a log predates explicit dive-site uuids. Subsurface similarly deduplicates
sites, but by GPS proximity; benthic currently creates one synthetic site per
distinct inline location. Improving this is tracked in
[ROADMAP.md](../ROADMAP.md) Phase 1.

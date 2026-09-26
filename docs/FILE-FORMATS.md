# File formats

benthic supports two formats today and is designed to grow more.

| Format | Extension | Read | Write | Purpose |
| --- | --- | --- | --- | --- |
| Native JSON | `.json`, `.benthic.json` | ✅ | ✅ | Lossless autosave + interchange |
| Subsurface XML | `.ssrf`, `.xml` | ✅ | ✅ | Interop with Subsurface |

Format is auto-detected on import
(`benthic_core::io::Format::detect`): leading `{` means JSON; an XML document
containing `divelog`/`subsurface` means SSRF.

## Native JSON

The canonical, lossless representation: the `serde` encoding of `DiveLog`,
pretty-printed so it can be diffed in version control. This is what
`storage.rs` persists. Because it is a direct mapping of the model, it survives
round-trips exactly (`json_roundtrip_is_lossless` test).

Shape (abridged):

```json
{
  "version": 1,
  "autogroup": true,
  "dives": [
    {
      "id": 1,
      "number": 1,
      "when": 1715501700,
      "site_id": 1,
      "notes": "Buoyancy drills...",
      "rating": 4,
      "tags": ["reef", "training"],
      "duration": { "seconds": 2550 },
      "max_depth": { "mm": 18400 },
      "cylinders": [
        { "size": { "ml": 12000 }, "gas": { "o2_permille": 210, "he_permille": 0 },
          "start_pressure": { "mbar": 200000 }, "end_pressure": { "mbar": 60000 } }
      ],
      "computers": [
        {
          "model": "Shearwater Perdix",
          "device_id": 439041101,
          "samples": [
            { "time": { "seconds": 0 }, "depth": { "mm": 0 } },
            { "time": { "seconds": 300 }, "depth": { "mm": 12000 } }
          ],
          "events": [
            { "time": { "seconds": 720 }, "name": "Marker" }
          ]
        }
      ]
    }
  ],
  "trips": [ { "id": 1, "date": 1715501700, "location": "Dahab" } ],
  "sites": [ { "uuid": 1, "name": "Blue Hole", "location": { "lat": 28.5721, "lon": 34.5367 } } ],
  "devices": [ { "model": "Shearwater Perdix", "device_id": 439041101 } ]
}
```

Notes:

* Units are integer base units, so the JSON is exact and unambiguous.
* `Option` fields are omitted when absent; empty collections are omitted.
* Air is `{ "o2_permille": 210, "he_permille": 0 }` (explicit), never "0 = air".

## Subsurface XML (`.ssrf`)

`io::ssrf` reads `program='subsurface' version='2'` documents and writes
compatible XML. The reader is tolerant of unknown elements/attributes; the
writer emits fully-specified elements.

### Delta-encoded samples

Subsurface only writes a sample attribute when it changes, so the reader
maintains a carried-forward `SampleState`:

```xml
<sample time='0:00 min' depth='0.0 m' temp='26.0 C' pressure0='200.0 bar' />
<sample time='5:00 min' depth='12.0 m' pressure0='180.0 bar' />
<!-- temperature is still 26.0 C here; omitted because unchanged -->
```

benthic writes every sample in full. Subsurface accepts this, and it keeps the
writer simple and obviously correct.

### Element support

| SSRF element | Read | Write | Notes |
| --- | --- | --- | --- |
| `<settings>`, `<divecomputerid>`, `<autogroup>` | ✅ | ✅ | device nicknames/serials/firmware |
| `<divesites>`, `<site>`, `<geo>` | ✅ | ✅ | ocean/country taxonomy preserved |
| `<trip>` | ✅ | ✅ | date/time/location/notes |
| `<dive>` attributes | ✅ | ✅ | number, tags, ratings, divesiteid, duration, salinity, air pressure |
| `<location>`, `<divemaster>`, `<buddy>`, `<notes>`, `<suit>` | ✅ | ✅ | inline location becomes a site on read |
| `<cylinder>`, `<weightsystem>` | ✅ | ✅ | size, working pressure, gas, start/end, use |
| `<divetemperature>` | ✅ | ✅ | |
| `<divecomputer>` + attrs | ✅ | ✅ | model, deviceid, diveid, serial/fw, dctype, no_o2sensors |
| `<depth>`, `<temperature>`, `<surface>`, `<water>` | ✅ | ✅ | |
| `<sample>` | ✅ | ✅ | pressure{N}/sensor{N}, po2, ndl/tts/rbt, stop, cns, heartbeat, bearing, in_deco |
| `<event>` | ✅ | ✅ | gaschange (cylinder/o2/he), modechange |
| `<extradata>` | ✅ | ✅ | |
| `<picture>` | ✅ | ✅ | filename/offset/hash/location |
| `<tanksensormapping>` | ⬜ | ⬜ | planned |
| `<filterpresets>` | ⬜ | ⬜ | planned (Phase 1) |
| fingerprints, git metadata | ⬜ | ⬜ | planned (Phase 3) |

### Formatting rules

To interop with Subsurface we mirror its number formatting:

* `put_milli` style: integer part + up to three fractional digits with trailing
  zeros trimmed (`10970 mm` → `10.97 m`, `13399 ml` → `13.399 l`).
* Durations as total `MM:SS` (so 73 minutes is `73:56`, not `1:13:56`).
* Durations/offsets tolerate `MM:SS`, `H:MM:SS` and a trailing `min` on read.

## Not yet supported (planned, see ROADMAP.md Phase 3)

* Subsurface **git storage** (`.git`-backed logs used by cloud sync)
* **UDDF**, **DL7**, **GPX**, **CSV** (Subsurface/Shearwater/Suunto/...),
  **Cobalt**, **Diving Log**, **SeaBear**, **Cochran** importers
* Dive-computer binary formats via **libdivecomputer**
* HTML/PDF/LaTeX export templates

Each new importer gets a golden-file test and a one-line entry in the support
matrix above.

//! Subsurface's git-storage tree format.
//!
//! Subsurface does not store XML inside its cloud/git storage. It uses a
//! directory hierarchy of small text blobs: `00-Subsurface` for settings,
//! `01-Divesites/Site-<uuid>` for sites, and dated directories containing
//! `Dive-*`, `Divecomputer-*` and `00-Trip` blobs for dives and trips.
//!
//! This module converts between that tree (a path -> bytes map) and the domain
//! model. It deliberately reads and writes the human-readable subset of the
//! format used by current Subsurface releases; unknown root entries can be
//! carried over by the transport so syncing does not delete data benthic does
//! not understand.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::gas::{GasMix, AIR};
use crate::model::*;
use crate::units::*;
use crate::Result;

type Files = BTreeMap<String, Vec<u8>>;

/// Parse a Subsurface git-storage tree into a [`DiveLog`].
pub fn parse(files: &Files) -> Result<DiveLog> {
    let mut log = DiveLog::new();

    if let Some(settings) = files.get("00-Subsurface") {
        parse_settings(&text(settings), &mut log);
    }

    for (path, blob) in files {
        if let Some(name) = path.strip_prefix("01-Divesites/") {
            if let Some(uuid) = name
                .strip_prefix("Site-")
                .and_then(|s| u32::from_str_radix(s, 16).ok())
            {
                let mut site = parse_site(&text(blob));
                site.uuid = uuid;
                log.sites.push(site);
            }
        }
    }
    log.sites.sort_by_key(|s| s.uuid);

    // Trips are identified by their directory; remember the id and date.
    let mut trip_dirs: Vec<(String, u32, Timestamp)> = Vec::new();
    let mut next_trip = 1u32;
    for (path, blob) in files {
        if path.ends_with("00-Trip") {
            let dir = path
                .trim_end_matches("00-Trip")
                .trim_end_matches('/')
                .to_string();
            let mut trip = parse_trip(&text(blob));
            trip.id = next_trip;
            next_trip += 1;
            if trip.date.is_none() {
                trip.date = files
                    .iter()
                    .filter(|(p, _)| p.starts_with(&format!("{dir}/")))
                    .filter_map(|(p, _)| dive_path_datetime(p).map(|(when, _)| when))
                    .min();
            }
            trip_dirs.push((dir, trip.id, trip.date.unwrap_or(0)));
            log.trips.push(trip);
        }
    }
    trip_dirs.sort_by_key(|entry| std::cmp::Reverse(entry.0.len()));

    let mut dives: Vec<(Timestamp, Dive)> = Vec::new();
    for (path, blob) in files {
        let Some(file_name) = path.rsplit('/').next() else {
            continue;
        };
        if !file_name.starts_with("Dive") || file_name.starts_with("Divecomputer") {
            continue;
        }
        let Some((when, number)) = dive_path_datetime(path) else {
            continue;
        };
        let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        let mut dive = parse_dive_file(&text(blob));
        dive.when = when;
        dive.number = number;
        dive.id = 0;
        dive.trip_id = trip_dirs
            .iter()
            .find(|(dir, _, _)| parent == dir || parent.starts_with(&format!("{dir}/")))
            .map(|(_, id, _)| *id);

        let mut computers: Vec<(String, DiveComputer)> = files
            .iter()
            .filter(|(p, _)| {
                let (dir, name) = match p.rsplit_once('/') {
                    Some(parts) => parts,
                    None => ("", p.as_str()),
                };
                dir == parent && name.starts_with("Divecomputer")
            })
            .map(|(p, b)| (p.clone(), parse_divecomputer(&text(b))))
            .collect();
        computers.sort_by(|a, b| a.0.cmp(&b.0));
        dive.computers = computers.into_iter().map(|(_, dc)| dc).collect();
        if dive.computers.is_empty() {
            dive.computers.push(DiveComputer {
                model: "Manually entered".to_string(),
                ..Default::default()
            });
        }
        let picture_prefix = format!("{parent}/Pictures/");
        let mut pictures: Vec<(String, Picture)> = files
            .iter()
            .filter_map(|(path, blob)| {
                let name = path.strip_prefix(&picture_prefix)?;
                let offset = parse_picture_offset(name)?;
                let mut picture = parse_picture(&text(blob));
                picture.offset = Some(offset);
                Some((path.clone(), picture))
            })
            .collect();
        pictures.sort_by(|a, b| a.0.cmp(&b.0));
        dive.pictures = pictures.into_iter().map(|(_, picture)| picture).collect();
        dives.push((when, dive));
    }

    dives.sort_by_key(|(when, _)| *when);
    for (index, (_, dive)) in dives.into_iter().enumerate() {
        let mut dive = dive;
        dive.id = index as u32 + 1;
        log.dives.push(dive);
    }
    log.fixup_all();
    Ok(log)
}

/// Serialize a [`DiveLog`] as a Subsurface git-storage tree.
pub fn write(log: &DiveLog) -> Files {
    let mut files = Files::new();
    files.insert(
        "00-Subsurface".to_string(),
        write_settings(log).into_bytes(),
    );

    for site in &log.sites {
        if site.name.is_empty()
            && site.description.is_empty()
            && site.notes.is_empty()
            && site.location.is_none()
        {
            continue;
        }
        files.insert(
            format!("01-Divesites/Site-{:08x}", site.uuid),
            write_site(site).into_bytes(),
        );
    }

    let mut used_dirs: std::collections::HashSet<String> = Default::default();
    let mut trip_ids: Vec<u32> = log.trips.iter().map(|t| t.id).collect();
    trip_ids.sort_unstable();
    for trip_id in trip_ids {
        let Some(trip) = log.trip_by_id(trip_id) else {
            continue;
        };
        let mut trip_dives: Vec<&Dive> = log
            .dives
            .iter()
            .filter(|d| d.trip_id == Some(trip_id))
            .collect();
        trip_dives.sort_by_key(|d| d.when);
        let date = trip
            .date
            .or_else(|| trip_dives.first().map(|d| d.when))
            .unwrap_or(0);
        let dt = utc(date);
        let root = format!("{:04}/{:02}", dt.0, dt.1);
        let mut name = format!("{:02}-{}", dt.2, trip_slug(&trip.location));
        let unique = format!("{name}~{trip_id:04x}");
        if !used_dirs.insert(format!("{root}/{unique}")) {
            name = unique.to_string();
        }
        let dir = format!("{root}/{unique}");
        files.insert(format!("{dir}/00-Trip"), write_trip(trip, dt).into_bytes());
        for dive in trip_dives {
            write_one_dive(&mut files, &dir, dive);
        }
    }

    let mut loose: Vec<&Dive> = log.dives.iter().filter(|d| d.trip_id.is_none()).collect();
    loose.sort_by_key(|d| d.when);
    for dive in loose {
        let dt = utc(dive.when);
        let root = format!("{:04}/{:02}", dt.0, dt.1);
        let mut dir = format!(
            "{root}/{}-{}-{:02}={:02}={:02}",
            dt.2,
            weekday(dt.6),
            dt.3,
            dt.4,
            dt.5
        );
        if !used_dirs.insert(dir.clone()) {
            dir.push_str(&format!("~{}", dive.id));
        }
        write_one_dive(&mut files, &dir, dive);
    }
    files
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn write_one_dive(files: &mut Files, parent: &str, dive: &Dive) {
    let dt = utc(dive.when);
    let name = format!(
        "{:04}-{:02}-{:02}-{}-{:02}={:02}={:02}",
        dt.0,
        dt.1,
        dt.2,
        weekday(dt.6),
        dt.3,
        dt.4,
        dt.5
    );
    let dir = format!("{parent}/{name}");
    files.insert(
        format!("{dir}/Dive-{}", dive.number),
        write_dive_file(dive).into_bytes(),
    );
    for (index, dc) in dive.computers.iter().enumerate() {
        let suffix = if dive.computers.len() > 1 {
            format!("-{:03}", index + 1)
        } else {
            String::new()
        };
        files.insert(
            format!("{dir}/Divecomputer{suffix}"),
            write_divecomputer(dc, dive).into_bytes(),
        );
    }
    let mut used_picture_names = std::collections::BTreeSet::new();
    for picture in &dive.pictures {
        let offset = picture.offset.unwrap_or_default().seconds;
        let sign = if offset < 0 { '-' } else { '+' };
        let offset = offset.unsigned_abs();
        let base = format!(
            "{sign}{:02}={:02}={:02}",
            offset / 3600,
            (offset % 3600) / 60,
            offset % 60
        );
        // Pictures without an offset all map to +00=00=00. Links commonly
        // have no offset, so keep a unique suffix rather than overwriting.
        let mut name = base.clone();
        let mut suffix = 1;
        while !used_picture_names.insert(name.clone()) {
            name = format!("{base}~{suffix}");
            suffix += 1;
        }
        files.insert(
            format!("{dir}/Pictures/{name}"),
            write_picture(picture).into_bytes(),
        );
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

fn parse_settings(text: &str, log: &mut DiveLog) {
    for line in text.lines() {
        let tokens = tokens(line);
        let Some(key) = tokens.first().map(String::as_str) else {
            continue;
        };
        match key {
            "autogroup" => log.autogroup = true,
            "divecomputerid" => {
                let kv = key_values(&tokens[1..]);
                log.devices.push(Device {
                    model: tokens.get(1).cloned().unwrap_or_default(),
                    device_id: kv
                        .get("deviceid")
                        .and_then(|v| u32::from_str_radix(v, 16).ok())
                        .unwrap_or(0),
                    nickname: kv.get("nickname").cloned(),
                    serial: kv.get("serial").cloned(),
                    firmware: kv.get("firmware").cloned(),
                });
            }
            _ => {}
        }
    }
}

fn parse_site(text: &str) -> DiveSite {
    let mut site = DiveSite::default();
    for line in text.lines() {
        let tokens = tokens(line);
        let Some(key) = tokens.first().map(String::as_str) else {
            continue;
        };
        match key {
            "name" => site.name = tokens.get(1).cloned().unwrap_or_default(),
            "description" => site.description = tokens.get(1).cloned().unwrap_or_default(),
            "notes" => site.notes = tokens.get(1).cloned().unwrap_or_default(),
            "gps" => {
                if let (Some(lat), Some(lon)) = (
                    tokens.get(1).and_then(|v| v.parse::<f64>().ok()),
                    tokens.get(2).and_then(|v| v.parse::<f64>().ok()),
                ) {
                    site.location = Some(Location::new(lat, lon));
                }
            }
            "geo" => {
                let kv = key_values(&tokens[1..]);
                let cat = kv
                    .get("cat")
                    .and_then(|v| v.parse::<u8>().ok())
                    .unwrap_or(0);
                let value = tokens.last().cloned().unwrap_or_default();
                match cat {
                    1 => site.ocean = Some(value),
                    2 => site.country = Some(value),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    site
}

fn parse_trip(text: &str) -> DiveTrip {
    let mut trip = DiveTrip::default();
    let mut date = None;
    let mut time = None;
    for line in text.lines() {
        let tokens = tokens(line);
        let Some(key) = tokens.first().map(String::as_str) else {
            continue;
        };
        match key {
            "date" => date = tokens.get(1).cloned(),
            "time" => time = tokens.get(1).cloned(),
            "location" => trip.location = tokens.get(1).cloned().unwrap_or_default(),
            "notes" => trip.notes = tokens.get(1).cloned().unwrap_or_default(),
            _ => {}
        }
    }
    if let (Some(date), Some(time)) = (date, time) {
        trip.date = parse_date_time(&date, &time);
    }
    trip
}

fn parse_dive_file(text: &str) -> Dive {
    let mut dive = Dive::default();
    for line in text.lines() {
        let tokens = tokens(line);
        let Some(key) = tokens.first().map(String::as_str) else {
            continue;
        };
        let value = tokens.get(1).map(String::as_str).unwrap_or("");
        match key {
            "duration" => dive.duration = parse_duration(value),
            "rating" => dive.rating = value.parse().unwrap_or(0),
            "visibility" => dive.visibility = value.parse().unwrap_or(0),
            "wavesize" => dive.wavesize = value.parse().unwrap_or(0),
            "current" => dive.current = value.parse().unwrap_or(0),
            "surge" => dive.surge = value.parse().unwrap_or(0),
            "chill" => dive.chill = value.parse().unwrap_or(0),
            "watersalinity" => dive.salinity = parse_salinity(value),
            "airpressure" => {
                dive.surface_pressure =
                    parse_pressure(value).or_else(|| value.parse().ok().map(Pressure::new))
            }
            "notrip" => dive.no_trip = true,
            "tags" => {
                dive.tags = tokens[1..]
                    .iter()
                    .flat_map(|t| t.split(','))
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect();
            }
            "divesiteid" => dive.site_id = u32::from_str_radix(value, 16).ok(),
            "divemaster" | "diveguide" => dive.diveguide = value.to_string(),
            "buddy" => dive.buddy = value.to_string(),
            "suit" => dive.suit = value.to_string(),
            "notes" => dive.notes = value.to_string(),
            "cylinder" => dive.cylinders.push(parse_cylinder(&tokens[1..])),
            "weightsystem" => dive.weights.push(parse_weight(&tokens[1..])),
            "airtemp" => dive.air_temp = parse_temperature(value),
            "watertemp" => dive.water_temp = parse_temperature(value),
            _ => {}
        }
    }
    dive
}

fn parse_cylinder(tokens: &[String]) -> Cylinder {
    let kv = key_values(tokens);
    let o2 = kv.get("o2").and_then(|v| parse_percent(v));
    let he = kv.get("he").and_then(|v| parse_percent(v));
    Cylinder {
        size: kv
            .get("vol")
            .and_then(|v| v.trim_end_matches(['l', 'L']).parse::<f64>().ok())
            .map(Volume::from_liters),
        working_pressure: kv.get("workpressure").and_then(|v| parse_pressure(v)),
        description: kv.get("description").cloned().unwrap_or_default(),
        gas: match (o2, he) {
            (Some(o2), he) if o2 > 0 => GasMix::new(o2, he.unwrap_or(0)),
            _ => AIR,
        },
        start_pressure: kv.get("start").and_then(|v| parse_pressure(v)),
        end_pressure: kv.get("end").and_then(|v| parse_pressure(v)),
        use_: kv
            .get("use")
            .map(|v| parse_cylinder_use(v))
            .unwrap_or_default(),
        depth: kv.get("depth").and_then(|v| parse_depth(v)),
        ..Default::default()
    }
}

fn parse_weight(tokens: &[String]) -> WeightSystem {
    let kv = key_values(tokens);
    WeightSystem {
        weight: kv
            .get("weight")
            .and_then(|v| v.trim_end_matches(['k', 'g']).parse::<f64>().ok())
            .map(Weight::from_kg)
            .unwrap_or_default(),
        description: kv.get("description").cloned().unwrap_or_default(),
        auto_filled: false,
    }
}

fn parse_divecomputer(text: &str) -> DiveComputer {
    let mut dc = DiveComputer::default();
    let mut sample_state = SampleState::default();
    for line in text.lines() {
        let tokens = tokens(line);
        let Some(key) = tokens.first().map(String::as_str) else {
            continue;
        };
        let kv = key_values(&tokens[1..]);
        match key {
            "model" => dc.model = tokens.get(1).cloned().unwrap_or_default(),
            "deviceid" => {
                dc.device_id = tokens
                    .get(1)
                    .and_then(|v| u32::from_str_radix(v, 16).ok())
                    .unwrap_or(0)
            }
            "diveid" => {
                dc.dive_id = tokens
                    .get(1)
                    .and_then(|v| u32::from_str_radix(v, 16).ok())
                    .unwrap_or(0)
            }
            "dctype" => {
                dc.divemode = parse_divemode(tokens.get(1).map(String::as_str).unwrap_or(""))
            }
            "numberofoxygensensors" => {
                dc.no_o2_sensors = tokens.get(1).and_then(|v| v.parse().ok()).unwrap_or(0)
            }
            "duration" => dc.duration = tokens.get(1).and_then(|v| parse_duration(v)),
            "maxdepth" => dc.max_depth = tokens.get(1).and_then(|v| parse_depth(v)),
            "meandepth" => dc.mean_depth = tokens.get(1).and_then(|v| parse_depth(v)),
            "airtemp" => dc.air_temp = tokens.get(1).and_then(|v| parse_temperature(v)),
            "watertemp" => dc.water_temp = tokens.get(1).and_then(|v| parse_temperature(v)),
            "surfacepressure" => {
                dc.surface_pressure = tokens.get(1).and_then(|v| parse_pressure(v))
            }
            "salinity" => dc.salinity = tokens.get(1).and_then(|v| parse_salinity(v)),
            "keyvalue" => {
                if let (Some(k), Some(v)) = (tokens.get(1), tokens.get(2)) {
                    dc.extra_data.push((k.clone(), v.clone()));
                }
            }
            "event" => {
                if let Some(event) = parse_event(&tokens[1..], &kv) {
                    dc.events.push(event);
                }
            }
            _ => {
                if tokens[0].contains(':')
                    && tokens[0].chars().next().is_some_and(|c| c.is_ascii_digit())
                {
                    dc.samples.push(parse_sample(&tokens, &mut sample_state));
                }
            }
        }
    }
    if dc.duration.is_none() {
        dc.duration = dc.samples.last().map(|s| s.time);
    }
    dc
}

#[derive(Default)]
struct SampleState {
    temperature: Option<Temperature>,
    setpoint: Option<O2Pressure>,
    o2_sensors: Vec<O2Pressure>,
    ndl: Option<Duration>,
    tts: Option<Duration>,
    rbt: Option<Duration>,
    stop_time: Option<Duration>,
    stop_depth: Option<Depth>,
    cns: Option<u16>,
    heartbeat: Option<u8>,
    bearing: Option<Bearing>,
    in_deco: bool,
}

fn parse_sample(tokens: &[String], state: &mut SampleState) -> Sample {
    let time = parse_duration(&tokens[0]).unwrap_or_default();
    let mut depth = Depth::ZERO;
    let mut pressures: Vec<SensorPressure> = Vec::new();
    let mut next_sensor = 0i16;
    let mut temp = state.temperature;
    let mut ndl = state.ndl;
    let mut tts = state.tts;
    let mut rbt = state.rbt;
    let mut stop_time = state.stop_time;
    let mut stop_depth = state.stop_depth;
    let mut cns = state.cns;
    let mut heartbeat = state.heartbeat;
    let mut bearing = state.bearing;
    let mut in_deco = state.in_deco;
    let mut setpoint = state.setpoint;
    let mut sensors = state.o2_sensors.clone();

    for token in &tokens[1..] {
        if let Some((key, value)) = token.split_once('=') {
            match key {
                "ndl" => ndl = parse_duration(value),
                "tts" => tts = parse_duration(value),
                "rbt" => rbt = parse_duration(value),
                "stoptime" => stop_time = parse_duration(value),
                "stopdepth" => stop_depth = parse_depth(value),
                "cns" => cns = value.trim_end_matches('%').parse().ok(),
                "heartbeat" => heartbeat = value.parse().ok(),
                "bearing" => bearing = value.trim_end_matches('°').parse().ok().map(Bearing::new),
                "in_deco" => in_deco = value == "1",
                "po2" => {
                    setpoint = parse_pressure(value)
                        .map(|p| O2Pressure::new(p.mbar.clamp(0, u16::MAX as i32) as u16))
                }
                _ if key.starts_with("sensor") => {
                    if let Ok(index) = key[6..].parse::<usize>() {
                        if let Some(p) = parse_pressure(value) {
                            while sensors.len() < index {
                                sensors.push(O2Pressure::default());
                            }
                            sensors[index - 1] =
                                O2Pressure::new(p.mbar.clamp(0, u16::MAX as i32) as u16);
                        }
                    }
                }
                "dc_supplied_ppo2" => {
                    if let Some(p) = parse_pressure(value) {
                        sensors.push(O2Pressure::new(p.mbar.clamp(0, u16::MAX as i32) as u16));
                    }
                }
                _ => {}
            }
        } else if let Some(c) = token.find(['m']) {
            if token[..c].parse::<f64>().is_ok() {
                depth = parse_depth(token).unwrap_or(depth);
            }
        } else if token.ends_with("bar") {
            let (number, sensor) = match token.split_once(':') {
                Some((n, s)) => (n, s.parse::<i16>().unwrap_or(next_sensor)),
                None => (token.as_str(), next_sensor),
            };
            if let Some(p) = parse_pressure(number) {
                pressures.push(SensorPressure {
                    sensor,
                    pressure: p,
                });
                next_sensor = sensor + 1;
            }
        } else if token.ends_with('°') || token.ends_with("°C") {
            temp = parse_temperature(token);
        }
    }

    state.temperature = temp;
    state.setpoint = setpoint;
    state.o2_sensors = sensors.clone();
    state.ndl = ndl;
    state.tts = tts;
    state.rbt = rbt;
    state.stop_time = stop_time;
    state.stop_depth = stop_depth;
    state.cns = cns;
    state.heartbeat = heartbeat;
    state.bearing = bearing;
    state.in_deco = in_deco;

    Sample {
        time,
        depth,
        temperature: temp,
        pressures,
        setpoint,
        o2_sensors: sensors,
        ndl,
        tts,
        rbt,
        stop_time,
        stop_depth,
        cns,
        heartbeat,
        bearing,
        in_deco,
        manually_entered: false,
    }
}

fn parse_event(tokens: &[String], kv: &BTreeMap<String, String>) -> Option<Event> {
    let time = tokens
        .first()
        .and_then(|v| parse_duration(v))
        .unwrap_or_default();
    let name = kv.get("name").cloned().unwrap_or_default();
    let gas = if name == "gaschange" {
        let o2 = kv.get("o2").and_then(|v| parse_percent(v)).unwrap_or(210);
        let he = kv.get("he").and_then(|v| parse_percent(v)).unwrap_or(0);
        let index = kv
            .get("cylinder")
            .and_then(|v| v.parse().ok())
            .unwrap_or(-1);
        Some((index, GasMix::new(o2, he)))
    } else {
        None
    };
    Some(Event {
        time,
        name,
        flags: kv.get("flags").and_then(|v| v.parse().ok()).unwrap_or(0),
        value: kv.get("value").and_then(|v| v.parse().ok()).unwrap_or(0),
        divemode: kv.get("divemode").map(|v| parse_divemode(v)),
        gas,
        hidden: false,
    })
}

fn parse_picture(text: &str) -> Picture {
    let mut picture = Picture::default();
    for line in text.lines() {
        let tokens = tokens(line);
        let Some(key) = tokens.first().map(String::as_str) else {
            continue;
        };
        match key {
            "filename" => picture.filename = tokens.get(1).cloned().unwrap_or_default(),
            "gps" => {
                if let (Some(lat), Some(lon)) = (
                    tokens.get(1).and_then(|v| v.parse::<f64>().ok()),
                    tokens.get(2).and_then(|v| v.parse::<f64>().ok()),
                ) {
                    picture.location = Some(Location::new(lat, lon));
                }
            }
            "hash" => picture.hash = tokens.get(1).cloned(),
            _ => {}
        }
    }
    picture
}

fn parse_picture_offset(name: &str) -> Option<Duration> {
    let name = name.split('~').next().unwrap_or(name);
    let mut chars = name.chars();
    let sign = chars.next()?;
    if sign != '+' && sign != '-' {
        return None;
    }
    let clock: String = chars.collect();
    let mut parts = clock.split(['=', ':']);
    let hour: i32 = parts.next()?.parse().ok()?;
    let minute: i32 = parts.next()?.parse().ok()?;
    let second: i32 = parts.next()?.parse().ok()?;
    let seconds = hour * 3600 + minute * 60 + second;
    Some(Duration::new(if sign == '-' { -seconds } else { seconds }))
}

fn write_picture(picture: &Picture) -> String {
    let mut out = String::from("filename ");
    quote(&mut out, &picture.filename);
    out.push('\n');
    if let Some(location) = picture.location.filter(|l| l.is_valid()) {
        let _ = writeln!(out, "gps {:.6} {:.6}", location.lat, location.lon);
    }
    if let Some(hash) = &picture.hash {
        out.push_str("hash ");
        quote(&mut out, hash);
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

fn write_settings(log: &DiveLog) -> String {
    let mut out = String::from("version 3\n");
    for device in &log.devices {
        out.push_str("divecomputerid ");
        quote(&mut out, &device.model);
        let _ = write!(out, " deviceid={:08x}", device.device_id);
        if let Some(serial) = &device.serial {
            out.push_str(" serial=");
            quote(&mut out, serial);
        }
        if let Some(nickname) = &device.nickname {
            out.push_str(" nickname=");
            quote(&mut out, nickname);
        }
        if let Some(firmware) = &device.firmware {
            out.push_str(" firmware=");
            quote(&mut out, firmware);
        }
        out.push('\n');
    }
    if log.autogroup {
        out.push_str("autogroup\n");
    }
    out.push_str("units METRIC\n");
    out
}

fn write_site(site: &DiveSite) -> String {
    let mut out = String::new();
    if !site.name.is_empty() {
        out.push_str("name ");
        quote(&mut out, &site.name);
        out.push('\n');
    }
    if !site.description.is_empty() {
        out.push_str("description ");
        quote(&mut out, &site.description);
        out.push('\n');
    }
    if !site.notes.is_empty() {
        out.push_str("notes ");
        quote(&mut out, &site.notes);
        out.push('\n');
    }
    if let Some(location) = site.location.filter(|l| l.is_valid()) {
        let _ = writeln!(out, "gps {:.6} {:.6}", location.lat, location.lon);
    }
    if let Some(ocean) = &site.ocean {
        let _ = write!(out, "geo cat 1 origin 0 ");
        quote(&mut out, ocean);
        out.push('\n');
    }
    if let Some(country) = &site.country {
        let _ = write!(out, "geo cat 2 origin 0 ");
        quote(&mut out, country);
        out.push('\n');
    }
    out
}

fn write_trip(trip: &DiveTrip, dt: (i32, u8, u8, u8, u8, u8, u8)) -> String {
    let mut out = format!("date {:04}-{:02}-{:02}\n", dt.0, dt.1, dt.2);
    let _ = writeln!(out, "time {:02}:{:02}:{:02}", dt.3, dt.4, dt.5);
    if !trip.location.is_empty() {
        out.push_str("location ");
        quote(&mut out, &trip.location);
        out.push('\n');
    }
    if !trip.notes.is_empty() {
        out.push_str("notes ");
        quote(&mut out, &trip.notes);
        out.push('\n');
    }
    out
}

fn write_dive_file(dive: &Dive) -> String {
    let mut out = String::new();
    if let Some(duration) = dive.duration {
        let _ = writeln!(out, "duration {}", fmt_duration(duration));
    }
    for (key, value) in [
        ("rating", dive.rating),
        ("visibility", dive.visibility),
        ("wavesize", dive.wavesize),
        ("current", dive.current),
        ("surge", dive.surge),
        ("chill", dive.chill),
    ] {
        if value != 0 {
            let _ = writeln!(out, "{key} {value}");
        }
    }
    if let Some(salinity) = dive.salinity {
        let _ = writeln!(out, "watersalinity {salinity} g/l");
    }
    if let Some(pressure) = dive.surface_pressure {
        let _ = writeln!(out, "airpressure {}", pressure.mbar);
    }
    if dive.no_trip {
        out.push_str("notrip\n");
    }
    if !dive.tags.is_empty() {
        out.push_str("tags ");
        for (index, tag) in dive.tags.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            quote(&mut out, tag);
        }
        out.push('\n');
    }
    if let Some(site) = dive.site_id {
        let _ = writeln!(out, "divesiteid {:08x}", site);
    }
    for (key, value) in [
        ("divemaster", &dive.diveguide),
        ("buddy", &dive.buddy),
        ("suit", &dive.suit),
        ("notes", &dive.notes),
    ] {
        if !value.is_empty() {
            let _ = write!(out, "{key} ");
            quote(&mut out, value);
            out.push('\n');
        }
    }
    for cylinder in &dive.cylinders {
        out.push_str("cylinder");
        if let Some(size) = cylinder.size {
            let _ = write!(out, " vol={}l", fmt_milli(size.ml as i64));
        }
        if let Some(pressure) = cylinder.working_pressure {
            let _ = write!(out, " workpressure={}bar", fmt_milli(pressure.mbar as i64));
        }
        if !cylinder.description.is_empty() {
            out.push_str(" description=");
            quote(&mut out, &cylinder.description);
        }
        if !cylinder.gas.is_air() {
            if cylinder.gas.o2_permille > 0 {
                let _ = write!(
                    out,
                    " o2={}%",
                    fmt_milli(cylinder.gas.o2_permille as i64 / 10)
                );
            }
            if cylinder.gas.he_permille > 0 {
                let _ = write!(
                    out,
                    " he={}%",
                    fmt_milli(cylinder.gas.he_permille as i64 / 10)
                );
            }
        }
        if let Some(pressure) = cylinder.start_pressure {
            let _ = write!(out, " start={}bar", fmt_milli(pressure.mbar as i64));
        }
        if let Some(pressure) = cylinder.end_pressure {
            let _ = write!(out, " end={}bar", fmt_milli(pressure.mbar as i64));
        }
        if cylinder.use_ != CylinderUse::OcGas {
            let _ = write!(out, " use={}", cylinder_use_text(cylinder.use_));
        }
        if let Some(depth) = cylinder.depth {
            let _ = write!(out, " depth={}m", fmt_milli(depth.mm as i64));
        }
        out.push('\n');
    }
    for weight in &dive.weights {
        let _ = write!(
            out,
            "weightsystem weight={}kg description=",
            fmt_milli(weight.weight.grams as i64)
        );
        quote(&mut out, &weight.description);
        out.push('\n');
    }
    if let Some(temp) = dive.air_temp {
        let _ = writeln!(
            out,
            "airtemp {}°C",
            fmt_milli(temp.mkelvin as i64 - ZERO_CELSIUS_MKELVIN as i64)
        );
    }
    if let Some(temp) = dive.water_temp {
        let _ = writeln!(
            out,
            "watertemp {}°C",
            fmt_milli(temp.mkelvin as i64 - ZERO_CELSIUS_MKELVIN as i64)
        );
    }
    out
}

fn write_divecomputer(dc: &DiveComputer, dive: &Dive) -> String {
    let mut out = String::new();
    if !dc.model.is_empty() {
        out.push_str("model ");
        quote(&mut out, &dc.model);
        out.push('\n');
    }
    if dc.device_id != 0 {
        let _ = writeln!(out, "deviceid {:08x}", dc.device_id);
    }
    if dc.dive_id != 0 {
        let _ = writeln!(out, "diveid {:08x}", dc.dive_id);
    }
    if dc.divemode != Divemode::OpenCircuit {
        let _ = writeln!(out, "dctype {}", divemode_text(dc.divemode));
        let _ = writeln!(out, "numberofoxygensensors {}", dc.no_o2_sensors);
    }
    if let Some(duration) = dc.duration {
        if Some(duration) != dive.duration {
            let _ = writeln!(out, "duration {}", fmt_duration(duration));
        }
    }
    if let Some(depth) = dc.max_depth {
        let _ = writeln!(out, "maxdepth {}m", fmt_milli(depth.mm as i64));
    }
    if let Some(depth) = dc.mean_depth {
        let _ = writeln!(out, "meandepth {}m", fmt_milli(depth.mm as i64));
    }
    if let Some(temp) = dc.air_temp {
        let _ = writeln!(
            out,
            "airtemp {}°C",
            fmt_milli(temp.mkelvin as i64 - ZERO_CELSIUS_MKELVIN as i64)
        );
    }
    if let Some(temp) = dc.water_temp {
        let _ = writeln!(
            out,
            "watertemp {}°C",
            fmt_milli(temp.mkelvin as i64 - ZERO_CELSIUS_MKELVIN as i64)
        );
    }
    if let Some(pressure) = dc.surface_pressure {
        let _ = writeln!(
            out,
            "surfacepressure {}bar",
            fmt_milli(pressure.mbar as i64)
        );
    }
    if let Some(salinity) = dc.salinity {
        let _ = writeln!(out, "salinity {salinity}g/l");
    }
    for (key, value) in &dc.extra_data {
        out.push_str("keyvalue ");
        quote(&mut out, key);
        out.push(' ');
        quote(&mut out, value);
        out.push('\n');
    }
    for event in &dc.events {
        let _ = write!(out, "event {}", fmt_duration(event.time));
        if event.flags != 0 {
            let _ = write!(out, " flags={}", event.flags);
        }
        if let Some(mode) = event.divemode {
            let _ = write!(out, " divemode={}", divemode_text(mode));
        } else if event.value != 0 {
            let _ = write!(out, " value={}", event.value);
        }
        out.push_str(" name=");
        quote(&mut out, &event.name);
        if let Some((index, gas)) = event.gas {
            if index >= 0 {
                let _ = write!(out, " cylinder={index}");
            }
            if gas.o2_permille > 0 {
                let _ = write!(out, " o2={}%", fmt_milli(gas.o2_permille as i64 / 10));
                if gas.he_permille > 0 {
                    let _ = write!(out, " he={}%", fmt_milli(gas.he_permille as i64 / 10));
                }
            }
        }
        out.push('\n');
    }
    for sample in &dc.samples {
        let _ = write!(
            out,
            " {:>3}:{:02} {}",
            sample.time.seconds / 60,
            sample.time.seconds % 60,
            fmt_milli(sample.depth.mm as i64)
        );
        out.push('m');
        if let Some(temp) = sample.temperature {
            let _ = write!(
                out,
                " {}°C",
                fmt_milli(temp.mkelvin as i64 - ZERO_CELSIUS_MKELVIN as i64)
            );
        }
        for pressure in &sample.pressures {
            let _ = write!(out, " {}bar", fmt_milli(pressure.pressure.mbar as i64));
            if pressure.sensor != 0 {
                let _ = write!(out, ":{}", pressure.sensor);
            }
        }
        if let Some(ndl) = sample.ndl {
            let _ = write!(out, " ndl={}", fmt_duration(ndl));
        }
        if let Some(tts) = sample.tts {
            let _ = write!(out, " tts={}", fmt_duration(tts));
        }
        if sample.in_deco {
            out.push_str(" in_deco=1");
        }
        if let Some(stop_time) = sample.stop_time {
            let _ = write!(out, " stoptime={}", fmt_duration(stop_time));
        }
        if let Some(stop_depth) = sample.stop_depth {
            let _ = write!(out, " stopdepth={}m", fmt_milli(stop_depth.mm as i64));
        }
        if let Some(cns) = sample.cns {
            let _ = write!(out, " cns={cns}%");
        }
        if let Some(rbt) = sample.rbt {
            let _ = write!(out, " rbt={}", fmt_duration(rbt));
        }
        if let Some(setpoint) = sample.setpoint {
            let _ = write!(out, " po2={}bar", fmt_milli(setpoint.mbar as i64));
        }
        for (index, sensor) in sample.o2_sensors.iter().enumerate() {
            if index < 6 {
                let _ = write!(
                    out,
                    " sensor{}={}bar",
                    index + 1,
                    fmt_milli(sensor.mbar as i64)
                );
            } else {
                let _ = write!(
                    out,
                    " dc_supplied_ppo2={}bar",
                    fmt_milli(sensor.mbar as i64)
                );
            }
        }
        if let Some(heartbeat) = sample.heartbeat {
            let _ = write!(out, " heartbeat={heartbeat}");
        }
        if let Some(bearing) = sample.bearing {
            let _ = write!(out, " bearing={}°", bearing.degrees);
        }
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn quote(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Split a Subsurface line into tokens, decoding quoted strings.
fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if c == '"' {
            let mut token = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => match chars.next() {
                        Some('n') => token.push('\n'),
                        Some('t') => token.push('\t'),
                        Some(c) => token.push(c),
                        None => break,
                    },
                    c => token.push(c),
                }
            }
            out.push(token);
        } else {
            let mut token = String::from(c);
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                let c = chars.next().unwrap();
                if c == '"' {
                    while let Some(c) = chars.next() {
                        match c {
                            '"' => break,
                            '\\' => match chars.next() {
                                Some('n') => token.push('\n'),
                                Some('t') => token.push('\t'),
                                Some(c) => token.push(c),
                                None => break,
                            },
                            c => token.push(c),
                        }
                    }
                } else {
                    token.push(c);
                }
            }
            out.push(token);
        }
    }
    out
}

fn key_values(tokens: &[String]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for token in tokens {
        if let Some((key, value)) = token.split_once('=') {
            out.insert(key.to_string(), value.to_string());
        }
    }
    out
}

fn parse_date_time(date: &str, time: &str) -> Option<Timestamp> {
    let mut d = date.split('-');
    let year: i32 = d.next()?.parse().ok()?;
    let month = time::Month::try_from(d.next()?.parse::<u8>().ok()?).ok()?;
    let day: u8 = d.next()?.parse().ok()?;
    let mut parts = time.split([':', '=']);
    let hour: u8 = parts.next()?.parse().ok()?;
    let minute: u8 = parts.next()?.parse().ok()?;
    let second: u8 = parts.next().unwrap_or("0").parse().ok()?;
    let t = time::Time::from_hms(hour, minute, second).ok()?;
    Some(
        time::PrimitiveDateTime::new(time::Date::from_calendar_date(year, month, day).ok()?, t)
            .assume_utc()
            .unix_timestamp(),
    )
}

fn parse_duration(value: &str) -> Option<Duration> {
    let value = value.trim().trim_end_matches("min").trim();
    let negative = value.starts_with('-');
    let value = value.trim_start_matches(['+', '-']);
    let parts: Vec<i64> = value
        .split(':')
        .map(|p| p.trim().parse().unwrap_or(0))
        .collect();
    let seconds = match parts.as_slice() {
        [s] => *s,
        [m, s] => m * 60 + s,
        [h, m, s] => h * 3600 + m * 60 + s,
        _ => return None,
    };
    Some(Duration::new(if negative {
        -seconds as i32
    } else {
        seconds as i32
    }))
}

fn parse_depth(value: &str) -> Option<Depth> {
    parse_number_unit(value, 'm').map(Depth::from_meters)
}

fn parse_pressure(value: &str) -> Option<Pressure> {
    if let Some(v) = value.strip_suffix("bar") {
        return v.trim().parse::<f64>().ok().map(Pressure::from_bar);
    }
    if let Some(v) = value.strip_suffix("psi") {
        return v.trim().parse::<f64>().ok().map(Pressure::from_psi);
    }
    None
}

fn parse_temperature(value: &str) -> Option<Temperature> {
    if let Some(v) = value.strip_suffix("°C") {
        return v.trim().parse::<f64>().ok().map(Temperature::from_celsius);
    }
    if let Some(v) = value.strip_suffix("°F") {
        return v
            .trim()
            .parse::<f64>()
            .ok()
            .map(Temperature::from_fahrenheit);
    }
    None
}

fn parse_number_unit(value: &str, unit: char) -> Option<f64> {
    value
        .trim()
        .trim_end_matches('°')
        .trim_end_matches(unit)
        .trim_end_matches('C')
        .trim_end_matches('F')
        .parse()
        .ok()
}

fn parse_percent(value: &str) -> Option<u16> {
    let value = value.trim().trim_end_matches('%');
    value.parse::<f64>().ok().map(|v| (v * 10.0).round() as u16)
}

fn parse_salinity(value: &str) -> Option<i32> {
    let number = value.trim().trim_end_matches("g/l").trim();
    if number.contains('.') {
        number
            .parse::<f64>()
            .ok()
            .map(|v| (v * 10.0).round() as i32)
    } else {
        number.parse().ok()
    }
}

fn parse_divemode(value: &str) -> Divemode {
    match value.trim().to_ascii_lowercase().as_str() {
        "ccr" | "cc" => Divemode::Ccr,
        "pscr" => Divemode::Pscr,
        "freedive" => Divemode::Freedive,
        _ => Divemode::OpenCircuit,
    }
}

fn divemode_text(mode: Divemode) -> &'static str {
    match mode {
        Divemode::OpenCircuit => "OC",
        Divemode::Ccr => "CCR",
        Divemode::Pscr => "PSCR",
        Divemode::Freedive => "Freedive",
    }
}

fn parse_cylinder_use(value: &str) -> CylinderUse {
    match value.to_ascii_lowercase().as_str() {
        "diluent" => CylinderUse::Diluent,
        "oxygen" => CylinderUse::Oxygen,
        "not used" | "not-used" => CylinderUse::NotUsed,
        "travel gas" | "travel" | "travel-gas" => CylinderUse::TravelOc,
        _ => CylinderUse::OcGas,
    }
}

fn cylinder_use_text(use_: CylinderUse) -> &'static str {
    match use_ {
        CylinderUse::OcGas => "OC-gas",
        CylinderUse::Diluent => "diluent",
        CylinderUse::Oxygen => "oxygen",
        CylinderUse::NotUsed => "not used",
        CylinderUse::TravelOc => "Travel Gas",
    }
}

fn fmt_duration(duration: Duration) -> String {
    let seconds = duration.seconds.max(0);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn fmt_milli(value: i64) -> String {
    let negative = value < 0;
    let abs = value.unsigned_abs();
    let mut frac = format!("{:03}", abs % 1000);
    while frac.ends_with('0') {
        frac.pop();
    }
    if frac.is_empty() {
        frac.push('0');
    }
    format!("{}{}.{}", if negative { "-" } else { "" }, abs / 1000, frac)
}

fn utc(timestamp: Timestamp) -> (i32, u8, u8, u8, u8, u8, u8) {
    let dt = time::OffsetDateTime::from_unix_timestamp(timestamp)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let date = dt.date();
    let weekday = date.weekday().number_days_from_sunday();
    (
        date.year(),
        u8::from(date.month()),
        date.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
        weekday,
    )
}

fn weekday(index: u8) -> &'static str {
    ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][index as usize % 7]
}

fn trip_slug(location: &str) -> String {
    let mut out = String::new();
    for c in location.chars() {
        if out.len() >= 15 {
            break;
        }
        if c == ',' || c == '.' {
            break;
        }
        if c.is_ascii_alphabetic() {
            out.push(c);
        }
    }
    if out.len() > 1 {
        out
    } else {
        "trip".to_string()
    }
}

/// Extract `(timestamp, dive_number)` from a dive file path.
fn dive_path_datetime(path: &str) -> Option<(Timestamp, i32)> {
    let (parent, file) = path.rsplit_once('/')?;
    let dir_name = parent.rsplit('/').next().unwrap_or(parent);
    let number = file
        .strip_prefix("Dive-")
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);
    let when = parse_dir_datetime(dir_name, parent)
        .or_else(|| parse_dir_datetime(file, parent))?
        .0;
    Some((when, number))
}

/// Parse a Subsurface dated directory/file name. `context` is the full parent
/// path, used for the year/month when the name only contains a day.
fn parse_dir_datetime(name: &str, context: &str) -> Option<(Timestamp, ())> {
    let name = name.split('~').next().unwrap_or(name);
    let mut parts: Vec<&str> = name.split('-').collect();
    let time_part = parts.pop()?;
    if !time_part.contains('=') && !time_part.contains(':') {
        return None;
    }
    let _weekday = parts.pop()?;
    let numbers: Vec<i32> = parts.iter().filter_map(|p| p.parse().ok()).collect();
    let (year, month, day) = match numbers.as_slice() {
        [year, month, day] => (*year, *month as u8, *day as u8),
        [day] => {
            let numeric: Vec<i32> = context
                .split('/')
                .rev()
                .filter(|part| part.chars().all(|c| c.is_ascii_digit()) && !part.is_empty())
                .filter_map(|part| part.parse().ok())
                .collect();
            let [month, year, ..] = numeric.as_slice() else {
                return None;
            };
            (*year, *month as u8, *day as u8)
        }
        _ => return None,
    };
    let mut clock = time_part.split(['=', ':']);
    let hour: u8 = clock.next()?.parse().ok()?;
    let minute: u8 = clock.next()?.parse().ok()?;
    let second: u8 = clock.next().unwrap_or("0").parse().ok()?;
    let date =
        time::Date::from_calendar_date(year, time::Month::try_from(month).ok()?, day).ok()?;
    let time = time::Time::from_hms(hour, minute, second).ok()?;
    Some((
        time::PrimitiveDateTime::new(date, time)
            .assume_utc()
            .unix_timestamp(),
        (),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = include_str!("../../../../dives/demo.ssrf");

    #[test]
    fn demo_roundtrips_through_the_git_tree() {
        let log = crate::io::ssrf::parse_str(DEMO).unwrap();
        let files = write(&log);
        let back = parse(&files).unwrap();
        assert_eq!(back.dives.len(), log.dives.len());
        assert_eq!(back.trips.len(), log.trips.len());
        assert_eq!(back.sites.len(), log.sites.len());
        assert_eq!(
            back.dives.iter().filter(|dive| dive.is_freedive()).count(),
            4
        );
        let sorted = log.dives_sorted();
        let first = sorted.first().unwrap();
        let same = back.dive_by_id(first.id).unwrap();
        assert_eq!(same.when, first.when);
        assert_eq!(same.buddy, first.buddy);
        assert_eq!(same.duration(), first.duration());
        assert_eq!(same.max_depth(), first.max_depth());
    }

    #[test]
    fn quoted_tokens_decode_escapes() {
        let tokens = tokens("notes \"line one\\nline two\" tag=\"a b\"");
        assert_eq!(tokens, vec!["notes", "line one\nline two", "tag=a b"]);
    }

    #[test]
    fn reads_and_writes_a_small_tree() {
        let mut log = DiveLog::new();
        log.sites.push(DiveSite {
            uuid: 0xabc,
            name: "Blue Hole".into(),
            location: Some(Location::new(28.5, 34.5)),
            country: Some("Egypt".into()),
            ..Default::default()
        });
        let mut dive = Dive::manual(1_700_000_000);
        dive.id = 1;
        dive.number = 7;
        dive.site_id = Some(0xabc);
        dive.duration = Some(Duration::new(1800));
        dive.max_depth = Some(Depth::from_meters(30.0));
        dive.pictures = vec![
            Picture {
                filename: "https://photos.app.goo.gl/abc123".into(),
                ..Default::default()
            },
            Picture {
                filename: "https://www.icloud.com/sharedalbum/#B0abc".into(),
                ..Default::default()
            },
        ];
        dive.computers[0] = DiveComputer {
            model: "Test".into(),
            divemode: Divemode::Freedive,
            max_depth: Some(Depth::from_meters(30.0)),
            duration: Some(Duration::new(1800)),
            samples: vec![Sample {
                time: Duration::new(0),
                depth: Depth::ZERO,
                ..Default::default()
            }],
            ..Default::default()
        };
        log.dives.push(dive);

        let files = write(&log);
        let back = parse(&files).unwrap();
        assert_eq!(back.sites.len(), 1);
        assert_eq!(back.sites[0].name, "Blue Hole");
        assert_eq!(back.dives.len(), 1);
        assert_eq!(back.dives[0].number, 7);
        assert_eq!(back.dives[0].when, 1_700_000_000);
        assert_eq!(back.dives[0].site_id, Some(0xabc));
        assert_eq!(back.dives[0].computers[0].divemode, Divemode::Freedive);
        assert_eq!(back.dives[0].pictures.len(), 2);
        let pictures: std::collections::BTreeSet<_> = back.dives[0]
            .pictures
            .iter()
            .map(|picture| picture.filename.as_str())
            .collect();
        assert!(pictures.contains("https://photos.app.goo.gl/abc123"));
        assert!(pictures.contains("https://www.icloud.com/sharedalbum/#B0abc"));
    }
}

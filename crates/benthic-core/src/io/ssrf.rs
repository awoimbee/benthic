//! Subsurface-compatible XML (`.ssrf`) reader and writer.
//!
//! Subsurface's format is delta-encoded: sample attributes are only written
//! when they change, so the reader carries the last-seen value forward. We
//! implement that faithfully on read, and write fully-specified samples on
//! write (which Subsurface accepts and which keeps our writer simple).
//!
//! Only `program='subsurface' version='2'` is targeted; the git storage format
//! and other importers live in separate modules (see the roadmap).

use quick_xml::events::Event as XmlEvent;
use quick_xml::Reader;

use crate::gas::{GasMix, AIR};
use crate::model::*;
use crate::units::*;
use crate::{Error, Result};

// ---------------------------------------------------------------------------
// Minimal DOM
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn text_trimmed(&self) -> &str {
        self.text.trim()
    }

    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
}

/// Parse XML text into a tree. We only need the subset of XML that Subsurface
/// emits, so a hand-rolled stack-based tree is plenty.
fn parse_document(xml: &str) -> Result<Node> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;

    loop {
        match reader.read_event_into(&mut buf)? {
            XmlEvent::Start(e) => {
                stack.push(node_from_start(&e)?);
            }
            XmlEvent::Empty(e) => {
                let node = node_from_start(&e)?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            XmlEvent::Text(e) => {
                if let Some(parent) = stack.last_mut() {
                    parent.text.push_str(&e.unescape()?);
                }
            }
            XmlEvent::CData(e) => {
                if let Some(parent) = stack.last_mut() {
                    parent.text.push_str(&String::from_utf8_lossy(e.as_ref()));
                }
            }
            XmlEvent::End(_) => {
                if let Some(node) = stack.pop() {
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(node),
                        None => root = Some(node),
                    }
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    root.ok_or_else(|| Error::Parse {
        what: "ssrf document",
        value: "empty input".into(),
    })
}

fn node_from_start(e: &quick_xml::events::BytesStart<'_>) -> Result<Node> {
    let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    let mut attrs = Vec::new();
    for attr in e.attributes() {
        let attr = attr?;
        let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
        let value = attr.unescape_value()?.into_owned();
        attrs.push((key, value));
    }
    Ok(Node {
        name,
        attrs,
        text: String::new(),
        children: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a Subsurface XML document.
pub fn parse_str(xml: &str) -> Result<DiveLog> {
    let mut root = parse_document(xml)?;
    if root.name != "divelog" {
        match root.children.iter().position(|c| c.name == "divelog") {
            Some(pos) => root = root.children.swap_remove(pos),
            None => {
                return Err(Error::Parse {
                    what: "dive log root element",
                    value: root.name,
                })
            }
        }
    }

    let mut log = DiveLog::new();
    let mut next_id = 1u32;
    let mut next_trip = 1u32;
    let mut next_site_uuid = 1u32;

    if let Some(settings) = root.child("settings") {
        log.autogroup = settings
            .child("autogroup")
            .and_then(|a| a.attr("state"))
            .map(|s| s == "1")
            .unwrap_or(false);
        for d in settings.children_named("divecomputerid") {
            log.devices.push(Device {
                model: d.attr("model").unwrap_or_default().to_string(),
                device_id: d.attr("deviceid").and_then(parse_hex_u32).unwrap_or(0),
                nickname: d.attr("nickname").map(str::to_string),
                serial: d.attr("serial").map(str::to_string),
                firmware: d.attr("firmware").map(str::to_string),
            });
        }
    }

    if let Some(sites) = root.child("divesites") {
        for s in sites.children_named("site") {
            let site = parse_site(s);
            next_site_uuid = next_site_uuid.max(site.uuid + 1);
            log.sites.push(site);
        }
    }

    if let Some(dives_el) = root.child("dives") {
        for child in &dives_el.children {
            match child.name.as_str() {
                "dive" => {
                    let mut dive = parse_dive(child, &mut log, &mut next_site_uuid);
                    dive.id = next_id;
                    next_id += 1;
                    log.dives.push(dive);
                }
                "trip" => {
                    let trip_id = next_trip;
                    next_trip += 1;
                    let mut trip = DiveTrip {
                        id: trip_id,
                        location: child.attr("location").unwrap_or_default().to_string(),
                        ..Default::default()
                    };
                    if let (Some(date), Some(time)) = (child.attr("date"), child.attr("time")) {
                        trip.date = parse_date_time(date, time).ok();
                    }
                    if let Some(notes) = child.child("notes") {
                        trip.notes = notes.text_trimmed().to_string();
                    }
                    log.trips.push(trip);

                    for dchild in child.children_named("dive") {
                        let mut dive = parse_dive(dchild, &mut log, &mut next_site_uuid);
                        dive.id = next_id;
                        next_id += 1;
                        dive.trip_id = Some(trip_id);
                        log.dives.push(dive);
                    }
                }
                _ => {}
            }
        }
    }

    log.fixup_all();
    Ok(log)
}

/// Serialize a dive log as Subsurface `version='2'` XML.
pub fn write_string(log: &DiveLog) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("<divelog program='subsurface' version='2'>\n");

    // settings
    out.push_str("<settings>\n");
    for device in &log.devices {
        out.push_str("  <divecomputerid");
        attr(&mut out, "model", &device.model);
        wattr(&mut out, "deviceid", &format!("{:08x}", device.device_id));
        if let Some(s) = &device.serial {
            attr(&mut out, "serial", s);
        }
        if let Some(s) = &device.nickname {
            attr(&mut out, "nickname", s);
        }
        if let Some(s) = &device.firmware {
            attr(&mut out, "firmware", s);
        }
        out.push_str("/>\n");
    }
    if log.autogroup {
        out.push_str("  <autogroup state='1' />\n");
    }
    out.push_str("</settings>\n");

    // dive sites
    out.push_str("<divesites>\n");
    for site in &log.sites {
        write_site(&mut out, site);
    }
    out.push_str("</divesites>\n");

    // dives, grouped by trip
    out.push_str("<dives>\n");
    let mut trips_written = std::collections::HashSet::new();
    for dive in &log.dives {
        match dive.trip_id.and_then(|id| log.trip_by_id(id)) {
            None => write_dive(&mut out, dive),
            Some(trip) => {
                if trips_written.insert(trip.id) {
                    out.push_str("<trip");
                    if let Some(date) = trip.date {
                        let (d, t) = fmt_date_time(date);
                        wattr(&mut out, "date", &d);
                        wattr(&mut out, "time", &t);
                    }
                    attr(&mut out, "location", &trip.location);
                    out.push_str(">\n");
                    if !trip.notes.is_empty() {
                        out.push_str(&format!("  <notes>{}</notes>\n", esc(&trip.notes)));
                    }
                    for d in log.dives.iter().filter(|d| d.trip_id == Some(trip.id)) {
                        write_dive(&mut out, d);
                    }
                    out.push_str("</trip>\n");
                }
            }
        }
    }
    out.push_str("</dives>\n</divelog>\n");
    out
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

fn parse_site(node: &Node) -> DiveSite {
    let mut site = DiveSite {
        uuid: node.attr("uuid").and_then(parse_hex_u32).unwrap_or(0),
        name: node.attr("name").unwrap_or_default().to_string(),
        description: node.attr("description").unwrap_or_default().to_string(),
        ..Default::default()
    };
    if let Some(gps) = node.attr("gps") {
        site.location = parse_location(gps);
    }
    if let Some(notes) = node.child("notes") {
        site.notes = notes.text_trimmed().to_string();
    }
    for geo in node.children_named("geo") {
        // TC_OCEAN = 1, TC_COUNTRY = 2 (see core/taxonomy.h)
        match geo.attr("cat") {
            Some("1") => site.ocean = geo.attr("value").map(str::to_string),
            Some("2") => site.country = geo.attr("value").map(str::to_string),
            _ => {}
        }
    }
    site
}

fn parse_dive(node: &Node, log: &mut DiveLog, next_site_uuid: &mut u32) -> Dive {
    let mut dive = Dive {
        number: node
            .attr("number")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        ..Default::default()
    };

    if let (Some(date), Some(time)) = (node.attr("date"), node.attr("time")) {
        dive.when = parse_date_time(date, time).unwrap_or(0);
    }
    if let Some(dur) = node.attr("duration") {
        dive.duration = Some(parse_duration(dur));
    }
    if let Some(p) = node.attr("airpressure") {
        dive.surface_pressure = Some(parse_pressure(p));
    }
    if let Some(s) = node.attr("watersalinity") {
        dive.salinity = s.parse::<f64>().ok().map(|v| (v * 10.0).round() as i32);
    }
    for (key, slot) in [
        ("rating", &mut dive.rating),
        ("visibility", &mut dive.visibility),
        ("wavesize", &mut dive.wavesize),
        ("current", &mut dive.current),
        ("surge", &mut dive.surge),
        ("chill", &mut dive.chill),
    ] {
        if let Some(v) = node.attr(key).and_then(|s| s.parse::<u8>().ok()) {
            *slot = v;
        }
    }
    if let Some(tags) = node.attr("tags") {
        dive.tags = tags
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
    }

    // Site: prefer an explicit id, otherwise synthesize one from inline data.
    if let Some(id) = node.attr("divesiteid").and_then(parse_hex_u32) {
        dive.site_id = Some(id);
    }

    for child in &node.children {
        match child.name.as_str() {
            "location" if dive.site_id.is_none() => {
                let name = child.text_trimmed().to_string();
                if !name.is_empty() {
                    let uuid = *next_site_uuid;
                    *next_site_uuid += 1;
                    log.sites.push(DiveSite {
                        uuid,
                        name,
                        ..Default::default()
                    });
                    dive.site_id = Some(uuid);
                }
            }
            "divemaster" => dive.diveguide = child.text_trimmed().to_string(),
            "buddy" => dive.buddy = child.text_trimmed().to_string(),
            "notes" => dive.notes = child.text_trimmed().to_string(),
            "suit" => dive.suit = child.text_trimmed().to_string(),
            "cylinder" => dive.cylinders.push(parse_cylinder(child)),
            "weightsystem" => dive.weights.push(parse_weight_system(child)),
            "divetemperature" => {
                if let Some(v) = child.attr("air") {
                    dive.air_temp = Some(parse_temperature(v));
                }
                if let Some(v) = child.attr("water") {
                    dive.water_temp = Some(parse_temperature(v));
                }
            }
            "divecomputer" => dive.computers.push(parse_divecomputer(child)),
            "picture" => dive.pictures.push(parse_picture(child)),
            _ => {}
        }
    }

    if let Some(gps) = node.attr("gps") {
        if let Some(loc) = parse_location_str(gps) {
            // Attach to the dive's site, creating one if necessary.
            let uuid = match dive
                .site_id
                .and_then(|id| log.sites.iter().position(|s| s.uuid == id))
            {
                Some(idx) => {
                    if log.sites[idx].location.is_none()
                        || !log.sites[idx].location.unwrap().is_valid()
                    {
                        log.sites[idx].location = Some(loc);
                    }
                    return dive;
                }
                None => {
                    let uuid = *next_site_uuid;
                    *next_site_uuid += 1;
                    uuid
                }
            };
            log.sites.push(DiveSite {
                uuid,
                location: Some(loc),
                ..Default::default()
            });
            dive.site_id = Some(uuid);
        }
    }

    dive
}

fn parse_cylinder(node: &Node) -> Cylinder {
    let o2 = node.attr("o2").and_then(parse_permille);
    let he = node.attr("he").and_then(parse_permille);
    Cylinder {
        size: node.attr("size").map(parse_volume),
        working_pressure: node.attr("workpressure").map(parse_pressure),
        description: node.attr("description").unwrap_or_default().to_string(),
        gas: match (o2, he) {
            (Some(o2), he) if o2 > 0 => GasMix::new(o2, he.unwrap_or(0)),
            _ => AIR,
        },
        start_pressure: node.attr("start").map(parse_pressure),
        end_pressure: node.attr("end").map(parse_pressure),
        use_: node.attr("use").map(parse_cylinder_use).unwrap_or_default(),
        depth: node.attr("depth").map(parse_depth),
        ..Default::default()
    }
}

fn parse_weight_system(node: &Node) -> WeightSystem {
    WeightSystem {
        weight: node.attr("weight").map(parse_weight).unwrap_or_default(),
        description: node.attr("description").unwrap_or_default().to_string(),
        auto_filled: false,
    }
}

fn parse_picture(node: &Node) -> Picture {
    Picture {
        filename: node.attr("filename").unwrap_or_default().to_string(),
        offset: node.attr("offset").map(parse_duration),
        location: node.attr("gps").and_then(parse_location_str),
        hash: node.attr("hash").map(str::to_string),
    }
}

fn parse_divecomputer(node: &Node) -> DiveComputer {
    let mut dc = DiveComputer {
        model: node.attr("model").unwrap_or_default().to_string(),
        device_id: node.attr("deviceid").and_then(parse_hex_u32).unwrap_or(0),
        dive_id: node.attr("diveid").and_then(parse_hex_u32).unwrap_or(0),
        serial: node.attr("serial").map(str::to_string),
        firmware: node.attr("fw").map(str::to_string),
        divemode: node.attr("dctype").map(parse_divemode).unwrap_or_default(),
        no_o2_sensors: node
            .attr("no_o2sensors")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        ..Default::default()
    };
    if let Some(d) = node.attr("duration") {
        dc.duration = Some(parse_duration(d));
    }

    let mut state = SampleState::default();
    for child in &node.children {
        match child.name.as_str() {
            "depth" => {
                dc.max_depth = child.attr("max").map(parse_depth);
                dc.mean_depth = child.attr("mean").map(parse_depth);
            }
            "temperature" => {
                dc.air_temp = child.attr("air").map(parse_temperature);
                dc.water_temp = child.attr("water").map(parse_temperature);
            }
            "surface" => dc.surface_pressure = child.attr("pressure").map(parse_pressure),
            "water" => {
                if let Some(s) = child.attr("salinity") {
                    dc.salinity = s.parse::<f64>().ok().map(|v| (v * 10.0).round() as i32);
                }
            }
            "sample" => dc.samples.push(parse_sample(child, &mut state)),
            "event" => dc.events.push(parse_event(child)),
            "extradata" => {
                if let (Some(k), Some(v)) = (child.attr("key"), child.attr("value")) {
                    dc.extra_data.push((k.to_string(), v.to_string()));
                }
            }
            _ => {}
        }
    }
    if dc.duration.is_none() {
        dc.duration = dc.samples.last().map(|s| s.time);
    }
    dc
}

#[derive(Debug, Default, Clone)]
struct SampleState {
    temperature: Option<Temperature>,
    pressures: Vec<SensorPressure>,
    setpoint: Option<O2Pressure>,
    o2_sensors: Vec<O2Pressure>,
    dc_supplied_ppo2: Option<O2Pressure>,
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

fn parse_sample(node: &Node, state: &mut SampleState) -> Sample {
    if let Some(v) = node.attr("temp") {
        state.temperature = Some(parse_temperature(v));
    }
    if let Some(v) = node.attr("po2") {
        state.setpoint = Some(parse_o2pressure(v));
    }
    if let Some(v) = node.attr("ndl") {
        state.ndl = Some(parse_duration(v));
    }
    if let Some(v) = node.attr("tts") {
        state.tts = Some(parse_duration(v));
    }
    if let Some(v) = node.attr("rbt") {
        state.rbt = Some(parse_duration(v));
    }
    if let Some(v) = node.attr("stoptime") {
        state.stop_time = Some(parse_duration(v));
    }
    if let Some(v) = node.attr("stopdepth") {
        state.stop_depth = Some(parse_depth(v));
    }
    if let Some(v) = node.attr("cns") {
        state.cns = v.trim_end_matches('%').trim().parse().ok();
    }
    if let Some(v) = node.attr("heartbeat") {
        state.heartbeat = v.parse().ok();
    }
    if let Some(v) = node.attr("bearing") {
        state.bearing = v.parse().ok().map(Bearing::new);
    }
    if let Some(v) = node.attr("in_deco") {
        state.in_deco = v == "1" || v.eq_ignore_ascii_case("true");
    }

    // Cylinder pressures: `pressureN='.. bar'` or `pressure='.. bar' sensor='N'`.
    for (key, value) in &node.attrs {
        if let Some(rest) = key.strip_prefix("pressure") {
            let sensor = if rest.is_empty() {
                node.attr("sensor")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0i16)
            } else {
                rest.parse().unwrap_or(0i16)
            };
            let pressure = parse_pressure(value);
            upsert_pressure(&mut state.pressures, SensorPressure { sensor, pressure });
        } else if let Some(rest) = key.strip_prefix("sensor") {
            if let Ok(idx) = rest.parse::<usize>() {
                if (1..=6).contains(&idx) {
                    let p = parse_o2pressure(value);
                    if state.o2_sensors.len() < idx {
                        state.o2_sensors.resize(idx, O2Pressure::default());
                    }
                    state.o2_sensors[idx - 1] = p;
                }
            }
        } else if key == "dc_supplied_ppo2" {
            state.dc_supplied_ppo2 = Some(parse_o2pressure(value));
        }
    }

    let mut o2_sensors = state.o2_sensors.clone();
    if let Some(p) = state.dc_supplied_ppo2 {
        o2_sensors.push(p);
    }

    Sample {
        time: node.attr("time").map(parse_duration).unwrap_or_default(),
        depth: node.attr("depth").map(parse_depth).unwrap_or_default(),
        temperature: state.temperature,
        pressures: state.pressures.clone(),
        setpoint: state.setpoint,
        o2_sensors,
        ndl: state.ndl,
        tts: state.tts,
        rbt: state.rbt,
        stop_time: state.stop_time,
        stop_depth: state.stop_depth,
        cns: state.cns,
        heartbeat: state.heartbeat,
        bearing: state.bearing,
        in_deco: state.in_deco,
        manually_entered: false,
    }
}

fn upsert_pressure(pressures: &mut Vec<SensorPressure>, entry: SensorPressure) {
    match pressures.iter_mut().find(|p| p.sensor == entry.sensor) {
        Some(existing) => *existing = entry,
        None => pressures.push(entry),
    }
}

fn parse_event(node: &Node) -> Event {
    let name = node.attr("name").unwrap_or_default().to_string();
    let divemode = node.attr("divemode").map(parse_divemode);
    let gas = if name == "gaschange" {
        let o2 = node.attr("o2").and_then(parse_permille).unwrap_or(0);
        let he = node.attr("he").and_then(parse_permille).unwrap_or(0);
        let mix = if o2 > 0 { GasMix::new(o2, he) } else { AIR };
        let index = node
            .attr("cylinder")
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(-1);
        Some((index, mix))
    } else {
        None
    };
    Event {
        time: node.attr("time").map(parse_duration).unwrap_or_default(),
        name,
        flags: node.attr("flags").and_then(|s| s.parse().ok()).unwrap_or(0),
        value: node.attr("value").and_then(|s| s.parse().ok()).unwrap_or(0),
        divemode,
        gas,
        hidden: false,
    }
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

fn write_site(out: &mut String, site: &DiveSite) {
    out.push_str("<site");
    wattr(out, "uuid", &format!("{:08x}", site.uuid));
    if !site.name.is_empty() {
        attr(out, "name", &site.name);
    }
    if let Some(loc) = site.location.filter(|l| l.is_valid()) {
        wattr(out, "gps", &format!("{:.6} {:.6}", loc.lat, loc.lon));
    }
    if !site.description.is_empty() {
        attr(out, "description", &site.description);
    }
    out.push_str(">\n");
    if !site.notes.is_empty() {
        out.push_str(&format!("  <notes>{}</notes>\n", esc(&site.notes)));
    }
    if let Some(ocean) = &site.ocean {
        out.push_str(&format!(
            "  <geo cat='1' origin='0' value='{}'/>\n",
            esc(ocean)
        ));
    }
    if let Some(country) = &site.country {
        out.push_str(&format!(
            "  <geo cat='2' origin='0' value='{}'/>\n",
            esc(country)
        ));
    }
    out.push_str("</site>\n");
}

fn write_dive(out: &mut String, dive: &Dive) {
    out.push_str("<dive");
    if dive.number != 0 {
        wattr(out, "number", &dive.number.to_string());
    }
    if !dive.tags.is_empty() {
        attr(out, "tags", &dive.tags.join(", "));
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
            wattr(out, key, &value.to_string());
        }
    }
    if let Some(site_id) = dive.site_id {
        wattr(out, "divesiteid", &format!("{site_id:08x}"));
    }
    if let Some(salinity) = dive.salinity {
        wattr(
            out,
            "watersalinity",
            &format!("{:.1}", salinity as f64 / 10.0),
        );
    }
    let (date, time) = fmt_date_time(dive.when);
    wattr(out, "date", &date);
    wattr(out, "time", &time);
    if let Some(duration) = dive.duration {
        wattr(out, "duration", &fmt_duration_min(duration));
    }
    out.push_str(">\n");

    text_element(out, "divemaster", &dive.diveguide);
    text_element(out, "buddy", &dive.buddy);
    text_element(out, "notes", &dive.notes);
    text_element(out, "suit", &dive.suit);

    for cyl in &dive.cylinders {
        out.push_str("  <cylinder");
        if let Some(size) = cyl.size {
            wattr(out, "size", &format!("{} l", fmt_milli(size.ml as i64)));
        }
        if let Some(wp) = cyl.working_pressure {
            wattr(
                out,
                "workpressure",
                &format!("{} bar", fmt_milli(wp.mbar as i64)),
            );
        }
        if !cyl.description.is_empty() {
            attr(out, "description", &cyl.description);
        }
        if !cyl.gas.is_air() {
            wattr(
                out,
                "o2",
                &format!("{}%", fmt_milli(cyl.gas.o2_permille as i64)),
            );
            if cyl.gas.he_permille != 0 {
                wattr(
                    out,
                    "he",
                    &format!("{}%", fmt_milli(cyl.gas.he_permille as i64)),
                );
            }
        }
        if let Some(p) = cyl.start_pressure {
            wattr(out, "start", &format!("{} bar", fmt_milli(p.mbar as i64)));
        }
        if let Some(p) = cyl.end_pressure {
            wattr(out, "end", &format!("{} bar", fmt_milli(p.mbar as i64)));
        }
        if cyl.use_ != CylinderUse::OcGas {
            attr(out, "use", cylinder_use_text(cyl.use_));
        }
        if let Some(d) = cyl.depth {
            wattr(out, "depth", &format!("{} m", fmt_milli(d.mm as i64)));
        }
        out.push_str(" />\n");
    }

    for ws in &dive.weights {
        out.push_str("  <weightsystem");
        wattr(
            out,
            "weight",
            &format!("{} kg", fmt_milli(ws.weight.grams as i64)),
        );
        if !ws.description.is_empty() {
            attr(out, "description", &ws.description);
        }
        out.push_str(" />\n");
    }

    if dive.air_temp.is_some() || dive.water_temp.is_some() {
        out.push_str("  <divetemperature");
        if let Some(t) = dive.air_temp {
            wattr(
                out,
                "air",
                &format!(
                    "{} C",
                    fmt_milli(t.mkelvin as i64 - ZERO_CELSIUS_MKELVIN as i64)
                ),
            );
        }
        if let Some(t) = dive.water_temp {
            wattr(
                out,
                "water",
                &format!(
                    "{} C",
                    fmt_milli(t.mkelvin as i64 - ZERO_CELSIUS_MKELVIN as i64)
                ),
            );
        }
        out.push_str(" />\n");
    }

    for dc in &dive.computers {
        write_divecomputer(out, dc);
    }
    for pic in &dive.pictures {
        out.push_str("  <picture");
        attr(out, "filename", &pic.filename);
        if let Some(offset) = pic.offset {
            wattr(out, "offset", &format!("+{} min", fmt_duration_min(offset)));
        }
        if let Some(hash) = &pic.hash {
            attr(out, "hash", hash);
        }
        out.push_str("/>\n");
    }

    out.push_str("</dive>\n");
}

fn write_divecomputer(out: &mut String, dc: &DiveComputer) {
    out.push_str("  <divecomputer");
    if !dc.model.is_empty() {
        attr(out, "model", &dc.model);
    }
    if dc.device_id != 0 {
        wattr(out, "deviceid", &format!("{:08x}", dc.device_id));
    }
    if dc.dive_id != 0 {
        wattr(out, "diveid", &format!("{:08x}", dc.dive_id));
    }
    if let Some(serial) = &dc.serial {
        attr(out, "serial", serial);
    }
    if let Some(fw) = &dc.firmware {
        attr(out, "fw", fw);
    }
    if dc.divemode != Divemode::OpenCircuit {
        attr(out, "dctype", divemode_text(dc.divemode));
        if dc.no_o2_sensors != 0 {
            wattr(out, "no_o2sensors", &dc.no_o2_sensors.to_string());
        }
    }
    if let Some(duration) = dc.duration {
        wattr(out, "duration", &fmt_duration_min(duration));
    }
    out.push_str(">\n");

    if dc.max_depth.is_some() || dc.mean_depth.is_some() {
        out.push_str("  <depth");
        if let Some(d) = dc.max_depth {
            wattr(out, "max", &format!("{} m", fmt_milli(d.mm as i64)));
        }
        if let Some(d) = dc.mean_depth {
            wattr(out, "mean", &format!("{} m", fmt_milli(d.mm as i64)));
        }
        out.push_str(" />\n");
    }
    if dc.air_temp.is_some() || dc.water_temp.is_some() {
        out.push_str("  <temperature");
        if let Some(t) = dc.air_temp {
            wattr(
                out,
                "air",
                &format!(
                    "{} C",
                    fmt_milli(t.mkelvin as i64 - ZERO_CELSIUS_MKELVIN as i64)
                ),
            );
        }
        if let Some(t) = dc.water_temp {
            wattr(
                out,
                "water",
                &format!(
                    "{} C",
                    fmt_milli(t.mkelvin as i64 - ZERO_CELSIUS_MKELVIN as i64)
                ),
            );
        }
        out.push_str(" />\n");
    }
    if let Some(p) = dc.surface_pressure {
        wattr_start(out, "  <surface");
        wattr(
            out,
            "pressure",
            &format!("{} bar", fmt_milli(p.mbar as i64)),
        );
        out.push_str(" />\n");
    }
    if let Some(s) = dc.salinity {
        wattr_start(out, "  <water");
        wattr(out, "salinity", &format!("{:.1} g/l", s as f64 / 10.0));
        out.push_str(" />\n");
    }
    for sample in &dc.samples {
        write_sample(out, sample);
    }
    for event in &dc.events {
        write_event(out, event);
    }
    for (key, value) in &dc.extra_data {
        out.push_str("  <extradata");
        attr(out, "key", key);
        attr(out, "value", value);
        out.push_str(" />\n");
    }
    out.push_str("  </divecomputer>\n");
}

fn write_sample(out: &mut String, s: &Sample) {
    out.push_str("  <sample");
    wattr(out, "time", &fmt_duration_min(s.time));
    wattr(out, "depth", &format!("{} m", fmt_milli(s.depth.mm as i64)));
    if let Some(t) = s.temperature {
        wattr(
            out,
            "temp",
            &format!(
                "{} C",
                fmt_milli(t.mkelvin as i64 - ZERO_CELSIUS_MKELVIN as i64)
            ),
        );
    }
    for p in &s.pressures {
        wattr(
            out,
            &format!("pressure{}", p.sensor),
            &format!("{} bar", fmt_milli(p.pressure.mbar as i64)),
        );
    }
    if let Some(p) = s.setpoint {
        wattr(out, "po2", &format!("{} bar", fmt_milli(p.mbar as i64)));
    }
    for (i, p) in s.o2_sensors.iter().enumerate() {
        if i < 6 {
            wattr(
                out,
                &format!("sensor{}", i + 1),
                &format!("{} bar", fmt_milli(p.mbar as i64)),
            );
        } else {
            wattr(
                out,
                "dc_supplied_ppo2",
                &format!("{} bar", fmt_milli(p.mbar as i64)),
            );
        }
    }
    if let Some(ndl) = s.ndl {
        wattr(out, "ndl", &fmt_duration_min(ndl));
    }
    if let Some(tts) = s.tts {
        wattr(out, "tts", &fmt_duration_min(tts));
    }
    if let Some(rbt) = s.rbt {
        wattr(out, "rbt", &fmt_duration_min(rbt));
    }
    if let Some(st) = s.stop_time {
        wattr(out, "stoptime", &fmt_duration_min(st));
    }
    if let Some(sd) = s.stop_depth {
        wattr(out, "stopdepth", &format!("{} m", fmt_milli(sd.mm as i64)));
    }
    if let Some(cns) = s.cns {
        wattr(out, "cns", &format!("{cns}%"));
    }
    if let Some(hb) = s.heartbeat {
        wattr(out, "heartbeat", &hb.to_string());
    }
    if let Some(b) = s.bearing {
        wattr(out, "bearing", &b.degrees.to_string());
    }
    if s.in_deco {
        out.push_str(" in_deco='1'");
    }
    out.push_str(" />\n");
}

fn write_event(out: &mut String, ev: &Event) {
    out.push_str("  <event");
    wattr(out, "time", &fmt_duration_min(ev.time));
    if ev.flags != 0 {
        wattr(out, "flags", &ev.flags.to_string());
    }
    if let Some(mode) = ev.divemode {
        attr(out, "divemode", divemode_text(mode));
    } else if ev.value != 0 {
        wattr(out, "value", &ev.value.to_string());
    }
    attr(out, "name", &ev.name);
    if let Some((index, mix)) = ev.gas {
        if index >= 0 {
            wattr(out, "cylinder", &index.to_string());
        }
        if !mix.is_air() {
            wattr(
                out,
                "o2",
                &format!("{}%", fmt_milli(mix.o2_permille as i64)),
            );
            if mix.he_permille != 0 {
                wattr(
                    out,
                    "he",
                    &format!("{}%", fmt_milli(mix.he_permille as i64)),
                );
            }
        }
    }
    out.push_str(" />\n");
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn divemode_text(mode: Divemode) -> &'static str {
    match mode {
        Divemode::OpenCircuit => "OC",
        Divemode::Ccr => "CCR",
        Divemode::Pscr => "PSCR",
        Divemode::Freedive => "Freedive",
    }
}

fn parse_divemode(s: &str) -> Divemode {
    match s.to_ascii_lowercase().as_str() {
        "ccr" | "cc" => Divemode::Ccr,
        "pscr" => Divemode::Pscr,
        "freedive" => Divemode::Freedive,
        _ => Divemode::OpenCircuit,
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

fn parse_cylinder_use(s: &str) -> CylinderUse {
    match s.to_ascii_lowercase().as_str() {
        "diluent" => CylinderUse::Diluent,
        "oxygen" => CylinderUse::Oxygen,
        "not used" | "not-used" => CylinderUse::NotUsed,
        "travel gas" | "travel" | "travel-gas" => CylinderUse::TravelOc,
        _ => CylinderUse::OcGas,
    }
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn attr(out: &mut String, key: &str, value: &str) {
    out.push_str(&format!(" {key}='{}'", esc(value)));
}

fn wattr(out: &mut String, key: &str, value: &str) {
    attr(out, key, value);
}

fn wattr_start(out: &mut String, prefix: &str) {
    out.push_str(prefix);
}

fn text_element(out: &mut String, tag: &str, value: &str) {
    if !value.is_empty() {
        out.push_str(&format!("  <{tag}>{}</{tag}>\n", esc(value)));
    }
}

/// Format a value in milli-units the way Subsurface's `put_milli` does:
/// integer part, then up to three fractional digits with trailing zeros
/// (but at least one digit) removed.
fn fmt_milli(value: i64) -> String {
    let neg = value < 0;
    let v = value.unsigned_abs();
    let whole = v / 1000;
    let frac = v % 1000;
    if frac == 0 {
        return format!("{}{}.0", if neg { "-" } else { "" }, whole);
    }
    let mut frac_str = format!("{frac:03}");
    while frac_str.ends_with('0') {
        frac_str.pop();
    }
    format!("{}{}.{}", if neg { "-" } else { "" }, whole, frac_str)
}

fn fmt_duration_min(d: Duration) -> String {
    let total = d.seconds.unsigned_abs();
    format!("{}:{:02} min", total / 60, total % 60)
}

fn parse_duration(s: &str) -> Duration {
    let s = s.trim();
    let s = s.strip_suffix("min").unwrap_or(s).trim();
    let neg = s.starts_with('-');
    let s = s.trim_start_matches(['+', '-']);
    let parts: Vec<i64> = s
        .split(':')
        .map(|p| p.trim().parse::<i64>().unwrap_or(0))
        .collect();
    let secs = match parts.as_slice() {
        [s] => *s,
        [m, s] => m * 60 + s,
        [h, m, s] => h * 3600 + m * 60 + s,
        _ => 0,
    };
    Duration::new(if neg { -(secs as i32) } else { secs as i32 })
}

fn value_and_unit(s: &str) -> (f64, String) {
    let s = s.trim();
    let bytes = s.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        let c = bytes[idx] as char;
        if c.is_ascii_digit() || c == '.' || c == '-' || c == '+' {
            idx += 1;
        } else {
            break;
        }
    }
    let num = s[..idx].parse::<f64>().unwrap_or(0.0);
    (num, s[idx..].trim().to_lowercase())
}

fn parse_depth(s: &str) -> Depth {
    let (v, u) = value_and_unit(s);
    if u.starts_with("ft") {
        Depth::from_feet(v)
    } else {
        Depth::from_meters(v)
    }
}

fn parse_pressure(s: &str) -> Pressure {
    let (v, u) = value_and_unit(s);
    if u.starts_with("psi") {
        Pressure::from_psi(v)
    } else {
        Pressure::from_bar(v)
    }
}

fn parse_temperature(s: &str) -> Temperature {
    let (v, u) = value_and_unit(s);
    if u.starts_with('f') {
        Temperature::from_fahrenheit(v)
    } else if u.starts_with('k') {
        Temperature::new((v * 1000.0).round() as u32)
    } else {
        Temperature::from_celsius(v)
    }
}

fn parse_volume(s: &str) -> Volume {
    let (v, u) = value_and_unit(s);
    if u.contains("cuft") || u.contains("ft3") {
        Volume::from_cubic_feet(v)
    } else {
        Volume::from_liters(v)
    }
}

fn parse_weight(s: &str) -> Weight {
    let (v, u) = value_and_unit(s);
    if u.starts_with("lb") {
        Weight::from_lbs(v)
    } else {
        Weight::from_kg(v)
    }
}

fn parse_o2pressure(s: &str) -> O2Pressure {
    let (v, _) = value_and_unit(s);
    O2Pressure::from_bar(v)
}

fn parse_permille(s: &str) -> Option<u16> {
    let s = s.trim().trim_end_matches('%').trim();
    s.parse::<f64>().ok().map(|v| (v * 10.0).round() as u16)
}

fn parse_hex_u32(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim(), 16).ok()
}

fn parse_location(s: &str) -> Option<Location> {
    parse_location_str(s)
}

fn parse_location_str(s: &str) -> Option<Location> {
    let cleaned = s.replace(',', " ");
    let mut parts = cleaned.split_whitespace();
    let lat: f64 = parts.next()?.parse().ok()?;
    let lon: f64 = parts.next()?.parse().ok()?;
    Some(Location::new(lat, lon))
}

fn parse_date_time(date: &str, time_str: &str) -> Result<Timestamp> {
    use time::{Date, Month, PrimitiveDateTime, Time};

    let mut d = date.split('-');
    let year: i32 = d
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error::DateTime(date.into()))?;
    let month: u8 = d
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error::DateTime(date.into()))?;
    let day: u8 = d
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error::DateTime(date.into()))?;

    let mut t = time_str.split(':');
    let hour: u8 = t.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minute: u8 = t.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let second: u8 = t.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    let month = Month::try_from(month).map_err(|_| Error::DateTime(date.into()))?;
    let date =
        Date::from_calendar_date(year, month, day).map_err(|e| Error::DateTime(e.to_string()))?;
    let time = Time::from_hms(hour, minute, second).map_err(|e| Error::DateTime(e.to_string()))?;
    Ok(PrimitiveDateTime::new(date, time)
        .assume_utc()
        .unix_timestamp())
}

fn fmt_date_time(ts: Timestamp) -> (String, String) {
    let dt =
        time::OffsetDateTime::from_unix_timestamp(ts).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let date = dt.date();
    let time = dt.time();
    (
        format!(
            "{:04}-{:02}-{:02}",
            date.year(),
            u8::from(date.month()),
            date.day()
        ),
        format!(
            "{:02}:{:02}:{:02}",
            time.hour(),
            time.minute(),
            time.second()
        ),
    )
}

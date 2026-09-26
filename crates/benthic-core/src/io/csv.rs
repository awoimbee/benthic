//! CSV import.
//!
//! This handles the common, hand-maintained "one dive per row" spreadsheet
//! export rather than the many vendor-specific sample-level CSV dialects.
//! Columns are recognised by header name (with generous aliases); the
//! delimiter is detected from the header line.

use std::collections::HashMap;

use super::super::model::{Cylinder, Dive, DiveLog, DiveSite, WeightSystem};
use crate::gas::GasMix;
use crate::units::*;
use crate::{Error, Result};

/// Parse a one-dive-per-row CSV file.
pub fn parse_str(text: &str) -> Result<DiveLog> {
    let delimiter = detect_delimiter(text);
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'));

    let Some(header) = lines.next() else {
        return Err(Error::Parse {
            what: "CSV header",
            value: "empty input".into(),
        });
    };
    let columns = Columns::from_header(header, delimiter);

    if columns.date.is_none() && columns.datetime.is_none() {
        return Err(Error::Parse {
            what: "CSV header",
            value: "no date column found".into(),
        });
    }

    let mut log = DiveLog::new();
    let mut sites: HashMap<String, u32> = HashMap::new();
    let mut next_id = 1u32;
    let mut next_site = 1u32;

    for line in lines {
        let fields: Vec<&str> = line.split(delimiter).map(str::trim).collect();
        let Some(dive) = columns.build_dive(&fields) else {
            continue;
        };
        let mut dive = dive;

        if let Some(location) = columns.field(&fields, columns.location) {
            if !location.is_empty() {
                let uuid = *sites.entry(location.to_lowercase()).or_insert_with(|| {
                    let uuid = next_site;
                    next_site += 1;
                    uuid
                });
                if uuid == next_site - 1 && !log.sites.iter().any(|s| s.uuid == uuid) {
                    log.sites.push(DiveSite {
                        uuid,
                        name: location.to_string(),
                        ..Default::default()
                    });
                }
                dive.site_id = Some(uuid);
            }
        }

        dive.id = next_id;
        next_id += 1;
        if dive.number == 0 {
            dive.number = dive.id as i32;
        }
        log.dives.push(dive);
    }

    log.fixup_all();
    Ok(log)
}

fn detect_delimiter(text: &str) -> char {
    let header = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let counts = [
        (';', header.matches(';').count()),
        (',', header.matches(',').count()),
        ('\t', header.matches('\t').count()),
    ];
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .filter(|(_, count)| *count > 0)
        .map(|(delimiter, _)| delimiter)
        .unwrap_or(',')
}

#[derive(Default)]
struct Columns {
    date: Option<usize>,
    time: Option<usize>,
    datetime: Option<usize>,
    duration: Option<usize>,
    max_depth: Option<usize>,
    avg_depth: Option<usize>,
    water_temp: Option<usize>,
    air_temp: Option<usize>,
    buddy: Option<usize>,
    divemaster: Option<usize>,
    location: Option<usize>,
    suit: Option<usize>,
    notes: Option<usize>,
    rating: Option<usize>,
    tags: Option<usize>,
    weight: Option<usize>,
    o2: Option<usize>,
    he: Option<usize>,
    cylinder_size: Option<usize>,
}

impl Columns {
    fn from_header(header: &str, delimiter: char) -> Self {
        let mut columns = Columns::default();
        for (index, name) in header.split(delimiter).enumerate() {
            let key: String = name
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect();
            match key.as_str() {
                "date" | "datum" | "divedate" => columns.date = Some(index),
                "time" | "starttime" | "entrytime" => columns.time = Some(index),
                "datetime" | "timestamp" => columns.datetime = Some(index),
                "duration" | "divetime" | "runtime" | "bottomtime" | "length" => {
                    columns.duration = Some(index)
                }
                "depth" | "maxdepth" | "maximumdepth" => columns.max_depth = Some(index),
                "avgdepth" | "meandepth" | "averagedepth" => columns.avg_depth = Some(index),
                "watertemp" | "watertemperature" | "temp" | "temperature" => {
                    columns.water_temp = Some(index)
                }
                "airtemp" | "airtemperature" => columns.air_temp = Some(index),
                "buddy" | "buddyname" | "buddy1" => columns.buddy = Some(index),
                "divemaster" | "diveguide" | "guide" | "instructor" => {
                    columns.divemaster = Some(index)
                }
                "location" | "site" | "divesite" | "place" | "spot" => {
                    columns.location = Some(index)
                }
                "suit" => columns.suit = Some(index),
                "notes" | "note" | "description" | "comment" | "comments" => {
                    columns.notes = Some(index)
                }
                "rating" | "stars" => columns.rating = Some(index),
                "tags" | "tag" => columns.tags = Some(index),
                "weight" => columns.weight = Some(index),
                "o2" | "oxygen" | "o2percent" | "fo2" => columns.o2 = Some(index),
                "he" | "helium" | "hepercent" | "fhe" => columns.he = Some(index),
                "tanksize" | "cylindersize" | "volume" | "size" | "tank" => {
                    columns.cylinder_size = Some(index)
                }
                _ => {}
            }
        }
        columns
    }

    fn field<'a>(&self, fields: &'a [&'a str], column: Option<usize>) -> Option<&'a str> {
        column
            .and_then(|index| fields.get(index).copied())
            .map(|value| value.trim().trim_matches('"'))
    }

    fn build_dive(&self, fields: &[&str]) -> Option<Dive> {
        let mut dive = Dive::default();

        if let Some(datetime) = self.field(fields, self.datetime).and_then(parse_datetime) {
            dive.when = datetime;
        } else {
            let date = self.field(fields, self.date)?;
            let time = self.field(fields, self.time);
            dive.when = parse_date_time(date, time)?;
        }

        dive.duration = self
            .field(fields, self.duration)
            .and_then(parse_duration)
            .map(Duration::new);
        dive.max_depth = self.field(fields, self.max_depth).and_then(parse_depth);
        dive.mean_depth = self.field(fields, self.avg_depth).and_then(parse_depth);
        dive.water_temp = self
            .field(fields, self.water_temp)
            .and_then(parse_temperature);
        dive.air_temp = self
            .field(fields, self.air_temp)
            .and_then(parse_temperature);
        dive.buddy = self
            .field(fields, self.buddy)
            .unwrap_or_default()
            .to_string();
        dive.diveguide = self
            .field(fields, self.divemaster)
            .unwrap_or_default()
            .to_string();
        dive.suit = self
            .field(fields, self.suit)
            .unwrap_or_default()
            .to_string();
        dive.notes = self
            .field(fields, self.notes)
            .unwrap_or_default()
            .to_string();
        dive.rating = self
            .field(fields, self.rating)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if let Some(tags) = self.field(fields, self.tags) {
            dive.tags = tags
                .split([';', ','])
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
        }
        if let Some(weight) = self.field(fields, self.weight).and_then(parse_weight) {
            dive.weights.push(WeightSystem {
                weight,
                description: "belt".to_string(),
                auto_filled: false,
            });
        }

        let o2 = self
            .field(fields, self.o2)
            .and_then(parse_fraction)
            .unwrap_or(210);
        let he = self
            .field(fields, self.he)
            .and_then(parse_fraction)
            .unwrap_or(0);
        let size = self
            .field(fields, self.cylinder_size)
            .and_then(parse_volume);
        if o2 > 210 || he > 0 || size.is_some() {
            dive.cylinders.push(Cylinder {
                gas: GasMix::new(o2, he),
                size,
                ..Default::default()
            });
        }

        Some(dive)
    }
}

fn parse_number(value: &str) -> Option<f64> {
    let cleaned = value.trim().replace(',', ".");
    let number: String = cleaned
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .collect();
    number.parse().ok()
}

fn parse_unit(value: &str) -> String {
    value
        .trim()
        .chars()
        .skip_while(|c| {
            c.is_ascii_digit() || *c == '.' || *c == ',' || *c == '-' || *c == '+' || *c == ' '
        })
        .collect::<String>()
        .trim()
        .to_lowercase()
}

fn parse_duration(value: &str) -> Option<i32> {
    let value = value.trim();
    let parts: Vec<&str> = value.split(':').collect();
    match parts.as_slice() {
        [h, m, s] => Some(
            h.trim().parse::<i32>().ok()? * 3600
                + m.trim().parse::<i32>().ok()? * 60
                + s.trim().parse::<i32>().ok()?,
        ),
        [m, s] => Some(m.trim().parse::<i32>().ok()? * 60 + s.trim().parse::<i32>().ok()?),
        [minutes] => Some((parse_number(minutes)? * 60.0).round() as i32),
        _ => None,
    }
}

fn parse_depth(value: &str) -> Option<Depth> {
    let number = parse_number(value)?;
    let unit = parse_unit(value);
    if unit.starts_with("ft") || unit.starts_with('\'') {
        Some(Depth::from_feet(number))
    } else {
        Some(Depth::from_meters(number))
    }
}

fn parse_temperature(value: &str) -> Option<Temperature> {
    let number = parse_number(value)?;
    let unit = parse_unit(value);
    if unit.starts_with('f') {
        Some(Temperature::from_fahrenheit(number))
    } else {
        Some(Temperature::from_celsius(number))
    }
}

fn parse_weight(value: &str) -> Option<Weight> {
    let number = parse_number(value)?;
    let unit = parse_unit(value);
    if unit.starts_with("lb") {
        Some(Weight::from_lbs(number))
    } else {
        Some(Weight::from_kg(number))
    }
}

fn parse_volume(value: &str) -> Option<Volume> {
    let number = parse_number(value)?;
    let unit = parse_unit(value);
    if unit.contains("cuft") || unit.contains("ft3") {
        Some(Volume::from_cubic_feet(number))
    } else {
        Some(Volume::from_liters(number))
    }
}

/// Parse an O2/He column: either a percentage ("32", "32%") or a fraction
/// ("0.32").
fn parse_fraction(value: &str) -> Option<u16> {
    let number = parse_number(value)?;
    let permille = if number <= 1.0 {
        (number * 1000.0).round()
    } else {
        (number * 10.0).round()
    };
    Some(permille.clamp(0.0, 1000.0) as u16)
}

fn parse_datetime(value: &str) -> Option<Timestamp> {
    let (date, time) = value.split_once(['T', ' '])?;
    parse_date_time(date, Some(time))
}

fn parse_date_time(date: &str, time: Option<&str>) -> Option<Timestamp> {
    use time::{Date, Month, PrimitiveDateTime, Time};

    let date = date.trim();
    let parts: Vec<&str> = date.split(['-', '/', '.']).collect();
    if parts.len() != 3 {
        return None;
    }
    let (year, month, day): (i32, u8, u8) = if parts[0].len() == 4 {
        (
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
        )
    } else if parts[2].len() == 4 {
        // Day-first for ambiguous values, the common European convention.
        (
            parts[2].parse().ok()?,
            parts[1].parse().ok()?,
            parts[0].parse().ok()?,
        )
    } else {
        return None;
    };

    let (hour, minute, second): (u8, u8, u8) = match time.map(str::trim).filter(|t| !t.is_empty()) {
        Some(time) => {
            let parts: Vec<&str> = time.split(':').collect();
            let hour = parts.first().and_then(|h| h.parse().ok()).unwrap_or(0);
            let minute = parts.get(1).and_then(|m| m.parse().ok()).unwrap_or(0);
            let second = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            (hour, minute, second)
        }
        None => (0, 0, 0),
    };

    let month = Month::try_from(month).ok()?;
    let date = Date::from_calendar_date(year, month, day).ok()?;
    let time = Time::from_hms(hour, minute, second).ok()?;
    Some(
        PrimitiveDateTime::new(date, time)
            .assume_utc()
            .unix_timestamp(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_one_dive_per_row() {
        let csv = "\
date,time,duration,depth,water temp,buddy,location,o2,rating,tags
2024-05-12,08:15,42:30,18.4,26,Alex,Blue Hole,32,4,\"reef; teaching\"
2024-05-12,11:30,38:00,22.0,27,Alex,Blue Hole,32,5,reef
";
        let log = parse_str(csv).unwrap();
        assert_eq!(log.dives.len(), 2);
        assert_eq!(log.sites.len(), 1);
        let dive = &log.dives[0];
        assert_eq!(dive.max_depth, Some(Depth::from_meters(18.4)));
        assert_eq!(dive.duration, Some(Duration::new(42 * 60 + 30)));
        assert_eq!(dive.buddy, "Alex");
        assert_eq!(dive.rating, 4);
        assert_eq!(dive.tags, vec!["reef", "teaching"]);
        assert_eq!(dive.cylinders[0].gas.o2_permille, 320);
        assert_eq!(log.sites[0].name, "Blue Hole");
        // Both dives share the site.
        assert_eq!(log.dives[0].site_id, log.dives[1].site_id);
    }

    #[test]
    fn handles_semicolon_delimiter_and_units() {
        let csv = "\
date;duration;depth;weight
12/05/2024;45;30,5 ft;6 kg
";
        let log = parse_str(csv).unwrap();
        assert_eq!(log.dives.len(), 1);
        let dive = &log.dives[0];
        // 30.5 ft is a touch over 9 m.
        assert!((dive.max_depth.unwrap().meters() - 9.3).abs() < 0.2);
        assert_eq!(dive.weights[0].weight, Weight::from_kg(6.0));
        assert_eq!(dive.duration, Some(Duration::new(45 * 60)));
    }
}

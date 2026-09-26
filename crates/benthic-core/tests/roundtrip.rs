//! End-to-end tests for the native model and the two file formats.

use benthic_core::gas::{GasMix, AIR};
use benthic_core::io::{self, json, ssrf, Format};
use benthic_core::units::*;

const DEMO: &str = include_str!("../../../dives/demo.ssrf");

#[test]
fn parses_demo_ssrf() {
    let log = ssrf::parse_str(DEMO).expect("demo parses");
    assert_eq!(log.dives.len(), 3);
    assert_eq!(log.trips.len(), 1);
    // One explicit site + one synthesized from the inline <location>.
    assert_eq!(log.sites.len(), 2);

    let first = log.dives_sorted().first().copied().unwrap();
    assert_eq!(first.number, 1);
    assert_eq!(first.tags, vec!["reef", "training"]);
    assert_eq!(first.cylinders.len(), 1);
    assert!(first.cylinders[0].gas.is_air());
    let dc = first.primary_computer().unwrap();
    assert_eq!(dc.samples.len(), 7);
    assert_eq!(dc.events.len(), 2);
    assert_eq!(dc.max_depth, Some(Depth::from_meters(18.4)));
}

#[test]
fn ssrf_roundtrip_is_stable() {
    let log = ssrf::parse_str(DEMO).unwrap();
    let written = ssrf::write_string(&log);
    let again = ssrf::parse_str(&written).expect("written ssrf reparses");

    assert_eq!(again.dives.len(), log.dives.len());
    assert_eq!(again.trips.len(), log.trips.len());
    assert_eq!(again.sites.len(), log.sites.len());

    for (a, b) in log.dives_sorted().iter().zip(again.dives_sorted()) {
        assert_eq!(a.number, b.number);
        assert_eq!(a.when, b.when);
        assert_eq!(a.duration(), b.duration());
        assert_eq!(a.max_depth(), b.max_depth());
        let (ac, bc) = (a.primary_computer().unwrap(), b.primary_computer().unwrap());
        assert_eq!(ac.samples.len(), bc.samples.len());
        assert_eq!(ac.events.len(), bc.events.len());
    }
}

#[test]
fn json_roundtrip_is_lossless() {
    let log = ssrf::parse_str(DEMO).unwrap();
    let json_text = json::to_string(&log).unwrap();
    assert_eq!(Format::detect(&json_text), Some(Format::BenthicJson));
    let again = json::from_str(&json_text).unwrap();
    assert_eq!(log, again);
}

#[test]
fn format_detection() {
    assert_eq!(Format::detect(DEMO), Some(Format::SubsurfaceXml));
    assert_eq!(Format::detect("{\"version\":1}"), Some(Format::BenthicJson));
    assert_eq!(Format::detect("not a log"), None);
    assert_eq!(
        Format::from_extension("foo.SSRF"),
        Some(Format::SubsurfaceXml)
    );
    assert_eq!(
        Format::from_extension("foo.benthic.json"),
        Some(Format::BenthicJson)
    );
}

#[test]
fn parse_auto_handles_both_formats() {
    let log = io::parse_auto(DEMO).unwrap();
    let as_json = json::to_string(&log).unwrap();
    let reparsed = io::parse_auto(&as_json).unwrap();
    assert_eq!(reparsed.dives.len(), 3);
}

#[test]
fn merge_renumbers_ids() {
    let mut a = ssrf::parse_str(DEMO).unwrap();
    let b = ssrf::parse_str(DEMO).unwrap();
    let (dives, trips, sites) = (a.dives.len(), a.trips.len(), a.sites.len());
    a.merge(b);
    assert_eq!(a.dives.len(), dives * 2);
    assert_eq!(a.trips.len(), trips * 2);
    assert_eq!(a.sites.len(), sites * 2);
    // All dive ids must remain unique.
    let mut ids: Vec<u32> = a.dives.iter().map(|d| d.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), a.dives.len());
}

#[test]
fn autogroup_groups_consecutive_dives() {
    use benthic_core::{Dive, DiveLog};

    let mut log = DiveLog::new();
    log.autogroup = true;
    // Three dives across a day, a long gap, then two more.
    let times = [0, 3_600, 7_200, 10 * 86_400, 10 * 86_400 + 3_600];
    for (i, when) in times.iter().enumerate() {
        log.dives.push(Dive {
            id: i as u32 + 1,
            when: *when,
            ..Default::default()
        });
    }

    let created = log.autogroup_trips(3);
    assert_eq!(created, 2);
    assert_eq!(log.trips.len(), 2);
    assert_eq!(log.dives[0].trip_id, log.dives[1].trip_id);
    assert_eq!(log.dives[1].trip_id, log.dives[2].trip_id);
    assert_ne!(log.dives[2].trip_id, log.dives[3].trip_id);
    assert_eq!(log.dives[3].trip_id, log.dives[4].trip_id);

    // Idempotent: re-running does not accumulate trips.
    assert_eq!(log.autogroup_trips(3), 2);
    assert_eq!(log.trips.len(), 2);

    // Manual clearing removes automatic trips and their links.
    log.clear_auto_trips();
    assert!(log.trips.is_empty());
    assert!(log.dives.iter().all(|d| d.trip_id.is_none()));
}

#[test]
fn autogroup_respects_no_trip_and_disabled_flag() {
    use benthic_core::{Dive, DiveLog};

    let mut log = DiveLog::new();
    log.autogroup = true;
    log.dives.push(Dive {
        id: 1,
        when: 0,
        no_trip: true,
        ..Default::default()
    });
    log.dives.push(Dive {
        id: 2,
        when: 100,
        ..Default::default()
    });
    log.dives.push(Dive {
        id: 3,
        when: 200,
        ..Default::default()
    });
    // The excluded dive splits the group; the remaining pair is grouped.
    assert_eq!(log.autogroup_trips(3), 1);
    assert_eq!(log.dives[0].trip_id, None);
    assert_eq!(log.dives[1].trip_id, log.dives[2].trip_id);

    // A log with autogroup disabled is left untouched.
    let mut disabled = DiveLog::new();
    disabled.autogroup = false;
    disabled.dives.push(Dive::default());
    disabled.dives.push(Dive::default());
    assert_eq!(disabled.autogroup_trips(3), 0);
}

#[test]
fn filter_narrows_the_demo_log() {
    use benthic_core::DiveFilter;

    let log = ssrf::parse_str(DEMO).unwrap();
    let filter = DiveFilter {
        query: "turtles".into(),
        ..Default::default()
    };
    let matches: Vec<_> = log
        .dives
        .iter()
        .filter(|d| filter.matches(d, &log))
        .collect();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].number, 2);
}

#[test]
fn history_undo_redo_through_a_real_log() {
    use benthic_core::{Command, History};

    let mut log = ssrf::parse_str(DEMO).unwrap();
    let mut history = History::new();
    let before = log.dives.len();

    let (index, removed) = log.take_dive(log.dives[0].id).unwrap();
    // Recreate the delete as a command so it can be undone.
    log.insert_dive(index, removed.clone());
    history.record(
        Command::DeleteDive {
            dive: removed,
            index,
        },
        &mut log,
    );
    assert_eq!(log.dives.len(), before - 1);
    history.undo(&mut log);
    assert_eq!(log.dives.len(), before);
    history.redo(&mut log);
    assert_eq!(log.dives.len(), before - 1);
}

#[test]
fn unit_conversions_round_trip() {
    assert!((Depth::from_meters(10.0).meters() - 10.0).abs() < 1e-9);
    assert_eq!(Depth::from_meters(1.0), Depth::new(1000));
    assert!((Pressure::from_bar(200.0).bar() - 200.0).abs() < 1e-9);
    assert!((Temperature::from_celsius(20.0).celsius() - 20.0).abs() < 1e-9);
    assert!((Temperature::from_fahrenheit(68.0).celsius() - 20.0).abs() < 1e-6);
    assert!((Volume::from_liters(12.0).liters() - 12.0).abs() < 1e-9);
    assert!((Weight::from_kg(6.0).kg() - 6.0).abs() < 1e-9);
    assert!((Fraction::from_percent(32.0).percent() - 32.0).abs() < 1e-9);
}

#[test]
fn duration_formatting() {
    assert_eq!(format_duration(Duration::new(90)), "1:30");
    assert_eq!(format_duration(Duration::new(3661)), "1:01:01");
    assert_eq!(format_duration(Duration::new(-90)), "-1:30");
}

#[test]
fn gas_naming() {
    assert_eq!(AIR.name(), "Air");
    assert_eq!(GasMix::percent(32.0, 0.0).name(), "EAN32");
    assert_eq!(GasMix::percent(21.0, 35.0).name(), "Tx21/35");
    assert_eq!(GasMix::percent(100.0, 0.0).name(), "O2");
    assert!(GasMix::percent(32.0, 0.0).is_nitrox());
    assert!(GasMix::percent(21.0, 35.0).is_trimix());
    assert_eq!(GasMix::percent(21.0, 35.0).n2_permille(), 440);
}

//! End-to-end tests for the native model and the two file formats.

use benthic_core::gas::{GasMix, AIR};
use benthic_core::io::{self, json, ssrf, Format};
use benthic_core::units::*;

const DEMO: &str = include_str!("../../../dives/demo.ssrf");

#[test]
fn parses_demo_ssrf() {
    let log = ssrf::parse_str(DEMO).expect("demo parses");
    assert_eq!(log.dives.len(), 16);
    assert_eq!(log.trips.len(), 4);
    assert_eq!(log.sites.len(), 11);

    let first = log.dives_sorted().first().copied().unwrap();
    assert_eq!(first.number, 1);
    assert_eq!(first.tags, vec!["wreck", "technical", "deco"]);
    assert_eq!(first.cylinders.len(), 2);
    assert!(first.cylinders[0].gas.is_air());
    let dc = first.primary_computer().unwrap();
    assert_eq!(dc.samples.len(), 13);
    assert_eq!(dc.events.len(), 3);
    assert_eq!(dc.max_depth, Some(Depth::from_meters(38.4)));
}

/// The demo log is also the showcase, so it must exercise the range of dive
/// styles the model supports: air, nitrox, trimix, CCR and staged deco.
#[test]
fn demo_covers_typical_dive_styles() {
    use benthic_core::Divemode;

    let log = ssrf::parse_str(DEMO).expect("demo parses");

    let nitrox = log
        .dives
        .iter()
        .filter(|d| {
            d.cylinders
                .first()
                .is_some_and(|c| c.gas.he_permille == 0 && (220..=400).contains(&c.gas.o2_permille))
        })
        .count();
    assert_eq!(nitrox, 4, "recreational nitrox dives");

    let trimix = log
        .dives
        .iter()
        .filter(|d| {
            d.computers
                .iter()
                .all(|dc| dc.divemode == Divemode::OpenCircuit)
                && d.cylinders.iter().any(|c| c.gas.he_permille > 0)
        })
        .count();
    assert_eq!(trimix, 1, "open-circuit trimix dives");

    let rebreather = log
        .dives
        .iter()
        .filter(|d| d.computers.iter().any(|dc| dc.divemode == Divemode::Ccr))
        .count();
    assert_eq!(rebreather, 1, "rebreather dive");

    let deco = log
        .dives
        .iter()
        .filter(|d| {
            d.computers
                .iter()
                .any(|dc| dc.samples.iter().any(|s| s.in_deco))
        })
        .count();
    assert_eq!(deco, 8, "dives with a deco obligation");

    let gas_switches: usize = log
        .dives
        .iter()
        .flat_map(|d| d.computers.iter())
        .flat_map(|dc| dc.events.iter())
        .filter(|e| e.is_gas_change())
        .count();
    assert_eq!(gas_switches, 8, "staged gas switches across the demo");

    let freedives = log.dives.iter().filter(|d| d.is_freedive()).count();
    assert_eq!(freedives, 4, "freedives with their own daily sessions");
}

/// The example logbook is a French wreck-diving showcase: every site is in
/// France and the four Var wrecks are dived on air with Nx70 deco.
#[test]
fn demo_is_set_in_france_with_70_percent_deco() {
    let log = ssrf::parse_str(DEMO).expect("demo parses");
    assert!(
        log.sites
            .iter()
            .all(|s| s.country.as_deref() == Some("France")),
        "every demo site must be in France"
    );

    for name in ["Le Donator", "Le Grec", "Le Rubis", "Le Liban"] {
        let site = log
            .sites
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("missing site {name}"));
        let dives: Vec<_> = log
            .dives
            .iter()
            .filter(|d| d.site_id == Some(site.uuid))
            .collect();
        assert!(!dives.is_empty(), "no dive for {name}");
        for dive in dives {
            assert!(
                dive.cylinders.first().is_some_and(|c| c.gas.is_air()),
                "{name} should be dived on air"
            );
            assert!(
                dive.cylinders
                    .iter()
                    .skip(1)
                    .any(|c| c.gas.o2_permille == 700),
                "{name} should have an Nx70 deco bottle"
            );
        }
    }

    for name in ["U-171", "Le Cyrano"] {
        assert!(
            log.sites.iter().any(|s| s.name == name),
            "missing Groix site {name}"
        );
    }
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
    assert_eq!(reparsed.dives.len(), 16);
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
        query: "Liban".into(),
        ..Default::default()
    };
    let matches: Vec<_> = log
        .dives
        .iter()
        .filter(|d| filter.matches(d, &log))
        .collect();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].number, 4);
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
fn average_depth_integrates_the_profile() {
    use benthic_core::{Dive, DiveComputer, Sample};

    // A symmetric triangle: 0 m -> 10 m -> 0 m over 20 s averages 5 m.
    let samples = vec![
        Sample {
            time: Duration::new(0),
            depth: Depth::from_meters(0.0),
            ..Default::default()
        },
        Sample {
            time: Duration::new(10),
            depth: Depth::from_meters(10.0),
            ..Default::default()
        },
        Sample {
            time: Duration::new(20),
            depth: Depth::from_meters(0.0),
            ..Default::default()
        },
    ];
    let dive = Dive {
        computers: vec![DiveComputer {
            samples,
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(dive.average_depth(), Some(Depth::from_meters(5.0)));
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

#[test]
fn location_distance_metres() {
    use benthic_core::Location;

    let a = Location::new(28.5721, 34.5367);
    assert!(a.distance_m(a) < 1.0);
    // About 1.1 km due north.
    let b = Location::new(28.5821, 34.5367);
    let distance = a.distance_m(b);
    assert!(
        (1000.0..1300.0).contains(&distance),
        "distance was {distance}"
    );
}

#[test]
fn merge_duplicate_sites_by_name_and_gps() {
    use benthic_core::{Dive, DiveLog, DiveSite, Location};

    let mut log = DiveLog::new();
    log.sites.push(DiveSite {
        uuid: 1,
        name: "Blue Hole".into(),
        location: Some(Location::new(28.5, 34.5)),
        ..Default::default()
    });
    // Same name, ~11 m away: a duplicate.
    log.sites.push(DiveSite {
        uuid: 2,
        name: "blue  hole".into(),
        location: Some(Location::new(28.5001, 34.5)),
        ..Default::default()
    });
    // Same name but on the other side of the planet: keep separate.
    log.sites.push(DiveSite {
        uuid: 3,
        name: "Blue Hole".into(),
        location: Some(Location::new(10.0, 20.0)),
        ..Default::default()
    });
    log.sites.push(DiveSite {
        uuid: 4,
        name: "Other".into(),
        ..Default::default()
    });
    log.dives.push(Dive {
        id: 1,
        when: 0,
        site_id: Some(2),
        ..Default::default()
    });

    let groups = log.mergeable_site_groups(500.0);
    assert_eq!(groups, vec![vec![1, 2]]);

    let removed = log.merge_duplicate_sites(500.0);
    assert_eq!(removed, 1);
    assert_eq!(log.sites.len(), 3);
    assert_eq!(log.dives[0].site_id, Some(1));
}

#[test]
fn merge_trips_moves_dives_and_keeps_details() {
    use benthic_core::{Dive, DiveLog, DiveTrip};

    let mut log = DiveLog::new();
    log.trips.push(DiveTrip {
        id: 1,
        location: String::new(),
        date: None,
        ..Default::default()
    });
    log.trips.push(DiveTrip {
        id: 2,
        location: "Dahab".into(),
        date: Some(1_000),
        notes: "fun".into(),
        ..Default::default()
    });
    log.dives.push(Dive {
        id: 1,
        when: 1_000,
        trip_id: Some(2),
        ..Default::default()
    });

    assert!(log.merge_trips(1, 2));
    assert_eq!(log.trips.len(), 1);
    let kept = log.trip_by_id(1).unwrap();
    assert_eq!(kept.location, "Dahab");
    assert_eq!(kept.date, Some(1_000));
    assert_eq!(kept.notes, "fun");
    assert_eq!(log.dives[0].trip_id, Some(1));

    // Merging into a missing trip is a no-op.
    assert!(!log.merge_trips(999, 1));
}

#[test]
fn gas_used_and_rmv() {
    use benthic_core::{Cylinder, Dive, DiveComputer, Sample};

    let cylinder = Cylinder {
        size: Some(Volume::from_liters(12.0)),
        start_pressure: Some(Pressure::from_bar(200.0)),
        end_pressure: Some(Pressure::from_bar(60.0)),
        ..Default::default()
    };
    assert_eq!(cylinder.gas_used_liters(), Some(1680.0));

    // 40 minutes at a constant 10 m: RMV is around 21 L/min.
    let samples = vec![
        Sample {
            time: Duration::new(0),
            depth: Depth::from_meters(10.0),
            ..Default::default()
        },
        Sample {
            time: Duration::new(2400),
            depth: Depth::from_meters(10.0),
            ..Default::default()
        },
    ];
    let dive = Dive {
        cylinders: vec![cylinder],
        computers: vec![DiveComputer {
            samples,
            ..Default::default()
        }],
        duration: Some(Duration::new(2400)),
        salinity: Some(10_300),
        ..Default::default()
    };
    let rmv = dive.rmv_l_per_min().expect("rmv");
    assert!((20.0..22.0).contains(&rmv), "rmv was {rmv}");
}

#[test]
fn dives_recent_first_orders_newest_first() {
    let log = ssrf::parse_str(DEMO).unwrap();

    let recent: Vec<i32> = log.dives_recent_first().iter().map(|d| d.number).collect();
    let mut chronological: Vec<i32> = log.dives_sorted().iter().map(|d| d.number).collect();
    chronological.reverse();

    assert_eq!(recent, chronological);
    assert_eq!(recent, (1..=16).rev().collect::<Vec<i32>>());
}

#[test]
fn preparing_a_log_keeps_unnumbered_dives_unnumbered() {
    let mut log = benthic_core::DiveLog::new();
    for (id, number) in [(1, 7), (2, 0)] {
        log.dives.push(benthic_core::Dive {
            id,
            number,
            when: id as i64 * 100,
            ..Default::default()
        });
    }
    log.prepare();
    assert_eq!(log.dive_by_id(1).unwrap().number, 7);
    assert_eq!(log.dive_by_id(2).unwrap().number, 0);
    assert_eq!(log.next_number(), 8);
}

#[test]
fn renumber_is_chronological_and_reports_changes() {
    let mut log = benthic_core::DiveLog::new();
    for (id, number, when) in [(1, 5, 300), (2, 5, 100), (3, 0, 200), (4, 9, 200)] {
        log.dives.push(benthic_core::Dive {
            id,
            number,
            when,
            ..Default::default()
        });
    }

    assert_eq!(log.dives_numbered(5, 1).len(), 1);
    assert!(log.dives_numbered(0, 1).is_empty());

    assert_eq!(log.renumber(10), 4);
    let numbers: Vec<(u32, i32)> = log.dives.iter().map(|d| (d.id, d.number)).collect();
    // Oldest first; the two dives at t=200 keep their id order.
    assert_eq!(numbers, vec![(1, 13), (2, 10), (3, 11), (4, 12)]);
    assert_eq!(log.renumber(10), 0);
}

#[test]
fn site_position_is_the_centre_of_its_dives() {
    use benthic_core::{Dive, DiveLog, DiveSite, Location};
    let mut log = DiveLog::new();
    log.sites.push(DiveSite {
        uuid: 1,
        name: "Reef".into(),
        location: Some(Location::new(10.0, 10.0)),
        ..Default::default()
    });
    log.sites.push(DiveSite {
        uuid: 2,
        name: "Wreck".into(),
        location: Some(Location::new(-5.0, 20.0)),
        ..Default::default()
    });
    for (id, site, location) in [
        (1, 1, Some(Location::new(43.0, 6.0))),
        (2, 1, Some(Location::new(43.002, 6.004))),
        (3, 1, None),
        (4, 2, None),
    ] {
        log.dives.push(Dive {
            id,
            when: id as i64,
            site_id: Some(site),
            location,
            ..Default::default()
        });
    }

    let reef = log.site_location(1).unwrap();
    assert!((reef.lat - 43.001).abs() < 1e-6 && (reef.lon - 6.002).abs() < 1e-6);
    // No dive at the wreck has a point, so its stored position is used.
    assert_eq!(log.site_location(2), Some(Location::new(-5.0, 20.0)));
    // A dive without its own point falls back to its site's position.
    assert_eq!(log.dive_location(&log.dives[2]), Some(reef));

    assert_eq!(log.adopt_site_locations(), 2);
    assert_eq!(log.dives[2].location, Some(Location::new(10.0, 10.0)));
    assert_eq!(log.site_by_name("  reef ").map(|s| s.uuid), Some(1));
    assert_eq!(log.site_dive_counts()[&1], 3);
}

#[test]
fn ssrf_dive_gps_lands_on_the_dive_and_round_trips() {
    let xml = "<divelog program='subsurface' version='2'><divesites>\
        <site uuid='00000001' name='Reef' gps='43.000000 6.000000'></site>\
        </divesites><dives>\
        <dive number='1' date='2024-01-01' time='10:00:00' divesiteid='00000001'></dive>\
        <dive number='2' date='2024-01-02' time='10:00:00' divesiteid='00000001' gps='43.010000 6.010000'></dive>\
        <dive number='3' date='2024-01-03' time='10:00:00'><location gps='45.000000 7.000000'>Lake</location></dive>\
        </dives></divelog>";
    let log = ssrf::parse_str(xml).unwrap();
    let by_number = |n: i32| log.dives.iter().find(|d| d.number == n).unwrap();
    assert_eq!(by_number(1).location, Some(Location::new(43.0, 6.0)));
    assert_eq!(by_number(2).location, Some(Location::new(43.01, 6.01)));
    assert_eq!(by_number(3).location, Some(Location::new(45.0, 7.0)));
    assert_eq!(log.site_name_of(by_number(3)), Some("Lake"));

    let again = ssrf::parse_str(&ssrf::write_string(&log)).unwrap();
    let reef = again.site_by_name("Reef").unwrap();
    let centre = again.site_location(reef.uuid).unwrap();
    assert!((centre.lat - 43.005).abs() < 1e-3 && (centre.lon - 6.005).abs() < 1e-3);
}

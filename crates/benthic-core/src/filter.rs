//! Dive filtering and full-text search.
//!
//! A [`DiveFilter`] is plain data so it can be serialized as a saved filter
//! preset later. Matching is intentionally simple and predictable: every query
//! token must appear somewhere in the dive, and every structured constraint
//! must hold.

use serde::{Deserialize, Serialize};

use crate::model::{Dive, DiveLog};
use crate::units::Depth;

/// Criteria for narrowing the dive list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiveFilter {
    /// Free-text query. All whitespace-separated tokens must match.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub query: String,
    /// Tags that must all be present (case-insensitive).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Minimum star rating (0 = any).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub min_rating: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_depth: Option<Depth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<Depth>,
}

impl DiveFilter {
    /// Whether this filter would exclude anything.
    pub fn is_active(&self) -> bool {
        !self.query.trim().is_empty()
            || !self.tags.is_empty()
            || self.min_rating > 0
            || self.min_depth.is_some()
            || self.max_depth.is_some()
    }

    /// Whether `dive` passes every constraint.
    pub fn matches(&self, dive: &Dive, log: &DiveLog) -> bool {
        if self.min_rating > 0 && dive.rating < self.min_rating {
            return false;
        }
        if let Some(min) = self.min_depth {
            if dive.max_depth().map(|d| d.mm < min.mm).unwrap_or(true) {
                return false;
            }
        }
        if let Some(max) = self.max_depth {
            if dive.max_depth().map(|d| d.mm > max.mm).unwrap_or(true) {
                return false;
            }
        }
        if !self.tags.is_empty() {
            let dive_tags: Vec<String> = dive.tags.iter().map(|t| t.to_lowercase()).collect();
            if !self.tags.iter().all(|want| {
                let want = want.to_lowercase();
                dive_tags.iter().any(|have| have == &want)
            }) {
                return false;
            }
        }
        for token in self.query.split_whitespace() {
            let token = token.to_lowercase();
            if !self.haystack(dive, log).contains(&token) {
                return false;
            }
        }
        true
    }

    /// Everything a free-text query searches over, lower-cased.
    fn haystack(&self, dive: &Dive, log: &DiveLog) -> String {
        let mut parts: Vec<String> = Vec::with_capacity(8);
        parts.push(dive.notes.to_lowercase());
        parts.push(dive.buddy.to_lowercase());
        parts.push(dive.diveguide.to_lowercase());
        parts.push(dive.suit.to_lowercase());
        parts.push(dive.tags.join(" ").to_lowercase());
        if let Some(name) = log.site_name_of(dive) {
            parts.push(name.to_lowercase());
        }
        if dive.number != 0 {
            parts.push(dive.number.to_string());
        }
        if let Some(computer) = dive.primary_computer() {
            parts.push(computer.model.to_lowercase());
        }
        parts.push(crate::units::format_timestamp_utc(dive.when));
        parts.join("\n")
    }
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiveSite, DiveTrip};

    fn log() -> DiveLog {
        let mut log = DiveLog::new();
        log.sites.push(DiveSite {
            uuid: 1,
            name: "Blue Hole".into(),
            ..Default::default()
        });
        log.trips.push(DiveTrip {
            id: 1,
            location: "Dahab".into(),
            ..Default::default()
        });
        log.dives.push(Dive {
            id: 1,
            number: 7,
            when: 1_700_000_000,
            site_id: Some(1),
            trip_id: Some(1),
            buddy: "Alex".into(),
            notes: "Turtles and a friendly moray".into(),
            tags: vec!["reef".into(), "training".into()],
            rating: 4,
            max_depth: Some(Depth::from_meters(24.0)),
            ..Default::default()
        });
        log
    }

    #[test]
    fn empty_filter_matches_everything() {
        let log = log();
        let filter = DiveFilter::default();
        assert!(!filter.is_active());
        assert!(filter.matches(&log.dives[0], &log));
    }

    #[test]
    fn free_text_searches_multiple_fields() {
        let log = log();
        for query in ["turtles", "alex", "blue hole", "reef", "7", "2023"] {
            let filter = DiveFilter {
                query: query.into(),
                ..Default::default()
            };
            assert!(
                filter.matches(&log.dives[0], &log),
                "query {query:?} should match"
            );
        }
        let filter = DiveFilter {
            query: "shark".into(),
            ..Default::default()
        };
        assert!(!filter.matches(&log.dives[0], &log));
    }

    #[test]
    fn all_query_tokens_must_match() {
        let log = log();
        let filter = DiveFilter {
            query: "turtles shark".into(),
            ..Default::default()
        };
        assert!(!filter.matches(&log.dives[0], &log));
    }

    #[test]
    fn structured_constraints() {
        let log = log();
        let dive = &log.dives[0];

        assert!(DiveFilter {
            min_rating: 4,
            ..Default::default()
        }
        .matches(dive, &log));
        assert!(!DiveFilter {
            min_rating: 5,
            ..Default::default()
        }
        .matches(dive, &log));

        assert!(DiveFilter {
            tags: vec!["Reef".into()],
            ..Default::default()
        }
        .matches(dive, &log));
        assert!(!DiveFilter {
            tags: vec!["cave".into()],
            ..Default::default()
        }
        .matches(dive, &log));

        assert!(DiveFilter {
            min_depth: Some(Depth::from_meters(10.0)),
            max_depth: Some(Depth::from_meters(30.0)),
            ..Default::default()
        }
        .matches(dive, &log));
        assert!(!DiveFilter {
            min_depth: Some(Depth::from_meters(30.0)),
            ..Default::default()
        }
        .matches(dive, &log));
    }
}

//! Undo/redo support.
//!
//! Every user-visible mutation goes through a [`Command`]. Applying a command
//! to a [`DiveLog`] and reverting it must be exact inverses, which gives us
//! undo/redo for free and keeps mutations auditable.
//!
//! Most commands store only the minimal data needed to invert them (e.g. the
//! before/after of a single dive). Bulk operations that are awkward to express
//! as fine-grained diffs (import, autogroup) use [`Command::Snapshot`].

use crate::model::{Dive, DiveLog, DiveSite, DiveTrip};

/// A single, invertible mutation of a [`DiveLog`].
///
/// The `Dive` payloads are kept inline: commands are short-lived, the embedded
/// collections are heap-allocated already, and history is bounded, so boxing
/// would add allocation churn for little benefit.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Insert a new dive at `index`.
    AddDive {
        dive: Dive,
        index: usize,
    },
    /// Remove an existing dive; `index` is where it should be put back.
    DeleteDive {
        dive: Dive,
        index: usize,
    },
    /// Replace a dive (matched by id) with an edited copy.
    UpdateDive {
        before: Dive,
        after: Dive,
    },

    AddTrip {
        trip: DiveTrip,
    },
    DeleteTrip {
        trip: DiveTrip,
    },
    UpdateTrip {
        before: DiveTrip,
        after: DiveTrip,
    },

    AddSite {
        site: DiveSite,
    },
    DeleteSite {
        site: DiveSite,
    },
    UpdateSite {
        before: DiveSite,
        after: DiveSite,
    },

    /// A sequence of commands treated as one undo step.
    Compound {
        label: String,
        commands: Vec<Command>,
    },

    /// A coarse-grained, always-correct fallback for bulk edits.
    Snapshot {
        label: String,
        before: Box<DiveLog>,
        after: Box<DiveLog>,
    },
}

impl Command {
    /// A short description shown in undo/redo affordances.
    pub fn label(&self) -> String {
        match self {
            Command::AddDive { .. } => "Add dive".into(),
            Command::DeleteDive { .. } => "Delete dive".into(),
            Command::UpdateDive { .. } => "Edit dive".into(),
            Command::AddTrip { .. } => "Add trip".into(),
            Command::DeleteTrip { .. } => "Delete trip".into(),
            Command::UpdateTrip { .. } => "Edit trip".into(),
            Command::AddSite { .. } => "Add dive site".into(),
            Command::DeleteSite { .. } => "Delete dive site".into(),
            Command::UpdateSite { .. } => "Edit dive site".into(),
            Command::Compound { label, .. } | Command::Snapshot { label, .. } => label.clone(),
        }
    }

    /// Apply the command to a log.
    pub fn apply(&self, log: &mut DiveLog) {
        match self {
            Command::AddDive { dive, index } => log.insert_dive(*index, dive.clone()),
            Command::DeleteDive { dive, .. } => {
                log.remove_dive(dive.id);
            }
            Command::UpdateDive { after, .. } => {
                log.replace_dive(after.clone());
            }
            Command::AddTrip { trip } => log.trips.push(trip.clone()),
            Command::DeleteTrip { trip } => {
                log.trips.retain(|t| t.id != trip.id);
            }
            Command::UpdateTrip { after, .. } => {
                log.replace_trip(after.clone());
            }
            Command::AddSite { site } => log.sites.push(site.clone()),
            Command::DeleteSite { site } => {
                log.sites.retain(|s| s.uuid != site.uuid);
            }
            Command::UpdateSite { after, .. } => {
                log.replace_site(after.clone());
            }
            Command::Compound { commands, .. } => {
                for command in commands {
                    command.apply(log);
                }
            }
            Command::Snapshot { after, .. } => {
                *log = (**after).clone();
            }
        }
    }

    /// Undo the command, restoring the log to its previous state.
    pub fn revert(&self, log: &mut DiveLog) {
        match self {
            Command::AddDive { dive, .. } => {
                log.remove_dive(dive.id);
            }
            Command::DeleteDive { dive, index } => log.insert_dive(*index, dive.clone()),
            Command::UpdateDive { before, .. } => {
                log.replace_dive(before.clone());
            }
            Command::AddTrip { trip } => {
                log.trips.retain(|t| t.id != trip.id);
            }
            Command::DeleteTrip { trip } => log.trips.push(trip.clone()),
            Command::UpdateTrip { before, .. } => {
                log.replace_trip(before.clone());
            }
            Command::AddSite { site } => {
                log.sites.retain(|s| s.uuid != site.uuid);
            }
            Command::DeleteSite { site } => log.sites.push(site.clone()),
            Command::UpdateSite { before, .. } => {
                log.replace_site(before.clone());
            }
            Command::Compound { commands, .. } => {
                for command in commands.iter().rev() {
                    command.revert(log);
                }
            }
            Command::Snapshot { before, .. } => {
                *log = (**before).clone();
            }
        }
    }
}

/// Build the commands that delete several dives as one undo step.
///
/// Commands are ordered by *decreasing* original index, so applying them in
/// sequence never invalidates a later index. [`Command::Compound`] reverts in
/// reverse order, which restores every dive to its original position.
/// Unknown ids are ignored.
pub fn delete_dives(log: &DiveLog, ids: &[u32]) -> Vec<Command> {
    let mut pairs: Vec<(usize, Dive)> = ids
        .iter()
        .filter_map(|id| {
            log.dives
                .iter()
                .position(|d| d.id == *id)
                .map(|index| (index, log.dives[index].clone()))
        })
        .collect();
    pairs.sort_by_key(|(index, _)| std::cmp::Reverse(*index));
    pairs
        .into_iter()
        .map(|(index, dive)| Command::DeleteDive { dive, index })
        .collect()
}

/// An undo/redo stack of commands.
#[derive(Debug, Clone, PartialEq)]
pub struct History {
    undo: Vec<Command>,
    redo: Vec<Command>,
    limit: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    /// Default maximum number of retained undo steps.
    pub const DEFAULT_LIMIT: usize = 200;

    pub fn new() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit: Self::DEFAULT_LIMIT,
        }
    }

    pub fn with_limit(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Apply `command` to `log`, record it, and clear the redo stack.
    pub fn record(&mut self, command: Command, log: &mut DiveLog) {
        command.apply(log);
        self.undo.push(command);
        self.redo.clear();
        while self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    /// Undo the most recent command. Returns its label.
    pub fn undo(&mut self, log: &mut DiveLog) -> Option<String> {
        let command = self.undo.pop()?;
        command.revert(log);
        let label = command.label();
        self.redo.push(command);
        Some(label)
    }

    /// Redo the most recently undone command. Returns its label.
    pub fn redo(&mut self, log: &mut DiveLog) -> Option<String> {
        let command = self.redo.pop()?;
        command.apply(log);
        let label = command.label();
        self.undo.push(command);
        Some(label)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Label of the next undo step, if any.
    pub fn undo_label(&self) -> Option<String> {
        self.undo.last().map(Command::label)
    }

    /// Label of the next redo step, if any.
    pub fn redo_label(&self) -> Option<String> {
        self.redo.last().map(Command::label)
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gas::AIR;
    use crate::model::{Cylinder, DiveComputer, Divemode};
    use crate::units::{Depth, Duration, Timestamp};

    fn sample_log() -> DiveLog {
        let mut log = DiveLog::new();
        log.dives.push(Dive {
            id: 1,
            number: 1,
            when: 1_000,
            cylinders: vec![Cylinder {
                gas: AIR,
                ..Default::default()
            }],
            computers: vec![DiveComputer::default()],
            ..Default::default()
        });
        log
    }

    #[test]
    fn add_and_undo_restores_order() {
        let mut log = sample_log();
        let mut history = History::new();
        let dive = Dive {
            id: 2,
            number: 2,
            when: 2_000,
            ..Default::default()
        };
        let index = log.dives.len();
        history.record(
            Command::AddDive {
                dive: dive.clone(),
                index,
            },
            &mut log,
        );
        assert_eq!(log.dives.len(), 2);
        history.undo(&mut log);
        assert_eq!(log.dives.len(), 1);
        assert!(log.dive_by_id(2).is_none());
        history.redo(&mut log);
        assert_eq!(log.dives.len(), 2);
        assert_eq!(log.dives[1].id, 2);
    }

    #[test]
    fn update_is_invertible() {
        let mut log = sample_log();
        let mut history = History::new();
        let mut after = log.dives[0].clone();
        after.notes = "hello".into();
        after.max_depth = Some(Depth::from_meters(20.0));
        after.duration = Some(Duration::new(1800));
        after.computers[0].divemode = Divemode::Ccr;
        history.record(
            Command::UpdateDive {
                before: log.dives[0].clone(),
                after,
            },
            &mut log,
        );
        assert_eq!(log.dives[0].notes, "hello");
        history.undo(&mut log);
        assert_eq!(log.dives[0].notes, "");
        assert_eq!(log.dives[0].max_depth, None);
    }

    #[test]
    fn compound_reverts_in_reverse() {
        let mut log = sample_log();
        let mut history = History::new();
        let site = DiveSite {
            uuid: 7,
            name: "Test".into(),
            ..Default::default()
        };
        history.record(
            Command::Compound {
                label: "add dive and site".into(),
                commands: vec![
                    Command::AddSite { site: site.clone() },
                    Command::AddDive {
                        dive: Dive {
                            id: 5,
                            when: 3_000,
                            site_id: Some(7),
                            ..Default::default()
                        },
                        index: 1,
                    },
                ],
            },
            &mut log,
        );
        assert_eq!(log.dives.len(), 2);
        assert_eq!(log.sites.len(), 1);
        history.undo(&mut log);
        assert_eq!(log.dives.len(), 1);
        assert_eq!(log.sites.len(), 0);
    }

    #[test]
    fn snapshot_restores_exactly() {
        let mut log = sample_log();
        let before = log.clone();
        let mut history = History::new();
        let mut after = log.clone();
        after.autogroup = false;
        after.dives[0].notes = "changed".into();
        history.record(
            Command::Snapshot {
                label: "autogroup".into(),
                before: Box::new(before.clone()),
                after: Box::new(after),
            },
            &mut log,
        );
        assert_eq!(log.dives[0].notes, "changed");
        history.undo(&mut log);
        assert_eq!(log, before);
    }

    #[test]
    fn redo_is_cleared_by_new_command() {
        let mut log = sample_log();
        let mut history = History::new();
        history.record(
            Command::AddDive {
                dive: Dive {
                    id: 2,
                    when: 2,
                    ..Default::default()
                },
                index: 1,
            },
            &mut log,
        );
        history.undo(&mut log);
        assert!(history.can_redo());
        history.record(
            Command::AddDive {
                dive: Dive {
                    id: 3,
                    when: 3,
                    ..Default::default()
                },
                index: 1,
            },
            &mut log,
        );
        assert!(!history.can_redo());
    }

    #[test]
    fn limit_is_enforced() {
        let mut log = DiveLog::new();
        let mut history = History::with_limit(3);
        for i in 0..10u32 {
            history.record(
                Command::AddDive {
                    dive: Dive {
                        id: i + 1,
                        when: i as Timestamp,
                        ..Default::default()
                    },
                    index: log.dives.len(),
                },
                &mut log,
            );
        }
        let mut undone = 0;
        while history.undo(&mut log).is_some() {
            undone += 1;
        }
        assert_eq!(undone, 3);
        assert_eq!(log.dives.len(), 7);
    }

    #[test]
    fn bulk_delete_is_exactly_reversible() {
        let mut log = DiveLog::new();
        for i in 0..5u32 {
            log.dives.push(Dive {
                id: i + 1,
                when: i as Timestamp,
                ..Default::default()
            });
        }
        let before = log.clone();
        let mut history = History::new();

        let commands = delete_dives(&log, &[2, 4, 5]);
        history.record(
            Command::Compound {
                label: "delete dives".into(),
                commands,
            },
            &mut log,
        );
        assert_eq!(
            log.dives.iter().map(|d| d.id).collect::<Vec<_>>(),
            vec![1, 3]
        );

        history.undo(&mut log);
        assert_eq!(log, before);
    }

    #[test]
    fn delete_dives_ignores_unknown_ids() {
        let log = sample_log();
        let commands = delete_dives(&log, &[1, 999]);
        assert_eq!(commands.len(), 1);
    }
}

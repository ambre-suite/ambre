//! The persisted schema version -- the protocol half
//! (`ambre-spec-schema-version.md`).
//!
//! A stored document is a valid CRDT whatever it holds, so an engine imports it
//! happily and then reads containers that no longer mean what it expects. A
//! successful open of a wrong document is silent loss, and this module is what
//! makes that case decidable: where the number lives, what an absent one means,
//! what each comparison implies, and how a registry of forward steps is walked.
//!
//! What is *not* here, deliberately:
//!
//! * **the number itself.** Each application declares its own current version
//!   beside its own containers, because a Trame layout change says nothing about
//!   a Toile one.
//! * **the migrations.** What a container meant in version 2 and means in
//!   version 3 is the application's own semantics (`SCHEMA-010`), so the steps
//!   are supplied by the caller and only walked here.
//! * **any CRDT type.** This crate is linked by `ambre-server`, which must never
//!   be able to decode a document (`ZK-001`), so nothing here knows what a
//!   `LoroDoc` is. [`migrate_schema`] is generic over the document it walks and
//!   the applications instantiate it at their own engine type.
//!
//! Shaped like [`crate::docfile`] on purpose: two versions guard a stored
//! document -- the container header guards the bytes around the payload, this
//! one guards what is inside -- and a reader who has learnt one carries the
//! other for free.

use thiserror::Error;

/// The root map every application writes its schema version into.
pub const SCHEMA_META_MAP: &str = "meta";

/// The key inside [`SCHEMA_META_MAP`] holding the version, an unsigned integer.
pub const SCHEMA_VERSION_KEY: &str = "schema_version";

/// What a document with no version register reads as (`SCHEMA-005`).
///
/// Absent means zero so that no document written before the register existed
/// needs rewriting; the alternative is a migration whose only purpose is to
/// enable migrations.
pub const SCHEMA_VERSION_ABSENT: u32 = 0;

/// The comparison of a document's schema version against the binary's, and the
/// only three outcomes there are (`SCHEMA-006`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaVerdict {
    /// The versions match: the document is used as it is.
    Current,
    /// The document predates this binary: its migration chain runs first
    /// (`SCHEMA-007`).
    Older,
    /// The document was written by a newer binary: the open is refused whole
    /// (`SCHEMA-008`).
    Newer,
}

/// Compare a document's version with this binary's.
///
/// A free function rather than an `Ord` impl: the three names are the point --
/// a call site that reads `Older` runs a migration, where one that reads
/// `Less` has to remember which way round the arguments went.
#[must_use]
pub fn verdict(document: u32, binary: u32) -> SchemaVerdict {
    match document.cmp(&binary) {
        std::cmp::Ordering::Equal => SchemaVerdict::Current,
        std::cmp::Ordering::Less => SchemaVerdict::Older,
        std::cmp::Ordering::Greater => SchemaVerdict::Newer,
    }
}

/// Why a full-state read refused a document.
///
/// Every variant is fatal to *that read* and to nothing else: the caller still
/// holds the state it had, because no path here is reached with the document
/// already applied (`SCHEMA-012`).
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    /// The document was written by a newer client. Not recoverable by anything
    /// this binary can do: reading it on a best-effort basis is the silent-loss
    /// case the register exists to remove, so the user is asked to update
    /// (`SCHEMA-008`).
    #[error(
        "this document was written by a newer client (schema {document}, this build reads {binary}); update to open it"
    )]
    DocumentTooNew { document: u32, binary: u32 },

    /// The document is older and the registry has no step leaving `from`. A
    /// gap in the chain, not a property of the document.
    #[error("no migration path from schema version {from}")]
    NoMigrationPath { from: u32 },

    /// A registry step ran and failed. Carries the step so a report names the
    /// hop rather than the whole chain.
    #[error("migrating schema {from} -> {to} failed: {reason}")]
    MigrationFailed { from: u32, to: u32, reason: String },

    /// The register is present and this build cannot read it: not an integer, or
    /// an integer outside `u32`.
    ///
    /// Refused rather than read as absent. A register this build does not
    /// understand belongs to a document it does not understand, and reading it
    /// as version 0 would open a *newer* document as an older one -- the
    /// silent-loss case the whole register exists to remove. `found` describes
    /// the value for a report, never for a decision.
    #[error("the schema version register holds {found}, which this build cannot read")]
    VersionUnreadable { found: String },

    /// The bytes never reached a version check: the CRDT engine refused them.
    /// Distinct from every variant above, which are all *successful* reads of a
    /// document this binary declines to use.
    #[error("the snapshot was rejected by the CRDT engine")]
    PayloadRejected,
}

/// One forward step of an application's schema registry: `from` -> `to`, run
/// against a document already loaded into that application's engine.
///
/// Generic over the document type so the walk can live here while the steps
/// live in the application crate (`SCHEMA-010`). The step mutates through a
/// shared reference because that is the shape a CRDT handle has: a `LoroDoc`
/// writes through `&self`.
pub struct SchemaMigration<D> {
    pub from: u32,
    pub to: u32,
    /// Returns the reason on failure; the walk wraps it into
    /// [`SchemaError::MigrationFailed`] with the hop it came from.
    pub apply: fn(&D) -> Result<(), String>,
}

/// Walk an application's registry from `from` up to `target`, one step at a
/// time (`SCHEMA-007`).
///
/// `from == target` is the identity and runs nothing. A `from` above `target`
/// is [`SchemaError::DocumentTooNew`]: the caller is expected to have refused it
/// already, and a chain that silently accepted it would be a downgrade path.
///
/// # Errors
/// [`SchemaError::NoMigrationPath`] when no step leaves the version reached, and
/// [`SchemaError::MigrationFailed`] when a step itself fails.
pub fn migrate_schema<D>(
    steps: &[SchemaMigration<D>],
    from: u32,
    target: u32,
    doc: &D,
) -> Result<(), SchemaError> {
    if from > target {
        return Err(SchemaError::DocumentTooNew {
            document: from,
            binary: target,
        });
    }
    let mut current = from;
    while current < target {
        // `to > current` is required, not merely expected: it is what makes the
        // walk terminate, whatever a future registry row looks like.
        let step = steps
            .iter()
            .find(|step| step.from == current && step.to > current)
            .ok_or(SchemaError::NoMigrationPath { from: current })?;
        (step.apply)(doc).map_err(|reason| SchemaError::MigrationFailed {
            from: step.from,
            to: step.to,
            reason,
        })?;
        current = step.to;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A document the walk can write to, standing in for a `LoroDoc`: the steps
    /// record the hops they ran, which is what the ordering assertions read.
    #[derive(Default)]
    struct Trace(RefCell<Vec<u32>>);

    fn step_to(to: u32) -> fn(&Trace) -> Result<(), String> {
        // One body per version this test registry reaches; a closure cannot be
        // used because a registry row holds a plain `fn`.
        match to {
            1 => |t: &Trace| {
                t.0.borrow_mut().push(1);
                Ok(())
            },
            2 => |t: &Trace| {
                t.0.borrow_mut().push(2);
                Ok(())
            },
            _ => |t: &Trace| {
                t.0.borrow_mut().push(3);
                Ok(())
            },
        }
    }

    fn registry() -> Vec<SchemaMigration<Trace>> {
        (1..=3)
            .map(|to| SchemaMigration {
                from: to - 1,
                to,
                apply: step_to(to),
            })
            .collect()
    }

    #[test]
    fn an_absent_register_reads_as_zero_and_zero_is_older_than_any_release() {
        assert_eq!(SCHEMA_VERSION_ABSENT, 0);
        assert_eq!(verdict(SCHEMA_VERSION_ABSENT, 1), SchemaVerdict::Older);
    }

    #[test]
    fn the_three_comparisons_are_the_three_verdicts() {
        assert_eq!(verdict(3, 3), SchemaVerdict::Current);
        assert_eq!(verdict(2, 3), SchemaVerdict::Older);
        assert_eq!(verdict(4, 3), SchemaVerdict::Newer);
    }

    #[test]
    fn a_chain_runs_every_hop_in_order() {
        let doc = Trace::default();
        migrate_schema(&registry(), 0, 3, &doc).expect("the registry is complete");
        assert_eq!(*doc.0.borrow(), vec![1, 2, 3]);
    }

    #[test]
    fn a_document_at_the_current_version_runs_nothing() {
        let doc = Trace::default();
        migrate_schema(&registry(), 3, 3, &doc).expect("the identity");
        assert!(doc.0.borrow().is_empty());
    }

    #[test]
    fn a_gap_in_the_registry_stops_the_walk_where_it_broke() {
        let doc = Trace::default();
        let gapped: Vec<_> = registry().into_iter().filter(|s| s.from != 1).collect();
        let err = migrate_schema(&gapped, 0, 3, &doc).expect_err("the walk cannot reach 3");
        assert_eq!(err, SchemaError::NoMigrationPath { from: 1 });
        // The hop that did run is not rolled back -- the caller discards the
        // scratch document, which is why nothing here has to undo.
        assert_eq!(*doc.0.borrow(), vec![1]);
    }

    #[test]
    fn a_failing_step_names_its_own_hop() {
        let doc = Trace::default();
        let steps = vec![SchemaMigration {
            from: 0,
            to: 1,
            apply: (|_: &Trace| Err("the register would not write".to_owned()))
                as fn(&Trace) -> Result<(), String>,
        }];
        let err = migrate_schema(&steps, 0, 1, &doc).expect_err("the step fails");
        assert_eq!(
            err,
            SchemaError::MigrationFailed {
                from: 0,
                to: 1,
                reason: "the register would not write".to_owned(),
            }
        );
    }

    #[test]
    fn the_walk_refuses_to_run_backwards() {
        let doc = Trace::default();
        let err = migrate_schema(&registry(), 4, 3, &doc).expect_err("no downgrade path");
        assert_eq!(
            err,
            SchemaError::DocumentTooNew {
                document: 4,
                binary: 3
            }
        );
        assert!(doc.0.borrow().is_empty());
    }
}

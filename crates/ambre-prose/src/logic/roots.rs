//! The root containers a document holds, known without opening them
//! (`features/ambre-spec-prose-pagination.md section 2`, `PROSE-142`).
//!
//! Loro 1.13 reads the presence of a root only two ways. Opening the root
//! attaches it when it is absent. Building its value walks the whole tree,
//! which `PROSE-140` refuses. So the Logic thread notes each root as the
//! operations it commits and imports reach it. A read asks this set first,
//! and an absent root reads as an empty view, with the document untouched.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use ambre_core::crdt::CrdtName as _;
use loro::event::DiffEvent;
use loro::{ContainerID, ContainerType, Frontiers, ID, LoroDoc, Subscription};

use crate::doc::Root;

type Found = Arc<Mutex<HashSet<Root>>>;

/// The roots of one document that its operations created, kept current by a
/// subscription to every commit and every import (`PROSE-142`).
///
/// The set only grows. A root a peer created stays in the history, so no later
/// change removes it.
pub struct Roots {
    found: Found,
    /// Held so the subscription lives as long as the set.
    _subscription: Subscription,
}

impl Roots {
    /// Note the roots of `document` from now on, and the roots its committed
    /// history already holds.
    ///
    /// The scan of the history commits the pending transaction, so the caller
    /// watches a document with none: at construction, before the first write.
    /// The scan reads the operations, so a shallow history hides the roots
    /// written before its start. Watch before the first import of one.
    pub(crate) fn watch(document: &LoroDoc) -> Self {
        let found: Found = Arc::default();
        let sink = Arc::clone(&found);
        // The subscription comes first, so no commit falls between the scan
        // and the first event.
        let subscription = document.subscribe_root(Arc::new(move |event: DiffEvent<'_>| {
            let mut roots = sink.lock().unwrap_or_else(PoisonError::into_inner);
            for change in &event.events {
                // The path starts at the root of the container written to.
                let root = change.path.first().map_or(change.target, |(id, _)| id);
                roots.extend(root_named(root));
            }
        }));
        let history =
            document.find_id_spans_between(&Frontiers::default(), &document.oplog_frontiers());
        {
            let mut roots = found.lock().unwrap_or_else(PoisonError::into_inner);
            for (peer, span) in history.forward.iter() {
                let Ok(len) = usize::try_from(span.end.saturating_sub(span.start)) else {
                    continue;
                };
                let written = document.get_changed_containers_in(ID::new(*peer, span.start), len);
                roots.extend(written.iter().filter_map(root_named));
            }
        }
        Self {
            found,
            _subscription: subscription,
        }
    }

    /// Whether an operation of the document reached `root`. A reader opens
    /// `root` only when it did, so the read attaches nothing (`PROSE-142`).
    #[must_use]
    pub(crate) fn holds(&self, root: Root) -> bool {
        self.found
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&root)
    }
}

impl std::fmt::Debug for Roots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let found = self.found.lock().unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("Roots").field("found", &*found).finish()
    }
}

/// The root of the schema `id` names: the same name and the same kind. A
/// container of another kind under a schema name is not that root.
fn root_named(id: &ContainerID) -> Option<Root> {
    let ContainerID::Root {
        name,
        container_type,
    } = id
    else {
        return None;
    };
    Root::ALL
        .iter()
        .copied()
        .find(|root| root.as_str() == name.as_str() && root.kind() == *container_type)
}

impl Root {
    /// The kind of container the schema opens under this name.
    pub(crate) const fn kind(self) -> ContainerType {
        match self {
            Self::Blocks => ContainerType::Tree,
            Self::Meta | Self::Styles | Self::Annots => ContainerType::Map,
        }
    }
}

#[cfg(test)]
mod tests {
    use ambre_core::crdt::CrdtName as _;
    use loro::{ExportMode, LoroDoc, LoroValue};

    use super::Roots;
    use crate::doc::{self, Root};

    /// A document with operations in `annots` alone: no tree, no `meta`.
    fn annotations_alone() -> LoroDoc {
        let document = LoroDoc::new();
        doc::annots(&document)
            .insert("committed", 1)
            .expect("the map takes a value");
        document.commit();
        document
    }

    /// Every reader that opens the tree or `meta`, once each.
    fn read_all(document: &LoroDoc, roots: &Roots) {
        assert!(doc::blocks_read(document, roots).is_none());
        assert!(doc::meta_read(document, roots).is_none());
        let _ = doc::read_bands(document, roots);
        let _ = doc::read_page_format(document, roots);
        let _ = doc::read_defaults(document, roots);
        let _ = doc::read_title(document, roots);
        let _ = doc::read_locale(document, roots);
        let _ = doc::read_updated_at(document, roots);
        let count = crate::count::compute_word_count(document, roots);
        assert_eq!(count, crate::events::WordCount::default());
    }

    /// The names of the roots the document state holds.
    fn root_names(document: &LoroDoc) -> Vec<String> {
        match document.get_deep_value() {
            LoroValue::Map(map) => map.keys().cloned().collect(),
            _ => Vec::new(),
        }
    }

    /// `PROSE-142`: a read of a document with no tree attaches no root. The
    /// frontiers, the pending operations, the roots of the state and the
    /// encoded bytes stay as they were.
    #[test]
    fn a_read_of_a_document_with_no_tree_attaches_no_root() {
        let document = annotations_alone();
        let roots = Roots::watch(&document);
        assert!(!roots.holds(Root::Blocks));
        assert!(!roots.holds(Root::Meta));
        // A write still in its transaction: the reads must leave it there.
        doc::annots(&document)
            .insert("pending", 2)
            .expect("the map takes a value");
        let frontiers = document.state_frontiers();
        let pending = document.get_pending_txn_len();
        assert!(pending > 0, "one write waits in the transaction");
        read_all(&document, &roots);
        assert_eq!(document.state_frontiers(), frontiers);
        assert_eq!(document.get_pending_txn_len(), pending);

        // The export commits, so the bytes are compared on a second copy
        // with nothing pending.
        let settled = annotations_alone();
        let roots = Roots::watch(&settled);
        let names = root_names(&settled);
        let snapshot = settled.export(ExportMode::Snapshot).expect("a snapshot");
        let updates = settled
            .export(ExportMode::all_updates())
            .expect("the updates");
        read_all(&settled, &roots);
        assert_eq!(root_names(&settled), names);
        assert!(!names.iter().any(|name| name == Root::Blocks.as_str()));
        assert!(!names.iter().any(|name| name == Root::Meta.as_str()));
        assert_eq!(settled.export(ExportMode::Snapshot).ok(), Some(snapshot));
        assert_eq!(
            settled.export(ExportMode::all_updates()).ok(),
            Some(updates)
        );
    }

    /// `PROSE-142`: the set follows an import that adds the tree and `meta`,
    /// and a local commit that adds them. A map a peer wrote under the name
    /// of the tree is not the tree.
    #[test]
    fn the_roots_follow_an_import_that_adds_the_tree() {
        let reader = annotations_alone();
        let roots = Roots::watch(&reader);

        let rival = LoroDoc::new();
        rival
            .get_map(Root::Blocks.as_str())
            .insert("forged", 1)
            .expect("the map takes a value");
        rival.commit();
        reader
            .import(
                &rival
                    .export(ExportMode::all_updates())
                    .expect("the updates"),
            )
            .expect("the import");
        assert!(!roots.holds(Root::Blocks), "a map is not the tree");
        assert!(doc::blocks_read(&reader, &roots).is_none());

        let author = LoroDoc::new();
        let paragraph = doc::seed(&author).expect("the scaffolding");
        let _ = doc::get_block_text(&author, &paragraph).insert(0, "one two");
        author.commit();
        reader
            .import(
                &author
                    .export(ExportMode::all_updates())
                    .expect("the updates"),
            )
            .expect("the import");
        assert!(roots.holds(Root::Blocks));
        assert!(roots.holds(Root::Meta));
        assert!(doc::blocks_read(&reader, &roots).is_some());
        let count = crate::count::compute_word_count(&reader, &roots);
        assert_eq!(count.words.get(), 2);

        let local = annotations_alone();
        let roots = Roots::watch(&local);
        let _ = doc::seed(&local).expect("the scaffolding");
        assert!(!roots.holds(Root::Blocks), "no commit yet");
        local.commit();
        assert!(roots.holds(Root::Blocks));
        assert!(roots.holds(Root::Meta));
    }

    /// A snapshot imported into a watched document names its roots too.
    #[test]
    fn the_roots_follow_a_snapshot_import() {
        let author = LoroDoc::new();
        let _ = doc::seed(&author).expect("the scaffolding");
        author.commit();
        let reader = LoroDoc::new();
        let roots = Roots::watch(&reader);
        reader
            .import(&author.export(ExportMode::Snapshot).expect("a snapshot"))
            .expect("the import");
        assert!(roots.holds(Root::Blocks));
        assert!(roots.holds(Root::Meta));
    }
}

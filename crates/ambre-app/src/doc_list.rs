// ambre-app/src/doc_list.rs
// ============================================================================
// The order, the groups and the search of the launcher's document list
// (`ambre-spec-launcher.md` section 1.8).
//
// Pure functions over the rows the Sync thread already decrypted. Nothing here
// opens a document, decrypts a byte or sends a request, so a search tells the
// relay nothing and costs no document read (`CHROME-122`). The view in
// `landing.rs` renders what `arrange` returns, and the host tests below hold
// the rules without a browser.
// ============================================================================

use std::cmp::Ordering;

use ambre_bus::DocType;
use ambre_core::id::{Document, Id};
use ambre_core::{Instant, Relayed, Sec, TimeSpan, UnixMs};

/// One row in the document list, already reduced to display-ready fields.
/// `name` is the plaintext title decrypted by the Sync thread (`None` = never
/// named -> "Untitled" placeholder, `ambre-spec-launcher.md`, CHROME-036).
#[derive(Clone)]
pub struct DocItem {
    pub document_id: Id<Document>,
    pub doc_type: DocType,
    pub name: Option<ambre_core::Validated<ambre_core::Name>>,
    /// The relay's clock at the last modification (routing metadata,
    /// plaintext). `None` when no relay listed the document.
    pub updated_at: Option<Instant<Relayed>>,
    /// Total member count; `> 1` => shared.
    pub member_count: ambre_core::Count<ambre_core::Members>,
}

/// One day, the width of the first band.
const DAY: TimeSpan<Sec> = TimeSpan::new(86_400);

/// The band a document's last write falls in, measured from the reader's
/// clock (`CHROME-119`). The declaration order is the display order: the most
/// recent band first, and the documents no relay listed last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Recency {
    /// Written less than one day ago.
    PastDay,
    /// Written less than seven days ago.
    PastWeek,
    /// Written less than thirty days ago.
    PastMonth,
    /// Written thirty days ago or more.
    Older,
    /// No relay listed the document: it is known to this session alone.
    Unlisted,
}

impl Recency {
    /// The band of a last write `updated_at`, read at `now`.
    ///
    /// The bands measure elapsed time, never calendar days, so they need no
    /// time zone and agree with the relative time on the card. A reader whose
    /// clock runs behind the relay's sees a write in the first band.
    pub fn of(updated_at: Option<Instant<Relayed>>, now: Instant<UnixMs>) -> Self {
        let Some(updated_at) = updated_at else {
            return Self::Unlisted;
        };
        let age = updated_at.age_at(now).to_floor::<Sec>();
        if age < DAY {
            Self::PastDay
        } else if age < DAY.times(7) {
            Self::PastWeek
        } else if age < DAY.times(30) {
            Self::PastMonth
        } else {
            Self::Older
        }
    }

    /// The heading the list shows above the band.
    pub fn heading(self) -> &'static str {
        match self {
            Self::PastDay => "Past day",
            Self::PastWeek => "Past week",
            Self::PastMonth => "Past month",
            Self::Older => "Older",
            Self::Unlisted => "On this device",
        }
    }
}

/// One row of the arranged list: a document, and the band it opens when it is
/// the first of its band.
#[derive(Clone)]
pub struct Row {
    /// The band whose heading the list shows above this row, if any.
    pub opens: Option<Recency>,
    pub doc: DocItem,
}

/// The list order of two documents: the most recent write first, then the
/// name, then the identifier. A UUID orders as its bytes; the identifier keeps
/// no `Ord` of its own.
fn list_order(a: &DocItem, b: &DocItem) -> Ordering {
    b.updated_at
        .cmp(&a.updated_at)
        .then_with(|| name_order(a, b))
        .then_with(|| a.document_id.as_bytes().cmp(b.document_id.as_bytes()))
}

/// The order of two names: a named document first, then the names folded to
/// lower case.
fn name_order(a: &DocItem, b: &DocItem) -> Ordering {
    let named = |n: &&ambre_core::Validated<ambre_core::Name>| !n.as_str().trim().is_empty();
    match (a.name.as_ref().filter(named), b.name.as_ref().filter(named)) {
        (Some(a), Some(b)) => a
            .as_str()
            .trim()
            .to_lowercase()
            .cmp(&b.as_str().trim().to_lowercase()),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// The documents of `docs` that answer `query`, in list order, each band
/// opened on its first row.
///
/// The order is the relay's last write, the most recent first. A tie is broken
/// by the folded name, a named document before an unnamed one, then by the
/// identifier (`CHROME-118`). So two listings of one set read the same,
/// whatever order the relay or the local store handed them in.
///
/// A query keeps the documents whose name contains it in any case, with the
/// rule of the command palette (`CHROME-120`). A blank query keeps every
/// document, and an unnamed one answers no other (`CHROME-121`). A band opens
/// on a row alone, so a band no document falls in has no heading
/// (`CHROME-126`).
pub fn arrange(docs: &[DocItem], query: &str, now: Instant<UnixMs>) -> Vec<Row> {
    let blank = query.trim().is_empty();
    let mut kept: Vec<&DocItem> = docs
        .iter()
        .filter(|d| {
            blank
                || d.name
                    .as_ref()
                    .is_some_and(|n| ambre_chrome::command_matches(n.as_str(), query))
        })
        .collect();
    // An insertion sort, not the slice sort. The relay already lists by its
    // last write, so the rows arrive nearly in order and the pass is close to
    // linear. The slice sort would add its machinery to the first download.
    for next in 1..kept.len() {
        let mut at = next;
        while at > 0 {
            let (Some(before), Some(here)) = (kept.get(at - 1), kept.get(at)) else {
                break;
            };
            if list_order(before, here) != Ordering::Greater {
                break;
            }
            kept.swap(at - 1, at);
            at -= 1;
        }
    }
    // The order is by recency, so each band is one run of the sorted rows.
    let mut current = None;
    kept.into_iter()
        .map(|doc| {
            let band = Recency::of(doc.updated_at, now);
            let opens = (current != Some(band)).then_some(band);
            current = Some(band);
            Row {
                opens,
                doc: doc.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::{Count, Members, Ms, Validated};

    const NOW_MS: u64 = 1_800_000_000_000;

    fn now() -> Instant<UnixMs> {
        Instant::new(NOW_MS)
    }

    /// A document last written `age` before `now`, or never listed.
    fn doc(name: Option<&str>, age: Option<TimeSpan<Ms>>) -> DocItem {
        DocItem {
            document_id: Id::<Document>::mint(),
            doc_type: DocType::Prose,
            name: name.map(|n| Validated::parse(n).unwrap()),
            updated_at: age.map(|a| Instant::new(NOW_MS - a.get())),
            member_count: Count::<Members>::ONE,
        }
    }

    fn hours(n: u64) -> Option<TimeSpan<Ms>> {
        Some(TimeSpan::new(n * 3_600_000))
    }

    /// One band and its documents, rebuilt from the rows that open bands.
    struct Group {
        recency: Recency,
        docs: Vec<DocItem>,
    }

    fn arranged(docs: &[DocItem], query: &str, now: Instant<UnixMs>) -> Vec<Group> {
        let mut groups: Vec<Group> = Vec::new();
        for row in arrange(docs, query, now) {
            match (row.opens, groups.last_mut()) {
                (Some(recency), _) => groups.push(Group {
                    recency,
                    docs: vec![row.doc],
                }),
                (None, Some(group)) => group.docs.push(row.doc),
                (None, None) => panic!("a first row opens no band"),
            }
        }
        groups
    }

    fn names(groups: &[Group]) -> Vec<Vec<Option<String>>> {
        groups
            .iter()
            .map(|g| {
                g.docs
                    .iter()
                    .map(|d| d.name.as_ref().map(|n| n.as_str().to_owned()))
                    .collect()
            })
            .collect()
    }

    fn flat(groups: &[Group]) -> Vec<Id<Document>> {
        groups
            .iter()
            .flat_map(|g| g.docs.iter().map(|d| d.document_id))
            .collect()
    }

    /// The most recent write comes first. A tie reads the folded name, a named
    /// document before an unnamed one, then the identifier, so the input order
    /// never shows (`CHROME-118`).
    #[test]
    fn the_most_recent_write_comes_first_and_a_tie_is_broken_by_name() {
        let old = doc(Some("Archive"), hours(5));
        let recent = doc(Some("Zebra"), hours(1));
        let tie_b = doc(Some("beta"), hours(3));
        let tie_a = doc(Some("Alpha"), hours(3));
        let tie_unnamed = doc(None, hours(3));
        let tie_blank = doc(Some("  "), hours(3));
        let input = vec![
            old.clone(),
            tie_unnamed.clone(),
            tie_b.clone(),
            recent.clone(),
            tie_blank.clone(),
            tie_a.clone(),
        ];
        let ordered = flat(&arranged(&input, "", now()));
        let mut unnamed = [tie_unnamed.document_id, tie_blank.document_id];
        unnamed.sort_by_key(Id::to_simple);
        assert_eq!(
            ordered,
            vec![
                recent.document_id,
                tie_a.document_id,
                tie_b.document_id,
                unnamed[0],
                unnamed[1],
                old.document_id,
            ]
        );

        let mut reversed = input;
        reversed.reverse();
        assert_eq!(
            flat(&arranged(&reversed, "", now())),
            ordered,
            "the input order never shows"
        );
    }

    /// Each document lands in the band of its age, the bands read in their
    /// display order, and an unlisted document comes last (`CHROME-119`).
    #[test]
    fn each_document_lands_in_the_band_of_its_age() {
        let input = vec![
            doc(Some("unlisted"), None),
            doc(Some("older"), hours(30 * 24)),
            doc(Some("month"), hours(7 * 24)),
            doc(Some("week"), hours(24)),
            doc(Some("day"), hours(23)),
            doc(Some("now"), Some(TimeSpan::new(0))),
        ];
        let groups = arranged(&input, "", now());
        let bands: Vec<Recency> = groups.iter().map(|g| g.recency).collect();
        assert_eq!(
            bands,
            vec![
                Recency::PastDay,
                Recency::PastWeek,
                Recency::PastMonth,
                Recency::Older,
                Recency::Unlisted,
            ]
        );
        assert_eq!(
            names(&groups),
            vec![
                vec![Some("now".to_owned()), Some("day".to_owned())],
                vec![Some("week".to_owned())],
                vec![Some("month".to_owned())],
                vec![Some("older".to_owned())],
                vec![Some("unlisted".to_owned())],
            ]
        );
    }

    /// The arrangement hands each row whole: its kind and its member count
    /// reach the card as the listing gave them.
    #[test]
    fn a_row_keeps_every_field_the_listing_gave() {
        let mut shared = doc(Some("Plan"), hours(1));
        shared.doc_type = DocType::Trame;
        shared.member_count = Count::<Members>::new(3);
        let rows = arrange(std::slice::from_ref(&shared), "", now());
        let [row] = rows.as_slice() else {
            panic!("one row");
        };
        assert_eq!(row.doc.document_id, shared.document_id);
        assert_eq!(row.doc.doc_type, DocType::Trame);
        assert!(row.doc.member_count == Count::<Members>::new(3));
    }

    /// Each band shows a heading of its own, so two bands never read alike.
    #[test]
    fn each_band_has_a_heading_of_its_own() {
        let bands = [
            Recency::PastDay,
            Recency::PastWeek,
            Recency::PastMonth,
            Recency::Older,
            Recency::Unlisted,
        ];
        let headings: std::collections::BTreeSet<&str> =
            bands.iter().map(|b| b.heading()).collect();
        assert_eq!(headings.len(), bands.len());
    }

    /// A write the reader's clock places in the future reads as the first band.
    #[test]
    fn a_write_ahead_of_the_reader_clock_is_in_the_first_band() {
        let ahead = Some(Instant::<Relayed>::new(NOW_MS + 60_000));
        assert_eq!(Recency::of(ahead, now()), Recency::PastDay);
    }

    /// A band no document falls in is not shown (`CHROME-126`).
    #[test]
    fn an_empty_band_is_not_shown() {
        let input = vec![doc(Some("a"), hours(1)), doc(Some("b"), hours(40 * 24))];
        let bands: Vec<Recency> = arranged(&input, "", now())
            .iter()
            .map(|g| g.recency)
            .collect();
        assert_eq!(bands, vec![Recency::PastDay, Recency::Older]);
        assert!(arranged(&[], "", now()).is_empty());
    }

    /// A query keeps exactly the names that contain it in any case, in the
    /// order and the bands of the whole list (`CHROME-120`).
    #[test]
    fn a_query_keeps_the_names_that_contain_it_in_list_order() {
        let input = vec![
            doc(Some("Budget 2026"), hours(1)),
            doc(Some("Payroll"), hours(2)),
            doc(Some("old BUDGET"), hours(40 * 24)),
            doc(Some("Budgie"), hours(3)),
        ];
        assert_eq!(
            names(&arranged(&input, "  bUdGeT ", now())),
            vec![
                vec![Some("Budget 2026".to_owned())],
                vec![Some("old BUDGET".to_owned())],
            ]
        );
        assert!(arranged(&input, "invoice", now()).is_empty());
        assert_eq!(
            flat(&arranged(&input, "   ", now())),
            flat(&arranged(&input, "", now())),
            "a blank query keeps every document"
        );
    }

    /// An unnamed document answers the blank query alone: the placeholder it
    /// shows is no name (`CHROME-121`).
    #[test]
    fn an_unnamed_document_answers_only_the_blank_query() {
        let input = vec![doc(None, hours(1)), doc(Some(""), hours(2))];
        assert_eq!(flat(&arranged(&input, "", now())).len(), 2);
        for query in ["u", "untitled", "Untitled", " "] {
            let kept = flat(&arranged(&input, query, now())).len();
            let expected = if query.trim().is_empty() { 2 } else { 0 };
            assert_eq!(kept, expected, "query {query:?}");
        }
    }
}

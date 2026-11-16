//! The import of an arbitrary form, as the fuzz target in `fuzz/` drives it
//! (`PROSE-IO-112`, `PROSE-IO-010`).
//!
//! A form reaches the import as the postcard bytes a converter module
//! answers. [`import_form`] decodes those bytes and imports the form on an
//! author. A second peer applies every delta the author sends. The import
//! owes one of two outcomes. The form lands whole on both peers, or the
//! author receives a refusal and both peers hold the document as it was.

use ambre_bus::interop::{Direction, FileFormat, InteropAnswer};
use ambre_core::{Count, Octets, Payload};

use super::ProseState;
use crate::events::{ProseEvent, ProseOutcome};
use crate::interop::{IrBlock, ProseIr};
use crate::message::ConversionReport;
use ambre_prose_form::FormBytes;

/// The longest input decoded. Postcard recurses once per nesting level of
/// the form, so the length bounds the depth of the decode.
const MAX_INPUT: Count<Octets> = Count::new(1024);

/// The deepest nesting of lists and table cells imported. A reader caps a
/// list at 16 levels, so a deeper form is one no module answers.
const MAX_NESTING: Count<Level> = Count::new(32);

/// The unit of [`MAX_NESTING`]: one list item or one table cell.
pub enum Level {}

impl ambre_core::NarrowCount for Level {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("FormLevel");
}

/// The resumes an import may take before it counts as stalled. Each resume
/// writes one slice, and an input of [`MAX_INPUT`] holds fewer operations.
const MAX_RESUMES: Count<Resume> = Count::new(4096);

/// The unit of [`MAX_RESUMES`]: one pass of the Logic thread over the
/// running import.
pub enum Resume {}

impl ambre_core::NarrowCount for Resume {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ImportResume");
}

/// How an import of an arbitrary form ended, each outcome the import allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landing {
    /// The bytes hold no form, or a form past a bound no module passes.
    Unread,
    /// The form landed, and the peer holds what the author holds.
    Completed,
    /// The author received a refusal, and both peers hold the document as
    /// it was.
    Refused,
}

/// An outcome the import forbids: the fuzz target fails on each one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Breach {
    /// The import completed, and the peer holds another document.
    PeerDiverged,
    /// The author received a refusal, and a peer kept part of the import.
    RefusalLeftWrites,
    /// The import ended with no answer, or ran past [`MAX_RESUMES`].
    NoAnswer,
}

/// Import the form `bytes` hold on a signed-in author. Check the outcome on
/// the author and on a peer that applies each delta the author sends.
///
/// # Errors
///
/// The [`Breach`] the import committed.
pub fn import_form(bytes: &Payload<FormBytes>) -> Result<Landing, Breach> {
    if Count::<Octets>::of(bytes.as_bytes().len()) > MAX_INPUT {
        return Ok(Landing::Unread);
    }
    let Ok(ir) = ambre_prose_form::decode::<ProseIr>(bytes) else {
        return Ok(Landing::Unread);
    };
    if nesting(&ir.blocks) > MAX_NESTING {
        return Ok(Landing::Unread);
    }
    let Some((mut author, mut peer)) = author_and_peer() else {
        return Ok(Landing::Unread);
    };
    let before = author.shape();
    let report = ConversionReport::new(Direction::Import, FileFormat::Docx);
    let now = ambre_core::Instant::<ambre_core::Authored>::new(1_800_000_000_000);
    let mut answer = Answer::None;
    let first = author.land_import(ir, report, now);
    answer = answer.read(&first, &mut peer);
    let mut resumes = Count::<Resume>::ZERO;
    while author.has_pending_work() {
        if resumes >= MAX_RESUMES {
            return Err(Breach::NoAnswer);
        }
        resumes = resumes.plus(Count::new(1));
        let out = author.resume_conversion();
        answer = answer.read(&out, &mut peer);
    }
    match answer {
        Answer::None => Err(Breach::NoAnswer),
        Answer::Completed if peer.shape() == author.shape() => Ok(Landing::Completed),
        Answer::Completed => Err(Breach::PeerDiverged),
        Answer::Refused if author.shape() == before && peer.shape() == before => {
            Ok(Landing::Refused)
        }
        Answer::Refused => Err(Breach::RefusalLeftWrites),
    }
}

/// An author signed in, so the form's threads and suggestions land too, and
/// a peer that joined from the author's snapshot.
fn author_and_peer() -> Option<(ProseState, ProseState)> {
    let mut author = ProseState::with_peer(ambre_core::PeerId::new(1))?;
    author.set_user_id(ambre_core::Id::from_bytes([1; 16]));
    let mut peer = ProseState::with_peer(ambre_core::PeerId::new(2))?;
    peer.import_snapshot(&author.export_snapshot()).ok()?;
    Some((author, peer))
}

/// The answer an import gave so far.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    None,
    Completed,
    Refused,
}

impl Answer {
    /// Fold the answer `outcome` carries, and hand `peer` its delta.
    fn read(self, outcome: &ProseOutcome, peer: &mut ProseState) -> Self {
        if let Some(delta) = &outcome.update {
            let _ = peer.apply_remote(delta);
        }
        outcome
            .display
            .iter()
            .fold(self, |answer, event| match event {
                ProseEvent::Interop(InteropAnswer::ImportCompleted { .. }) => Self::Completed,
                ProseEvent::Interop(InteropAnswer::Failed(_)) | ProseEvent::Error(_) => {
                    Self::Refused
                }
                _ => answer,
            })
    }
}

/// What a reader of the document sees: every live block in document order.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Shape(Vec<Seen>);

/// One live block as a reader sees it: its type and its text.
#[derive(Debug, PartialEq, Eq)]
struct Seen {
    kind: Option<crate::message::BlockType>,
    text: String,
}

impl Shape {
    /// How many live blocks the document holds, for the tests that watch a
    /// peer grow.
    #[cfg(test)]
    pub(super) fn blocks(&self) -> Count<Level> {
        Count::of(self.0.len())
    }
}

impl ProseState {
    /// The [`Shape`] of the document, which two peers compare.
    pub(super) fn shape(&self) -> Shape {
        let tree = crate::doc::blocks(&self.doc);
        Shape(
            crate::doc::flow(&tree)
                .into_iter()
                .map(|id| Seen {
                    kind: crate::doc::block_type_of(&tree, &id),
                    text: self
                        .body_of(&id)
                        .map(|body| body.to_string())
                        .unwrap_or_default(),
                })
                .collect(),
        )
    }
}

/// How deep `blocks` nest: one level per list item and per table cell.
fn nesting(blocks: &[IrBlock]) -> Count<Level> {
    blocks
        .iter()
        .map(|block| match block {
            IrBlock::ListItem { children, .. } => nesting(children).plus(Count::new(1)),
            IrBlock::Table { rows, .. } => rows
                .iter()
                .flat_map(|row| &row.cells)
                .map(|cell| nesting(&cell.blocks).plus(Count::new(1)))
                .max()
                .unwrap_or_default(),
            _ => Count::ZERO,
        })
        .max()
        .unwrap_or_default()
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::interop::{IrListStyle, IrRun};

    fn paragraph(text: &str) -> IrBlock {
        IrBlock::Paragraph {
            styled: ambre_prose_form::OwnStyle::default(),
            runs: vec![IrRun {
                text: text.to_owned(),
                ..IrRun::default()
            }],
        }
    }

    fn bytes(ir: &ProseIr) -> Payload<FormBytes> {
        ambre_prose_form::encode(ir).expect("a form encodes")
    }

    fn raw(bytes: &[u8]) -> Payload<FormBytes> {
        Payload::new(bytes.to_vec())
    }

    /// A form of many slices lands on the author and on the peer alike.
    #[test]
    fn a_form_lands_whole_on_both_peers() {
        let ir = ProseIr {
            blocks: (0..30).map(|i| paragraph(&format!("p{i}"))).collect(),
            ..ProseIr::default()
        };
        let encoded = bytes(&ir);
        assert!(
            encoded.as_bytes().len() <= MAX_INPUT.get() as usize,
            "{}",
            encoded.as_bytes().len()
        );
        assert_eq!(import_form(&encoded), Ok(Landing::Completed));
    }

    /// Bytes that hold no form read as nothing, and import nothing.
    #[test]
    fn bytes_that_hold_no_form_are_unread() {
        assert_eq!(import_form(&raw(&[0xff; 7])), Ok(Landing::Unread));
        assert_eq!(import_form(&raw(&[0; 1025])), Ok(Landing::Unread));
    }

    /// A list nested past the bound reads as nothing. The decode of the
    /// deepest input the bound lets through holds the stack of a test thread.
    #[test]
    fn a_form_nested_past_the_bound_is_unread() {
        // Four bytes a level: the deepest list the bound lets through.
        let mut block = paragraph("leaf");
        for _ in 0..240 {
            block = IrBlock::ListItem {
                style: IrListStyle::Bullet,
                runs: Vec::new(),
                children: vec![block],
            };
        }
        let ir = ProseIr {
            blocks: vec![block],
            ..ProseIr::default()
        };
        let encoded = bytes(&ir);
        assert!(
            encoded.as_bytes().len() <= MAX_INPUT.get() as usize,
            "{}",
            encoded.as_bytes().len()
        );
        assert_eq!(import_form(&encoded), Ok(Landing::Unread));
    }
}

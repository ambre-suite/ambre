// ambre-logic/src/app.rs
// ============================================================================
// The two closed sums of the content plane: one variant per document type.
//
// Each application declares its own command and event in its crate
// (`BUS-032`). The sums live here, downstream of every application. The
// dispatch over them stays an exhaustive `match` the compiler proves
// (`BUS-002`), and the envelopes of `ambre-bus` carry them as a type parameter.
// `Embed` is how a view's own command enters the sum and its own event leaves
// it, at the launcher.
// ============================================================================

use ambre_bus::{Embed, Mutation};
use ambre_murmure::message::Murmure;
use ambre_prose::message::{ProseCommand, ProseEvent};
use ambre_toile::message::{ToileCommand, ToileEvent};
use ambre_trame::message::{TrameCommand, TrameEvent};

/// The UI to Logic envelope as the Logic runtime receives it: the document
/// sum, and Murmure's channel plane.
pub type LogicCommand = ambre_bus::LogicCommand<AppCommand, Murmure>;
/// The Logic to UI envelope as the Logic runtime sends it.
pub type UiEvent = ambre_bus::UiEvent<AppEvent, Murmure>;

/// Application-specific command, UI Thread -> Logic thread.
///
/// Carried inside `LogicCommand::App`. The Logic dispatcher matches the
/// variant, routes it to the document's state, and produces zero or more
/// [`AppEvent`]s.
#[derive(Debug, Clone)]
pub enum AppCommand {
    /// Command for a Trame (spreadsheet) document.
    Trame(TrameCommand),
    /// Command for a Toile (whiteboard) document.
    Toile(ToileCommand),
    /// Command for a Prose (word processor) document.
    Prose(ProseCommand),
}

impl Mutation for AppCommand {
    fn is_mutating(&self) -> bool {
        match self {
            Self::Trame(cmd) => cmd.is_mutating(),
            Self::Toile(cmd) => cmd.is_mutating(),
            Self::Prose(cmd) => cmd.is_mutating(),
        }
    }

    fn traffic(&self) -> ambre_bus::Traffic {
        match self {
            Self::Trame(cmd) => cmd.traffic(),
            Self::Toile(cmd) => cmd.traffic(),
            Self::Prose(cmd) => cmd.traffic(),
        }
    }
}

/// Application-specific event, Logic thread -> UI Thread.
///
/// Carried inside `UiEvent::Viewport` and `UiEvent::Presence`. The `Command` /
/// `Event` suffix marks the direction at both the envelope and the payload
/// level.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Display event for a Trame (spreadsheet) document.
    Trame(TrameEvent),
    /// Display event for a Toile (whiteboard) document.
    Toile(ToileEvent),
    /// Display event for a Prose (word processor) document.
    Prose(ProseEvent),
}

/// One `Embed` per variant of each sum: the variant wraps the part, and the
/// projection gives back any other variant unchanged.
macro_rules! embed {
    ($sum:ident :: $variant:ident ($part:ty)) => {
        impl Embed<$part> for $sum {
            fn embed(part: $part) -> Self {
                Self::$variant(part)
            }

            fn project(self) -> Result<$part, Self> {
                match self {
                    Self::$variant(part) => Ok(part),
                    other => Err(other),
                }
            }

            fn peek(&self) -> Option<&$part> {
                match self {
                    Self::$variant(part) => Some(part),
                    _ => None,
                }
            }
        }
    };
}

embed!(AppCommand::Trame(TrameCommand));
embed!(AppCommand::Toile(ToileCommand));
embed!(AppCommand::Prose(ProseCommand));
embed!(AppEvent::Trame(TrameEvent));
embed!(AppEvent::Toile(ToileEvent));
embed!(AppEvent::Prose(ProseEvent));

// A sender that carries no document command, such as Murmure's page: nothing
// embeds into the sum from it, and nothing ever projects out to it.
ambre_bus::embed_nothing!(AppCommand, AppEvent);

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::{Col, Instant, PageLoad, Row};
    use ambre_core::id::{Element, Id, Sheet};
    use ambre_core::index::Pos;
    use ambre_toile::message::{PresencePayload, Viewport as ToileViewport};
    use ambre_trame::message::SelectionKind;

    /// Whether `T` can be encoded. The inherent constant applies when `T`
    /// implements `Serialize`, and the trait's default applies otherwise.
    struct Encodable<T>(std::marker::PhantomData<T>);

    trait NotEncodable {
        const ENCODES: bool = false;
    }

    impl<T> NotEncodable for Encodable<T> {}

    impl<T: serde::Serialize> Encodable<T> {
        const ENCODES: bool = true;
    }

    /// `BUS-001`: the two sums the UI to Logic path carries implement no
    /// `Serialize`, as the envelopes around them do not
    /// (`ambre_bus::host::the_ui_to_logic_path_carries_nothing_an_encoder_takes`).
    #[test]
    fn the_sums_on_the_ui_to_logic_path_carry_nothing_an_encoder_takes() {
        // Each probe is a constant, so a derive added tomorrow fails the build.
        const {
            assert!(!Encodable::<AppCommand>::ENCODES);
            assert!(!Encodable::<AppEvent>::ENCODES);
            assert!(Encodable::<ambre_bus::SyncCommand>::ENCODES);
        }
    }

    #[test]
    fn a_projection_gives_back_its_own_variant_and_refuses_another() {
        let embedded = <AppCommand as Embed<TrameCommand>>::embed(TrameCommand::Undo);
        assert!(matches!(
            Embed::<TrameCommand>::project(embedded.clone()),
            Ok(TrameCommand::Undo)
        ));
        assert!(matches!(
            Embed::<ToileCommand>::project(embedded),
            Err(AppCommand::Trame(TrameCommand::Undo))
        ));
    }

    // -- Role gate: AppCommand::is_mutating (`architecture`) ------------------
    //
    // The classification decides what a viewer may still do, and getting it wrong is silent in
    // both directions. A misclassified read-only command costs a viewer its navigation, and a
    // misclassified write slips past the client gate. The two ends of each family are pinned
    // here rather than trusted to review.

    #[test]
    fn read_only_and_local_commands_are_not_mutating() {
        let cases: Vec<AppCommand> = vec![
            // Trame: local view state.
            AppCommand::Trame(TrameCommand::ScrollViewport {
                version: ambre_core::Seq::nth(1),
                row_start: Pos::<Row>::new(0),
                row_end: Pos::<Row>::new(4),
                col_start: Pos::<Col>::new(0),
                col_end: Pos::<Col>::new(4),
            }),
            AppCommand::Trame(TrameCommand::SetActiveCell {
                row: Pos::<Row>::new(0),
                col: Pos::<Col>::new(0),
                anchor_row: Pos::<Row>::new(0),
                anchor_col: Pos::<Col>::new(0),
                extra: vec![],
                kind: SelectionKind::Cells,
            }),
            AppCommand::Trame(TrameCommand::ActivateSheet {
                sheet_id: Id::<Sheet>::from_text("s1".to_owned()),
            }),
            // Trame: read-only queries.
            AppCommand::Trame(TrameCommand::RequestSelectionStats),
            AppCommand::Trame(TrameCommand::CopyRange),
            AppCommand::Trame(TrameCommand::ExportActiveSheetCsv),
            // Toile: local playback and view state.
            AppCommand::Toile(ToileCommand::NextSlide),
            AppCommand::Toile(ToileCommand::ExitPresent),
            AppCommand::Toile(ToileCommand::SetViewport(ToileViewport::default())),
            // Toile: pure reads a viewer must keep (`toile-comments`,
            // `toile-presentation`): both answer with a projection and write nothing.
            AppCommand::Toile(ToileCommand::ReadComments { thread: None }),
            AppCommand::Toile(ToileCommand::ReadDeck),
            // Toile: ephemeral only -- a viewer still shows a cursor (`toile-presence`).
            AppCommand::Toile(ToileCommand::Presence {
                payload: PresencePayload::default(),
                now: Instant::<PageLoad>::new(0.0),
            }),
        ];
        for cmd in cases {
            assert!(
                !cmd.is_mutating(),
                "a viewer must keep this command: {cmd:?}"
            );
        }
    }

    #[test]
    fn writing_commands_are_mutating() {
        let cases: Vec<AppCommand> = vec![
            AppCommand::Trame(TrameCommand::SetCellText {
                row: Pos::<Row>::new(0),
                col: Pos::<Col>::new(0),
                text: "x".into(),
            }),
            AppCommand::Trame(TrameCommand::InsertRowAbove),
            AppCommand::Trame(TrameCommand::PasteText { text: "x".into() }),
            AppCommand::Trame(TrameCommand::Undo),
            AppCommand::Toile(ToileCommand::DeleteElements { ids: Vec::new() }),
            AppCommand::Toile(ToileCommand::DuplicateElements {
                ids: Vec::new(),
                delta: Default::default(),
            }),
            AppCommand::Toile(ToileCommand::Undo),
            // Reacting writes a per-user CRDT entry (`toile-reactions`): not presence.
            AppCommand::Toile(ToileCommand::React {
                target: Id::<Element>::mint_from(
                    ambre_core::PeerId::new(1),
                    ambre_toile::message::IdCounter::new(1),
                ),
                reaction: None,
            }),
        ];
        for cmd in cases {
            assert!(cmd.is_mutating(), "this command writes the CRDT: {cmd:?}");
        }
    }
}

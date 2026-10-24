//! Headers and footers (`features/ambre-spec-prose-headers-footers.md`).
//!
//! A band's text is an ordinary paragraph: a root of the block tree marked as
//! a band and kept out of the body flow (`PROSE-006`). The metadata names it
//! and holds nothing of its text (`PROSE-FEAT-281`). The ordinary text commands
//! edit it by naming that block, so no command here carries another
//! (`PROSE-FEAT-160`).
//!
//! A field is one object-replacement character carrying an `f:` mark. It is
//! one code point, so the caret steps over it and a deletion takes it whole
//! (`PROSE-FEAT-163`, `PROSE-FEAT-283`). Its value is resolved when a page is
//! drawn and never stored (`PROSE-FEAT-065`).

use ambre_bus::app::prose::DocOffset;
use loro::TreeID;

use crate::doc;
use crate::events::{
    Align, BandSet, BlockType, HeaderField, HfTarget, ProseEvent, ProseOutcome, ProseRefusal,
    block_id,
};

use super::ProseState;

/// The character a field occupies in its block's text.
pub const FIELD_CHAR: char = doc::CARRIER;

/// One side of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    Header,
    Footer,
}

impl Side {
    /// The register of the side's regular band, and of its first-page band.
    fn registers(self) -> (doc::MetaKey, doc::MetaKey) {
        match self {
            Self::Header => (doc::MetaKey::Header, doc::MetaKey::HeaderFirst),
            Self::Footer => (doc::MetaKey::Footer, doc::MetaKey::FooterFirst),
        }
    }
}

impl ProseState {
    /// The bands as the view needs them.
    #[must_use]
    pub fn bands(&self) -> BandSet {
        let bands = doc::read_bands(&self.doc);
        BandSet {
            header: bands.header.block.map(block_id),
            footer: bands.footer.block.map(block_id),
            header_first: bands.header_first.block.map(block_id),
            footer_first: bands.footer_first.block.map(block_id),
            header_enabled: bands.header_enabled,
            footer_enabled: bands.footer_enabled,
            different_first_page: bands.different_first_page,
        }
    }

    /// Show or hide one side. The block it names is left alone, so showing it
    /// again brings its text back (`PROSE-FEAT-167`, `PROSE-FEAT-168`).
    pub(super) fn set_side_enabled(&mut self, side: Side, enabled: bool) -> ProseOutcome {
        if enabled && let Err(outcome) = self.ensure_side(side) {
            return outcome;
        }
        let (regular, _) = side.registers();
        let written = self
            .register(regular)
            .and_then(|map| map.insert(doc::BandKey::Enabled, enabled).ok());
        if written.is_none() {
            return Self::err_write(doc::WriteError::Refused(
                "the band register refused the setting",
            ));
        }
        self.commit_bands()
    }

    /// Turn the first page's own bands on or off. It touches neither side's
    /// own setting nor any band block (`PROSE-FEAT-170`).
    pub(super) fn set_different_first_page(&mut self, enabled: bool) -> ProseOutcome {
        let meta = doc::meta(&self.doc);
        if let Err(e) = meta.insert(doc::MetaKey::DifferentFirstPage, enabled) {
            return Self::err_write(e);
        }
        self.commit_bands()
    }

    /// Put a field in a band, in the order section 3 gives: resolve the
    /// target, refuse an inert first-page block, clamp, write
    /// (`PROSE-FEAT-067`, `PROSE-FEAT-068`, `PROSE-FEAT-165`).
    pub(super) fn insert_field(
        &mut self,
        side: Side,
        field: HeaderField,
        target: HfTarget,
        pos: DocOffset,
    ) -> ProseOutcome {
        let bands = doc::read_bands(&self.doc);
        if target == HfTarget::FirstPage && !bands.different_first_page {
            return ProseOutcome::refused(ProseRefusal::FirstPageBandOff);
        }
        let block = match self.ensure_side(side) {
            Ok((regular, first)) => match target {
                HfTarget::Regular => regular,
                HfTarget::FirstPage => first,
            },
            Err(outcome) => return outcome,
        };
        let Some(body) = self.body_of(&block) else {
            return Self::err_no_block(&block);
        };
        let at = pos.to_usize().min(body.len_unicode());
        let (key, value) = crate::band::field_mark(field);
        let mut buffer = [0_u8; 4];
        if let Err(e) = body.insert(at, FIELD_CHAR.encode_utf8(&mut buffer)) {
            return Self::err_write(e);
        }
        if let Err(e) = body.mark(at..at + 1, key, value) {
            return Self::err_write(e);
        }
        self.commit_bands()
    }

    /// Align the band whose block is `id` (`PROSE-FEAT-171`). The text and
    /// the other bands are left alone (`PROSE-FEAT-172`).
    pub(super) fn set_band_align(&mut self, id: &TreeID, align: Align) -> ProseOutcome {
        let Some(band_align) = doc::BandAlign::of(align) else {
            return ProseOutcome::refused(ProseRefusal::BandJustified);
        };
        let Some(register) = doc::read_bands(&self.doc).register_of(id) else {
            return Self::err_no_block(id);
        };
        let written = self
            .register(register)
            .and_then(|map| map.insert(doc::BandKey::Align, band_align.as_str()).ok());
        if written.is_none() {
            return Self::err_write(doc::WriteError::Refused(
                "the band register refused the alignment",
            ));
        }
        self.commit_bands()
    }

    /// Write the view's clock as the last-modified stamp, when this peer
    /// wrote since the stamp before (`PROSE-FEAT-066`).
    ///
    /// The stamp is the author's clock, read once and stored. Every reader then
    /// resolves the same date, whatever its own clock says. A collaborator's
    /// change is dated by the collaborator, so nothing here stamps it.
    pub(super) fn stamp_modified(&mut self, now_ms: ambre_core::Timestamp) -> ProseOutcome {
        let ours = self.own_counter();
        if ours <= self.stamped_counter {
            return ProseOutcome::nothing();
        }
        let now = now_ms.to_i64_saturating();
        if let Err(e) = doc::meta(&self.doc).insert(doc::MetaKey::UpdatedAt, now) {
            return Self::err_write(e);
        }
        self.doc
            .set_next_commit_origin(ambre_core::crdt::CommitOrigin::Stamp.as_str());
        let outcome = self.commit_with(Vec::new());
        self.stamped_counter = self.own_counter();
        outcome
    }

    /// This session's own operation counter: how far its PeerID has written.
    fn own_counter(&self) -> ambre_core::OpCounter {
        ambre_core::OpCounter(self.doc.oplog_vv().get(&self.peer.0).copied().unwrap_or(0))
    }

    /// The side's two blocks, minted on first use. Each is a root paragraph
    /// marked as a band, so the body flow never meets it.
    ///
    /// Two peers minting one side concurrently each name their own block, and
    /// the register keeps one of the two names. The other block stays in the
    /// tree, out of the flow and named by nothing.
    fn ensure_side(&mut self, side: Side) -> Result<(TreeID, TreeID), ProseOutcome> {
        let (regular, first) = side.registers();
        Ok((self.ensure_band(regular)?, self.ensure_band(first)?))
    }

    fn ensure_band(&mut self, register: doc::MetaKey) -> Result<TreeID, ProseOutcome> {
        let bands = doc::read_bands(&self.doc);
        let named = match register {
            doc::MetaKey::Header => bands.header.block,
            doc::MetaKey::Footer => bands.footer.block,
            doc::MetaKey::HeaderFirst => bands.header_first.block,
            _ => bands.footer_first.block,
        };
        if let Some(id) = named {
            return Ok(id);
        }
        let Some(id) = doc::create_block(&self.doc, None, BlockType::Paragraph) else {
            return Err(Self::err_write(doc::WriteError::Refused(
                "the tree refused a band block",
            )));
        };
        let tree = doc::blocks(&self.doc);
        let marked =
            doc::node_of(&tree, &id).and_then(|node| node.insert(doc::NodeKey::Band, true).ok());
        let named = self
            .register(register)
            .and_then(|map| map.insert(doc::BandKey::Block, id.to_string()).ok());
        if marked.is_none() || named.is_none() {
            return Err(Self::err_write(doc::WriteError::Refused(
                "the band block could not be named",
            )));
        }
        Ok(id)
    }

    /// The band register a key names, opened once and never replaced.
    fn register(&self, key: doc::MetaKey) -> Option<doc::Fields<doc::BandKey>> {
        doc::map_at(&doc::meta(&self.doc), key).map(doc::Fields::of)
    }

    /// Commit a band write and tell the view the bands as they now stand.
    fn commit_bands(&mut self) -> ProseOutcome {
        let bands = self.bands();
        self.commit_with(vec![ProseEvent::BandsUpdated { bands }])
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::events::{BlockId, DateFieldFormat, PageCount, PageIndex, ProseCommand};
    use ambre_bus::app::prose::DocSpan;
    use ambre_core::DocPx;

    pub(crate) fn with_header() -> (ProseState, BlockId) {
        let mut state = ProseState::new().expect("a fresh document");
        state.handle_command(ProseCommand::SetHeaderEnabled { enabled: true });
        let header = state.bands().header.expect("the header block");
        (state, header)
    }

    fn marks_of(state: &ProseState, id: &BlockId) -> Vec<(String, String)> {
        let text = doc::get_block_text(state.document(), &crate::events::tree_id(*id));
        let mut found = Vec::new();
        for span in text.to_delta() {
            if let loro::TextDelta::Insert {
                attributes: Some(attributes),
                ..
            } = span
            {
                for (key, value) in attributes {
                    let value = match value {
                        loro::LoroValue::String(s) => s.to_string(),
                        other => format!("{other:?}"),
                    };
                    found.push((key.to_string(), value));
                }
            }
        }
        found
    }

    fn refusal(outcome: &ProseOutcome) -> Option<ProseRefusal> {
        outcome.display.iter().find_map(|e| match e {
            ProseEvent::Error(e) => Some(e.reason.clone()),
            _ => None,
        })
    }

    #[test]
    fn a_band_is_an_ordinary_block_edited_by_the_text_commands() {
        let (mut state, header) = with_header();
        let outcome = state.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: DocOffset::ZERO,
            text: "Report".to_owned(),
        });
        assert!(outcome.update.is_some());
        assert_eq!(state.text_of(&header).as_deref(), Some("Report"));
        state.handle_command(ProseCommand::ApplyMark {
            block_id: header,
            range: DocSpan::from_usize_range(0..6),
            key: "b".into(),
            value: None,
        });
        assert_eq!(marks_of(&state, &header).len(), 1);
    }

    #[test]
    fn a_band_stays_out_of_the_body_flow() {
        let (state, header) = with_header();
        assert!(!state.root_blocks().contains(&header));
        assert_eq!(state.root_blocks().len(), 1);
    }

    #[test]
    fn a_band_block_cannot_be_deleted_moved_or_split() {
        let (mut state, header) = with_header();
        let body = state.root_blocks()[0];
        for command in [
            ProseCommand::DeleteBlock { block_id: header },
            ProseCommand::MoveBlock {
                block_id: header,
                new_parent: None,
                after_id: Some(body),
            },
            ProseCommand::MoveBlock {
                block_id: body,
                new_parent: None,
                after_id: Some(header),
            },
            ProseCommand::SplitBlock {
                block_id: header,
                pos: DocOffset::ZERO,
            },
        ] {
            let outcome = state.handle_command(command);
            assert_eq!(refusal(&outcome), Some(ProseRefusal::NestingRefused));
        }
    }

    #[test]
    fn the_configuration_names_the_block_and_holds_no_text() {
        let (mut state, header) = with_header();
        state.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: DocOffset::ZERO,
            text: "Secret title".to_owned(),
        });
        let meta = doc::meta(state.document()).map().get_deep_value();
        let dump = format!("{meta:?}");
        assert!(dump.contains(&crate::events::tree_id(header).to_string()));
        assert!(dump.contains("enabled") && dump.contains(doc::BandKey::Block.as_str()));
        assert!(!dump.contains("Secret title"));
        let bands = doc::read_bands(state.document());
        assert_eq!(bands.header.align, Align::Left);
    }

    #[test]
    fn showing_one_side_changes_that_side_alone() {
        let mut state = ProseState::new().expect("a fresh document");
        state.handle_command(ProseCommand::SetFooterEnabled { enabled: true });
        let before = state.bands();
        state.handle_command(ProseCommand::SetHeaderEnabled { enabled: true });
        let after = state.bands();
        assert!(after.header_enabled);
        assert_eq!(after.footer_enabled, before.footer_enabled);
        state.handle_command(ProseCommand::SetHeaderEnabled { enabled: false });
        assert!(!state.bands().header_enabled);
        assert!(state.bands().footer_enabled);
    }

    #[test]
    fn hiding_a_side_keeps_its_text_for_when_it_returns() {
        let (mut state, header) = with_header();
        state.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: DocOffset::ZERO,
            text: "Kept".to_owned(),
        });
        state.handle_command(ProseCommand::SetHeaderEnabled { enabled: false });
        assert_eq!(state.text_of(&header).as_deref(), Some("Kept"));
        state.handle_command(ProseCommand::SetHeaderEnabled { enabled: true });
        assert_eq!(state.bands().header, Some(header));
        assert_eq!(state.text_of(&header).as_deref(), Some("Kept"));
    }

    #[test]
    fn one_setting_governs_both_first_page_bands_and_touches_nothing_else() {
        let (mut state, _) = with_header();
        state.handle_command(ProseCommand::SetFooterEnabled { enabled: true });
        let first = state.bands().header_first.expect("minted with the side");
        state.handle_command(ProseCommand::InsertText {
            block_id: first,
            pos: DocOffset::ZERO,
            text: "Cover".to_owned(),
        });
        let before = state.bands();
        state.handle_command(ProseCommand::SetDifferentFirstPage { enabled: true });
        let on = state.bands();
        assert!(on.different_first_page);
        assert_eq!(
            (on.header, on.footer, on.header_first, on.footer_first),
            (
                before.header,
                before.footer,
                before.header_first,
                before.footer_first
            )
        );
        assert_eq!(
            (on.header_enabled, on.footer_enabled),
            (before.header_enabled, before.footer_enabled)
        );
        state.handle_command(ProseCommand::SetDifferentFirstPage { enabled: false });
        assert_eq!(state.text_of(&first).as_deref(), Some("Cover"));
        let meta = doc::meta(state.document());
        // A key of `meta` looked for in a band register, so the raw map.
        let per_side = doc::map_read(&meta, doc::MetaKey::Header)
            .is_some_and(|map| map.get(doc::MetaKey::DifferentFirstPage.as_str()).is_some());
        assert!(!per_side, "the setting is the document's, not a side's");
    }

    #[test]
    fn a_field_lands_in_the_target_block_as_one_marked_character() {
        let (mut state, header) = with_header();
        state.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: DocOffset::ZERO,
            text: "Page ".to_owned(),
        });
        state.handle_command(ProseCommand::SetDifferentFirstPage { enabled: true });
        let before = state.bands();
        state.handle_command(ProseCommand::InsertHeaderField {
            field: HeaderField::PageNumber,
            target: HfTarget::Regular,
            pos: DocOffset::new(5),
        });
        assert_eq!(state.text_of(&header), Some(format!("Page {FIELD_CHAR}")));
        assert_eq!(
            marks_of(&state, &header),
            vec![("f:pn".to_owned(), String::new())]
        );
        let first = state.bands().header_first.expect("the first-page header");
        assert_eq!(state.text_of(&first).as_deref(), Some(""));
        assert_eq!(state.bands(), before);
    }

    #[test]
    fn a_position_past_the_end_is_clamped_to_it() {
        let (mut state, header) = with_header();
        state.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: DocOffset::ZERO,
            text: "ab".to_owned(),
        });
        state.handle_command(ProseCommand::InsertHeaderField {
            field: HeaderField::Title,
            target: HfTarget::Regular,
            pos: DocOffset::new(99),
        });
        assert_eq!(state.text_of(&header), Some(format!("ab{FIELD_CHAR}")));
    }

    #[test]
    fn a_field_in_the_first_page_block_is_refused_while_the_setting_is_off() {
        let (mut state, _) = with_header();
        let before = state.document().oplog_vv();
        let outcome = state.handle_command(ProseCommand::InsertFooterField {
            field: HeaderField::PageCount,
            target: HfTarget::FirstPage,
            pos: DocOffset::ZERO,
        });
        assert_eq!(refusal(&outcome), Some(ProseRefusal::FirstPageBandOff));
        assert!(outcome.update.is_none());
        assert_eq!(state.document().oplog_vv(), before);
    }

    #[test]
    fn the_date_field_stores_a_format_and_no_date() {
        let (mut state, header) = with_header();
        state.handle_command(ProseCommand::StampModified {
            now_ms: ambre_core::Timestamp(1_700_000_000_000),
        });
        state.handle_command(ProseCommand::InsertHeaderField {
            field: HeaderField::Date(DateFieldFormat::Long),
            target: HfTarget::Regular,
            pos: DocOffset::ZERO,
        });
        assert_eq!(
            marks_of(&state, &header),
            vec![("f:dt".to_owned(), "long".to_owned())]
        );
        assert_eq!(state.text_of(&header), Some(FIELD_CHAR.to_string()));
    }

    #[test]
    fn typing_beside_a_field_never_extends_it_and_a_deletion_takes_it_whole() {
        let (mut state, header) = with_header();
        state.handle_command(ProseCommand::InsertHeaderField {
            field: HeaderField::PageNumber,
            target: HfTarget::Regular,
            pos: DocOffset::ZERO,
        });
        for pos in [DocOffset::new(1), DocOffset::ZERO] {
            state.handle_command(ProseCommand::InsertText {
                block_id: header,
                pos,
                text: "x".to_owned(),
            });
        }
        let marked: Vec<_> = marks_of(&state, &header);
        assert_eq!(marked.len(), 1, "the mark covers the field alone");
        state.handle_command(ProseCommand::DeleteText {
            block_id: header,
            range: DocSpan::from_usize_range(1..2),
        });
        assert_eq!(state.text_of(&header).as_deref(), Some("xx"));
        assert!(marks_of(&state, &header).is_empty());
    }

    #[test]
    fn an_alignment_lands_on_the_band_named_and_on_no_other() {
        let (mut state, header) = with_header();
        state.handle_command(ProseCommand::SetFooterEnabled { enabled: true });
        state.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: DocOffset::ZERO,
            text: "Left".to_owned(),
        });
        let style = state.paragraph_style_of(&header);
        state.handle_command(ProseCommand::SetBandAlign {
            block_id: header,
            align: Align::Right,
        });
        let bands = doc::read_bands(state.document());
        assert_eq!(bands.header.align, Align::Right);
        assert_eq!(bands.footer.align, Align::Left);
        assert_eq!(bands.header_first.align, Align::Left);
        assert_eq!(state.text_of(&header).as_deref(), Some("Left"));
        assert_eq!(state.paragraph_style_of(&header), style);
        let justified = state.handle_command(ProseCommand::SetBandAlign {
            block_id: header,
            align: Align::Justify,
        });
        assert_eq!(refusal(&justified), Some(ProseRefusal::BandJustified));
    }

    #[test]
    fn a_stamp_dates_this_peers_writes_and_never_undoes() {
        let (mut state, header) = with_header();
        state.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: DocOffset::ZERO,
            text: "a".to_owned(),
        });
        let stamped = state.handle_command(ProseCommand::StampModified {
            now_ms: ambre_core::Timestamp(42),
        });
        assert!(stamped.update.is_some());
        assert_eq!(
            doc::read_updated_at(state.document()),
            Some(ambre_core::Timestamp(42))
        );
        let again = state.handle_command(ProseCommand::StampModified {
            now_ms: ambre_core::Timestamp(43),
        });
        assert!(again.update.is_none(), "nothing was written since");
        state.handle_command(ProseCommand::Undo);
        assert_eq!(state.text_of(&header).as_deref(), Some(""));
        assert_eq!(
            doc::read_updated_at(state.document()),
            Some(ambre_core::Timestamp(42))
        );
    }

    /// Two peers, each holding the other's opening state.
    fn two_peers() -> (ProseState, ProseState) {
        let mut alice = ProseState::with_peer(ambre_core::PeerId(1)).expect("alice");
        let bob = ProseState::with_peer(ambre_core::PeerId(2)).expect("bob");
        let snapshot = alice.export_snapshot();
        let mut bob = bob;
        bob.import_snapshot(&snapshot)
            .expect("bob opens alice's document");
        let _ = alice.handle_command(ProseCommand::Noop);
        (alice, bob)
    }

    fn send(from: &ProseOutcome, to: &mut ProseState) {
        if let Some(delta) = &from.update {
            to.apply_remote(delta);
        }
    }

    #[test]
    fn two_peers_typing_in_one_band_converge_as_a_paragraph_would() {
        let (mut alice, mut bob) = two_peers();
        let shown = alice.handle_command(ProseCommand::SetHeaderEnabled { enabled: true });
        send(&shown, &mut bob);
        let header = alice.bands().header.expect("the header");
        assert_eq!(bob.bands().header, Some(header));
        let a = alice.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: DocOffset::ZERO,
            text: "Alice ".to_owned(),
        });
        let b = bob.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: DocOffset::ZERO,
            text: "Bob ".to_owned(),
        });
        send(&a, &mut bob);
        send(&b, &mut alice);
        let text = alice.text_of(&header);
        assert_eq!(text, bob.text_of(&header));
        let text = text.unwrap_or_default();
        assert!(text.contains("Alice ") && text.contains("Bob "));
    }

    #[test]
    fn two_readers_resolve_one_date_from_the_stamp_and_never_their_clock() {
        let (mut alice, mut bob) = two_peers();
        let shown = alice.handle_command(ProseCommand::SetHeaderEnabled { enabled: true });
        send(&shown, &mut bob);
        let field = alice.handle_command(ProseCommand::InsertHeaderField {
            field: HeaderField::Date(DateFieldFormat::Iso),
            target: HfTarget::Regular,
            pos: DocOffset::ZERO,
        });
        send(&field, &mut bob);
        let stamp = alice.handle_command(ProseCommand::StampModified {
            now_ms: ambre_core::Timestamp(1_772_712_000_000),
        });
        send(&stamp, &mut bob);
        let header = crate::events::tree_id(alice.bands().header.expect("the header"));
        let seen_by_alice =
            crate::band::resolve(alice.document(), &header, PageIndex::ZERO, PageCount::ONE);
        let seen_by_bob =
            crate::band::resolve(bob.document(), &header, PageIndex::ZERO, PageCount::ONE);
        assert_eq!(seen_by_alice, "2026-03-05");
        assert_eq!(seen_by_bob, seen_by_alice);
    }

    #[test]
    fn the_date_is_drawn_in_the_document_language() {
        let (mut state, header) = with_header();
        state.handle_command(ProseCommand::InsertHeaderField {
            field: HeaderField::Date(DateFieldFormat::Long),
            target: HfTarget::Regular,
            pos: DocOffset::ZERO,
        });
        state.handle_command(ProseCommand::StampModified {
            now_ms: ambre_core::Timestamp(1_772_712_000_000),
        });
        let header = crate::events::tree_id(header);
        // A new document carries no language, so the date takes the neutral
        // form rather than a fixed language (`LOCALE-013`).
        let seeded =
            crate::band::resolve(state.document(), &header, PageIndex::ZERO, PageCount::ONE);
        assert_eq!(seeded, "2026-03-05");
        let _ = doc::meta(state.document()).insert(doc::MetaKey::Lang, "en");
        let english =
            crate::band::resolve(state.document(), &header, PageIndex::ZERO, PageCount::ONE);
        assert_eq!(english, "March 5, 2026");
        let _ = doc::meta(state.document()).insert(doc::MetaKey::Lang, "fr");
        let french =
            crate::band::resolve(state.document(), &header, PageIndex::ZERO, PageCount::ONE);
        assert_eq!(french, "5 mars 2026");
        // A peer's malformed tag is no language, not French nor English.
        let _ = doc::meta(state.document()).insert(doc::MetaKey::Lang, "fr_FR");
        let unnamed =
            crate::band::resolve(state.document(), &header, PageIndex::ZERO, PageCount::ONE);
        assert_eq!(unnamed, "2026-03-05");
    }

    #[test]
    fn a_field_is_resolved_from_the_page_it_is_drawn_on() {
        let (mut state, header) = with_header();
        state.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: DocOffset::ZERO,
            text: "Page  of ".to_owned(),
        });
        for (pos, field) in [
            (DocOffset::new(9), HeaderField::PageCount),
            (DocOffset::new(5), HeaderField::PageNumber),
        ] {
            state.handle_command(ProseCommand::InsertHeaderField {
                field,
                target: HfTarget::Regular,
                pos,
            });
        }
        let header = crate::events::tree_id(header);
        let drawn: Vec<String> = (0..3)
            .map(|page| {
                crate::band::resolve(
                    state.document(),
                    &header,
                    crate::events::PageIndex::new(page),
                    crate::events::PageCount::new(3),
                )
            })
            .collect();
        assert_eq!(drawn, vec!["Page 1 of 3", "Page 2 of 3", "Page 3 of 3"]);
    }

    #[test]
    fn the_first_page_draws_its_own_bands_only_while_the_setting_is_on() {
        let (mut state, _) = with_header();
        state.handle_command(ProseCommand::SetFooterEnabled { enabled: true });
        let set = state.bands();
        let block = |band: Option<doc::Band>| band.and_then(|b| b.block).map(block_id);
        let off = state.paginate(&[]).clone();
        for page in (0..2).map(PageIndex::new) {
            let (header, footer) = off.bands_on_page(page);
            assert_eq!((block(header), block(footer)), (set.header, set.footer));
        }
        state.handle_command(ProseCommand::SetDifferentFirstPage { enabled: true });
        let on = state.paginate(&[]).clone();
        let (header, footer) = on.bands_on_page(PageIndex::new(0));
        assert_eq!(
            (block(header), block(footer)),
            (set.header_first, set.footer_first)
        );
        let (header, footer) = on.bands_on_page(PageIndex::new(1));
        assert_eq!((block(header), block(footer)), (set.header, set.footer));
    }

    #[test]
    fn an_alignment_applies_on_every_page_its_band_is_drawn_on() {
        let (mut state, header) = with_header();
        state.handle_command(ProseCommand::SetBandAlign {
            block_id: header,
            align: Align::Center,
        });
        let layout = state.paginate(&[]).clone();
        for page in (0..3).map(PageIndex::new) {
            let (drawn, _) = layout.bands_on_page(page);
            assert_eq!(drawn.map(|band| band.align), Some(Align::Center));
        }
    }

    #[test]
    fn a_shown_band_is_measured_before_the_body_is_placed() {
        let mut state = ProseState::new().expect("a fresh document");
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let bare = state.paginate(&[]).clone();
        assert_eq!(bare.header_height_px, DocPx::new(0.0));
        state.handle_command(ProseCommand::SetHeaderEnabled { enabled: true });
        let header = state.bands().header.expect("the header");
        state.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: DocOffset::ZERO,
            text: "A header".to_owned(),
        });
        let banded = state.paginate(&[]).clone();
        assert!(banded.header_height_px > DocPx::new(0.0));
        assert!(banded.body_height_px() < bare.body_height_px());
    }

    #[test]
    fn a_band_taller_than_a_quarter_of_the_page_is_clipped_to_it() {
        let (mut state, header) = with_header();
        state.load_faces(&crate::layout::fonts::shipped_faces());
        state.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: DocOffset::ZERO,
            text: "a very long header line ".repeat(400),
        });
        let layout = state.paginate(&[]).clone();
        let cap = layout.content_height_px * crate::layout::geometry::BAND_SHARE;
        assert!((layout.header_height_px - cap).abs() < DocPx::new(0.01));
        assert!(layout.body_height_px() >= layout.content_height_px * 0.5);
        assert_eq!(layout.page_count, crate::events::PageCount::new(1));
    }
}

//! The settings document of the signed-in user (`ambre-spec-user-settings.md`).
//!
//! One Loro document holds one map per section, and one last-writer-wins
//! entry per setting (`SETTINGS-004`). A sealed marker names the document as
//! the settings (`SETTINGS-018`). A relay that points a client at another
//! document gets no setting written into it (`SETTINGS-030`).

#[cfg(test)]
use ambre_client_core::SettingsSection;
use ambre_client_core::settings::catalogue;
use ambre_client_core::{
    BootHint, Setting, SettingChange, SettingValue, SettingWord, UserSettings,
};
use ambre_core::authorship::PeerEntry;
use ambre_core::crdt::CrdtName as _;
use ambre_core::id::{Document, Id, User};
use ambre_core::{
    DeltaBytes, FixedBytes, Instant, LinkState, Ms, PageLoad, Payload, PeerId, SessionNonce,
    SnapshotBytes, TimeSpan,
};
use loro::{ExportMode, LoroDoc, LoroValue, VersionVector};

/// The map that holds the kind marker, beside the section maps.
const META: &str = "meta";
/// The key of the kind marker inside [`META`].
const KIND: &str = "kind";
/// The word the marker holds in a settings document.
const SETTINGS_KIND: &str = "settings";

/// The coalescing delay of the user settings (`ambre-spec-budgets.md`
/// section 8, `SETTINGS-036`): every change made within it leaves as one
/// delta.
pub const COALESCING_DELAY: TimeSpan<Ms> = TimeSpan::<Ms>::new(1000);

/// What the open of a settings document found (`SETTINGS-030`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsKind {
    /// The marker names the settings.
    Settings,
    /// The document holds nothing yet: its first delta has not arrived.
    Empty,
    /// The document holds content and no settings marker: it is another
    /// document, and the client writes nothing into it.
    Foreign,
}

/// Why a settings document refused an import or a write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsRefusal {
    /// The bytes are no Loro update this document can import.
    Undecodable,
    /// The document is no settings document (`SETTINGS-030`).
    Foreign,
}

/// Where an import came from. The relay holds what it served. The local
/// store holds what earlier sessions of this device wrote, which the relay may
/// lack (`SETTINGS-035`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Relay,
    Store,
}

/// The settings document, held by the Logic thread while the user is signed
/// in (`SETTINGS-005`).
pub struct SettingsDoc {
    doc: LoroDoc,
    /// What this session handed over or needs not send: its own flushes, and
    /// every import.
    flushed: VersionVector,
    /// What the relay is known to hold: what it served, the echoes of this
    /// session's own deltas included.
    held: VersionVector,
}

impl SettingsDoc {
    /// A new settings document whose first write is its kind marker
    /// (`SETTINGS-008`, `SETTINGS-018`).
    #[must_use]
    pub fn create() -> Self {
        let doc = LoroDoc::new();
        let meta = doc.get_map(META);
        // A fresh map takes any insert; a failure would leave the marker out
        // and the open refuses the document, which is the safe side.
        if meta.insert(KIND, SETTINGS_KIND).is_ok() {
            doc.commit();
        }
        Self {
            doc,
            flushed: VersionVector::default(),
            held: VersionVector::default(),
        }
    }

    /// An empty document a device opens before any delta arrives.
    #[must_use]
    pub fn opened() -> Self {
        Self {
            doc: LoroDoc::new(),
            flushed: VersionVector::default(),
            held: VersionVector::default(),
        }
    }

    /// What this document is (`SETTINGS-030`).
    #[must_use]
    pub fn kind(&self) -> SettingsKind {
        let marker = self.doc.get_map(META).get(KIND);
        match marker.and_then(|value| value.into_value().ok()) {
            Some(LoroValue::String(word)) if word.as_str() == SETTINGS_KIND => {
                SettingsKind::Settings
            }
            _ if self.doc.oplog_vv().is_empty() => SettingsKind::Empty,
            _ => SettingsKind::Foreign,
        }
    }

    /// Imports a delta or a snapshot of the relay.
    ///
    /// # Errors
    /// [`SettingsRefusal::Undecodable`] for bytes that are no update.
    pub fn import(&mut self, bytes: &Payload<DeltaBytes>) -> Result<(), SettingsRefusal> {
        self.take_in(bytes.as_bytes(), Source::Relay)
    }

    /// Imports `bytes`, a delta or a snapshot. What arrived needs no second
    /// send under this session's nonce. Only the operations the bytes carry
    /// count, so a write of this session still waiting on its delay leaves.
    fn take_in(&mut self, bytes: &[u8], source: Source) -> Result<(), SettingsRefusal> {
        let end = LoroDoc::decode_import_blob_meta(bytes, false)
            .map_err(|_| SettingsRefusal::Undecodable)?
            .partial_end_vv;
        self.doc
            .import(bytes)
            .map_err(|_| SettingsRefusal::Undecodable)?;
        self.flushed.merge(&end);
        if source == Source::Relay {
            self.held.merge(&end);
        }
        Ok(())
    }

    /// Imports the local copy of an earlier session (`SETTINGS-035`). Its
    /// operations leave under the nonce of the session that wrote them, never
    /// under this one's. A row that does not import is skipped.
    pub fn restore(
        &mut self,
        snapshot: Option<&Payload<SnapshotBytes>>,
        deltas: &[Payload<DeltaBytes>],
    ) {
        let rows = snapshot
            .map(Payload::as_bytes)
            .into_iter()
            .chain(deltas.iter().map(Payload::as_bytes));
        for row in rows {
            if self.take_in(row, Source::Store).is_err() {
                tracing::warn!("Logic: a row of the settings copy did not import");
            }
        }
    }

    /// The whole document, for the local copy.
    #[must_use]
    pub fn snapshot(&self) -> Option<Payload<SnapshotBytes>> {
        self.doc.commit();
        self.doc.export(ExportMode::Snapshot).ok().map(Payload::new)
    }

    /// Hands again, at the next flush, what `peer` wrote past the echoes of
    /// the relay. A socket that went down dropped those deltas
    /// (`SETTINGS-035`).
    pub fn rewind(&mut self, peer: PeerId) {
        let echoed = self.held.get(&peer.get()).copied().unwrap_or(0);
        self.flushed.set_end(loro::ID::new(peer.get(), echoed));
    }

    /// What `peer` wrote past what the relay holds, as one delta, or `None`
    /// when the relay holds it all (`AUTHOR-028`).
    #[must_use]
    pub fn unsent(&self, peer: PeerId) -> Option<Payload<DeltaBytes>> {
        let end = self.doc.oplog_vv().get(&peer.get()).copied().unwrap_or(0);
        let start = self.held.get(&peer.get()).copied().unwrap_or(0);
        if end <= start {
            return None;
        }
        let span = loro::IdSpan::new(peer.get(), start, end);
        self.doc
            .export(ExportMode::updates_in_range(vec![span]))
            .ok()
            .map(Payload::new)
    }

    /// The value of one setting: its default when the document holds none,
    /// or a word this release does not know (`SETTINGS-010`, `SETTINGS-021`).
    #[must_use]
    pub fn read<V: SettingValue>(&self, setting: Setting<V>) -> V {
        if self.kind() != SettingsKind::Settings {
            return V::default();
        }
        let stored = self
            .doc
            .get_map(setting.section().word())
            .get(setting.key().as_str())
            .and_then(|value| value.into_value().ok())
            .and_then(|value| match value {
                LoroValue::String(word) => Some(SettingWord::new(word.as_str())),
                _ => None,
            });
        setting.read(stored.as_ref())
    }

    /// Writes one setting into its own section alone (`SETTINGS-013`). The
    /// write leaves on the next [`SettingsDoc::flush`] (`SETTINGS-036`).
    ///
    /// # Errors
    /// [`SettingsRefusal::Foreign`] when the document is no settings
    /// document, which then stays unchanged (`SETTINGS-030`).
    pub fn write<V: SettingValue>(
        &mut self,
        setting: Setting<V>,
        value: &V,
    ) -> Result<(), SettingsRefusal> {
        if self.kind() != SettingsKind::Settings {
            return Err(SettingsRefusal::Foreign);
        }
        let section = self.doc.get_map(setting.section().word());
        section
            .insert(setting.key().as_str(), value.to_word().as_str())
            .map_err(|_| SettingsRefusal::Foreign)?;
        Ok(())
    }

    /// The writes since the last flush, as one delta, or `None` when none
    /// waits. Writes made between two flushes leave together, so a burst of
    /// changes is one delta (`SETTINGS-036`).
    #[must_use]
    pub fn flush(&mut self) -> Option<Payload<DeltaBytes>> {
        self.doc.commit();
        let now = self.doc.oplog_vv();
        if now == self.flushed {
            return None;
        }
        let bytes = self.doc.export(ExportMode::updates(&self.flushed)).ok()?;
        self.flushed = now;
        Some(Payload::new(bytes))
    }

    /// Imports a snapshot of the relay.
    ///
    /// # Errors
    /// [`SettingsRefusal::Undecodable`] for bytes that are no snapshot.
    pub fn import_snapshot(
        &mut self,
        bytes: &Payload<SnapshotBytes>,
    ) -> Result<(), SettingsRefusal> {
        self.take_in(bytes.as_bytes(), Source::Relay)
    }

    /// Every setting of the catalogue, read (`SETTINGS-010`).
    #[must_use]
    pub fn settings(&self) -> UserSettings {
        UserSettings {
            theme: self.read(catalogue::THEME),
            panel_side: self.read(catalogue::PANEL_SIDE),
            panel_fold_prose: self.read(catalogue::PANEL_FOLD_PROSE),
            panel_fold_trame: self.read(catalogue::PANEL_FOLD_TRAME),
            panel_fold_toile: self.read(catalogue::PANEL_FOLD_TOILE),
            interface_language: self.read(catalogue::INTERFACE_LANGUAGE),
            reader_locale: self.read(catalogue::READER_LOCALE),
            number_convention: self.read(catalogue::NUMBER_CONVENTION),
        }
    }

    /// Writes the setting one change names (`SETTINGS-012`).
    ///
    /// # Errors
    /// [`SettingsRefusal::Foreign`], as [`SettingsDoc::write`].
    pub fn apply(&mut self, change: &SettingChange) -> Result<(), SettingsRefusal> {
        match change {
            SettingChange::Theme(theme) => self.write(catalogue::THEME, theme),
            SettingChange::PanelSide(side) => self.write(catalogue::PANEL_SIDE, side),
            SettingChange::PanelFold(application, fold) => {
                self.write(catalogue::panel_fold(*application), fold)
            }
            SettingChange::InterfaceLanguage(language) => {
                self.write(catalogue::INTERFACE_LANGUAGE, language)
            }
            SettingChange::ReaderLocale(locale) => self.write(catalogue::READER_LOCALE, locale),
            SettingChange::NumberConvention(convention) => {
                self.write(catalogue::NUMBER_CONVENTION, convention)
            }
        }
    }

    /// Sets the PeerID this session writes under, and the registry entry its
    /// first delta carries (`AUTHOR-009`).
    ///
    /// # Errors
    /// [`SettingsRefusal::Foreign`] when the marker or the entry does not
    /// write; [`SettingsRefusal::Undecodable`] when Loro refuses the PeerID.
    pub fn adopt_session(
        &mut self,
        peer: PeerId,
        entry: &PeerEntry,
    ) -> Result<(), SettingsRefusal> {
        self.doc
            .set_peer_id(peer.get())
            .map_err(|_| SettingsRefusal::Undecodable)?;
        // A document a first unlock claimed holds nothing yet: its first
        // delta names its kind, under this session's PeerID (`SETTINGS-018`).
        if self.kind() == SettingsKind::Empty {
            self.doc
                .get_map(META)
                .insert(KIND, SETTINGS_KIND)
                .map_err(|_| SettingsRefusal::Foreign)?;
        }
        if self.kind() == SettingsKind::Settings {
            let registry = self
                .doc
                .get_map(ambre_core::crdt::FoundationRoot::Peers.as_str());
            registry
                .insert(ambre_core::PeerKey::of(peer).as_str(), entry.to_bytes())
                .map_err(|_| SettingsRefusal::Foreign)?;
        }
        Ok(())
    }

    /// Imports a delta of the relay once it passed two checks. Every operation
    /// carries the PeerID its signer and nonce derive (`AUTHOR-012`). The
    /// signer is this user, the only member (`SETTINGS-001`).
    ///
    /// # Errors
    /// [`SettingsRefusal::Foreign`] for a delta of another writer, which
    /// changes nothing; [`SettingsRefusal::Undecodable`] for bytes that do
    /// not import.
    pub fn import_signed(
        &mut self,
        payload: &Payload<DeltaBytes>,
        peer: PeerId,
        signer: Option<Id<User>>,
        me: Id<User>,
    ) -> Result<(), SettingsRefusal> {
        let by_peer =
            LoroDoc::decode_import_blob_meta(payload.as_bytes(), false).is_ok_and(|meta| {
                meta.mode == loro::EncodedBlobMode::Updates
                    && meta.partial_end_vv.keys().all(|p| *p == peer.get())
            });
        if !by_peer || signer != Some(me) {
            return Err(SettingsRefusal::Foreign);
        }
        self.import(payload)
    }

    /// The boot hint these settings give the device (`SETTINGS-029`).
    #[must_use]
    pub fn boot_hint(&self) -> BootHint {
        BootHint {
            theme: self.read(catalogue::THEME),
            interface_language: self.read(catalogue::INTERFACE_LANGUAGE),
        }
    }

    /// Whether the section holds an entry under `key`, read or not.
    #[cfg(test)]
    fn holds(&self, section: SettingsSection, key: &str) -> bool {
        self.doc.get_map(section.word()).get(key).is_some()
    }

    /// Writes a raw word, as a later release would.
    #[cfg(test)]
    fn write_raw(&mut self, section: SettingsSection, key: &str, word: &str) {
        let map = self.doc.get_map(section.word());
        assert!(map.insert(key, word).is_ok());
    }

    /// Fixes the peer, so a test orders two writers.
    #[cfg(test)]
    fn with_peer(self, peer: u64) -> Self {
        assert!(self.doc.set_peer_id(peer).is_ok());
        self
    }
}

/// Whether the session identity of the settings document arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Identity {
    /// Not yet: a change waits.
    Awaited,
    /// The session writes under this PeerID.
    Known(PeerId),
}

/// Whether a step the earlier sessions wait on is done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Awaited,
    Done,
}

/// What an earlier session of this device left unsent, and what the relay
/// holds whole (`AUTHOR-028`).
#[derive(Default)]
pub struct Unsent {
    /// Each delta, with the nonce of the session that wrote it.
    pub sends: Vec<(Payload<DeltaBytes>, FixedBytes<SessionNonce>)>,
    /// The sessions the relay holds whole: their rows can go.
    pub forgotten: Vec<PeerId>,
}

/// The settings document of this session, with what a write waits on: the
/// PeerID of the session, and the end of the coalescing delay.
pub struct SettingsSession {
    document_id: Id<Document>,
    doc: SettingsDoc,
    /// Whether the session identity arrived. No write lands before it
    /// (`AUTHOR-009`): the changes wait in `pending`.
    identity: Identity,
    pending: Vec<SettingChange>,
    /// The instant the coalesced writes leave (`SETTINGS-036`).
    due: Option<Instant<PageLoad>>,
    /// The earlier sessions the local copy names (`SETTINGS-035`).
    earlier: Vec<(PeerId, FixedBytes<SessionNonce>)>,
    /// Whether the local copy landed. No copy is written over it before.
    copy: Step,
    /// Whether the relay served its whole replay, so `held` is worth reading.
    replay: Step,
    /// The socket as the Sync worker last reported it.
    link: LinkState,
}

impl SettingsSession {
    /// The session of the settings document the relay named, opened empty.
    #[must_use]
    pub fn new(document_id: Id<Document>) -> Self {
        Self {
            document_id,
            doc: SettingsDoc::opened(),
            identity: Identity::Awaited,
            pending: Vec::new(),
            due: None,
            earlier: Vec::new(),
            copy: Step::Awaited,
            replay: Step::Awaited,
            link: LinkState::Connected,
        }
    }

    /// The settings document's id.
    #[must_use]
    pub const fn document_id(&self) -> Id<Document> {
        self.document_id
    }

    /// The document itself.
    #[must_use]
    pub const fn doc(&self) -> &SettingsDoc {
        &self.doc
    }

    /// The document, to import into.
    pub const fn doc_mut(&mut self) -> &mut SettingsDoc {
        &mut self.doc
    }

    /// The settings as the view sees them: the stored ones, with the
    /// changes still waiting on the identity laid over.
    #[must_use]
    pub fn settings(&self) -> UserSettings {
        let mut settings = self.doc.settings();
        for change in &self.pending {
            lay_over(&mut settings, change);
        }
        settings
    }

    /// The session identity arrived: an empty document a first unlock just
    /// claimed takes its marker, then the waiting changes land.
    pub fn identified(&mut self, peer: PeerId, entry: &PeerEntry, now: Option<Instant<PageLoad>>) {
        if self.doc.adopt_session(peer, entry).is_err() {
            return;
        }
        self.identity = Identity::Known(peer);
        for change in std::mem::take(&mut self.pending) {
            let _ = self.doc.apply(&change);
        }
        self.arm(now);
    }

    /// One change of the view. It lands now, or once the identity arrives,
    /// and leaves at the end of the coalescing delay (`SETTINGS-036`).
    pub fn change(&mut self, change: SettingChange, now: Option<Instant<PageLoad>>) {
        if matches!(self.identity, Identity::Known(_)) {
            if self.doc.apply(&change).is_err() {
                return;
            }
            self.arm(now);
        } else {
            self.pending.push(change);
        }
    }

    /// The local copy landed, with the earlier sessions it names.
    pub fn restored(
        &mut self,
        snapshot: Option<&Payload<SnapshotBytes>>,
        deltas: &[Payload<DeltaBytes>],
        sessions: Vec<(PeerId, FixedBytes<SessionNonce>)>,
    ) {
        self.doc.restore(snapshot, deltas);
        for (peer, nonce) in sessions {
            if !self.earlier.iter().any(|(known, _)| *known == peer) {
                self.earlier.push((peer, nonce));
            }
        }
        self.copy = Step::Done;
    }

    /// The relay served its whole replay.
    pub fn replayed(&mut self) {
        self.replay = Step::Done;
    }

    /// The whole document for the local copy, once the copy it replaces
    /// landed.
    #[must_use]
    pub fn copy(&self) -> Option<Payload<SnapshotBytes>> {
        match self.copy {
            Step::Done => self.doc.snapshot(),
            Step::Awaited => None,
        }
    }

    /// The socket went down or came back. Back after a fall, what this
    /// session wrote past the echoes leaves again at once: the fall dropped
    /// it (`SETTINGS-035`).
    pub fn relinked(&mut self, link: LinkState, now: Option<Instant<PageLoad>>) {
        let fell = self.link == LinkState::Disconnected;
        self.link = link;
        if link == LinkState::Connected
            && fell
            && let Identity::Known(peer) = self.identity
        {
            self.doc.rewind(peer);
            self.due = Some(now.unwrap_or_else(|| Instant::<PageLoad>::new(0.0)));
        }
    }

    /// What the earlier sessions left unsent, once the copy landed, the
    /// replay ended and the identity arrived. Empty before (`AUTHOR-028`).
    pub fn unsent(&mut self) -> Unsent {
        let mut unsent = Unsent::default();
        let Identity::Known(me) = self.identity else {
            return unsent;
        };
        if self.copy == Step::Awaited || self.replay == Step::Awaited {
            return unsent;
        }
        let doc = &self.doc;
        self.earlier.retain(|(peer, nonce)| {
            if *peer == me {
                return false;
            }
            match doc.unsent(*peer) {
                Some(delta) => {
                    unsent.sends.push((delta, *nonce));
                    true
                }
                None => {
                    unsent.forgotten.push(*peer);
                    false
                }
            }
        });
        unsent
    }

    /// Opens the coalescing delay, unless one runs.
    fn arm(&mut self, now: Option<Instant<PageLoad>>) {
        if self.due.is_none() {
            self.due = Some(now.map_or_else(
                || Instant::<PageLoad>::new(0.0),
                |now| now.after(COALESCING_DELAY.to_fractional()),
            ));
        }
    }

    /// The instant the coalesced writes leave.
    #[must_use]
    pub const fn due(&self) -> Option<Instant<PageLoad>> {
        self.due
    }

    /// The delta that leaves at `now`, once the delay ended. With no clock
    /// the delay ends at once, since nothing would end it.
    pub fn release(&mut self, now: Option<Instant<PageLoad>>) -> Option<Payload<DeltaBytes>> {
        let due = self.due?;
        if now.is_some_and(|now| now < due) {
            return None;
        }
        self.due = None;
        self.doc.flush()
    }
}

/// `settings` with `change` laid over.
fn lay_over(settings: &mut UserSettings, change: &SettingChange) {
    match change {
        SettingChange::Theme(theme) => settings.theme = *theme,
        SettingChange::PanelSide(side) => settings.panel_side = *side,
        SettingChange::PanelFold(application, fold) => match application {
            ambre_client_core::PanelApplication::Prose => settings.panel_fold_prose = *fold,
            ambre_client_core::PanelApplication::Trame => settings.panel_fold_trame = *fold,
            ambre_client_core::PanelApplication::Toile => settings.panel_fold_toile = *fold,
        },
        SettingChange::InterfaceLanguage(language) => {
            settings.interface_language = language.clone();
        }
        SettingChange::ReaderLocale(locale) => settings.reader_locale = locale.clone(),
        SettingChange::NumberConvention(convention) => settings.number_convention = *convention,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_client_core::{PanelFold, PanelSide, ThemeChoice};

    /// Two devices that share the settings once, as the relay serves them.
    fn two_devices() -> (SettingsDoc, SettingsDoc) {
        let mut laptop = SettingsDoc::create().with_peer(1);
        let mut phone = SettingsDoc::opened().with_peer(2);
        let first = laptop.flush().expect("the marker leaves");
        phone.import(&first).expect("the phone imports it");
        (laptop, phone)
    }

    /// Hands every pending write of `from` to `to`.
    fn sync(from: &mut SettingsDoc, to: &mut SettingsDoc) {
        if let Some(delta) = from.flush() {
            to.import(&delta).expect("a delta imports");
        }
    }

    /// `SETTINGS-012`: a change reaches the other device as a delta.
    #[test]
    fn a_change_reaches_the_other_device_as_one_delta() {
        let (mut laptop, mut phone) = two_devices();
        laptop
            .write(catalogue::PANEL_SIDE, &PanelSide::Left)
            .expect("a write");
        sync(&mut laptop, &mut phone);
        assert_eq!(phone.read(catalogue::PANEL_SIDE), PanelSide::Left);
    }

    /// `SETTINGS-016`, `SETTINGS-004`: two devices that change one setting
    /// apart converge on one value, and two settings never race.
    #[test]
    fn two_devices_converge_after_writing_apart() {
        let (mut laptop, mut phone) = two_devices();
        laptop
            .write(catalogue::THEME, &ThemeChoice::Dark)
            .expect("a write");
        phone
            .write(catalogue::THEME, &ThemeChoice::Light)
            .expect("a write");
        phone
            .write(catalogue::PANEL_FOLD_TRAME, &PanelFold::Folded)
            .expect("a write");
        let from_laptop = laptop.flush().expect("a delta");
        let from_phone = phone.flush().expect("a delta");
        laptop.import(&from_phone).expect("imports");
        phone.import(&from_laptop).expect("imports");
        assert_eq!(laptop.read(catalogue::THEME), phone.read(catalogue::THEME));
        assert_eq!(laptop.read(catalogue::PANEL_FOLD_TRAME), PanelFold::Folded);
        assert_eq!(laptop.read(catalogue::PANEL_FOLD_PROSE), PanelFold::Open);
    }

    /// `SETTINGS-036`: writes between two flushes leave as one delta, and a
    /// flush with nothing new sends nothing.
    #[test]
    fn a_burst_of_writes_leaves_as_one_delta() {
        let (mut laptop, mut phone) = two_devices();
        for fold in [PanelFold::Folded, PanelFold::Open, PanelFold::Folded] {
            laptop
                .write(catalogue::PANEL_FOLD_PROSE, &fold)
                .expect("a write");
        }
        let delta = laptop.flush().expect("one delta");
        assert!(laptop.flush().is_none(), "nothing more waits");
        phone.import(&delta).expect("imports");
        assert_eq!(phone.read(catalogue::PANEL_FOLD_PROSE), PanelFold::Folded);
    }

    /// `SETTINGS-013`: a write touches its own section alone.
    #[test]
    fn a_write_changes_no_other_section() {
        let mut settings = SettingsDoc::create();
        settings.write_raw(SettingsSection::Murmure, "layout", "compact");
        settings
            .write(catalogue::THEME, &ThemeChoice::Dark)
            .expect("a write");
        assert!(settings.holds(SettingsSection::Murmure, "layout"));
        assert!(!settings.holds(SettingsSection::Locale, "theme"));
        assert!(settings.holds(SettingsSection::Shell, "theme"));
    }

    /// `SETTINGS-021`, `SETTINGS-038`: a later release's word reads as the
    /// default, and a write of another setting keeps it stored.
    #[test]
    fn an_unknown_setting_survives_an_older_client() {
        let (mut later, mut older) = two_devices();
        later.write_raw(SettingsSection::Shell, "theme", "high-contrast");
        later.write_raw(SettingsSection::Shell, "panel.width.rule", "fraction");
        sync(&mut later, &mut older);
        assert_eq!(older.read(catalogue::THEME), ThemeChoice::System);
        older
            .write(catalogue::PANEL_SIDE, &PanelSide::Left)
            .expect("a write");
        sync(&mut older, &mut later);
        assert!(later.holds(SettingsSection::Shell, "panel.width.rule"));
        assert_eq!(
            later.read(catalogue::PANEL_SIDE),
            PanelSide::Left,
            "the older client's write lands"
        );
        let raw = later
            .doc
            .get_map("shell")
            .get("theme")
            .and_then(|value| value.into_value().ok());
        assert_eq!(raw, Some(LoroValue::String("high-contrast".into())));
    }

    /// `SETTINGS-030`: a document with content and no marker
    /// is refused, reads as the defaults and takes no write.
    #[test]
    fn a_document_with_no_settings_marker_takes_no_write() {
        let other = LoroDoc::new();
        assert!(other.get_text("body").insert(0, "a novel").is_ok());
        other.commit();
        let bytes = other.export(ExportMode::all_updates()).expect("an export");
        let mut settings = SettingsDoc::opened();
        assert_eq!(settings.kind(), SettingsKind::Empty);
        settings
            .import(&Payload::new(bytes))
            .expect("the bytes import");
        assert_eq!(settings.kind(), SettingsKind::Foreign);
        assert_eq!(
            settings.write(catalogue::THEME, &ThemeChoice::Dark),
            Err(SettingsRefusal::Foreign)
        );
        assert!(settings.flush().is_none(), "nothing is written into it");
        assert_eq!(settings.read(catalogue::THEME), ThemeChoice::System);
    }

    /// `SETTINGS-018`: a created document carries its marker in its first
    /// delta.
    #[test]
    fn a_created_document_names_its_kind_in_its_first_delta() {
        let mut created = SettingsDoc::create();
        assert_eq!(created.kind(), SettingsKind::Settings);
        let first = created.flush().expect("the first delta");
        let mut opened = SettingsDoc::opened();
        opened.import(&first).expect("imports");
        assert_eq!(opened.kind(), SettingsKind::Settings);
    }

    /// The boot hint mirrors the two settings it carries.
    #[test]
    fn the_boot_hint_mirrors_the_theme_and_the_language() {
        let mut settings = SettingsDoc::create();
        settings
            .write(catalogue::THEME, &ThemeChoice::Dark)
            .expect("a write");
        let french = ambre_client_core::DocumentLocale::parse("fr").expect("a tag");
        settings
            .write(catalogue::INTERFACE_LANGUAGE, &french)
            .expect("a write");
        let hint = settings.boot_hint();
        assert_eq!(hint.theme, ThemeChoice::Dark);
        assert_eq!(hint.interface_language, french);
    }

    /// `SETTINGS-006`: every setting of the catalogue at its longest word
    /// holds the document far under its row of `ambre-spec-budgets.md`
    /// section 6, 256 KB.
    #[test]
    fn the_whole_catalogue_stays_within_its_byte_budget() {
        const BUDGET: usize = 256 * 1024;
        let mut settings = SettingsDoc::create();
        let longest =
            ambre_client_core::DocumentLocale::parse("zh-Hant-TW-x-private1").expect("a tag");
        for change in [
            SettingChange::Theme(ThemeChoice::System),
            SettingChange::PanelSide(PanelSide::Right),
            SettingChange::PanelFold(
                ambre_client_core::PanelApplication::Prose,
                PanelFold::Folded,
            ),
            SettingChange::PanelFold(
                ambre_client_core::PanelApplication::Trame,
                PanelFold::Folded,
            ),
            SettingChange::PanelFold(
                ambre_client_core::PanelApplication::Toile,
                PanelFold::Folded,
            ),
            SettingChange::InterfaceLanguage(longest.clone()),
            SettingChange::ReaderLocale(longest),
            SettingChange::NumberConvention(ambre_client_core::NumberConvention::TypeMine),
        ] {
            settings.apply(&change).expect("a write");
        }
        let delta = settings.flush().expect("a delta");
        assert!(
            delta.as_bytes().len() < BUDGET / 100,
            "{} bytes",
            delta.as_bytes().len()
        );
    }

    /// An import is never sent back.
    #[test]
    fn an_imported_delta_is_never_sent_back() {
        let (_, mut phone) = two_devices();
        assert!(phone.flush().is_none());
    }
}

//! How the settings document rides the Logic thread
//! (`ambre-spec-user-settings.md`).
//!
//! The relay names the document once the session is ready. The device opens
//! the one it kept from an earlier session, or the one the relay names when
//! it kept none (`SETTINGS-031`). The thread opens it like any document and
//! imports what Sync hands it. It writes the changes of the view, keeps a
//! local copy, and hands the view every setting after each change.

use super::*;
use crate::settings::SettingsSession;

/// What the local store answered for the settings document this device kept
/// (`SETTINGS-031`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum KeptSettings {
    /// The store has not answered: a relay answer waits here.
    #[default]
    Reading,
    /// The relay answered first, and waits on the store.
    Named(Id<Document>),
    /// The store answered.
    Read,
}

impl LogicState {
    /// The relay named the settings document of the account. It waits until
    /// the store answers which one the device kept (`SETTINGS-031`).
    pub(crate) fn settings_named(&mut self, document_id: Id<Document>) {
        match self.kept_settings {
            KeptSettings::Reading | KeptSettings::Named(_) => {
                self.kept_settings = KeptSettings::Named(document_id);
            }
            KeptSettings::Read => self.settings_found(document_id),
        }
    }

    /// The store answered which settings document the device kept. That one
    /// opens first, so a relay answer naming another is refused
    /// (`SETTINGS-031`, `SETTINGS-040`).
    pub(crate) fn settings_kept(&mut self, kept: Option<Id<Document>>) {
        let named = match self.kept_settings {
            KeptSettings::Read => return,
            KeptSettings::Named(named) => Some(named),
            KeptSettings::Reading => None,
        };
        self.kept_settings = KeptSettings::Read;
        for document_id in kept.into_iter().chain(named) {
            self.settings_found(document_id);
        }
    }

    /// Open the settings document (`SETTINGS-007`). A second name in one
    /// session is refused, and the document opened first stays
    /// (`SETTINGS-040`). The device keeps its name and its local copy.
    fn settings_found(&mut self, document_id: Id<Document>) {
        if let Some(session) = &self.settings {
            if session.document_id() != document_id {
                tracing::warn!("Logic: a second settings document named, refused");
                // The user sees the refusal, not only a log line
                // (`SETTINGS-050`).
                self.send_to_ui(UiEvent::SyncWarning {
                    document_id,
                    code: ambre_bus::sync_warning::SyncWarningCode::SettingsRefused,
                });
            }
            return;
        }
        self.settings = Some(SettingsSession::new(document_id));
        // Shown for the whole session: the copy never leaves (`SETTINGS-005`).
        self.send_to_persist(document_id, PersistWrite::Shown);
        self.send_to_persist(document_id, PersistWrite::KeepSettings);
        self.send_to_persist(
            document_id,
            PersistWrite::ReadBack {
                doc_type: DocType::Settings,
            },
        );
        self.send_to_sync(SyncCommand::OpenDocument {
            document_id,
            doc_type: DocType::Settings,
            meta: None,
        });
        self.settings_changed();
    }

    /// Whether `document_id` is the settings document.
    pub(crate) fn is_settings(&self, document_id: Id<Document>) -> bool {
        self.settings
            .as_ref()
            .is_some_and(|session| session.document_id() == document_id)
    }

    /// The session identity of the settings document arrived. The store
    /// keeps the session, so what it leaves unsent travels later
    /// (`AUTHOR-028`).
    pub(crate) fn settings_identified(
        &mut self,
        peer: ambre_core::PeerId,
        entry: &ambre_core::PeerEntry,
        nonce: FixedBytes<SessionNonce>,
    ) {
        let now = self.clock.now();
        let Some(session) = self.settings.as_mut() else {
            return;
        };
        session.identified(peer, entry, now);
        let document_id = session.document_id();
        self.send_to_persist(document_id, PersistWrite::Session { peer, nonce });
        self.release_settings(now);
        self.send_settings_unsent();
        self.settings_changed();
    }

    /// The local copy of the settings document landed (`SETTINGS-035`).
    pub(crate) fn settings_restored(
        &mut self,
        snapshot: Option<&ambre_core::Payload<ambre_core::SnapshotBytes>>,
        deltas: &[ambre_core::Payload<ambre_core::DeltaBytes>],
        sessions: Vec<(ambre_core::PeerId, FixedBytes<SessionNonce>)>,
    ) {
        if let Some(session) = self.settings.as_mut() {
            session.restored(snapshot, deltas, sessions);
        }
        self.send_settings_unsent();
        self.settings_changed();
    }

    /// The relay served the whole replay of the settings document.
    pub(crate) fn settings_ready(&mut self) {
        if let Some(session) = self.settings.as_mut() {
            session.replayed();
        }
        self.send_settings_unsent();
    }

    /// The socket of the settings document went down or came back. Back
    /// after a fall, what the fall dropped leaves again (`SETTINGS-035`).
    pub(crate) fn settings_link(&mut self, link: ambre_core::LinkState) {
        let now = self.clock.now();
        if let Some(session) = self.settings.as_mut() {
            session.relinked(link, now);
        }
        if link == ambre_core::LinkState::Connected {
            self.release_settings(now);
            self.send_settings_unsent();
        }
    }

    /// Sends what earlier sessions of this device left unsent, each under
    /// its own nonce (`AUTHOR-028`, `SETTINGS-035`).
    fn send_settings_unsent(&mut self) {
        let Some(session) = self.settings.as_mut() else {
            return;
        };
        let document_id = session.document_id();
        let unsent = session.unsent();
        for (payload, nonce) in unsent.sends {
            self.send_to_sync(SyncCommand::SendUnsent {
                document_id,
                payload,
                nonce,
            });
        }
        for peer in unsent.forgotten {
            self.send_to_persist(document_id, PersistWrite::ForgetSession { peer });
        }
    }

    /// Signing out deletes the local copy of the settings document
    /// (`SETTINGS-014`). The name the device kept stays: it holds no setting.
    pub(crate) fn settings_signed_out(&mut self) {
        if let Some(session) = self.settings.take() {
            self.send_to_persist(session.document_id(), PersistWrite::Erase);
        }
    }

    /// A snapshot of the settings document.
    pub(crate) fn settings_snapshot(
        &mut self,
        payload: &ambre_core::Payload<ambre_core::SnapshotBytes>,
    ) {
        if let Some(session) = self.settings.as_mut()
            && session.doc_mut().import_snapshot(payload).is_err()
        {
            tracing::warn!("Logic: a settings snapshot did not import");
        }
        self.settings_changed();
    }

    /// Deltas of the settings document, each checked before import.
    pub(crate) fn settings_deltas(&mut self, deltas: Vec<SignedDelta>) {
        let Some(me) = self.my_user_id else {
            return;
        };
        if let Some(session) = self.settings.as_mut() {
            for delta in deltas {
                if session
                    .doc_mut()
                    .import_signed(&delta.payload, delta.peer, delta.user, me)
                    .is_err()
                {
                    tracing::warn!("Logic: a settings delta refused");
                }
            }
        }
        self.settings_changed();
    }

    /// One change of the view (`SETTINGS-012`). After a change of the
    /// interface language, the page reloads once the local copy holds it
    /// (`SETTINGS-048`).
    pub(crate) fn change_setting(&mut self, change: ambre_client_core::SettingChange) {
        let now = self.clock.now();
        let language = matches!(
            change,
            ambre_client_core::SettingChange::InterfaceLanguage(_)
        );
        if let Some(session) = self.settings.as_mut() {
            session.change(change, now);
        }
        self.release_settings(now);
        self.settings_changed();
        if language && let Some(session) = &self.settings {
            self.send_to_persist(session.document_id(), PersistWrite::Reload);
        }
    }

    /// Sends the coalesced writes once their delay ended (`SETTINGS-036`).
    pub(crate) fn release_settings(&mut self, now: Option<Instant<PageLoad>>) {
        let Some(session) = self.settings.as_mut() else {
            return;
        };
        let document_id = session.document_id();
        if let Some(payload) = session.release(now) {
            self.send_to_sync(SyncCommand::SendUpdate {
                document_id,
                payload,
                summary: None,
            });
        }
    }

    /// Hands the view every setting (`SETTINGS-012`, `SETTINGS-029`), and
    /// the local store the whole document (`SETTINGS-035`).
    fn settings_changed(&mut self) {
        if let Some(session) = &self.settings {
            let settings = session.settings();
            let copy = session.copy();
            let document_id = session.document_id();
            self.send_to_ui(UiEvent::SettingsChanged { settings });
            if let Some(snapshot) = copy {
                self.send_to_persist(document_id, PersistWrite::Snapshot(snapshot));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_client_core::{PanelApplication, PanelFold, PanelSide, SettingChange, ThemeChoice};

    struct Rig {
        state: LogicState,
        sync: Receiver<SyncCommand>,
        ui: Receiver<UiEvent>,
        persist: Receiver<(Id<Document>, PersistWrite)>,
        document_id: Id<Document>,
    }

    /// A state with no settings document yet, its clock at `now`.
    fn bare(now: f64) -> Rig {
        let (to_ui, ui) = test_link();
        let (to_sync, sync) = test_link();
        let (to_persist, persist) = test_link();
        let mut state = LogicState::new(to_ui, to_sync, to_persist);
        state.clock = SendClock::At(Instant::<PageLoad>::new(now));
        Rig {
            state,
            sync,
            ui,
            persist,
            document_id: Id::<Document>::mint(),
        }
    }

    /// A state whose relay named the settings document, its clock at `now`.
    /// The store kept none.
    fn rig(now: f64) -> Rig {
        let mut rig = bare(now);
        rig.state
            .handle_logic_command(LogicCommand::KeptSettings { document_id: None });
        named(&mut rig.state, rig.document_id);
        rig
    }

    /// The relay names `document_id` as the settings document.
    fn named(state: &mut LogicState, document_id: Id<Document>) {
        state.handle_logic_event(LogicEvent::Rest {
            id: None,
            evt: RestEvent::SettingsDocumentFound { document_id },
        });
    }

    /// The local copy of the settings document landed: `snapshot`, written
    /// by the earlier `sessions`.
    fn restore(
        rig: &mut Rig,
        snapshot: Option<ambre_core::Payload<ambre_core::SnapshotBytes>>,
        sessions: Vec<(ambre_core::PeerId, FixedBytes<SessionNonce>)>,
    ) {
        rig.state
            .handle_logic_command(LogicCommand::RestoreLocalSnapshot {
                document_id: rig.document_id,
                doc_type: DocType::Settings,
                snapshot,
                deltas: Vec::new(),
                sessions,
            });
    }

    /// The session identity of the settings document, under `nonce`.
    fn identify(rig: &mut Rig, nonce: FixedBytes<SessionNonce>) -> ambre_core::PeerId {
        let peer = crate::test_peer();
        rig.state.handle_logic_event(LogicEvent::SessionIdentity {
            document_id: rig.document_id,
            peer,
            entry: test_entry(peer),
            nonce,
        });
        peer
    }

    /// The socket of the settings document as Sync reports it.
    fn link(rig: &mut Rig, connected: ambre_core::LinkState) {
        rig.state.handle_logic_event(LogicEvent::ConnectionStatus {
            document_id: rig.document_id,
            connected,
        });
    }

    /// Moves the clock past the coalescing delay, and releases.
    fn later(rig: &mut Rig, now: f64) {
        rig.state.clock = SendClock::At(Instant::<PageLoad>::new(now));
        rig.state.release_windows();
    }

    /// The last copy the store received for the settings document.
    fn last_copy(rig: &Rig) -> Option<ambre_core::Payload<ambre_core::SnapshotBytes>> {
        rig.persist
            .try_iter()
            .filter(|(document_id, _)| *document_id == rig.document_id)
            .filter_map(|(_, write)| match write {
                PersistWrite::Snapshot(snapshot) => Some(snapshot),
                _ => None,
            })
            .last()
    }

    /// A device of the same user that takes `delta`, written under `peer`.
    fn phone_takes(
        phone: &mut crate::settings::SettingsDoc,
        delta: &ambre_core::Payload<ambre_core::DeltaBytes>,
        peer: ambre_core::PeerId,
    ) {
        let me = Id::<User>::from_bytes([0x5E; 16]);
        phone
            .import_signed(delta, peer, Some(me), me)
            .expect("the phone takes the delta");
    }

    fn last_settings(ui: &Receiver<UiEvent>) -> Option<ambre_client_core::UserSettings> {
        ui.try_iter()
            .filter_map(|event| match event {
                UiEvent::SettingsChanged { settings } => Some(settings),
                _ => None,
            })
            .last()
    }

    fn updates(sync: &Receiver<SyncCommand>) -> Vec<ambre_core::Payload<ambre_core::DeltaBytes>> {
        sync.try_iter()
            .filter_map(|command| match command {
                SyncCommand::SendUpdate { payload, .. } => Some(payload),
                _ => None,
            })
            .collect()
    }

    /// `SETTINGS-007`, `SETTINGS-010`: the named document opens like any
    /// document, the view reads the defaults until it loads, and no view
    /// receives its name.
    #[test]
    fn the_named_settings_document_opens_and_the_view_reads_the_defaults() {
        let rig = rig(0.0);
        let opens: Vec<_> = rig
            .sync
            .try_iter()
            .filter_map(|command| match command {
                SyncCommand::OpenDocument {
                    document_id,
                    doc_type,
                    meta,
                } => Some((document_id, doc_type, meta)),
                _ => None,
            })
            .collect();
        assert_eq!(opens, vec![(rig.document_id, DocType::Settings, None)]);
        let events: Vec<_> = rig.ui.try_iter().collect();
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, UiEvent::Rest { .. })),
            "no view receives the name"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            UiEvent::SettingsChanged { settings } if *settings == Default::default()
        )));
    }

    /// `SETTINGS-036`, `SETTINGS-018`: a burst of changes leaves as one delta
    /// once the coalescing delay ends, and the first delta names the kind.
    #[test]
    fn a_burst_of_changes_leaves_as_one_delta_after_the_delay() {
        let mut rig = rig(0.0);
        rig.sync.try_iter().for_each(drop);
        let peer = rig.state.grant_identity(rig.document_id);
        for fold in [PanelFold::Folded, PanelFold::Open, PanelFold::Folded] {
            rig.state.handle_logic_command(LogicCommand::ChangeSetting {
                change: SettingChange::PanelFold(PanelApplication::Trame, fold),
            });
        }
        rig.state.handle_logic_command(LogicCommand::ChangeSetting {
            change: SettingChange::PanelSide(PanelSide::Left),
        });
        assert!(updates(&rig.sync).is_empty(), "the delay still runs");
        rig.state.clock = SendClock::At(Instant::<PageLoad>::new(1500.0));
        rig.state.release_windows();
        let sent = updates(&rig.sync);
        assert_eq!(sent.len(), 1, "one delta for the whole burst");

        let mut phone = crate::settings::SettingsDoc::opened();
        let me = test_entry(peer).user;
        phone
            .import_signed(&sent[0], peer, Some(me), me)
            .expect("the phone takes the delta");
        assert_eq!(phone.kind(), crate::settings::SettingsKind::Settings);
        assert_eq!(
            phone.settings().panel_fold(PanelApplication::Trame),
            PanelFold::Folded
        );
        assert_eq!(phone.settings().panel_side, PanelSide::Left);
    }

    /// A change before the session identity shows at once in
    /// the view, and lands once the identity arrives.
    #[test]
    fn a_change_before_the_identity_shows_at_once_and_lands_after() {
        let mut rig = rig(0.0);
        rig.ui.try_iter().for_each(drop);
        rig.state.handle_logic_command(LogicCommand::ChangeSetting {
            change: SettingChange::Theme(ThemeChoice::Dark),
        });
        assert_eq!(
            last_settings(&rig.ui).map(|settings| settings.theme),
            Some(ThemeChoice::Dark)
        );
        rig.sync.try_iter().for_each(drop);
        rig.state.grant_identity(rig.document_id);
        rig.state.clock = SendClock::At(Instant::<PageLoad>::new(2000.0));
        rig.state.release_windows();
        assert_eq!(updates(&rig.sync).len(), 1);
    }

    /// `SETTINGS-001`: a delta of another device of the
    /// user moves the view; a delta signed by another user changes nothing.
    #[test]
    fn a_delta_of_the_user_moves_the_view_and_another_users_does_not() {
        let mut rig = rig(0.0);
        let mut laptop = crate::settings::SettingsDoc::opened();
        let laptop_peer = ambre_core::PeerId::new(0x1A);
        assert!(
            laptop
                .adopt_session(laptop_peer, &test_entry(laptop_peer))
                .is_ok()
        );
        assert!(
            laptop
                .write(
                    ambre_client_core::settings::catalogue::THEME,
                    &ThemeChoice::Light
                )
                .is_ok()
        );
        let payload = laptop.flush().expect("a delta");
        let me = Id::<User>::mint();
        rig.state.my_user_id = Some(me);
        let stranger = SignedDelta {
            seq: Seq::<Relay>::new(1),
            payload: payload.clone(),
            nonce: FixedBytes::from_array([0; 16]),
            peer: laptop_peer,
            user: Some(Id::<User>::mint()),
            summary: None,
        };
        rig.ui.try_iter().for_each(drop);
        rig.state.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id: rig.document_id,
            delta: stranger,
        });
        assert_eq!(
            last_settings(&rig.ui).map(|settings| settings.theme),
            Some(ThemeChoice::System)
        );
        let mine = SignedDelta {
            seq: Seq::<Relay>::new(1),
            payload,
            nonce: FixedBytes::from_array([0; 16]),
            peer: laptop_peer,
            user: Some(me),
            summary: None,
        };
        rig.state.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id: rig.document_id,
            delta: mine,
        });
        assert_eq!(
            last_settings(&rig.ui).map(|settings| settings.theme),
            Some(ThemeChoice::Light)
        );
    }

    /// `SETTINGS-011`: a document opened while the settings wait on their
    /// identity and their content opens at once.
    #[test]
    fn the_settings_open_holds_back_no_other_open() {
        let mut rig = rig(0.0);
        rig.sync.try_iter().for_each(drop);
        let prose = Id::<Document>::mint();
        rig.state.handle_logic_command(LogicCommand::OpenDocument {
            document_id: prose,
            doc_type: DocType::Prose,
        });
        assert!(rig.sync.try_iter().any(|command| matches!(
            command,
            SyncCommand::OpenDocument { document_id, .. } if document_id == prose
        )));
    }

    /// `SETTINGS-040`: a second name in one session is refused, and the
    /// document opened first stays.
    #[test]
    fn a_second_settings_document_named_in_a_session_is_refused() {
        let mut rig = rig(0.0);
        rig.sync.try_iter().for_each(drop);
        rig.state.handle_logic_event(LogicEvent::Rest {
            id: None,
            evt: RestEvent::SettingsDocumentFound {
                document_id: Id::<Document>::mint(),
            },
        });
        assert_eq!(rig.sync.try_iter().count(), 0, "nothing more opens");
        assert!(rig.state.is_settings(rig.document_id));
    }

    /// `SETTINGS-050`: the refusal of a second settings document reaches the
    /// view as a warning, and the same document named again warns nothing.
    #[test]
    fn a_refused_settings_document_warns_the_user() {
        use ambre_bus::sync_warning::SyncWarningCode;
        let mut rig = rig(0.0);
        let warnings = |ui: &Receiver<UiEvent>| -> usize {
            ui.try_iter()
                .filter(|event| {
                    matches!(
                        event,
                        UiEvent::SyncWarning {
                            code: SyncWarningCode::SettingsRefused,
                            ..
                        }
                    )
                })
                .count()
        };
        rig.ui.try_iter().for_each(drop);
        named(&mut rig.state, rig.document_id);
        assert_eq!(warnings(&rig.ui), 0, "the kept document warns nothing");
        named(&mut rig.state, Id::<Document>::mint());
        assert_eq!(warnings(&rig.ui), 1);
        assert!(rig.state.is_settings(rig.document_id));
    }

    /// `SETTINGS-031`: the settings document the device kept opens, and a
    /// relay answer naming another is refused, whichever answer comes first.
    /// With none kept, the relay's answer opens and the device keeps it.
    #[test]
    fn the_kept_settings_document_opens_and_another_relay_answer_is_refused() {
        let kept = Id::<Document>::mint();
        let other = Id::<Document>::mint();
        let opens = |sync: &Receiver<SyncCommand>| -> Vec<Id<Document>> {
            sync.try_iter()
                .filter_map(|command| match command {
                    SyncCommand::OpenDocument { document_id, .. } => Some(document_id),
                    _ => None,
                })
                .collect()
        };
        for relay_first in [true, false] {
            let mut rig = bare(0.0);
            if relay_first {
                named(&mut rig.state, other);
                assert!(opens(&rig.sync).is_empty(), "the relay answer waits");
            }
            rig.state.handle_logic_command(LogicCommand::KeptSettings {
                document_id: Some(kept),
            });
            if !relay_first {
                named(&mut rig.state, other);
            }
            assert_eq!(opens(&rig.sync), vec![kept]);
            assert!(rig.state.is_settings(kept));
        }
        let mut rig = bare(0.0);
        named(&mut rig.state, other);
        rig.state
            .handle_logic_command(LogicCommand::KeptSettings { document_id: None });
        assert_eq!(opens(&rig.sync), vec![other]);
        assert!(rig.persist.try_iter().any(|(document_id, write)| {
            document_id == other && matches!(write, PersistWrite::KeepSettings)
        }));
    }

    /// `SETTINGS-035`: a change made while the socket is down leaves again
    /// once it is back, and alone: what the relay echoed stays put.
    #[test]
    fn a_change_made_while_the_socket_is_down_leaves_once_it_is_back() {
        let mut rig = rig(0.0);
        rig.state.my_user_id = Some(Id::<User>::mint());
        restore(&mut rig, None, Vec::new());
        let peer = identify(&mut rig, FixedBytes::from_array([1; 16]));
        rig.state.handle_logic_command(LogicCommand::ChangeSetting {
            change: SettingChange::PanelSide(PanelSide::Left),
        });
        later(&mut rig, 1500.0);
        let first = updates(&rig.sync);
        assert_eq!(first.len(), 1);
        // The relay echoes the first delta: it holds it.
        let me = rig.state.my_user_id;
        rig.state.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id: rig.document_id,
            delta: SignedDelta {
                seq: Seq::<Relay>::new(1),
                payload: first[0].clone(),
                nonce: FixedBytes::from_array([1; 16]),
                peer,
                user: me,
                summary: None,
            },
        });
        link(&mut rig, ambre_core::LinkState::Disconnected);
        rig.state.handle_logic_command(LogicCommand::ChangeSetting {
            change: SettingChange::Theme(ThemeChoice::Dark),
        });
        later(&mut rig, 3000.0);
        assert_eq!(updates(&rig.sync).len(), 1, "the fall drops this one");
        link(&mut rig, ambre_core::LinkState::Connected);
        let again = updates(&rig.sync);
        assert_eq!(again.len(), 1, "the change leaves again");

        let mut phone = crate::settings::SettingsDoc::opened();
        phone_takes(&mut phone, &first[0], peer);
        phone_takes(&mut phone, &again[0], peer);
        assert_eq!(phone.settings().theme, ThemeChoice::Dark);
        assert_eq!(phone.settings().panel_side, PanelSide::Left);
        link(&mut rig, ambre_core::LinkState::Connected);
        assert!(updates(&rig.sync).is_empty(), "no fall, nothing again");
    }

    /// `SETTINGS-048`: a change of the interface language asks the store
    /// for a reload after the copy that holds it.
    #[test]
    fn a_language_change_reloads_the_page_after_its_copy() {
        let mut rig = rig(0.0);
        restore(&mut rig, None, Vec::new());
        identify(&mut rig, FixedBytes::from_array([3; 16]));
        rig.sync.try_iter().for_each(drop);
        rig.persist.try_iter().for_each(drop);
        let french = ambre_client_core::DocumentLocale::parse("fr").expect("a tag");
        rig.state.handle_logic_command(LogicCommand::ChangeSetting {
            change: SettingChange::InterfaceLanguage(french),
        });
        let writes: Vec<PersistWrite> = rig.persist.try_iter().map(|(_, write)| write).collect();
        let copy = writes
            .iter()
            .position(|write| matches!(write, PersistWrite::Snapshot(_)))
            .expect("the copy");
        let reload = writes
            .iter()
            .position(|write| matches!(write, PersistWrite::Reload))
            .expect("the reload");
        assert!(copy < reload, "the reload waits for the copy: {writes:?}");

        // Another setting coalesces as before, and asks for no reload.
        rig.state.handle_logic_command(LogicCommand::ChangeSetting {
            change: SettingChange::Theme(ThemeChoice::Dark),
        });
        assert!(
            !rig.persist
                .try_iter()
                .any(|(_, write)| matches!(write, PersistWrite::Reload))
        );
    }

    /// `SETTINGS-049`: after the reload, the language the local copy holds
    /// wins over an earlier language the relay replays.
    #[test]
    fn the_reloaded_page_keeps_the_language_of_its_copy_over_the_replay() {
        let mut page = rig(0.0);
        restore(&mut page, None, Vec::new());
        let nonce = FixedBytes::from_array([5; 16]);
        let peer = identify(&mut page, nonce);
        page.sync.try_iter().for_each(drop);
        let english = ambre_client_core::DocumentLocale::parse("en").expect("a tag");
        page.state
            .handle_logic_command(LogicCommand::ChangeSetting {
                change: SettingChange::InterfaceLanguage(english),
            });
        later(&mut page, 2000.0);
        let replayed = page
            .sync
            .try_iter()
            .find_map(|command| match command {
                SyncCommand::SendUpdate { payload, .. } => Some(payload),
                _ => None,
            })
            .expect("the English change left");
        link(&mut page, ambre_core::LinkState::Disconnected);
        let french = ambre_client_core::DocumentLocale::parse("fr").expect("a tag");
        page.state
            .handle_logic_command(LogicCommand::ChangeSetting {
                change: SettingChange::InterfaceLanguage(french.clone()),
            });
        let copy = last_copy(&page).expect("the copy holds the French change");

        let mut next = bare(0.0);
        next.document_id = page.document_id;
        next.state.my_user_id = page.state.my_user_id;
        next.state.handle_logic_command(LogicCommand::KeptSettings {
            document_id: Some(page.document_id),
        });
        restore(&mut next, Some(copy), vec![(peer, nonce)]);
        identify(&mut next, FixedBytes::from_array([6; 16]));
        let me = next.state.my_user_id;
        next.state.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id: next.document_id,
            delta: SignedDelta {
                seq: Seq::<Relay>::new(1),
                payload: replayed,
                nonce,
                peer,
                user: me,
                summary: None,
            },
        });
        next.state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: next.document_id,
            role: ambre_core::Role::Owner,
        });
        assert_eq!(
            last_settings(&next.ui).map(|settings| settings.interface_language),
            Some(french)
        );
    }

    /// `SETTINGS-035`: a change an earlier session made offline survives the
    /// page in the local copy. Once the relay's replay is in, the next open
    /// sends it under the nonce of that session. A session the relay holds
    /// whole is forgotten.
    #[test]
    fn a_change_left_offline_by_an_earlier_page_leaves_at_the_next_open() {
        let mut page = rig(0.0);
        restore(&mut page, None, Vec::new());
        let old_nonce = FixedBytes::from_array([7; 16]);
        let old_peer = identify(&mut page, old_nonce);
        link(&mut page, ambre_core::LinkState::Disconnected);
        page.state
            .handle_logic_command(LogicCommand::ChangeSetting {
                change: SettingChange::Theme(ThemeChoice::Dark),
            });
        let copy = last_copy(&page).expect("the change reaches the local copy");
        let sessions = vec![(old_peer, old_nonce)];

        let mut next = bare(0.0);
        next.document_id = page.document_id;
        next.state.handle_logic_command(LogicCommand::KeptSettings {
            document_id: Some(page.document_id),
        });
        restore(&mut next, Some(copy), sessions.clone());
        assert_eq!(
            last_settings(&next.ui).map(|settings| settings.theme),
            Some(ThemeChoice::Dark),
            "the copy shows before the relay answers"
        );
        identify(&mut next, FixedBytes::from_array([8; 16]));
        let unsent = |sync: &Receiver<SyncCommand>| -> Vec<_> {
            sync.try_iter()
                .filter_map(|command| match command {
                    SyncCommand::SendUnsent { payload, nonce, .. } => Some((payload, nonce)),
                    _ => None,
                })
                .collect()
        };
        assert!(unsent(&next.sync).is_empty(), "the replay is not in yet");
        next.state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: next.document_id,
            role: ambre_core::Role::Owner,
        });
        let sent = unsent(&next.sync);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].1, old_nonce);
        let mut phone = crate::settings::SettingsDoc::opened();
        phone_takes(&mut phone, &sent[0].0, old_peer);
        assert_eq!(phone.settings().theme, ThemeChoice::Dark);

        // A relay that holds the earlier session whole owes it nothing.
        let mut held = bare(0.0);
        held.document_id = page.document_id;
        held.state.my_user_id = Some(Id::<User>::mint());
        held.state.handle_logic_command(LogicCommand::KeptSettings {
            document_id: Some(held.document_id),
        });
        let me = held.state.my_user_id;
        held.state.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id: held.document_id,
            delta: SignedDelta {
                seq: Seq::<Relay>::new(1),
                payload: sent[0].0.clone(),
                nonce: old_nonce,
                peer: old_peer,
                user: me,
                summary: None,
            },
        });
        restore(&mut held, None, sessions);
        identify(&mut held, FixedBytes::from_array([9; 16]));
        held.state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: held.document_id,
            role: ambre_core::Role::Owner,
        });
        assert!(unsent(&held.sync).is_empty());
        assert!(held.persist.try_iter().any(|(_, write)| matches!(
            write,
            PersistWrite::ForgetSession { peer } if peer == old_peer
        )));
    }

    /// A write still waiting on its delay leaves after a delta of another
    /// device lands in the meantime.
    #[test]
    fn a_write_waiting_on_its_delay_survives_a_remote_delta() {
        let mut rig = rig(0.0);
        let me = Id::<User>::mint();
        rig.state.my_user_id = Some(me);
        let peer = identify(&mut rig, FixedBytes::from_array([1; 16]));
        rig.state.handle_logic_command(LogicCommand::ChangeSetting {
            change: SettingChange::Theme(ThemeChoice::Dark),
        });
        let mut laptop = crate::settings::SettingsDoc::create();
        let laptop_peer = ambre_core::PeerId::new(0x1B);
        assert!(
            laptop
                .adopt_session(laptop_peer, &test_entry(laptop_peer))
                .is_ok()
        );
        let payload = laptop.flush().expect("a delta");
        rig.state.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id: rig.document_id,
            delta: SignedDelta {
                seq: Seq::<Relay>::new(1),
                payload,
                nonce: FixedBytes::from_array([0; 16]),
                peer: laptop_peer,
                user: Some(me),
                summary: None,
            },
        });
        later(&mut rig, 1500.0);
        let sent = updates(&rig.sync);
        assert_eq!(sent.len(), 1);
        let mut phone = crate::settings::SettingsDoc::opened();
        phone_takes(&mut phone, &sent[0], peer);
        assert_eq!(phone.settings().theme, ThemeChoice::Dark);
    }

    /// `SETTINGS-014`: signing out deletes the local copy of the settings
    /// document.
    #[test]
    fn signing_out_erases_the_local_copy_of_the_settings() {
        let mut rig = rig(0.0);
        rig.persist.try_iter().for_each(drop);
        rig.state.handle_logic_command(LogicCommand::Rest {
            id: ambre_bus::next_request(),
            cmd: ambre_bus::RestCommand::Logout,
        });
        assert!(rig.persist.try_iter().any(|(document_id, write)| {
            document_id == rig.document_id && matches!(write, PersistWrite::Erase)
        }));
    }
}

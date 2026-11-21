//! How the settings document rides the Logic thread
//! (`ambre-spec-user-settings.md`).
//!
//! The relay names the document once the session is ready. The thread opens
//! it like any document and imports what Sync hands it. It writes the
//! changes of the view, and hands the view every setting after each change.

use super::*;
use crate::settings::SettingsSession;

impl LogicState {
    /// The relay named the settings document of the account: open it
    /// (`SETTINGS-007`). A second name in one session is refused, and the
    /// document opened first stays (`SETTINGS-040`).
    pub(crate) fn settings_found(&mut self, document_id: Id<Document>) {
        if let Some(session) = &self.settings {
            if session.document_id() != document_id {
                tracing::warn!("Logic: a second settings document named, refused");
            }
            return;
        }
        self.settings = Some(SettingsSession::new(document_id));
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

    /// The session identity of the settings document arrived.
    pub(crate) fn settings_identified(
        &mut self,
        peer: ambre_core::PeerId,
        entry: &ambre_core::PeerEntry,
    ) {
        let now = self.clock.now();
        if let Some(session) = self.settings.as_mut() {
            session.identified(peer, entry, now);
        }
        self.release_settings(now);
        self.settings_changed();
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

    /// One change of the view (`SETTINGS-012`).
    pub(crate) fn change_setting(&mut self, change: ambre_client_core::SettingChange) {
        let now = self.clock.now();
        if let Some(session) = self.settings.as_mut() {
            session.change(change, now);
        }
        self.release_settings(now);
        self.settings_changed();
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

    /// Hands the view every setting (`SETTINGS-012`, `SETTINGS-029`).
    fn settings_changed(&mut self) {
        if let Some(session) = &self.settings {
            let settings = session.settings();
            self.send_to_ui(UiEvent::SettingsChanged { settings });
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
        document_id: Id<Document>,
    }

    /// A state whose relay named the settings document, its clock at `now`.
    fn rig(now: f64) -> Rig {
        let (to_ui, ui) = test_link();
        let (to_sync, sync) = test_link();
        let (to_persist, persist) = test_link();
        std::mem::forget(persist);
        let mut state = LogicState::new(to_ui, to_sync, to_persist);
        state.clock = SendClock::At(Instant::<PageLoad>::new(now));
        let document_id = Id::<Document>::mint();
        state.handle_logic_event(LogicEvent::Rest {
            id: None,
            evt: RestEvent::SettingsDocumentFound { document_id },
        });
        Rig {
            state,
            sync,
            ui,
            document_id,
        }
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
}

//! Connector lifecycle for [`ToileState`](super::ToileState): create,
//! retarget, delete, plus the map handle and the "which connectors touch these
//! elements" query used by element deletion. Split out of `state/mod.rs`.

use ambre_core::{Board, Px};
use std::cell::Ref;
use std::collections::{HashMap, HashSet};
use std::sync::PoisonError;

use loro::{LoroMap, LoroValue};

use crate::logic::command::{End, ToileError, ToileEvent, ToileOutcome};
use crate::logic::connector::Connector;
use ambre_core::id::{self, Element, Id};

use crate::logic::model::{Anchor, ElementStyle, EndCaps, NodeId, PathType};

use crate::logic::text_merge::merge_edits;
use crate::ui::label::LABEL_W;

use super::codec::{
    bin_of, connector_label_or_seed, encode_anchor, map_str, read_connector, write_connector,
};
use super::names::{ConnectorKey, FieldValues, Fields, Root, entry};
use super::{ToileState, write_edits};

/// One arrow as the cull reads it: its ends, and how far its paint reaches
/// past the box of its outline. The caption box rides the outline, so half
/// its width is the widest reach.
#[derive(Clone, Debug)]
pub(super) struct Wire {
    pub(super) conn: Connector,
    pub(super) reach: Px<Board>,
}

impl ToileState {
    pub(super) fn create_connector(
        &mut self,
        from: Id<Element>,
        from_anch: Anchor,
        to: Id<Element>,
        to_anch: Anchor,
        path_type: PathType,
        arrows: EndCaps,
    ) -> ToileOutcome {
        // Both ends must name an element the board holds, or the arrow would
        // dangle from birth (`TOILE-168`).
        for end in [&from, &to] {
            if self.element_map(end).is_none() {
                return self.err_no_element(end);
            }
        }
        match self.create_connector_inner(from, from_anch, to, to_anch, path_type, arrows) {
            Ok(_) => {
                self.doc.commit();
                self.emit_with_update(vec![self.viewport_event()])
            }
            Err(e) => ToileOutcome::display_only(vec![ToileEvent::Error(e)]),
        }
    }

    /// Create one connector and push it onto the incremental layer cache, **without**
    /// committing. Returns the minted id. The refusal is
    /// [`ToileError::IdsExhausted`] when the session has no fresh id, and a
    /// failed write when the container insert fails.
    /// The caller owns the `doc.commit()` so an import lands its connectors inside
    /// the one import undo group (`toile-import-export`).
    pub(super) fn create_connector_inner(
        &mut self,
        from: Id<Element>,
        from_anch: Anchor,
        to: Id<Element>,
        to_anch: Anchor,
        path_type: PathType,
        arrows: EndCaps,
    ) -> Result<Id<id::Connector>, ToileError> {
        let id = self.mint_connector_id()?;
        let conns = self.doc.get_map(Root::Connectors.as_str());
        let Ok(cm) = conns
            .insert_container(id.as_str(), LoroMap::new())
            .map(Fields::<ConnectorKey>::new)
        else {
            // Unreachable in practice: a container insert of a freshly minted,
            // unique id never collides. Reported defensively as a failed
            // write, since no connector id exists to name.
            return Err(ToileError::WriteFailed(
                "the board refused the new connector".to_owned(),
            ));
        };
        write_connector(&cm, &from, from_anch, &to, to_anch, path_type, arrows);
        let _ = self
            .doc
            .get_movable_list(Root::Layers.as_str())
            .push(NodeId::Connector(id.clone()).encode().as_str());
        self.layers.push(NodeId::Connector(id.clone()));
        Ok(id)
    }

    pub(super) fn retarget_connector(
        &mut self,
        id: Id<id::Connector>,
        endpoint: End,
        to: Id<Element>,
        anch: Anchor,
    ) -> ToileOutcome {
        let Some(cm) = self.connector_map(&id) else {
            return self.err_no_connector(&id);
        };
        match endpoint {
            End::From => {
                let _ = cm.insert(ConnectorKey::From, to.as_str());
                let _ = cm.insert(ConnectorKey::FromAnch, encode_anchor(anch));
            }
            End::To => {
                let _ = cm.insert(ConnectorKey::To, to.as_str());
                let _ = cm.insert(ConnectorKey::ToAnch, encode_anchor(anch));
            }
        }
        self.doc.commit();
        // A retarget to a missing element surfaces as dangling at derivation.
        let mut events = vec![self.viewport_event()];
        if !self.bounds.contains_key(&to) {
            events.push(ToileEvent::DanglingConnector(id));
        }
        self.emit_with_update(events)
    }

    /// Overwrite an arrow's style register, whole. A connector carries no lock
    /// flag -- locking is an element affordance -- so the write is
    /// unconditional once the arrow resolves.
    pub(super) fn restyle_connector(
        &mut self,
        id: Id<id::Connector>,
        style: ElementStyle,
    ) -> ToileOutcome {
        let Some(cm) = self.connector_map(&id) else {
            return self.err_no_connector(&id);
        };
        let _ = cm.insert(
            ConnectorKey::Style,
            LoroValue::Binary(style.to_bytes().into()),
        );
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    /// Write an arrow's caption. The caption is a `LoroText`. The change from
    /// `base`, the caption the editor opened on, to `text` is merged into the
    /// stored caption, and its edits are written into it. Two peers typing on one arrow then
    /// converge character by character (`TOILE-047`), even when one edit lands
    /// while the other's editor is open (`TOILE-263`).
    pub(super) fn set_connector_label(
        &mut self,
        id: Id<id::Connector>,
        base: &str,
        text: &str,
    ) -> ToileOutcome {
        let Some(cm) = self.connector_map(&id) else {
            return self.err_no_connector(&id);
        };
        let Some(label) = connector_label_or_seed(&cm) else {
            return self.err_no_connector(&id);
        };
        let merge = merge_edits(base, &label.to_string(), text);
        let _ = write_edits(&label, &merge.edits);
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    pub(super) fn delete_connector(&mut self, id: Id<id::Connector>) -> ToileOutcome {
        let conns = self.doc.get_map(Root::Connectors.as_str());
        if conns.get(id.as_str()).is_none() {
            return self.err_no_connector(&id);
        }
        let _ = conns.delete(id.as_str());
        self.strip_layers(|node| !matches!(node, NodeId::Connector(c) if c == &id));
        self.doc.commit();
        self.rematerialize_layers();
        self.emit_with_update(vec![self.viewport_event()])
    }

    /// The stored record of one arrow, read straight from the document.
    #[cfg(test)]
    pub(crate) fn connector_of(&self, id: &Id<id::Connector>) -> Option<Connector> {
        read_connector(id.clone(), &self.connector_map(id)?)
    }

    pub(super) fn connector_map(&self, id: &Id<id::Connector>) -> Option<Fields<ConnectorKey>> {
        entry::<ConnectorKey>(&self.doc, id)
    }

    /// Every connector of the board whose endpoint the board no longer holds
    /// (`TOILE-165`). Read before and after a write this viewer did not make,
    /// so the ones that just went dangling can be announced. It reads the
    /// arrow cache, not the document.
    pub(super) fn dangling_connectors(&self) -> HashSet<Id<id::Connector>> {
        self.wires()
            .iter()
            .filter(|(_, w)| {
                !self.bounds.contains_key(&w.conn.from) || !self.bounds.contains_key(&w.conn.to)
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The arrow cache, first brought in step with every arrow the document
    /// was written since the last read. The subscription notes each one, so
    /// no write path keeps the cache by hand (`TOILE-045`).
    pub(super) fn wires(&self) -> Ref<'_, HashMap<Id<id::Connector>, Wire>> {
        let noted = std::mem::take(
            &mut self
                .touched
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .wires,
        );
        let whole = noted.reach == super::caches::Reach::Whole;
        if whole || !noted.ids.is_empty() {
            let mut wires = self.wires.borrow_mut();
            let mut ids: Vec<Id<id::Connector>> = noted.ids.into_iter().collect();
            if whole {
                // Every arrow the cache held, and every one the map holds.
                ids.extend(wires.keys().cloned());
                ids.extend(
                    self.doc
                        .get_map(Root::Connectors.as_str())
                        .keys()
                        .map(|k| Id::<id::Connector>::from_text(k.to_string())),
                );
            }
            let mut gone = self.wires_gone.borrow_mut();
            for id in ids {
                match self.read_wire(&id) {
                    Some(w) => {
                        wires.insert(id, w);
                    }
                    None => {
                        if wires.remove(&id).is_some() {
                            gone.push(id);
                        }
                    }
                }
            }
        }
        self.wires.borrow()
    }

    /// The arrows the board lost since the last call, as one event, or none
    /// (`TOILE-UI-386`). The cache is brought in step first, so a write no
    /// derivation followed still counts.
    pub(super) fn arrows_gone(&self) -> Option<ToileEvent> {
        drop(self.wires());
        let gone = std::mem::take(&mut *self.wires_gone.borrow_mut());
        (!gone.is_empty()).then_some(ToileEvent::ArrowsGone(gone))
    }

    /// One arrow, read whole from the document, or `None` once it is gone.
    fn read_wire(&self, id: &Id<id::Connector>) -> Option<Wire> {
        let cm = self.connector_map(id)?;
        let conn = read_connector(id.clone(), &cm)?;
        let stroke = Px::<Board>::new(f64::from(
            self.style_read(bin_of(&cm, ConnectorKey::Style).as_deref())
                .stroke_w
                .get()
                .max(1.0),
        ));
        Some(Wire {
            conn,
            reach: LABEL_W / 2.0 + stroke / 2.0,
        })
    }

    /// Ids of every connector referencing one of `del` at either endpoint.
    pub(super) fn connectors_touching(
        &self,
        del: &HashSet<&Id<Element>>,
    ) -> HashSet<Id<id::Connector>> {
        let mut out = HashSet::new();
        if let LoroValue::Map(m) = self.doc.get_map(Root::Connectors.as_str()).get_deep_value() {
            for (key, v) in m.iter() {
                if let LoroValue::Map(f) = v {
                    let f = FieldValues::<ConnectorKey>::new(f);
                    let touches = |key| {
                        map_str(f, key)
                            .map(Id::<Element>::from_text)
                            .is_some_and(|id| del.contains(&id))
                    };
                    if touches(ConnectorKey::From) || touches(ConnectorKey::To) {
                        out.insert(Id::<id::Connector>::from_text(key.clone()));
                    }
                }
            }
        }
        out
    }
}

//! Connector lifecycle for [`ToileState`](super::ToileState): create,
//! retarget, delete, plus the map handle and the "which connectors touch these
//! elements" query used by element deletion. Split out of `state/mod.rs`.

use std::collections::HashSet;

use loro::{LoroMap, LoroValue};

use crate::logic::command::{End, ToileError, ToileEvent, ToileOutcome};
use crate::logic::model::{
    Anchor, ConnectorId, ElementId, ElementStyle, EndCaps, NodeId, PathType,
};

use crate::logic::text_merge::merge_text;

use super::codec::{connector_label_or_seed, encode_anchor, map_str, write_connector};
use super::{CONNECTORS, LAYERS, ToileState};

impl ToileState {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_connector(
        &mut self,
        from: ElementId,
        from_anch: Anchor,
        to: ElementId,
        to_anch: Anchor,
        path_type: PathType,
        arrows: EndCaps,
    ) -> ToileOutcome {
        match self.create_connector_inner(from, from_anch, to, to_anch, path_type, arrows) {
            Some(_) => {
                self.doc.commit();
                self.emit_with_update(vec![self.viewport_event()])
            }
            // Unreachable in practice: a container insert of a freshly minted,
            // unique id never collides. Reported defensively, no id to name.
            None => ToileOutcome::display_only(vec![ToileEvent::Error(
                ToileError::NoSuchConnector(String::new()),
            )]),
        }
    }

    /// Create one connector and push it onto the incremental layer cache, **without**
    /// committing. Returns the minted id (or `None` if the container insert failed).
    /// The caller owns the `doc.commit()` so an import lands its connectors inside
    /// the one import undo group (`toile-import-export`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_connector_inner(
        &mut self,
        from: ElementId,
        from_anch: Anchor,
        to: ElementId,
        to_anch: Anchor,
        path_type: PathType,
        arrows: EndCaps,
    ) -> Option<ConnectorId> {
        let id = self.mint_connector_id();
        let conns = self.doc.get_map(CONNECTORS);
        let Ok(cm) = conns.insert_container(id.as_str(), LoroMap::new()) else {
            return None;
        };
        write_connector(&cm, &from, from_anch, &to, to_anch, path_type, arrows);
        let _ = self
            .doc
            .get_movable_list(LAYERS)
            .push(NodeId::Connector(id.clone()).encode().as_str());
        self.layers.push(NodeId::Connector(id.clone()));
        Some(id)
    }

    pub(super) fn retarget_connector(
        &mut self,
        id: ConnectorId,
        endpoint: End,
        to: ElementId,
        anch: Anchor,
    ) -> ToileOutcome {
        let Some(cm) = self.connector_map(&id) else {
            return self.err_no_connector(&id);
        };
        match endpoint {
            End::From => {
                let _ = cm.insert("from", to.as_str());
                let _ = cm.insert("from_anch", encode_anchor(anch));
            }
            End::To => {
                let _ = cm.insert("to", to.as_str());
                let _ = cm.insert("to_anch", encode_anchor(anch));
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
        id: ConnectorId,
        style: ElementStyle,
    ) -> ToileOutcome {
        let Some(cm) = self.connector_map(&id) else {
            return self.err_no_connector(&id);
        };
        let _ = cm.insert("style", LoroValue::Binary(style.to_bytes().into()));
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    /// Write an arrow's caption. The caption is a `LoroText`. The change from
    /// `base`, the caption the editor opened on, to `text` is merged into the
    /// stored caption, then diffed into it. Two peers typing on one arrow then
    /// converge character by character (`TOILE-047`), even when one edit lands
    /// while the other's editor is open (`TOILE-263`).
    pub(super) fn set_connector_label(
        &mut self,
        id: ConnectorId,
        base: &str,
        text: &str,
    ) -> ToileOutcome {
        let Some(cm) = self.connector_map(&id) else {
            return self.err_no_connector(&id);
        };
        let Some(label) = connector_label_or_seed(&cm) else {
            return self.err_no_connector(&id);
        };
        let merged = merge_text(base, &label.to_string(), text);
        // The diff is bounded by the caption's length, which a caption is; a
        // timeout here would silently drop the edit, so none is set.
        let _ = label.update(&merged, loro::UpdateOptions::default());
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    pub(super) fn delete_connector(&mut self, id: ConnectorId) -> ToileOutcome {
        let conns = self.doc.get_map(CONNECTORS);
        if conns.get(id.as_str()).is_none() {
            return self.err_no_connector(&id);
        }
        let _ = conns.delete(id.as_str());
        self.strip_layers(|node| !matches!(node, NodeId::Connector(c) if c == &id));
        self.doc.commit();
        self.rematerialize_layers();
        self.emit_with_update(vec![self.viewport_event()])
    }

    pub(super) fn connector_map(&self, id: &ConnectorId) -> Option<LoroMap> {
        self.doc
            .get_map(CONNECTORS)
            .get(id.as_str())?
            .into_container()
            .ok()?
            .into_map()
            .ok()
    }

    /// Every connector of the board whose endpoint the board no longer holds
    /// (`TOILE-165`). Read before and after a write this viewer did not make,
    /// so the ones that just went dangling can be announced.
    pub(super) fn dangling_connectors(&self) -> HashSet<ConnectorId> {
        let mut out = HashSet::new();
        if let LoroValue::Map(m) = self.doc.get_map(CONNECTORS).get_deep_value() {
            for (key, v) in m.iter() {
                if let LoroValue::Map(f) = v {
                    let from = ElementId(map_str(f, "from"));
                    let to = ElementId(map_str(f, "to"));
                    if !self.bounds.contains_key(&from) || !self.bounds.contains_key(&to) {
                        out.insert(ConnectorId(key.clone()));
                    }
                }
            }
        }
        out
    }

    /// Ids of every connector referencing one of `del` at either endpoint.
    pub(super) fn connectors_touching(&self, del: &HashSet<&ElementId>) -> HashSet<ConnectorId> {
        let mut out = HashSet::new();
        if let LoroValue::Map(m) = self.doc.get_map(CONNECTORS).get_deep_value() {
            for (key, v) in m.iter() {
                if let LoroValue::Map(f) = v {
                    let from = map_str(f, "from");
                    let to = map_str(f, "to");
                    if del.contains(&ElementId(from)) || del.contains(&ElementId(to)) {
                        out.insert(ConnectorId(key.clone()));
                    }
                }
            }
        }
        out
    }
}

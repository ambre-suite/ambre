//! Clipboard copy / paste / duplicate for [`ToileState`](super::ToileState)
//! Split out of `state/mod.rs`. A selection is snapshotted into an
//! id-free [`ClipboardPayload`] (subtrees, slot-referenced connectors and
//! membership) and rematerialized with fresh ids.

use std::collections::{HashMap, HashSet};

use kurbo::{Point, Rect, Vec2};
use loro::{LoroMap, LoroMovableList, LoroValue};

use crate::logic::command::{
    ClipConnector, ClipElement, ClipMembership, ClipboardPayload, ToileEvent, ToileOutcome,
};
use crate::logic::model::{ConnectorId, ElementId, NodeId};

use super::codec::{capture_scalars, map_str, write_clip_fields};
use super::{CHILDREN, CONNECTORS, ELEMENTS, LAYERS, ToileState};

impl ToileState {
    /// Read the selection into a self-contained, id-free payload. Non-mutating:
    /// the UI stashes the result and may mirror it to the OS clipboard.
    pub(super) fn copy_elements(&self, ids: &[ElementId]) -> ToileOutcome {
        ToileOutcome::display_only(vec![ToileEvent::Clipboard(self.build_clipboard(ids))])
    }

    /// Insert a clipboard payload offset so `origin` lands at `at`. One undo item.
    pub(super) fn paste_elements(&mut self, payload: &ClipboardPayload, at: Point) -> ToileOutcome {
        let offset = Vec2::new(at.x - payload.origin.x0, at.y - payload.origin.y0);
        self.insert_clipboard(payload, offset)
    }

    /// Copy + paste fused into a single undo item, offset by `delta`.
    pub(super) fn duplicate_elements(&mut self, ids: &[ElementId], delta: Vec2) -> ToileOutcome {
        let payload = self.build_clipboard(ids);
        self.insert_clipboard(&payload, delta)
    }

    /// Snapshot a selection (its full subtrees) into an id-free [`ClipboardPayload`].
    /// Elements are captured in z-order and referenced by slot; connectors survive
    /// only when both endpoints are captured; group / frame membership is remapped
    /// to slots.
    fn build_clipboard(&self, ids: &[ElementId]) -> ClipboardPayload {
        // A copied group carries its members: expand every id to its full subtree.
        let mut set: HashSet<ElementId> = HashSet::new();
        for id in ids {
            self.collect_subtree(id, &mut set);
        }
        // Stable slot order = z-order, filtered to the captured set (a captured
        // node absent from `layers` still gets a trailing slot, defensively).
        let mut order: Vec<ElementId> = self
            .layers
            .iter()
            .filter_map(|n| match n {
                NodeId::Element(e) if set.contains(e) => Some(e.clone()),
                _ => None,
            })
            .collect();
        for id in &set {
            if !order.contains(id) {
                order.push(id.clone());
            }
        }
        let slot_of: HashMap<ElementId, usize> = order
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, id)| (id, i))
            .collect();

        let mut elements = Vec::with_capacity(order.len());
        let mut membership = Vec::new();
        let mut origin: Option<Rect> = None;
        for (slot, id) in order.iter().enumerate() {
            let fields = match self.element_map(id) {
                Some(em) => match em.get_deep_value() {
                    LoroValue::Map(f) => capture_scalars(&f, &[]),
                    _ => Vec::new(),
                },
                None => Vec::new(),
            };
            elements.push(ClipElement { fields });
            // Group / frame membership -> child slots (captured children only).
            let child_slots: Vec<usize> = self
                .children_of(id)
                .iter()
                .filter_map(|c| slot_of.get(c).copied())
                .collect();
            if !child_slots.is_empty() {
                membership.push(ClipMembership {
                    parent_slot: slot,
                    child_slots,
                });
            }
            if let Some(r) = self.bounds.get(id) {
                origin = Some(match origin {
                    Some(u) => u.union(*r),
                    None => *r,
                });
            }
        }

        // Connectors whose BOTH endpoints are captured: a half-copied
        // connector would dangle, so it is dropped.
        let mut connectors = Vec::new();
        if let LoroValue::Map(m) = self.doc.get_map(CONNECTORS).get_deep_value() {
            for (_key, v) in m.iter() {
                if let LoroValue::Map(f) = v {
                    let from = ElementId(map_str(f, "from"));
                    let to = ElementId(map_str(f, "to"));
                    if let (Some(&from_slot), Some(&to_slot)) =
                        (slot_of.get(&from), slot_of.get(&to))
                    {
                        connectors.push(ClipConnector {
                            from_slot,
                            to_slot,
                            fields: capture_scalars(f, &["from", "to"]),
                        });
                    }
                }
            }
        }

        ClipboardPayload {
            elements,
            connectors,
            membership,
            origin: origin.unwrap_or_default(),
        }
    }

    /// Materialize a clipboard payload with fresh ids, offset by `offset`, pushed
    /// above the current top of the z-order. One `doc.commit()` -> one undo item.
    fn insert_clipboard(&mut self, payload: &ClipboardPayload, offset: Vec2) -> ToileOutcome {
        if payload.elements.is_empty() {
            return ToileOutcome::display_only(Vec::new());
        }
        // Mint every fresh id up front so membership / connectors can remap slots
        // to new ids before anything is written.
        let new_ids: Vec<ElementId> = (0..payload.elements.len())
            .map(|_| self.mint_element_id())
            .collect();
        let new_conns: Vec<ConnectorId> = (0..payload.connectors.len())
            .map(|_| self.mint_connector_id())
            .collect();

        let elems = self.doc.get_map(ELEMENTS);
        let layers = self.doc.get_movable_list(LAYERS);
        let mut created = Vec::with_capacity(new_ids.len());
        for (slot, clip) in payload.elements.iter().enumerate() {
            let id = &new_ids[slot];
            let Ok(em) = elems.insert_container(id.as_str(), LoroMap::new()) else {
                continue;
            };
            write_clip_fields(&em, &clip.fields, offset);
            let _ = layers.push(NodeId::Element(id.clone()).encode().as_str());
            created.push(id.clone());
        }
        // Rebuild each group / frame `children` list with the remapped ids.
        for m in &payload.membership {
            if let Some(em) = self.element_map(&new_ids[m.parent_slot])
                && let Ok(children) = em.insert_container(CHILDREN, LoroMovableList::new())
            {
                for &cs in &m.child_slots {
                    let _ = children.push(new_ids[cs].as_str());
                }
            }
        }
        // Connectors: remap endpoints, keep anchors / path / arrows.
        let conns = self.doc.get_map(CONNECTORS);
        for (i, c) in payload.connectors.iter().enumerate() {
            let Ok(cm) = conns.insert_container(new_conns[i].as_str(), LoroMap::new()) else {
                continue;
            };
            write_clip_fields(&cm, &c.fields, Vec2::ZERO);
            let _ = cm.insert("from", new_ids[c.from_slot].as_str());
            let _ = cm.insert("to", new_ids[c.to_slot].as_str());
            let _ = layers.push(NodeId::Connector(new_conns[i].clone()).encode().as_str());
        }
        self.doc.commit();

        self.rebuild_caches();
        let mut display: Vec<ToileEvent> = created
            .into_iter()
            .map(ToileEvent::ElementCreated)
            .collect();
        display.push(self.viewport_event());
        display.push(self.undo_state_event());
        self.emit_with_update(display)
    }
}

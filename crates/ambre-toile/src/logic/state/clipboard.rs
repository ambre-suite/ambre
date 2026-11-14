//! Clipboard copy / paste / duplicate for [`ToileState`](super::ToileState)
//! Split out of `state/mod.rs`. A selection is snapshotted into an
//! id-free [`ClipboardPayload`] (subtrees, slot-referenced connectors and
//! membership) and rematerialized with fresh ids.

use ambre_core::crdt::CrdtName;
use ambre_core::hash::{HashMap, HashSet};
use ambre_core::index::{IdxVec, Pos};

use kurbo::{Point, Rect, Vec2};
use loro::{LoroMap, LoroMovableList, LoroText, LoroValue};

use crate::logic::command::{
    ClipConnector, ClipElement, ClipMembership, ClipSlot, ClipboardPayload, ToileEvent,
    ToileOutcome,
};
use ambre_core::id::{Connector, Element, Id};

use crate::logic::model::NodeId;

use super::ToileState;
use super::codec::{capture_scalars, map_str, write_clip_fields};
use super::names::{ConnectorKey, ElementKey, FieldValues, Fields, Root};

impl ToileState {
    /// Read the selection into a self-contained, id-free payload. Non-mutating:
    /// the UI stashes the result and may mirror it to the OS clipboard.
    pub(super) fn copy_elements(&self, ids: &[Id<Element>]) -> ToileOutcome {
        ToileOutcome::display_only(vec![ToileEvent::Clipboard(self.build_clipboard(ids))])
    }

    /// Insert a clipboard payload offset so `origin` lands at `at`. One undo item.
    pub(super) fn paste_elements(&mut self, payload: &ClipboardPayload, at: Point) -> ToileOutcome {
        let offset = Vec2::new(at.x - payload.origin.x0, at.y - payload.origin.y0);
        self.insert_clipboard(payload, offset)
    }

    /// Copy + paste fused into a single undo item, offset by `delta`.
    pub(super) fn duplicate_elements(&mut self, ids: &[Id<Element>], delta: Vec2) -> ToileOutcome {
        let payload = self.build_clipboard(ids);
        self.insert_clipboard(&payload, delta)
    }

    /// Snapshot a selection (its full subtrees) into an id-free [`ClipboardPayload`].
    /// Elements are captured in z-order and referenced by slot; connectors survive
    /// only when both endpoints are captured; group / frame membership is remapped
    /// to slots.
    fn build_clipboard(&self, ids: &[Id<Element>]) -> ClipboardPayload {
        // A copied group carries its members: expand every id to its full subtree.
        let mut set: HashSet<Id<Element>> = HashSet::default();
        for id in ids {
            self.collect_subtree(id, &mut set);
        }
        // Stable slot order = z-order, filtered to the captured set (a captured
        // node absent from `layers` still gets a trailing slot, defensively).
        let mut order: Vec<Id<Element>> = self
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
        let slot_of: HashMap<Id<Element>, Pos<ClipSlot>> = order
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, id)| (id, Pos::<ClipSlot>::from_usize(i)))
            .collect();

        let mut elements = Vec::with_capacity(order.len());
        let mut membership = Vec::new();
        let mut origin: Option<Rect> = None;
        for (slot, id) in order.iter().enumerate() {
            let slot = Pos::<ClipSlot>::from_usize(slot);
            // `notes` reads as a plain string through the deep value. Writing
            // it back as one makes a frame whose notes are a scalar register,
            // so it is skipped here and carried beside the fields
            // (`TOILE-137`).
            let fields = match self.element_map(id) {
                Some(em) => match em.map().get_deep_value() {
                    LoroValue::Map(f) => {
                        let f = FieldValues::<ElementKey>::new(&f);
                        capture_scalars(f, ElementKey::ALL, &[ElementKey::Notes])
                    }
                    _ => Vec::new(),
                },
                None => Vec::new(),
            };
            // A copied frame carries its notes, empty when it holds none.
            let notes = self
                .frame(id)
                .map(|frame| self.notes(&frame).unwrap_or_default());
            elements.push(ClipElement { fields, notes });
            // Group / frame membership -> child slots (captured children only).
            let child_slots: Vec<Pos<ClipSlot>> = self
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
        if let LoroValue::Map(m) = self.doc.get_map(Root::Connectors.as_str()).get_deep_value() {
            for v in m.values() {
                if let LoroValue::Map(f) = v {
                    let f = FieldValues::<ConnectorKey>::new(f);
                    let slot = |key| {
                        map_str(f, key).and_then(|id| slot_of.get(&Id::<Element>::from_text(id)))
                    };
                    if let (Some(&from_slot), Some(&to_slot)) =
                        (slot(ConnectorKey::From), slot(ConnectorKey::To))
                    {
                        connectors.push(ClipConnector {
                            from_slot,
                            to_slot,
                            fields: capture_scalars(
                                f,
                                ConnectorKey::ALL,
                                &[ConnectorKey::From, ConnectorKey::To],
                            ),
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
        // to new ids before anything is written. A session out of fresh ids
        // thus refuses the whole paste, and writes nothing.
        let new_ids: IdxVec<ClipSlot, Id<Element>> = match (0..payload.elements.len())
            .map(|_| self.mint_element_id())
            .collect()
        {
            Ok(ids) => ids,
            Err(e) => return ToileOutcome::display_only(vec![ToileEvent::Error(e)]),
        };
        let new_conns: Vec<Id<Connector>> = match (0..payload.connectors.len())
            .map(|_| self.mint_connector_id())
            .collect()
        {
            Ok(ids) => ids,
            Err(e) => return ToileOutcome::display_only(vec![ToileEvent::Error(e)]),
        };

        let elems = self.doc.get_map(Root::Elements.as_str());
        let layers = self.doc.get_movable_list(Root::Layers.as_str());
        let mut created = Vec::with_capacity(new_ids.len().to_usize());
        // `new_ids` and `new_conns` are minted one per payload entry. A slot
        // the payload names past them is a malformed paste, and it is skipped.
        for (id, clip) in new_ids.iter().zip(&payload.elements) {
            let Ok(em) = elems
                .insert_container(id.as_str(), LoroMap::new())
                .map(Fields::<ElementKey>::new)
            else {
                continue;
            };
            // A key the schema does not name was skipped when the payload was
            // read, like a dangling slot.
            let fields = clip.fields.iter().map(|(key, value)| (*key, value));
            write_clip_fields(&em, fields, offset);
            // A frame is born with its two containers, here as at every other
            // birth. The membership list makes the copy a frame an element can
            // be added to (`TOILE-138`). The notes text is the container every
            // peer must see before anyone edits it (`TOILE-137`, `TOILE-051`).
            if let Some(notes) = &clip.notes {
                let _ = em.insert_container(ElementKey::Children, LoroMovableList::new());
                if let Ok(text) = em.insert_container(ElementKey::Notes, LoroText::new())
                    && !notes.is_empty()
                {
                    let _ = text.insert(0, notes);
                }
            }
            let _ = layers.push(NodeId::Element(id.clone()).encode().as_str());
            created.push(id.clone());
        }
        // Rebuild each group / frame `children` list with the remapped ids.
        for m in &payload.membership {
            if let Some(parent) = new_ids.get(m.parent_slot)
                && let Some(em) = self.element_map(parent)
                && let Ok(children) =
                    em.insert_container(ElementKey::Children, LoroMovableList::new())
            {
                for child in m.child_slots.iter().filter_map(|&cs| new_ids.get(cs)) {
                    let _ = children.push(child.as_str());
                }
            }
        }
        // Connectors: remap endpoints, keep anchors / path / arrows.
        let conns = self.doc.get_map(Root::Connectors.as_str());
        for (conn_id, c) in new_conns.iter().zip(&payload.connectors) {
            let (Some(from), Some(to)) = (new_ids.get(c.from_slot), new_ids.get(c.to_slot)) else {
                continue;
            };
            let Ok(cm) = conns
                .insert_container(conn_id.as_str(), LoroMap::new())
                .map(Fields::<ConnectorKey>::new)
            else {
                continue;
            };
            let fields = c.fields.iter().map(|(key, value)| (*key, value));
            write_clip_fields(&cm, fields, Vec2::ZERO);
            let _ = cm.insert(ConnectorKey::From, from.as_str());
            let _ = cm.insert(ConnectorKey::To, to.as_str());
            let _ = layers.push(NodeId::Connector(conn_id.clone()).encode().as_str());
        }
        self.doc.commit();

        self.rebuild_caches();
        let mut display: Vec<ToileEvent> = created
            .into_iter()
            .map(|id| ToileEvent::ElementCreated {
                id,
                correlate: None,
            })
            .collect();
        display.push(self.viewport_event());
        display.push(self.undo_state_event());
        self.emit_with_update(display)
    }
}

#[cfg(test)]
mod tests {
    use ambre_core::crdt::CrdtName as _;
    use kurbo::{Point, Size};

    use crate::logic::command::{
        ClipValue, ClipboardPayload, InitContent, ToileCommand, ToileEvent,
    };
    use crate::logic::model::{ElementKind, ShapeKind};
    use crate::logic::state::ToileState;

    /// `TOILE-281`: a pasted payload is read through the schema of the map it lands in. A
    /// field the schema does not name is skipped as the clipboard bytes are
    /// read. A paste never writes a key the board does not read.
    #[test]
    fn a_paste_writes_only_the_keys_the_schema_names() {
        let mut st = ToileState::new();
        let out = st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(10.0, 10.0),
            size: Size::new(20.0, 20.0),
            content: InitContent::None,
            fill: None,
            correlate: None,
        });
        let Some(id) = out.display.into_iter().find_map(|e| match e {
            ToileEvent::ElementCreated { id, .. } => Some(id),
            _ => None,
        }) else {
            panic!("no ElementCreated");
        };
        let copy = st.handle_command(ToileCommand::CopyElements { ids: vec![id] });
        let Some(copied) = copy.display.into_iter().find_map(|e| match e {
            ToileEvent::Clipboard(p) => Some(p),
            _ => None,
        }) else {
            panic!("no Clipboard");
        };
        // The bytes a board that names one more key would put on the
        // clipboard: each key as the word it stores, then the stray one.
        let mut fields: Vec<(String, ClipValue)> = copied.elements[0]
            .fields
            .iter()
            .map(|(key, value)| (key.as_str().to_owned(), value.clone()))
            .collect();
        fields.push(("stray".to_owned(), ClipValue::Bool(true)));
        let raw = (
            vec![(fields, copied.elements[0].notes.clone())],
            &copied.connectors,
            &copied.membership,
            copied.origin,
        );
        let bytes = postcard::to_allocvec(&raw).expect("a payload encodes");
        let payload: ClipboardPayload = postcard::from_bytes(&bytes).expect("the payload reads");
        assert_eq!(
            payload, copied,
            "the stray key is dropped as the bytes are read"
        );
        let paste = st.handle_command(ToileCommand::PasteElements {
            payload,
            at: Point::new(100.0, 100.0),
        });
        let Some(pasted) = paste.display.into_iter().find_map(|e| match e {
            ToileEvent::ElementCreated { id, .. } => Some(id),
            _ => None,
        }) else {
            panic!("the paste created nothing");
        };
        let Some(em) = st.element_map(&pasted) else {
            panic!("no pasted map");
        };
        assert!(em.map().get("stray").is_none(), "the stray key is skipped");
        assert_eq!(
            st.bounds_of(&pasted),
            Some(kurbo::Rect::new(100.0, 100.0, 120.0, 120.0)),
            "the schema keys land, offset to the cursor"
        );
    }
}

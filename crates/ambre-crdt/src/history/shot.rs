//! A container captured by value (`ambre-spec-undo.md section 3`). A step keeps
//! a nested container a commit created or deleted this way, so a replay can
//! create it again.
//!
//! Loro mints a new identity for each container it creates. A replay that
//! creates a container again so notes the old identity against the new one.
//! Every later step that names the old one then reaches the new one.

use ambre_core::index::Pos;
use loro::{
    Container, ContainerID, ContainerTrait, ContainerType, InternalString, LoroDoc, LoroList,
    LoroMap, LoroMovableList, LoroText, LoroValue,
};

use super::rich::Rich;
use super::{Mirror, Mirrored, Renames, Slot};

/// A value a map key or a list slot holds: a plain value, or a container.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Held {
    Value(LoroValue),
    Container(Shot),
}

/// One container and, once captured, what it holds.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Shot {
    /// The identity the container had when the step recorded it.
    pub(super) id: ContainerID,
    /// What it holds. `None` until a replay or a deletion captures it.
    pub(super) body: Option<Body>,
}

/// What a captured container holds, by kind.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Body {
    Map(Vec<(InternalString, Held)>),
    List(Vec<Held>),
    Text(Rich),
}

impl Held {
    /// A value as the mirror or a read holds it: a container stands as its id.
    pub(super) fn of(value: &LoroValue) -> Self {
        match value {
            LoroValue::Container(id) => Self::Container(Shot {
                id: id.clone(),
                body: None,
            }),
            value => Self::Value(value.clone()),
        }
    }

    /// Whether `now`, the value a key holds, is this one. A container is
    /// itself under the identity a replay gave it.
    pub(super) fn is(&self, now: Option<&LoroValue>, renames: &Renames) -> bool {
        match (self, now) {
            (Self::Value(value), Some(now)) => value == now,
            (Self::Container(shot), Some(LoroValue::Container(id))) => {
                renames.resolve(&shot.id) == *id
            }
            _ => false,
        }
    }
}

impl Shot {
    /// The container `id` as the mirror holds it, every nested container
    /// with it. `None` when the mirror holds no such container.
    pub(super) fn from_mirror(mirror: &Mirror, id: &ContainerID) -> Self {
        let held = |value: &LoroValue| match value {
            LoroValue::Container(child) => Held::Container(Self::from_mirror(mirror, child)),
            value => Held::Value(value.clone()),
        };
        let body = match mirror.containers.get(id) {
            Some(Mirrored::Map(entries)) => {
                let mut entries: Vec<(InternalString, Held)> =
                    entries.iter().map(|(k, v)| (k.clone(), held(v))).collect();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                Some(Body::Map(entries))
            }
            Some(Mirrored::List(items)) => Some(Body::List(items.iter().map(held).collect())),
            Some(Mirrored::Text(rich)) => Some(Body::Text(rich.clone())),
            Some(Mirrored::Tree(_)) | None => None,
        };
        Self {
            id: id.clone(),
            body,
        }
    }

    /// The container `id` as `doc` holds it now, the writes a replay has not
    /// committed yet included.
    pub(super) fn from_doc(doc: &LoroDoc, id: &ContainerID) -> Self {
        let held = |value: &LoroValue| match value {
            LoroValue::Container(child) => Held::Container(Self::from_doc(doc, child)),
            value => Held::Value(value.clone()),
        };
        let items = |value: LoroValue| match value {
            LoroValue::List(items) => Some(Body::List(items.iter().map(held).collect())),
            _ => None,
        };
        let body = match doc.get_container(id.clone()) {
            Some(Container::Map(map)) => match map.get_value() {
                LoroValue::Map(entries) => {
                    let mut entries: Vec<(InternalString, Held)> = entries
                        .iter()
                        .map(|(k, v)| (InternalString::from(k.as_str()), held(v)))
                        .collect();
                    entries.sort_by(|a, b| a.0.cmp(&b.0));
                    Some(Body::Map(entries))
                }
                _ => None,
            },
            Some(Container::List(list)) => items(list.get_value()),
            Some(Container::MovableList(list)) => items(list.get_value()),
            Some(Container::Text(text)) => Some(Body::Text(Rich::from_delta(&text.to_delta()))),
            _ => None,
        };
        Self {
            id: id.clone(),
            body,
        }
    }
}

/// Where a replay puts a value: a key of a map, or a slot of a list.
pub(super) enum Parent<'a> {
    Map(&'a LoroMap, &'a str),
    List(&'a LoroList, Pos<Slot>),
    Movable(&'a LoroMovableList, Pos<Slot>),
}

impl Parent<'_> {
    /// Put a plain value there.
    fn value(&self, value: LoroValue) {
        let _ = match self {
            Self::Map(map, key) => map.insert(key, value),
            Self::List(list, at) => list.insert(at.to_usize(), value),
            Self::Movable(list, at) => list.insert(at.to_usize(), value),
        };
    }

    /// Put a new container there, and return it attached.
    fn child<C: ContainerTrait>(&self, child: C) -> Option<C> {
        match self {
            Self::Map(map, key) => map.insert_container(key, child).ok(),
            Self::List(list, at) => list.insert_container(at.to_usize(), child).ok(),
            Self::Movable(list, at) => list.insert_container(at.to_usize(), child).ok(),
        }
    }
}

/// Put `held` at `parent`. A container is created again by value, each
/// nested one with it, and each new identity is noted against the old one.
/// A container never captured puts nothing.
pub(super) fn attach(parent: &Parent<'_>, held: &Held, renames: &mut Renames) {
    let shot = match held {
        Held::Value(value) => return parent.value(value.clone()),
        Held::Container(shot) => shot,
    };
    match (&shot.body, shot.id.container_type()) {
        (Some(Body::Map(entries)), ContainerType::Map) => {
            if let Some(map) = parent.child(LoroMap::new()) {
                renames.note(shot.id.clone(), map.id());
                for (key, held) in entries {
                    attach(&Parent::Map(&map, key.as_str()), held, renames);
                }
            }
        }
        (Some(Body::List(items)), ContainerType::MovableList) => {
            if let Some(list) = parent.child(LoroMovableList::new()) {
                renames.note(shot.id.clone(), list.id());
                for (at, held) in items.iter().enumerate() {
                    attach(&Parent::Movable(&list, Pos::from_usize(at)), held, renames);
                }
            }
        }
        (Some(Body::List(items)), ContainerType::List) => {
            if let Some(list) = parent.child(LoroList::new()) {
                renames.note(shot.id.clone(), list.id());
                for (at, held) in items.iter().enumerate() {
                    attach(&Parent::List(&list, Pos::from_usize(at)), held, renames);
                }
            }
        }
        (Some(Body::Text(rich)), ContainerType::Text) => {
            if let Some(text) = parent.child(LoroText::new()) {
                renames.note(shot.id.clone(), text.id());
                rich.write_into(&text, Pos::ZERO);
            }
        }
        _ => {}
    }
}

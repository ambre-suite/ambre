//! Pure, `self`-free helpers for [`ToileState`](super::ToileState): geometry
//! (rotation-expanded AABBs), CRDT field (de)serialization, clipboard scalar
//! capture, and the small `LoroMap` / deep-value field readers. Split out of
//! `state/mod.rs` so the engine's mutation logic stays readable. Everything here
//! is stateless and shared by both the mutation and derivation impls via `use
//! super::codec::*`.

use ambre_core::crdt::FieldKey;
use kurbo::{Affine, BezPath, Point, Rect, Vec2};
use loro::{Container, LoroMap, LoroText, LoroValue, ValueOrContainer};

use crate::logic::command::{ClipValue, InitContent, MediaRef};
use crate::logic::connector::Connector;
use crate::logic::model::{
    Anchor, ConnectorId, ElementId, ElementKind, ElementStyle, EndCaps, PathType, ShapeKind,
    StrokePoint,
};

use super::names::{ConnectorKey, ElementKey, FieldValues, Fields};

/// Rotation-expanded axis-aligned bounds of `r` rotated by `rot` about its
/// center. For `rot == 0` this is the rect itself.
pub(super) fn expanded_aabb(r: Rect, rot: ambre_core::Radians) -> Rect {
    if rot == ambre_core::Radians::ZERO {
        return r;
    }
    let c = r.center();
    let a = Affine::rotate_about(rot.get(), c);
    let corners = [
        Point::new(r.x0, r.y0),
        Point::new(r.x1, r.y0),
        Point::new(r.x1, r.y1),
        Point::new(r.x0, r.y1),
    ];
    let mut min = Point::new(f64::INFINITY, f64::INFINITY);
    let mut max = Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    for corner in corners {
        let p = a * corner;
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        max.x = max.x.max(p.x);
        max.y = max.y.max(p.y);
    }
    Rect::new(min.x, min.y, max.x, max.y)
}

/// World AABB spanning a stroke's raw samples (with a small margin so a single
/// point still has non-zero area for culling).
pub(super) fn stroke_aabb(points: &[StrokePoint]) -> Rect {
    if points.is_empty() {
        return Rect::new(0.0, 0.0, 1.0, 1.0);
    }
    let mut min = Point::new(f64::INFINITY, f64::INFINITY);
    let mut max = Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in points {
        min.x = min.x.min(p.x as f64);
        min.y = min.y.min(p.y as f64);
        max.x = max.x.max(p.x as f64);
        max.y = max.y.max(p.y as f64);
    }
    Rect::new(
        min.x,
        min.y,
        (max.x).max(min.x + 1.0),
        (max.y).max(min.y + 1.0),
    )
}

/// Seed a group element's scalar fields. A group paints nothing and its geometry
/// is a derived union, so it carries no x/y/w/h. Only the type flag, lock state,
/// rotation and default style live in the CRDT; the `children` list is attached
/// by the caller.
pub(super) fn em_seed_group(em: &Fields<ElementKey>) {
    let _ = em.insert(ElementKey::Type, ElementKind::Group.type_tag());
    let _ = em.insert(ElementKey::Rot, 0.0);
    let _ = em.insert(ElementKey::Locked, false);
    let _ = em.insert(
        ElementKey::Style,
        LoroValue::Binary(ElementStyle::default().to_bytes().into()),
    );
}

/// Capture the scalar (non-container) fields of a map deep-value into an id-free
/// clipboard field list. Container fields (a group's `children` list)
/// are naturally skipped -- they arrive as `LoroValue::List` / `Map`, which do
/// not match a leaf kind. `skip` drops named fields the caller remaps itself
/// (connector endpoints).
pub(super) fn capture_scalars<K: FieldKey>(
    fields: FieldValues<'_, K>,
    skip: &[K],
) -> Vec<(String, ClipValue)> {
    let mut out = Vec::new();
    for (k, v) in fields.map().iter() {
        if skip.iter().any(|key| key.as_str() == k.as_str()) {
            continue;
        }
        let cv = match v {
            LoroValue::Double(d) => ClipValue::F64(*d),
            LoroValue::Bool(b) => ClipValue::Bool(*b),
            LoroValue::String(s) => ClipValue::Str(s.to_string()),
            LoroValue::Binary(b) => ClipValue::Bin(b.to_vec()),
            _ => continue,
        };
        out.push((k.to_string(), cv));
    }
    out
}

/// Write captured clipboard fields into a fresh element / connector map.
/// `offset` is added to the `x` / `y` coordinates so the paste lands at the
/// cursor.
pub(super) fn write_clip_fields(m: &LoroMap, fields: &[(String, ClipValue)], offset: Vec2) {
    for (k, v) in fields {
        match v {
            ClipValue::F64(d) => {
                let d = match ElementKey::parse(k) {
                    Some(ElementKey::X) => d + offset.x,
                    Some(ElementKey::Y) => d + offset.y,
                    _ => *d,
                };
                let _ = m.insert(k.as_str(), d);
            }
            ClipValue::Bool(b) => {
                let _ = m.insert(k.as_str(), *b);
            }
            // A body and a caption are text containers on the board, which
            // the capture read as their string. Written back as a string they
            // would race whole (`TOILE-030`, `TOILE-047`).
            ClipValue::Str(s)
                if k == ElementKey::Body.as_str() || k == ConnectorKey::Label.as_str() =>
            {
                if let Ok(text) = m.insert_container(k.as_str(), LoroText::new())
                    && !s.is_empty()
                {
                    let _ = text.insert(0, s);
                }
            }
            ClipValue::Str(s) => {
                let _ = m.insert(k.as_str(), s.as_str());
            }
            ClipValue::Bin(b) => {
                let _ = m.insert(k.as_str(), LoroValue::Binary(b.clone().into()));
            }
        }
    }
}

pub(super) fn content_matches(kind: ElementKind, content: &InitContent) -> bool {
    matches!(
        (kind, content),
        (ElementKind::Shape(ShapeKind::Path), InitContent::Path(_))
            | (ElementKind::Shape(_), InitContent::None)
            | (ElementKind::Image, InitContent::Image(MediaRef::Stored(_)))
            | (ElementKind::Embed, InitContent::Embed(_))
            | (ElementKind::DocEmbed, InitContent::DocEmbed { .. })
            | (
                ElementKind::Sticky | ElementKind::Text,
                InitContent::None | InitContent::Text(_)
            )
            | (ElementKind::Frame, InitContent::None)
    )
}

/// The kind an element map's type tag names. An unknown tag reads as a
/// rectangle, and so does a shape with no readable shape tag.
pub(super) fn kind_of(em: &Fields<ElementKey>) -> ElementKind {
    match str_of(em, ElementKey::Type).as_deref() {
        Some("shape") => ElementKind::Shape(shape_of(em)),
        Some("sticky") => ElementKind::Sticky,
        Some("text") => ElementKind::Text,
        Some("image") => ElementKind::Image,
        Some("frame") => ElementKind::Frame,
        Some("group") => ElementKind::Group,
        Some("drawing") => ElementKind::Drawing,
        Some("embed") => ElementKind::Embed,
        Some("doc_embed") => ElementKind::DocEmbed,
        _ => ElementKind::Shape(ShapeKind::Rect),
    }
}

/// The shape a shape element's tag names: a rectangle when it carries no
/// tag this build reads.
pub(super) fn shape_of(em: &Fields<ElementKey>) -> ShapeKind {
    str_of(em, ElementKey::Shape)
        .as_deref()
        .and_then(ShapeKind::from_tag)
        .unwrap_or(ShapeKind::Rect)
}

/// The outline a path element stores, `None` when it holds none this build
/// reads.
pub(super) fn path_of(em: &Fields<ElementKey>) -> Option<BezPath> {
    BezPath::from_svg(&str_of(em, ElementKey::Path)?).ok()
}

/// The unrotated box an element map stores, its corner at `x, y`. A
/// coordinate or a size the map lacks reads as zero, so an element written
/// in part still draws at the origin.
pub(super) fn stored_rect(em: &Fields<ElementKey>) -> Rect {
    let read = |key| f64_of(em, key).unwrap_or(0.0);
    let (x, y) = (read(ElementKey::X), read(ElementKey::Y));
    Rect::new(x, y, x + read(ElementKey::W), y + read(ElementKey::H))
}

/// The rotation an element map stores, in radians. A map that holds none is
/// not rotated.
pub(super) fn rotation_of(em: &Fields<ElementKey>) -> ambre_core::Radians {
    ambre_core::Radians::new(f64_of(em, ElementKey::Rot).unwrap_or(0.0))
}

/// Whether an element map's type tag is the tag of `kind`. Every shape kind
/// carries one tag.
pub(super) fn has_type(em: &Fields<ElementKey>, kind: ElementKind) -> bool {
    str_of(em, ElementKey::Type).as_deref() == Some(kind.type_tag())
}

/// Whether an element read whole carries the type tag of `kind`.
pub(super) fn value_has_type(fields: FieldValues<'_, ElementKey>, kind: ElementKind) -> bool {
    matches!(fields.get(ElementKey::Type), Some(LoroValue::String(t)) if t.as_str() == kind.type_tag())
}

pub(super) fn doc_type_from_tag(s: &str) -> Option<ambre_core::DocType> {
    use ambre_core::DocType;
    match s {
        "Trame" => Some(DocType::Trame),
        "Prose" => Some(DocType::Prose),
        "Toile" => Some(DocType::Toile),
        "Murmure" => Some(DocType::Murmure),
        _ => None,
    }
}

// -- connector map (de)serialization ------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(super) fn write_connector(
    cm: &Fields<ConnectorKey>,
    from: &ElementId,
    from_anch: Anchor,
    to: &ElementId,
    to_anch: Anchor,
    path_type: PathType,
    arrows: EndCaps,
) {
    let _ = cm.insert(ConnectorKey::From, from.as_str());
    let _ = cm.insert(ConnectorKey::FromAnch, encode_anchor(from_anch));
    let _ = cm.insert(ConnectorKey::To, to.as_str());
    let _ = cm.insert(ConnectorKey::ToAnch, encode_anchor(to_anch));
    let _ = cm.insert(ConnectorKey::PathType, encode_path_type(path_type));
    let _ = cm.insert(ConnectorKey::ArrowStart, arrows.start);
    let _ = cm.insert(ConnectorKey::ArrowEnd, arrows.end);
    // The style register is seeded like an element's: an LWW binary the
    // deriver reads back. So an arrow carries its own colour and width rather
    // than borrowing a default (`TOILE-046`).
    let _ = cm.insert(
        ConnectorKey::Style,
        LoroValue::Binary(ElementStyle::default().to_bytes().into()),
    );
    // The caption is seeded at birth, not on first edit. Seeding here gives
    // every peer the same `LoroText` container before anyone types, since it
    // travels in the creation delta. Two concurrent captions then merge inside
    // one text. A lazy seed on each peer would mint rival containers under the
    // same key, and LWW would drop one side's caption whole (`TOILE-047`).
    let _ = cm.insert_container(ConnectorKey::Label, LoroText::new());
}

/// The caption text of a connector map, empty when the connector predates the
/// field or carries none.
pub(super) fn connector_label(cm: &Fields<ConnectorKey>) -> String {
    cm.get(ConnectorKey::Label)
        .and_then(|v| v.into_container().ok())
        .and_then(|c| c.into_text().ok())
        .map(|t| t.to_string())
        .unwrap_or_default()
}

/// The caption `LoroText` of a connector map, seeding one for a connector
/// minted before the field existed. The seed is the LWW-collision window
/// `write_connector` avoids for every connector it makes.
pub(super) fn connector_label_or_seed(cm: &Fields<ConnectorKey>) -> Option<LoroText> {
    if let Some(text) = cm
        .get(ConnectorKey::Label)
        .and_then(|v| v.into_container().ok())
        .and_then(|c| c.into_text().ok())
    {
        return Some(text);
    }
    cm.insert_container(ConnectorKey::Label, LoroText::new())
        .ok()
}

/// One connector, read from its map. An endpoint the map lacks names no
/// element, so the arrow reads as dangling (`TOILE-165`). A missing anchor or
/// path type reads as the default one.
pub(super) fn read_connector(id: ConnectorId, cm: &Fields<ConnectorKey>) -> Option<Connector> {
    let text = |key| str_of(cm, key).unwrap_or_default();
    Some(Connector {
        id,
        from: ElementId(text(ConnectorKey::From)),
        from_anch: decode_anchor(&text(ConnectorKey::FromAnch)),
        to: ElementId(text(ConnectorKey::To)),
        to_anch: decode_anchor(&text(ConnectorKey::ToAnch)),
        path_type: decode_path_type(&text(ConnectorKey::PathType)),
        arrows: EndCaps {
            start: bool_of(cm, ConnectorKey::ArrowStart),
            end: bool_of(cm, ConnectorKey::ArrowEnd),
        },
    })
}

/// Anchors are encoded as compact strings; `Free` carries its `(u, v)`.
pub(super) fn encode_anchor(a: Anchor) -> String {
    match a {
        Anchor::Top => "t".into(),
        Anchor::Bottom => "b".into(),
        Anchor::Left => "l".into(),
        Anchor::Right => "r".into(),
        Anchor::Center => "c".into(),
        Anchor::Free { u, v } => format!("f:{u}:{v}"),
    }
}

pub(super) fn decode_anchor(s: &str) -> Anchor {
    match s {
        "t" => Anchor::Top,
        "b" => Anchor::Bottom,
        "l" => Anchor::Left,
        "r" => Anchor::Right,
        "c" => Anchor::Center,
        _ => {
            let mut it = s.split(':');
            if it.next() == Some("f") {
                let u = it.next().and_then(|x| x.parse().ok()).unwrap_or(0.5);
                let v = it.next().and_then(|x| x.parse().ok()).unwrap_or(0.5);
                Anchor::Free { u, v }
            } else {
                Anchor::Center
            }
        }
    }
}

pub(super) fn encode_path_type(p: PathType) -> &'static str {
    match p {
        PathType::Straight => "s",
        PathType::Elbow => "e",
        PathType::Curved => "c",
    }
}

pub(super) fn decode_path_type(s: &str) -> PathType {
    match s {
        "e" => PathType::Elbow,
        "c" => PathType::Curved,
        _ => PathType::Straight,
    }
}

// -- LoroMap handle field readers ---------------------------------------------

/// A number field, `None` when the map lacks it or holds another kind there.
pub(super) fn f64_of<K: FieldKey>(m: &Fields<K>, k: K) -> Option<f64> {
    match m.value(k) {
        Some(LoroValue::Double(d)) => Some(d),
        _ => None,
    }
}

pub(super) fn bool_of<K: FieldKey>(m: &Fields<K>, k: K) -> bool {
    matches!(m.value(k), Some(LoroValue::Bool(true)))
}

/// The body of a sticky or text element, read from its `LoroText`
/// (`TOILE-030`). A flat string reads as itself: a board before version 4
/// carries one, and so does a write from a client on an older build.
pub(super) fn body_of(m: &Fields<ElementKey>) -> String {
    match m.get(ElementKey::Body) {
        Some(ValueOrContainer::Container(Container::Text(t))) => t.to_string(),
        Some(ValueOrContainer::Value(LoroValue::String(s))) => s.to_string(),
        _ => String::new(),
    }
}

/// The body `LoroText` of an element map, seeding one in place of a missing
/// field or a flat string. The caller writes the whole body next, so the
/// string a seed replaces is not lost.
pub(super) fn body_text_or_seed(m: &Fields<ElementKey>) -> Option<LoroText> {
    if let Some(ValueOrContainer::Container(Container::Text(t))) = m.get(ElementKey::Body) {
        return Some(t);
    }
    m.insert_container(ElementKey::Body, LoroText::new()).ok()
}

/// A string field, `None` when the map lacks it or holds another kind there.
pub(super) fn str_of<K: FieldKey>(m: &Fields<K>, k: K) -> Option<String> {
    match m.value(k) {
        Some(LoroValue::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

/// A binary field, `None` when the map lacks it or holds another kind there.
pub(super) fn bin_of<K: FieldKey>(m: &Fields<K>, k: K) -> Option<Vec<u8>> {
    match m.value(k) {
        Some(LoroValue::Binary(b)) => Some(b.to_vec()),
        _ => None,
    }
}

// -- deep-value (LoroMapValue) field readers ----------------------------------
//
// Used on the from-scratch rebuild path (`rebuild_caches`), which reads whole
// container deep-values rather than per-field handles.

/// A number field of a map read whole, `None` when it lacks one there.
pub(super) fn map_f64<K: FieldKey>(m: FieldValues<'_, K>, k: K) -> Option<f64> {
    match m.get(k) {
        Some(LoroValue::Double(d)) => Some(*d),
        _ => None,
    }
}

/// A string field of a map read whole, `None` when it lacks one there.
pub(super) fn map_str<K: FieldKey>(m: FieldValues<'_, K>, k: K) -> Option<String> {
    match m.get(k) {
        Some(LoroValue::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

/// [`stored_rect`] and [`rotation_of`], for an element map read whole.
pub(super) fn map_placement(fields: FieldValues<'_, ElementKey>) -> (Rect, ambre_core::Radians) {
    let read = |key| map_f64(fields, key).unwrap_or(0.0);
    let (x, y) = (read(ElementKey::X), read(ElementKey::Y));
    let rect = Rect::new(x, y, x + read(ElementKey::W), y + read(ElementKey::H));
    (rect, ambre_core::Radians::new(read(ElementKey::Rot)))
}

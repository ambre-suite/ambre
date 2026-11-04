//! Pure, `self`-free helpers for [`ToileState`](super::ToileState): geometry
//! (rotation-expanded AABBs), CRDT field (de)serialization, clipboard scalar
//! capture, and the small `LoroMap` / deep-value field readers. Split out of
//! `state/mod.rs` so the engine's mutation logic stays readable. Everything here
//! is stateless and shared by both the mutation and derivation impls via `use
//! super::codec::*`.

use ambre_core::Bounded;
use ambre_core::crdt::FieldKey;
use ambre_core::{Board, Payload, Px};
use kurbo::{Affine, BezPath, Point, Rect, Size, Vec2};
use loro::{Container, LoroText, LoroValue, ValueOrContainer};

use crate::logic::command::{ClipBytes, ClipValue, InitContent, MediaRef};
use crate::logic::connector::Connector;
use crate::logic::interop::Limits;
use ambre_core::id::{self, Element, Id};

use crate::logic::model::{
    Anchor, ElementKind, ElementStyle, EndCaps, PathType, ShapeKind, StrokePoint, UnitFraction,
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
        min.x = min.x.min(p.x.widened().get());
        min.y = min.y.min(p.y.widened().get());
        max.x = max.x.max(p.x.widened().get());
        max.y = max.y.max(p.y.widened().get());
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
/// clipboard field list, one entry per key of `keys` the map holds. Container
/// fields (a group's `children` list) are naturally skipped -- they arrive as
/// `LoroValue::List` / `Map`, which do not match a leaf kind. `skip` drops named
/// fields the caller remaps itself (connector endpoints). A key outside the
/// schema is never captured, so a paste writes only keys the schema names.
pub(super) fn capture_scalars<K: FieldKey + PartialEq>(
    fields: FieldValues<'_, K>,
    keys: &[K],
    skip: &[K],
) -> Vec<(K, ClipValue)> {
    let mut out = Vec::new();
    for &key in keys.iter().filter(|key| !skip.contains(key)) {
        let cv = match fields.get(key) {
            Some(LoroValue::Double(d)) => ClipValue::F64(*d),
            Some(LoroValue::Bool(b)) => ClipValue::Bool(*b),
            Some(LoroValue::String(s)) => ClipValue::Str(s.to_string()),
            Some(LoroValue::Binary(b)) => ClipValue::Bin(Payload::<ClipBytes>::new(b.to_vec())),
            _ => continue,
        };
        out.push((key, cv));
    }
    out
}

/// Write captured clipboard fields into a fresh element / connector map.
/// `offset` is added to the `x` / `y` coordinates so the paste lands at the
/// cursor.
pub(super) fn write_clip_fields<'a, K: FieldKey>(
    m: &Fields<K>,
    fields: impl IntoIterator<Item = (K, &'a ClipValue)>,
    offset: Vec2,
) {
    for (k, v) in fields {
        match v {
            ClipValue::F64(d) => {
                // A pasted box is held inside the world bound, as every
                // other write of one is (`TOILE-274`).
                let bounded = |v| on_board(Px::<Board>::new(v)).get();
                let d = match ElementKey::parse(k.as_str()) {
                    Some(ElementKey::X) => bounded(d + offset.x),
                    Some(ElementKey::Y) => bounded(d + offset.y),
                    Some(ElementKey::W | ElementKey::H) => bounded(*d),
                    _ => *d,
                };
                let _ = m.insert(k, d);
            }
            ClipValue::Bool(b) => {
                let _ = m.insert(k, *b);
            }
            // A body and a caption are text containers on the board, which
            // the capture read as their string. Written back as a string they
            // would race whole (`TOILE-030`, `TOILE-047`).
            ClipValue::Str(s)
                if k.as_str() == ElementKey::Body.as_str()
                    || k.as_str() == ConnectorKey::Label.as_str() =>
            {
                if let Ok(text) = m.insert_container(k, LoroText::new())
                    && !s.is_empty()
                {
                    let _ = text.insert(0, s);
                }
            }
            ClipValue::Str(s) => {
                let _ = m.insert(k, s.as_str());
            }
            ClipValue::Bin(b) => {
                let _ = m.insert(k, LoroValue::Binary(b.as_bytes().to_vec().into()));
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
    let read = |key| on_board(Px::<Board>::new(f64_of(em, key).unwrap_or(0.0))).get();
    let (x, y) = (read(ElementKey::X), read(ElementKey::Y));
    Rect::new(x, y, x + read(ElementKey::W), y + read(ElementKey::H))
}

/// The rotation an element map stores, in radians. A map that holds none is
/// not rotated.
pub(super) fn rotation_of(em: &Fields<ElementKey>) -> ambre_core::Radians {
    turn(ambre_core::Radians::new(
        f64_of(em, ElementKey::Rot).unwrap_or(0.0),
    ))
}

/// Write the corner of an element's box. Each coordinate is held inside the
/// bound the readers apply, so a value read back is the value written
/// (`TOILE-274`).
pub(super) fn store_corner(em: &Fields<ElementKey>, at: Point) {
    let _ = em.insert(ElementKey::X, on_board(Px::<Board>::new(at.x)).get());
    let _ = em.insert(ElementKey::Y, on_board(Px::<Board>::new(at.y)).get());
}

/// Write the size of an element's box, each side held inside the same bound
/// as [`store_corner`].
pub(super) fn store_size(em: &Fields<ElementKey>, size: Size) {
    let _ = em.insert(ElementKey::W, on_board(Px::<Board>::new(size.width)).get());
    let _ = em.insert(ElementKey::H, on_board(Px::<Board>::new(size.height)).get());
}

/// One coordinate or size of a stored box, as the geometry reads it. A peer
/// can store any number there. A value that is not finite reads as zero, and
/// a finite one is held inside the import world bound (`TOILE-274`). The
/// spatial index and the group bounds then never see a NaN.
fn on_board(v: Px<Board>) -> Px<Board> {
    Limits::default().clamp_coord(v)
}

/// A stored rotation, as the geometry reads it: one that is not finite reads
/// as no rotation (`TOILE-274`).
fn turn(rot: ambre_core::Radians) -> ambre_core::Radians {
    if rot.is_finite() {
        rot
    } else {
        ambre_core::Radians::ZERO
    }
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

pub(super) fn write_connector(
    cm: &Fields<ConnectorKey>,
    from: &Id<Element>,
    from_anch: Anchor,
    to: &Id<Element>,
    to_anch: Anchor,
    path_type: PathType,
    arrows: EndCaps,
) {
    let _ = cm.insert(ConnectorKey::From, from.as_str());
    let _ = cm.insert(ConnectorKey::FromAnch, encode_anchor(from_anch));
    let _ = cm.insert(ConnectorKey::To, to.as_str());
    let _ = cm.insert(ConnectorKey::ToAnch, encode_anchor(to_anch));
    let _ = cm.insert(ConnectorKey::PathType, encode_path_type(path_type));
    let _ = cm.insert(ConnectorKey::ArrowStart, arrows.start.as_bool());
    let _ = cm.insert(ConnectorKey::ArrowEnd, arrows.end.as_bool());
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

/// The caption text of a connector map, `None` when the connector predates
/// the field.
pub(super) fn connector_label(cm: &Fields<ConnectorKey>) -> Option<String> {
    cm.get(ConnectorKey::Label)
        .and_then(|v| v.into_container().ok())
        .and_then(|c| c.into_text().ok())
        .map(|t| t.to_string())
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
pub(super) fn read_connector(
    id: Id<id::Connector>,
    cm: &Fields<ConnectorKey>,
) -> Option<Connector> {
    let text = |key| str_of(cm, key).unwrap_or_default();
    Some(Connector {
        id,
        from: Id::<Element>::from_text(text(ConnectorKey::From)),
        from_anch: decode_anchor(&text(ConnectorKey::FromAnch)),
        to: Id::<Element>::from_text(text(ConnectorKey::To)),
        to_anch: decode_anchor(&text(ConnectorKey::ToAnch)),
        path_type: decode_path_type(&text(ConnectorKey::PathType)),
        // A connector that stores no cap draws none at that end.
        arrows: EndCaps {
            start: bool_of(cm, ConnectorKey::ArrowStart).map_or(
                ambre_bus::app::toile::Cap::Plain,
                ambre_bus::app::toile::Cap::from_bool,
            ),
            end: bool_of(cm, ConnectorKey::ArrowEnd).map_or(
                ambre_bus::app::toile::Cap::Plain,
                ambre_bus::app::toile::Cap::from_bool,
            ),
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
                // A fraction that does not parse, or falls off the side,
                // reads as the middle.
                let mut fraction = || {
                    it.next()
                        .and_then(|x| x.parse().ok())
                        .and_then(Bounded::<UnitFraction>::new)
                        .unwrap_or(Bounded::<UnitFraction>::HALF)
                };
                let u = fraction();
                let v = fraction();
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

/// A flag field, `None` when the map lacks it or holds another kind there.
pub(super) fn bool_of<K: FieldKey>(m: &Fields<K>, k: K) -> Option<bool> {
    match m.value(k) {
        Some(LoroValue::Bool(b)) => Some(b),
        _ => None,
    }
}

/// Whether an element map is locked. A map that stores no lock is unlocked:
/// an element is created so.
pub(super) fn locked(em: &Fields<ElementKey>) -> bool {
    bool_of(em, ElementKey::Locked).unwrap_or(false)
}

/// The body of a sticky or text element, read from its `LoroText`
/// (`TOILE-030`). `None` when the map holds no body.
pub(super) fn body_of(m: &Fields<ElementKey>) -> Option<String> {
    match m.get(ElementKey::Body) {
        Some(ValueOrContainer::Container(Container::Text(t))) => Some(t.to_string()),
        _ => None,
    }
}

/// The body `LoroText` of an element map, seeding one in place of a missing
/// field.
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
    let read = |key| on_board(Px::<Board>::new(map_f64(fields, key).unwrap_or(0.0))).get();
    let (x, y) = (read(ElementKey::X), read(ElementKey::Y));
    let rect = Rect::new(x, y, x + read(ElementKey::W), y + read(ElementKey::H));
    let rot = ambre_core::Radians::new(map_f64(fields, ElementKey::Rot).unwrap_or(0.0));
    (rect, turn(rot))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TOILE-282`: a free anchor reads back as it was written, and a fraction off the
    /// side reads as the middle rather than outside the box.
    #[test]
    fn a_free_anchor_reads_back_and_a_fraction_off_the_side_reads_as_the_middle() {
        let quarter = Bounded::<UnitFraction>::new(0.25).expect("a fraction of the side");
        let free = Anchor::Free {
            u: quarter,
            v: Bounded::<UnitFraction>::HALF,
        };
        assert_eq!(encode_anchor(free), "f:0.25:0.5");
        assert_eq!(decode_anchor("f:0.25:0.5"), free);
        let off = Anchor::Free {
            u: Bounded::<UnitFraction>::HALF,
            v: Bounded::<UnitFraction>::HALF,
        };
        assert_eq!(decode_anchor("f:1.5:NaN"), off);
    }
}

//! The schema reading belt: what each stored value means, frozen per
//! `SCHEMA_VERSION` (`SCHEMA-017`).
//!
//! The shape belt freezes the keys and the types. A position stored in world
//! units and read in screen pixels keeps both, so the shape stays green while
//! every older board is drawn wrong. This belt freezes the other half: one line
//! per stored value, carrying the value as stored and what this build reads it
//! as.
//!
//! **Two columns, two drifts.** A line reads `path = stored => reading`. A
//! second board that opened the first one's snapshot takes the reading,
//! through the readers the view uses. A reader that moves its
//! unit moves the reading. A writer and a reader that move together leave the
//! reading alone and move the stored value. Either way the frozen text turns
//! red under an unchanged `SCHEMA_VERSION`.
//!
//! **Absolute units.** A reading names its unit: a length in world px, an
//! angle in degrees, a duration in ms, a colour as `#aarrggbb`. A position in
//! a list is one-based.
//!
//! **What no reader reads.** A few registers are written and read by nothing
//! in this build. [`UNREAD`] names them, and their line says so. A reader
//! landing for one of them moves its line, which is the review it is owed.
//!
//! **What holds the freeze honest.** A stored value with no reading fails
//! [`every_stored_value_has_a_reading`], and a declared key the scenario does
//! not reach fails [`every_declared_key_has_a_reading`]. The scenario is the
//! shape belt's, so the two belts read the same board. Its values are frozen
//! here, so a command added to it lands with a raised version, like a new key.

use ambre_core::Argb;
use std::collections::BTreeSet;

use ambre_core::schema::{SCHEMA_META_MAP, SCHEMA_VERSION_KEY, verdict};
use kurbo::ParamCurve;
use loro::LoroValue;

use super::schema_shape::{
    ANY, KEY_SOURCES, board_of_every_key, declared_keys, keys_are_schema_words, section_of,
};
use super::{
    BOARD, COMMENTS, CONNECTORS, ELEMENTS, LAYERS, MEDIA, PRESENTATION_CONFIG, PRESENTATION_ORDER,
    REACTIONS, ToileState,
};
use crate::logic::autolayout::AutoLayout;
use crate::logic::command::{CommentAnchor, DisplayGeom};
use crate::logic::model::{
    ConnectorId, ElementId, ElementStyle, FontSpec, LinkTarget, NodeId, Shadow,
};
use crate::logic::schema::SCHEMA_VERSION;

/// The frozen readings, one section per `SCHEMA_VERSION`.
const FROZEN: &str = include_str!("../../../schema/readings.txt");

/// The stored paths no reader of this build reads. Each is written, kept and
/// merged, and nothing draws or plays it yet.
const UNREAD: [&str; 7] = [
    "board/bg_color",
    "board/grid_snap",
    "presentation_config/autoplay",
    "presentation_config/loop",
    "media/*/h",
    "media/*/mime",
    "media/*/w",
];

/// One segment below a root: a map key, or a position in a list.
#[derive(Clone)]
enum Segment {
    Key(String),
    At(usize),
}

/// One stored value: the root it sits under, the segments down to it, and the
/// value.
struct Stored {
    root: String,
    segments: Vec<Segment>,
    value: LoroValue,
}

impl Stored {
    /// The map key at `depth`, if the segment there is one.
    fn key(&self, depth: usize) -> Option<&str> {
        match self.segments.get(depth)? {
            Segment::Key(key) => Some(key),
            Segment::At(_) => None,
        }
    }

    /// The list position at `depth`, if the segment there is one.
    fn at(&self, depth: usize) -> Option<usize> {
        match self.segments.get(depth)? {
            Segment::At(at) => Some(*at),
            Segment::Key(_) => None,
        }
    }

    /// The stored string, for a register that holds an id.
    fn string(&self) -> Option<&str> {
        match &self.value {
            LoroValue::String(text) => Some(text),
            _ => None,
        }
    }
}

/// Every stored value of a board, maps walked to their leaves and lists read
/// item by item.
fn stored_values(value: &LoroValue) -> Vec<Stored> {
    let mut found = Vec::new();
    let LoroValue::Map(roots) = value else {
        return found;
    };
    for (root, value) in roots.iter() {
        walk(root, Vec::new(), value, &mut found);
    }
    found
}

fn walk(root: &str, segments: Vec<Segment>, value: &LoroValue, found: &mut Vec<Stored>) {
    let below = |segment| {
        let mut path = segments.clone();
        path.push(segment);
        path
    };
    match value {
        LoroValue::Map(entries) => {
            for (key, child) in entries.iter() {
                walk(root, below(Segment::Key(key.clone())), child, found);
            }
        }
        LoroValue::List(items) => {
            for (at, item) in items.iter().enumerate() {
                walk(root, below(Segment::At(at)), item, found);
            }
        }
        _ => found.push(Stored {
            root: root.to_owned(),
            segments,
            value: value.clone(),
        }),
    }
}

/// A stored value in the one spelling the freeze uses.
fn stored_text(value: &LoroValue) -> String {
    match value {
        LoroValue::String(text) => format!("{:?}", text.as_str()),
        LoroValue::Binary(bytes) => bytes.iter().map(|b| format!("{b:02x}")).collect(),
        LoroValue::Double(n) => n.to_string(),
        LoroValue::I64(n) => n.to_string(),
        LoroValue::Bool(b) => b.to_string(),
        other => format!("{other:?}"),
    }
}

/// The path a value is frozen under, as in the shape. An id key and a list
/// position collapse to [`ANY`]; the reading names the entry.
fn frozen_path(stored: &Stored) -> String {
    let mut path = stored.root.clone();
    let mut schema_words = keys_are_schema_words(&stored.root);
    for segment in &stored.segments {
        let word = match segment {
            Segment::Key(key) if schema_words => key.as_str(),
            _ => ANY,
        };
        path.push('/');
        path.push_str(word);
        schema_words = true;
    }
    path
}

/// A packed `0xAARRGGBB` colour.
fn colour(argb: Argb) -> String {
    format!("#{:08x}", argb.bits())
}

/// A colour that may be absent, which the painter reads as none.
fn maybe_colour(argb: Option<Argb>) -> String {
    argb.map_or_else(|| "none".to_owned(), colour)
}

/// What a decoded style sets, field by field, each in its unit.
///
/// The records are destructured whole: a field added to one fails to compile
/// here until its reading is written.
fn style_reading(style: &ElementStyle) -> String {
    let ElementStyle {
        fill,
        stroke,
        stroke_w,
        opacity,
        corner,
        font,
        shadow,
        text_color,
    } = style;
    let FontSpec {
        family,
        size,
        weight,
        italic,
        underline,
        strikethrough,
    } = font;
    let mut parts = vec![
        format!("fill {}", maybe_colour(*fill)),
        format!("stroke {} {stroke_w} px", maybe_colour(*stroke)),
        format!("opacity {opacity}"),
        format!("corner {corner} px"),
        format!("font {:?} {size} pt weight {weight}", family.label()),
    ];
    for (set, name) in [
        (*italic, "italic"),
        (*underline, "underlined"),
        (*strikethrough, "struck"),
    ] {
        if set {
            parts.push(name.to_owned());
        }
    }
    if let Some(Shadow {
        color,
        dx,
        dy,
        blur,
    }) = shadow
    {
        parts.push(format!(
            "shadow {} offset ({dx}, {dy}) px blur {blur} px",
            colour(*color)
        ));
    }
    parts.push(format!("ink {}", maybe_colour(*text_color)));
    parts.join(", ")
}

/// Where a link leads.
fn link_reading(link: &LinkTarget) -> String {
    match link {
        LinkTarget::Url(url) => format!("links to {url:?}"),
        LinkTarget::Frame(frame) => format!("links to frame {}", frame.as_str()),
        LinkTarget::Document {
            document_id,
            doc_type,
        } => format!("links to {doc_type} document {document_id}"),
    }
}

/// A frame's layout spec, field by field.
fn layout_reading(layout: &AutoLayout) -> String {
    let AutoLayout {
        direction,
        gap,
        padding,
        align,
        sizing,
    } = layout;
    format!(
        "flows {direction:?}, gap {gap} px, padding {padding:?} px, aligned {align:?}, \
         sized {sizing:?}"
    )
}

/// One-based place of `item` in `list`.
fn place<T: PartialEq>(list: &[T], item: &T) -> Option<usize> {
    list.iter().position(|at| at == item).map(|at| at + 1)
}

/// What `reader` reads a stored value as, or `None` when no reader of this
/// build resolves it.
fn reading(reader: &ToileState, stored: &Stored) -> Option<String> {
    if UNREAD.contains(&frozen_path(stored).as_str()) {
        return Some("read by no reader of this build".to_owned());
    }
    match stored.root.as_str() {
        SCHEMA_META_MAP if stored.key(0)? == SCHEMA_VERSION_KEY => {
            let LoroValue::I64(n) = stored.value else {
                return None;
            };
            let version = u32::try_from(n).ok()?;
            Some(format!(
                "version {version}, {:?}",
                verdict(version, SCHEMA_VERSION)
            ))
        }
        BOARD if stored.key(0)? == "grid_size" => Some(format!(
            "grid unit {} world px",
            reader.debug_viewport().grid_unit
        )),
        PRESENTATION_CONFIG if stored.key(0)? == "transition_ms" => {
            Some(format!("slides change over {} ms", reader.transition_ms()))
        }
        PRESENTATION_ORDER => {
            let frame = ElementId(stored.string()?.to_owned());
            Some(format!("slide {}", place(&reader.slide_order(), &frame)?))
        }
        LAYERS => {
            let node = NodeId::decode(stored.string()?)?;
            Some(format!(
                "painted {} of {}",
                place(reader.layer_order(), &node)?,
                reader.layer_order().len()
            ))
        }
        ELEMENTS => element_reading(reader, stored),
        CONNECTORS => connector_reading(reader, stored),
        COMMENTS => comment_reading(reader, stored),
        REACTIONS => {
            let (target, _user) = stored.key(0)?.split_once('/')?;
            let tally = reader.reaction_tally(&ElementId(target.to_owned()));
            Some(format!(
                "tallies {:?}, {} votes summing {}",
                tally.emoji, tally.vote_count, tally.vote_total
            ))
        }
        MEDIA if stored.key(1)? == "thumb" => {
            let (w, h, _) = reader.decoded_thumbnail(stored.key(0)?)?;
            Some(format!("thumbnail {w}x{h} px"))
        }
        _ => None,
    }
}

/// The reading of a value under one element, through the element the display
/// paints and the frame readers.
fn element_reading(reader: &ToileState, stored: &Stored) -> Option<String> {
    let id = ElementId(stored.key(0)?.to_owned());
    let field = stored.key(1)?;
    let shown = reader.display_element(&id)?;
    let bounds = shown.bounds;
    match (field, &shown.geom) {
        ("type" | "shape", _) => Some(format!("kind {:?}", shown.kind)),
        ("x", _) => Some(format!("left edge at {} px", bounds.x0)),
        ("y", _) => Some(format!("top edge at {} px", bounds.y0)),
        ("w", _) => Some(format!("{} px wide", bounds.width())),
        ("h", _) => Some(format!("{} px tall", bounds.height())),
        ("rot", _) => {
            let [a, b, ..] = shown.transform.as_coeffs();
            Some(format!("turned {} degrees", b.atan2(a).to_degrees()))
        }
        ("style", _) => Some(style_reading(&shown.style)),
        ("link", _) => Some(link_reading(shown.link.as_ref()?)),
        ("locked", _) => Some(if shown.locked { "locked" } else { "unlocked" }.to_owned()),
        ("text", DisplayGeom::Sticky(body) | DisplayGeom::Text(body)) => {
            Some(format!("shows {body:?}"))
        }
        ("name", DisplayGeom::Frame(name)) => Some(format!("titled {name:?}")),
        ("hash", DisplayGeom::Image(hash)) => Some(format!("draws media {hash:?}")),
        ("url", DisplayGeom::Embed(url)) => Some(format!("embeds {url:?}")),
        (
            "doc_id" | "doc_type",
            DisplayGeom::DocEmbed {
                document_id,
                doc_type,
                ..
            },
        ) => Some(format!("embeds {doc_type} document {document_id}")),
        ("path", DisplayGeom::Path(path)) => Some(format!("outline {}", path.to_svg())),
        ("points", DisplayGeom::Stroke(points)) => {
            let (first, last) = (points.first()?, points.last()?);
            Some(format!(
                "stroke of {} points from ({}, {}) to ({}, {}) px",
                points.len(),
                first.x,
                first.y,
                last.x,
                last.y
            ))
        }
        ("children", _) => {
            let child = ElementId(stored.string()?.to_owned());
            let members = reader.membership_of(&id);
            Some(format!(
                "member {} of {}",
                place(&members, &child)?,
                members.len()
            ))
        }
        ("notes", _) => Some(format!("speaker notes {:?}", reader.notes(&id))),
        ("autolayout", _) => Some(layout_reading(&reader.auto_layout(&id)?)),
        _ => None,
    }
}

/// The reading of a value under one connector, through the arrow the display
/// paints along the route the cull takes.
fn connector_reading(reader: &ToileState, stored: &Stored) -> Option<String> {
    let id = ConnectorId(stored.key(0)?.to_owned());
    let connector = reader.connector_of(&id)?;
    let route = connector.route(&reader.bounds)?;
    let shown = reader.display_connector(&id, route.clone())?;
    let ends = route.segments().map(|s| (s.start(), s.end()));
    let start = ends.clone().next()?.0;
    let end = ends.last()?.1;
    match stored.key(1)? {
        "from" => Some(format!("starts at ({}, {}) px", start.x, start.y)),
        "to" => Some(format!("ends at ({}, {}) px", end.x, end.y)),
        "from_anch" => Some(format!("leaves by {:?}", connector.from_anch)),
        "to_anch" => Some(format!("arrives by {:?}", connector.to_anch)),
        "path_type" => Some(format!(
            "routed {:?} as {}",
            connector.path_type,
            route.to_svg()
        )),
        "arrow_start" => Some(format!("arrowhead at start {}", shown.arrows.start)),
        "arrow_end" => Some(format!("arrowhead at end {}", shown.arrows.end)),
        "style" => Some(style_reading(&shown.style)),
        "label" => Some(format!("captioned {:?}", shown.label)),
        _ => None,
    }
}

/// The reading of a value under one comment thread, through the thread a
/// reader opens.
fn comment_reading(reader: &ToileState, stored: &Stored) -> Option<String> {
    let thread = reader.comment_thread(&ElementId(stored.key(0)?.to_owned()))?;
    match stored.key(1)? {
        "anchor" => Some(match &thread.anchor {
            CommentAnchor::Element(element) => format!("pinned to element {}", element.as_str()),
            CommentAnchor::Point { x, y } => format!("pinned at ({x}, {y}) px"),
        }),
        "resolved" => Some(if thread.resolved { "resolved" } else { "open" }.to_owned()),
        "messages" => {
            let at = stored.at(2)?;
            let message = thread.messages.get(at)?;
            let what = match stored.key(3)? {
                "author" => format!("by {}", message.author),
                "ts" => format!("stamped {}", message.ts),
                "body" => format!("reads {:?}", message.body),
                _ => return None,
            };
            Some(format!("message {} {what}", at + 1))
        }
        _ => None,
    }
}

/// A board opened from `state`'s snapshot, through the full-state read every
/// reload takes.
fn reopened(state: &ToileState) -> ToileState {
    let mut reader = ToileState::with_peer(ambre_core::PeerId(2));
    reader
        .import_snapshot(&state.export_snapshot())
        .expect("a board this build wrote reopens");
    reader
}

/// A board's readings, as one reader reads them.
#[derive(Default)]
struct Readings {
    /// The lines the freeze holds.
    lines: BTreeSet<String>,
    /// The stored values no reading resolved.
    unread: Vec<String>,
    /// The roots and the keys above a resolved value, which the coverage
    /// assertion answers from.
    keys: BTreeSet<String>,
}

/// The readings of a board as `reader` reads it.
fn readings_of(state: &ToileState, reader: &ToileState) -> Readings {
    let mut readings = Readings::default();
    for stored in stored_values(&state.doc.get_deep_value()) {
        let path = frozen_path(&stored);
        let value = stored_text(&stored.value);
        match reading(reader, &stored) {
            Some(read) => {
                readings.lines.insert(format!("{path} = {value} => {read}"));
                readings.keys.insert(stored.root.clone());
                readings
                    .keys
                    .extend(stored.segments.iter().filter_map(|s| match s {
                        Segment::Key(key) => Some(key.clone()),
                        Segment::At(_) => None,
                    }));
            }
            None => readings.unread.push(format!("{path} = {value}")),
        }
    }
    readings
}

fn render(lines: &BTreeSet<String>) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

#[test]
fn the_reading_a_build_makes_is_the_reading_frozen_for_its_version() {
    let state = board_of_every_key();
    let written = render(&readings_of(&state, &reopened(&state)).lines);
    let Some(frozen) = section_of(FROZEN, SCHEMA_VERSION) else {
        panic!(
            "`schema/readings.txt` carries no `== v{SCHEMA_VERSION}` section. \
             The version rose without a freeze; add the section below:\n{written}"
        );
    };
    assert_eq!(
        written, frozen,
        "what this build stores or reads is not what was frozen for \
         SCHEMA_VERSION {SCHEMA_VERSION}. A value moved its unit or its \
         meaning, and every board written before reads wrong. Raise \
         SCHEMA_VERSION and freeze the new reading as its own section \
         (`SCHEMA-017`)."
    );
}

#[test]
fn every_stored_value_has_a_reading() {
    let state = board_of_every_key();
    let unread = readings_of(&state, &reopened(&state)).unread;
    assert!(
        unread.is_empty(),
        "no reader of this build resolves these stored values, so the freeze \
         holds nothing of what they mean. Write their reading in \
         `schema_reading.rs`, or name them in `UNREAD`:\n{}",
        unread.join("\n")
    );
}

#[test]
fn every_declared_key_has_a_reading() {
    let state = board_of_every_key();
    let keys = readings_of(&state, &reopened(&state)).keys;
    for source in KEY_SOURCES {
        for key in declared_keys(source) {
            assert!(
                keys.contains(&key),
                "`{key}` is declared as a schema key and no reading carries it"
            );
        }
    }
}

#[test]
fn a_stored_value_moving_moves_the_reading() {
    let state = board_of_every_key();
    let before = readings_of(&state, &reopened(&state)).lines;
    // The same length written in seconds: the key and the type stay, so only
    // the value column can show it.
    let mut moved = board_of_every_key();
    moved.set_transition_ms_for_test(0.3);
    let after = readings_of(&moved, &reopened(&moved)).lines;
    assert_ne!(render(&before), render(&after));
}

#[test]
fn a_reader_moving_moves_the_reading() {
    let state = board_of_every_key();
    let before = readings_of(&state, &reopened(&state)).lines;
    // A reader that takes the stored lengths as points: the stored bytes are
    // the same, only what this build makes of them moved.
    let mut misread = reopened(&state);
    for bounds in misread.bounds.values_mut() {
        *bounds = kurbo::Rect::new(
            bounds.x0 * 96.0 / 72.0,
            bounds.y0 * 96.0 / 72.0,
            bounds.x1 * 96.0 / 72.0,
            bounds.y1 * 96.0 / 72.0,
        );
    }
    let after = readings_of(&state, &misread).lines;
    assert_ne!(
        render(&before),
        render(&after),
        "a reader moving its unit has to move the reading, or the freeze \
         holds only the bytes"
    );
}

//! The SVG reader of the images module: a file to its drawing.
//!
//! `usvg` parses the file into a normalized tree. Its font stack is off: the
//! browser has no system fonts, and flattening text to glyph paths would lose
//! the editable string. So its tree carries no text node, and each `<text>` is
//! read from the source with `roxmltree`.
//!
//! **One walk over two trees.** The source and the usvg tree are stepped
//! together. A text node then lands where the drawing put it: in document
//! order, inside the group that holds it, under that group's transform. Each
//! usvg child consumes the next source child that is not a `<text>`, and every
//! `<text>` passed on the way lands at that point.

use ambre_core::index::{IdxVec, Pos};
use ambre_core::{FileBytes, Payload, Rgb};
use usvg::tiny_skia_path::PathSegment;

use crate::images::{
    SvgDepth, SvgImage, SvgItem, SvgKind, SvgNode, SvgPaint, SvgPath, SvgPicture, SvgPoint,
    SvgRect, SvgSegment, SvgSource, SvgStroke, SvgText, SvgTransform, SvgTree,
};

/// The deepest group the module walks. Toile refuses a drawing far shallower
/// than this (`Limits::max_depth`), so the bound only keeps a hostile file
/// from growing the module's stack.
const MAX_DEPTH: usize = 256;

/// The drawing of the SVG file `data`.
///
/// # Errors
///
/// The words of usvg when it refuses the file.
pub(super) fn read(data: &Payload<FileBytes>) -> Result<SvgTree, String> {
    let bytes = data.as_bytes();
    let tree =
        usvg::Tree::from_data(bytes, &usvg::Options::default()).map_err(|e| e.to_string())?;
    // A body that is not UTF-8, or not well-formed XML, costs the drawing its
    // text, and the answer says so.
    let doc = std::str::from_utf8(bytes)
        .ok()
        .and_then(|src| roxmltree::Document::parse(src).ok());
    let mut walk = Walk {
        nodes: IdxVec::new(),
        depth: SvgDepth::Within,
    };
    let root = walk.push(None, SvgKind::Group(transform(tree.root().abs_transform())));
    walk.group(
        tree.root(),
        root,
        doc.as_ref().map(roxmltree::Document::root_element),
        0,
    );
    let source = if doc.is_some() {
        SvgSource::Read
    } else {
        SvgSource::Unreadable
    };
    Ok(SvgTree::new(walk.nodes, source, walk.depth))
}

struct Walk {
    nodes: IdxVec<SvgNode, SvgItem>,
    depth: SvgDepth,
}

impl Walk {
    /// Append a node under `parent`, and return its position.
    fn push(&mut self, parent: Option<Pos<SvgNode>>, kind: SvgKind) -> Pos<SvgNode> {
        let at = self.nodes.push(SvgItem {
            children: Vec::new(),
            kind,
        });
        if let Some(parent) = parent.and_then(|p| self.nodes.get_mut(p)) {
            parent.children.push(at);
        }
        at
    }

    /// Walk the children of `group` into the node `at`, the source children
    /// of `xml` beside them. `depth` counts the groups above.
    fn group(
        &mut self,
        group: &usvg::Group,
        at: Pos<SvgNode>,
        xml: Option<roxmltree::Node>,
        depth: usize,
    ) {
        let tf = transform(group.abs_transform());
        // The source children paired against the usvg children, in document
        // order, popped from the back.
        let mut source: Vec<roxmltree::Node> = xml
            .map(|n| n.children().filter(|c| is_drawn(*c)).collect())
            .unwrap_or_default();
        source.reverse();
        for node in group.children() {
            let paired = self.text_before_next(&mut source, at, tf);
            match node {
                usvg::Node::Group(g) => {
                    if depth >= MAX_DEPTH {
                        self.depth = SvgDepth::Exceeded;
                        continue;
                    }
                    let child = self.push(Some(at), SvgKind::Group(transform(g.abs_transform())));
                    self.group(g, child, paired, depth + 1);
                }
                usvg::Node::Path(p) => {
                    self.push(Some(at), SvgKind::Path(path(p)));
                }
                // The font stack is off, so usvg yields no text node.
                usvg::Node::Text(_) => {}
                usvg::Node::Image(img) => {
                    self.push(Some(at), SvgKind::Image(image(img)));
                }
            }
        }
        // The text after the last drawn node of this group.
        while let Some(node) = source.pop() {
            if node.has_tag_name("text") {
                self.push(Some(at), SvgKind::Text(text(node, tf)));
            }
        }
    }

    /// Land every `<text>` up to the next drawn source child, and return that
    /// child.
    fn text_before_next<'a, 'input>(
        &mut self,
        source: &mut Vec<roxmltree::Node<'a, 'input>>,
        at: Pos<SvgNode>,
        tf: SvgTransform,
    ) -> Option<roxmltree::Node<'a, 'input>> {
        while let Some(node) = source.pop() {
            if node.has_tag_name("text") {
                self.push(Some(at), SvgKind::Text(text(node, tf)));
            } else {
                return Some(node);
            }
        }
        None
    }
}

/// Whether a source element carries drawn content, and therefore pairs with a
/// node of the usvg tree. `<defs>` and the metadata elements beside it describe
/// the drawing without being in it.
fn is_drawn(node: roxmltree::Node) -> bool {
    node.is_element()
        && !matches!(
            node.tag_name().name(),
            "defs" | "title" | "desc" | "metadata" | "style"
        )
}

/// A `<text>` of the source: the text of its descendants, its baseline and
/// its size, under the transform of the group around it.
fn text(node: roxmltree::Node, tf: SvgTransform) -> SvgText {
    // The text children alone: the element's own accessor would count twice.
    let body: String = node
        .descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect();
    let number = |name: &str| node.attribute(name).and_then(|v| v.parse::<f64>().ok());
    SvgText {
        body: body.trim().to_owned(),
        x: number("x").unwrap_or(0.0),
        y: number("y").unwrap_or(0.0),
        size: node
            .attribute("font-size")
            .and_then(|v| v.trim_end_matches("px").parse::<f64>().ok())
            .unwrap_or(16.0),
        transform: tf,
    }
}

fn path(p: &usvg::Path) -> SvgPath {
    let point = |p: usvg::tiny_skia_path::Point| SvgPoint { x: p.x, y: p.y };
    SvgPath {
        segments: p
            .data()
            .segments()
            .map(|seg| match seg {
                PathSegment::MoveTo(a) => SvgSegment::MoveTo(point(a)),
                PathSegment::LineTo(a) => SvgSegment::LineTo(point(a)),
                PathSegment::QuadTo(a, b) => SvgSegment::QuadTo(point(a), point(b)),
                PathSegment::CubicTo(a, b, c) => SvgSegment::CubicTo(point(a), point(b), point(c)),
                PathSegment::Close => SvgSegment::Close,
            })
            .collect(),
        transform: transform(p.abs_transform()),
        fill: p.fill().map(|f| paint(f.paint(), f.opacity().get())),
        stroke: p.stroke().map(|s| SvgStroke {
            paint: paint(s.paint(), s.opacity().get()),
            width: s.width().get(),
        }),
    }
}

fn paint(paint: &usvg::Paint, opacity: f32) -> SvgPaint {
    match paint {
        usvg::Paint::Color(c) => SvgPaint::Solid {
            color: Rgb::from_channels(c.red, c.green, c.blue),
            opacity,
        },
        _ => SvgPaint::Other,
    }
}

fn image(img: &usvg::Image) -> SvgImage {
    let picture = match img.kind() {
        usvg::ImageKind::PNG(d) => SvgPicture::Png(Payload::new(d.to_vec())),
        usvg::ImageKind::JPEG(d) => SvgPicture::Jpeg(Payload::new(d.to_vec())),
        usvg::ImageKind::GIF(_) => SvgPicture::Gif,
        usvg::ImageKind::WEBP(_) => SvgPicture::Webp,
        usvg::ImageKind::SVG(_) => SvgPicture::Svg,
    };
    let r = img.abs_bounding_box();
    SvgImage {
        picture,
        bounds: SvgRect {
            left: r.left(),
            top: r.top(),
            right: r.right(),
            bottom: r.bottom(),
        },
    }
}

const fn transform(tf: usvg::Transform) -> SvgTransform {
    SvgTransform {
        sx: tf.sx,
        kx: tf.kx,
        tx: tf.tx,
        ky: tf.ky,
        sy: tf.sy,
        ty: tf.ty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_str(svg: &str) -> SvgTree {
        read(&Payload::new(svg.as_bytes().to_vec())).expect("the drawing reads")
    }

    #[test]
    fn text_lands_between_the_shapes_the_source_puts_it_between() {
        let tree = read_str(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <rect width="1" height="1"/><text x="2" y="3">hi</text><rect width="2" height="2"/>
            </svg>"#,
        );
        let root = tree.get(SvgTree::ROOT).expect("a root group");
        let kinds: Vec<&str> = root
            .children
            .iter()
            .filter_map(|c| tree.get(*c))
            .map(|n| match &n.kind {
                SvgKind::Group(_) => "group",
                SvgKind::Path(_) => "path",
                SvgKind::Image(_) => "image",
                SvgKind::Text(_) => "text",
            })
            .collect();
        assert_eq!(kinds, ["path", "text", "path"]);
        assert_eq!(tree.source, SvgSource::Read);
    }

    #[test]
    fn a_text_reads_its_baseline_its_size_and_its_trimmed_body() {
        let tree = read_str(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="4" y="5" font-size="12px"> a <tspan>b</tspan> </text></svg>"#,
        );
        let text = tree
            .get(SvgTree::ROOT)
            .and_then(|root| root.children.first())
            .and_then(|c| tree.get(*c))
            .map(|n| n.kind.clone());
        let Some(SvgKind::Text(text)) = text else {
            panic!("a text node: {text:?}");
        };
        assert_eq!(
            (text.body.as_str(), text.x, text.y, text.size),
            ("a b", 4.0, 5.0, 12.0)
        );
    }

    #[test]
    fn bytes_that_are_no_svg_are_refused() {
        assert!(read(&Payload::new(b"not svg".to_vec())).is_err());
    }
}

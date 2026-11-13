//! The characters behind each glyph of a run (`PDF-005`).
//!
//! A shaped glyph is a shape, not a character. A PDF reader copies and
//! searches characters, so the writer needs, for each glyph, the text of the
//! cluster that produced it. Parley keeps that mapping as clusters, and
//! [`run_text`] reads it back for one [`GlyphRun`].
//!
//! The mapping is many-to-many. A ligature draws several characters with one
//! glyph, and a base with its combining mark may draw one character with
//! two glyphs. So each glyph names its cluster, and two glyphs of one cluster
//! name the same one. The PDF writer turns a single-glyph cluster into a
//! `ToUnicode` entry, and a multi-glyph one into an `ActualText` span.

use ambre_core::index::{Len, Pos, Span};
use ambre_core::text_offset::Byte;
use parley::{Glyph, GlyphRun};
use serde::{Deserialize, Serialize};

use crate::ParleyBrush;

/// The text a layout was built from, as far as its glyph runs need it.
///
/// Most callers hold the whole text, and [`LayoutText::whole`] wraps it.
/// Some callers keep only a tail of it, such as a run re-shaped from one of
/// its lines on. Such a caller names the byte its tail starts at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutText<'a> {
    /// The text, or the tail of it from `first` on.
    text: &'a str,
    /// The byte of the layout's text that `text` starts at.
    first: Pos<Byte>,
}

impl<'a> LayoutText<'a> {
    /// The whole text a layout was built from.
    #[must_use]
    pub const fn whole(text: &'a str) -> Self {
        Self {
            text,
            first: Pos::new(0),
        }
    }

    /// The layout's text from byte `first` on, held as `text`.
    #[must_use]
    pub const fn from_byte(text: &'a str, first: Pos<Byte>) -> Self {
        Self { text, first }
    }

    /// No text: the run draws its shapes and no characters.
    #[must_use]
    pub const fn none() -> Self {
        Self::whole("")
    }

    /// The characters of the layout's bytes `bytes`, `None` when they fall
    /// outside the text held or inside a character.
    fn get(self, bytes: Span<Byte>) -> Option<&'a str> {
        let first = self.first.to_usize();
        let start = bytes.start.to_usize().checked_sub(first)?;
        let end = bytes.end.to_usize().checked_sub(first)?;
        self.text.get(start..end)
    }
}

/// The source text of one glyph run, and the cluster each glyph draws.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunText {
    /// The slice of the layout's text that the run draws, in logical order.
    pub text: String,
    /// One cluster per glyph, in the order the run draws its glyphs.
    pub clusters: Vec<GlyphCluster>,
}

/// The bytes of [`RunText::text`] that one glyph draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GlyphCluster {
    /// The first byte of the cluster.
    pub start: Pos<Byte>,
    /// The bytes the cluster holds.
    pub len: Len<Byte>,
}

impl GlyphCluster {
    /// The bytes the cluster holds, from its start.
    pub fn span(self) -> Span<Byte> {
        Span::from_len(self.start, self.len)
    }
}

/// Whether a cluster draws a glyph of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Draws {
    /// A ligature component, or a mark merged into its base.
    Nothing,
    /// One glyph or more.
    Glyphs,
}

/// One cluster of a shaped run, in logical order.
#[derive(Debug, Clone, Copy)]
struct Shaped {
    /// The bytes of the layout's text the cluster stands for.
    bytes: Span<Byte>,
    /// Whether the cluster draws a glyph.
    draws: Draws,
}

/// The text a glyph run draws, read from `source`, the text its layout was
/// built from.
///
/// Each glyph gets the text range of its cluster. A cluster with no glyph is
/// a ligature component, or a mark the shaper merged into its base. Its
/// characters join the cluster that drew them, the one before it in logical
/// order.
///
/// Returns `None` when `source` is not the layout's text: a cluster range
/// falls past its end or inside a character. The run then draws shapes alone,
/// as it did before the mapping existed. It never panics.
pub fn run_text(run: &GlyphRun<'_, ParleyBrush>, source: LayoutText<'_>) -> Option<RunText> {
    let drawn: Vec<Glyph> = run.glyphs().collect();
    run_text_of(run, &drawn, source)
}

/// [`run_text`], for a run whose glyphs the caller already holds as `drawn`.
/// A recorder walks the glyphs of a run once for both the mapping and the
/// op it records.
pub(crate) fn run_text_of(
    run: &GlyphRun<'_, ParleyBrush>,
    drawn: &[Glyph],
    source: LayoutText<'_>,
) -> Option<RunText> {
    let shaped = run.run();
    let logical: Vec<Shaped> = shaped
        .clusters()
        .map(|cluster| Shaped {
            bytes: Span::from_usize_range(cluster.text_range()),
            draws: if cluster.glyphs().next().is_some() {
                Draws::Glyphs
            } else {
                Draws::Nothing
            },
        })
        .collect();
    let owned = own_glyphless_clusters(&logical);
    // Visual order is logical order, reversed for a right-to-left run: the
    // order `GlyphRun::glyphs` walks its clusters in. A cluster draws one
    // glyph most of the time, so the logical count sizes the list.
    let mut visual: Vec<(Glyph, Span<Byte>)> = Vec::with_capacity(logical.len());
    let mut place = |(cluster, bytes): (parley::Cluster<'_, ParleyBrush>, &Span<Byte>)| {
        visual.extend(cluster.glyphs().map(|glyph| (glyph, *bytes)));
    };
    if shaped.is_rtl() {
        shaped
            .visual_clusters()
            .zip(owned.iter().rev())
            .for_each(&mut place);
    } else {
        shaped.visual_clusters().zip(&owned).for_each(&mut place);
    }
    // A glyph run is a slice of its shaped run: the glyphs of one style. The
    // slice's first glyph is private to parley, so find it by its glyphs. The
    // first match serves. Two slices that draw the same glyphs differ in
    // characters only where two spellings share a glyph.
    let first = (0..=visual.len().saturating_sub(drawn.len())).find(|&at| {
        visual
            .get(at..at + drawn.len())
            .is_some_and(|slice| slice.iter().map(|(g, _)| g).eq(drawn.iter()))
    })?;
    let spans = visual.get(first..first + drawn.len())?;
    let low = spans.iter().map(|(_, b)| b.start).min()?;
    let high = spans.iter().map(|(_, b)| b.end).max()?;
    // Every span must cut `source` at character boundaries, or the text is
    // not the one the layout was built from.
    if spans.iter().any(|(_, b)| source.get(*b).is_none()) {
        return None;
    }
    let text = source.get(Span::new(low, high))?.to_owned();
    let clusters = spans
        .iter()
        .map(|(_, b)| GlyphCluster {
            start: Pos::from_usize(b.start.to_usize().saturating_sub(low.to_usize())),
            len: b.len(),
        })
        .collect();
    Some(RunText { text, clusters })
}

/// The bytes each cluster stands for, once each glyphless cluster has joined
/// its owner. `logical` holds the clusters in logical order.
///
/// The owner is the last cluster before it that draws a glyph. A glyphless
/// cluster at the head of the run has none, so it joins the first cluster
/// after it that draws one. Adjacent spans stay contiguous, so a merge is a
/// union.
fn own_glyphless_clusters(logical: &[Shaped]) -> Vec<Span<Byte>> {
    let mut owned: Vec<Span<Byte>> = logical.iter().map(|c| c.bytes).collect();
    let mut owner: Option<usize> = None;
    let mut orphan_start: Option<Pos<Byte>> = None;
    for (index, cluster) in logical.iter().enumerate() {
        if cluster.draws == Draws::Glyphs {
            if let (Some(start), Some(own)) = (orphan_start.take(), owned.get_mut(index)) {
                own.start = own.start.min(start);
            }
            owner = Some(index);
            continue;
        }
        let bytes = cluster.bytes;
        match owner.and_then(|at| owned.get_mut(at)) {
            Some(own) => {
                own.start = own.start.min(bytes.start);
                own.end = own.end.max(bytes.end);
            }
            None => {
                orphan_start = Some(orphan_start.map_or(bytes.start, |s| s.min(bytes.start)));
            }
        }
    }
    owned
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster(start: u32, end: u32, draws: Draws) -> Shaped {
        Shaped {
            bytes: Span::new(Pos::new(start), Pos::new(end)),
            draws,
        }
    }

    fn span(start: u32, end: u32) -> Span<Byte> {
        Span::new(Pos::new(start), Pos::new(end))
    }

    #[test]
    fn a_ligature_component_joins_the_glyph_before_it() {
        // "f", "i" ligated: the first cluster draws one glyph, the second none.
        let owned = own_glyphless_clusters(&[
            cluster(0, 1, Draws::Glyphs),
            cluster(1, 2, Draws::Nothing),
            cluster(2, 3, Draws::Glyphs),
        ]);
        assert_eq!(owned[0], span(0, 2));
        assert_eq!(owned[2], span(2, 3));
    }

    #[test]
    fn a_glyphless_head_joins_the_glyph_after_it() {
        let owned =
            own_glyphless_clusters(&[cluster(0, 2, Draws::Nothing), cluster(2, 3, Draws::Glyphs)]);
        assert_eq!(owned[1], span(0, 3));
    }

    #[test]
    fn a_cluster_spans_its_bytes() {
        let cluster = GlyphCluster {
            start: Pos::from_usize(2),
            len: Len::of(3),
        };
        assert_eq!(cluster.span(), span(2, 5));
    }

    #[test]
    fn a_tail_reads_only_the_bytes_it_holds() {
        let tail = LayoutText::from_byte("world", Pos::new(6));
        assert_eq!(tail.get(span(6, 11)), Some("world"));
        assert_eq!(tail.get(span(0, 5)), None);
        assert_eq!(LayoutText::whole("\u{e9}").get(span(0, 1)), None);
    }
}

//! What the view builds from a shaped run to draw it, kept between frames
//! (`features/ambre-spec-prose-page-rendering.md section 2`).
//!
//! The shaped data arrives with the layout and stays there: hit-testing and
//! the caret read it whether or not the block was ever painted
//! (`PROSE-UI-172`). What this cache holds is the drawing half, built the
//! first time a run is painted (`PROSE-UI-119`) and reused after
//! (`PROSE-UI-173`). The glyph atlas itself belongs to the drawing surface.

use ambre_core::Count;
use ambre_core::index::{Axis, Pos};
use std::collections::HashMap;

use ambre_core::id::{Footnote, Id};
use ambre_core::{Doc, Px, Seq, SeqKind};
use loro::TreeID;

use crate::layout::run::{ParleyRunData, RunLine, ShapedRun};

/// How many runs hold drawing state, or may.
pub enum CachedRuns {}

impl ambre_core::NarrowCount for CachedRuns {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("CachedRunCount");
}

/// How many times the cache built the drawing state of a run.
pub enum Builds {}

impl ambre_core::NarrowCount for Builds {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("BuildCount");
}

/// An item of one shaped line, in the order the shaper laid them out.
#[derive(PartialEq, Eq)]
pub enum LineItem {}

impl Axis for LineItem {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("LineItem");
}

/// How many runs the cache keeps drawing state for, about fifty pages
/// (`ambre-spec-budgets.md section 6`).
pub const RUN_CACHE_RUNS: Count<CachedRuns> = Count::<CachedRuns>::new(500);

/// When an entry was last painted: the cache counts each paint. Only the
/// order of two ticks means something.
enum PaintTick {}

impl SeqKind for PaintTick {
    type IssueRight = ();
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("PaintTick");
}

/// Where one glyph run of a shaped run sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphRunAt {
    /// The line of the run that holds it.
    pub line: Pos<RunLine>,
    /// Its place among the items of that line.
    pub item: Pos<LineItem>,
    /// The top of its line.
    pub top: Px<Doc>,
}

/// Which run an entry is for: a block, or a footnote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RunKey {
    Block(TreeID),
    Note(Id<Footnote>),
}

/// The drawing half of a run: where each of its glyph runs sits, in the
/// order the shaper laid them out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunDrawState {
    /// Where each glyph run sits.
    pub glyph_runs: Vec<GlyphRunAt>,
}

impl RunDrawState {
    /// Walk a shaped run once and note where every glyph run sits.
    fn build(data: &ParleyRunData) -> Self {
        let mut glyph_runs = Vec::new();
        let tops = data.line_tops();
        for (line_index, (line, _)) in data.lines().enumerate() {
            let top = tops.get(line_index).copied().unwrap_or(Px::<Doc>::ZERO);

            for (item_index, item) in line.items().enumerate() {
                if matches!(item, ambre_draw::PositionedLayoutItem::GlyphRun(_)) {
                    glyph_runs.push(GlyphRunAt {
                        line: Pos::<RunLine>::from_usize(line_index),
                        item: Pos::<LineItem>::from_usize(item_index),
                        top,
                    });
                }
            }
        }
        Self { glyph_runs }
    }
}

/// What a run the cache lost draws: nothing. It cannot happen, since the run
/// just inserted is never the one evicted.
static NO_GLYPHS: RunDrawState = RunDrawState {
    glyph_runs: Vec::new(),
};

#[derive(Clone, Debug)]
struct Entry {
    run: ShapedRun,
    draw: RunDrawState,
    used: Seq<PaintTick>,
}

/// The drawing state of the runs painted lately, bounded (`PROSE-UI-019`).
///
/// Beyond [`RUN_CACHE_RUNS`] the run painted least recently is dropped. A
/// run shaped again is a new run, and its old state is dropped with it.
#[derive(Clone, Debug)]
pub struct RunCache {
    entries: HashMap<RunKey, Entry>,
    capacity: Count<CachedRuns>,
    tick: Seq<PaintTick>,
    built: Count<Builds>,
}

impl Default for RunCache {
    fn default() -> Self {
        Self::with_capacity(RUN_CACHE_RUNS)
    }
}

impl RunCache {
    /// A cache holding at most `capacity` runs, and always at least one.
    #[must_use]
    pub fn with_capacity(capacity: Count<CachedRuns>) -> Self {
        Self {
            entries: HashMap::new(),
            capacity: capacity.max(Count::<CachedRuns>::new(1)),
            tick: Seq::ZERO,
            built: Count::<Builds>::ZERO,
        }
    }

    /// How many runs hold drawing state now.
    #[must_use]
    pub fn len(&self) -> Count<CachedRuns> {
        Count::<CachedRuns>::of(self.entries.len())
    }

    /// Whether no run holds drawing state.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many times drawing state was built, over the life of the cache.
    #[must_use]
    pub fn built(&self) -> Count<Builds> {
        self.built
    }

    /// Whether `key` holds drawing state for this very run.
    #[must_use]
    pub fn holds(&self, key: RunKey, run: &ShapedRun) -> bool {
        self.entries
            .get(&key)
            .is_some_and(|entry| entry.run == *run)
    }

    /// The drawing state of a run about to be painted: built on its first
    /// paint, reused after.
    pub fn draw_state(&mut self, key: RunKey, run: &ShapedRun) -> &RunDrawState {
        self.tick = self.tick.next();
        let now = self.tick;
        let hit = match self.entries.get_mut(&key) {
            Some(entry) if entry.run == *run => {
                entry.used = now;
                true
            }
            _ => false,
        };
        if !hit {
            self.built = self.built.incremented();
            let entry = Entry {
                run: run.clone(),
                draw: RunDrawState::build(run.data()),
                used: now,
            };
            self.entries.insert(key, entry);
            // The run just inserted is the most recent, so it survives.
            self.evict();
        }
        self.entries
            .get(&key)
            .map_or(&NO_GLYPHS, |entry| &entry.draw)
    }

    /// Drop the least recently painted runs until the cache fits.
    fn evict(&mut self) {
        while self.len() > self.capacity {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| *key);
            match oldest {
                Some(key) => self.entries.remove(&key),
                None => return,
            };
        }
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::view::tests::laid_out;

    #[test]
    fn the_run_painted_least_recently_is_the_one_dropped() {
        let (_, shaped) = laid_out(4, 5);
        let runs: Vec<(RunKey, ShapedRun)> = shaped
            .layout
            .blocks
            .iter()
            .map(|block| {
                (
                    RunKey::Block(block.block_id),
                    shaped.blocks[&block.block_id].clone(),
                )
            })
            .collect();
        let mut cache = RunCache::with_capacity(Count::<CachedRuns>::new(2));
        cache.draw_state(runs[0].0, &runs[0].1);
        cache.draw_state(runs[1].0, &runs[1].1);
        // Touch the first again, so the second is now the oldest.
        cache.draw_state(runs[0].0, &runs[0].1);
        cache.draw_state(runs[2].0, &runs[2].1);
        assert_eq!(cache.len(), Count::<CachedRuns>::new(2));
        assert!(cache.holds(runs[0].0, &runs[0].1));
        assert!(!cache.holds(runs[1].0, &runs[1].1));
        assert!(cache.holds(runs[2].0, &runs[2].1));
        assert_eq!(cache.built(), Count::<Builds>::new(3));
    }

    #[test]
    fn a_run_shaped_again_is_built_again() {
        let (_, first) = laid_out(1, 5);
        let (_, second) = laid_out(1, 5);
        let key_of = |shaped: &crate::layout::run::ShapedLayout| {
            let block = shaped.layout.blocks.last().expect("a block");
            (block.block_id, shaped.blocks[&block.block_id].clone())
        };
        let (id, old) = key_of(&first);
        let (_, new) = key_of(&second);
        let mut cache = RunCache::default();
        cache.draw_state(RunKey::Block(id), &old);
        cache.draw_state(RunKey::Block(id), &new);
        assert_eq!(cache.built(), Count::<Builds>::new(2));
        assert!(!cache.holds(RunKey::Block(id), &old));
    }
}

//! What the view builds from a shaped run to draw it, kept between frames
//! (`features/ambre-spec-prose-page-rendering.md section 2`).
//!
//! The shaped data arrives with the layout and stays there: hit-testing and
//! the caret read it whether or not the block was ever painted
//! (`PROSE-UI-172`). What this cache holds is the drawing half, built the
//! first time a run is painted (`PROSE-UI-119`) and reused after
//! (`PROSE-UI-173`). The glyph atlas itself belongs to the drawing surface.

use std::collections::HashMap;

use ambre_core::Uuid;
use loro::TreeID;

use crate::layout::run::{ParleyRunData, ShapedRun};

/// How many runs the cache keeps drawing state for, about fifty pages
/// (`ambre-spec-budgets.md section 6`).
pub const RUN_CACHE_RUNS: usize = 500;

/// Which run an entry is for: a block, or a footnote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RunKey {
    Block(TreeID),
    Note(Uuid),
}

/// The drawing half of a run: where each of its glyph runs sits, in the
/// order the shaper laid them out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunDrawState {
    /// The line and the item of each glyph run, with the top of its line.
    pub glyph_runs: Vec<(usize, usize, f32)>,
}

impl RunDrawState {
    /// Walk a shaped run once and note where every glyph run sits.
    fn build(data: &ParleyRunData) -> Self {
        let mut glyph_runs = Vec::new();
        let tops = data.line_tops();
        for (line_index, (line, _)) in data.lines().enumerate() {
            let top = tops.get(line_index).copied().unwrap_or(0.0);
            for (item_index, item) in line.items().enumerate() {
                if matches!(item, ambre_draw::PositionedLayoutItem::GlyphRun(_)) {
                    glyph_runs.push((line_index, item_index, top));
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
    used: u64,
}

/// The drawing state of the runs painted lately, bounded (`PROSE-UI-019`).
///
/// Beyond [`RUN_CACHE_RUNS`] the run painted least recently is dropped. A
/// run shaped again is a new run, and its old state is dropped with it.
#[derive(Clone, Debug)]
pub struct RunCache {
    entries: HashMap<RunKey, Entry>,
    capacity: usize,
    clock: u64,
    built: u64,
}

impl Default for RunCache {
    fn default() -> Self {
        Self::with_capacity(RUN_CACHE_RUNS)
    }
}

impl RunCache {
    /// A cache holding at most `capacity` runs, and always at least one.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            capacity: capacity.max(1),
            clock: 0,
            built: 0,
        }
    }

    /// How many runs hold drawing state now.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no run holds drawing state.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many times drawing state was built, over the life of the cache.
    #[must_use]
    pub fn built(&self) -> u64 {
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
        self.clock = self.clock.saturating_add(1);
        let now = self.clock;
        let hit = match self.entries.get_mut(&key) {
            Some(entry) if entry.run == *run => {
                entry.used = now;
                true
            }
            _ => false,
        };
        if !hit {
            self.built = self.built.saturating_add(1);
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
        while self.entries.len() > self.capacity {
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

#[cfg(all(test, not(target_arch = "wasm32")))]
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
        let mut cache = RunCache::with_capacity(2);
        cache.draw_state(runs[0].0, &runs[0].1);
        cache.draw_state(runs[1].0, &runs[1].1);
        // Touch the first again, so the second is now the oldest.
        cache.draw_state(runs[0].0, &runs[0].1);
        cache.draw_state(runs[2].0, &runs[2].1);
        assert_eq!(cache.len(), 2);
        assert!(cache.holds(runs[0].0, &runs[0].1));
        assert!(!cache.holds(runs[1].0, &runs[1].1));
        assert!(cache.holds(runs[2].0, &runs[2].1));
        assert_eq!(cache.built(), 3);
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
        assert_eq!(cache.built(), 2);
        assert!(!cache.holds(RunKey::Block(id), &old));
    }
}

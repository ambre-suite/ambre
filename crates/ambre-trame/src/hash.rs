//! The hash tables of Trame: foldhash under a key drawn once per process from
//! the platform entropy source (`TRAME-292`).
//!
//! A peer chooses the cell keys and the line ids that reach these tables. A
//! hasher with a key the peer can learn lets it send keys that collide, and
//! each lookup then walks a chain (`HashDoS`). The key therefore comes from
//! `getrandom`: the OS source natively, `crypto.getRandomValues` in the
//! browser. foldhash's own `RandomState` draws its key from addresses alone
//! on `wasm32-unknown-unknown`, which carries no address randomization, so it
//! would give every tab the same key. The std `RandomState` reads addresses
//! there too.
//!
//! SipHash, the std default, costs about a sixth of the undo of a block delete
//! (`ambre-spec-budgets.md` section 1). foldhash costs a fraction of it and
//! makes no cryptographic claim, which a secret key per process does not need.

use std::cell::Cell;
use std::hash::BuildHasher;
use std::sync::OnceLock;

use foldhash::SharedSeed;
use foldhash::fast::{FoldHasher, SeedableRandomState};

/// A hash map of Trame. Build one with `HashMap::default()`: `new` exists only
/// for the std hasher, and `clippy.toml` bans the std alias in this crate.
#[expect(
    clippy::disallowed_types,
    reason = "the one place the std map is named, with the hasher of this module"
)]
pub type HashMap<K, V> = std::collections::HashMap<K, V, TableHasher>;

/// A hash set of Trame, under the same hasher as [`HashMap`].
#[expect(
    clippy::disallowed_types,
    reason = "the one place the std set is named, with the hasher of this module"
)]
pub type HashSet<T> = std::collections::HashSet<T, TableHasher>;

/// The hasher every table of Trame builds from: foldhash under the key of the
/// process, and a seed of its own per table.
///
/// The seed per table changes the order of iteration from one table to the
/// next. A loop that copies one table into another then never meets its keys
/// in bucket order, which would cost a quadratic number of probes.
#[derive(Clone, Debug)]
pub struct TableHasher(SeedableRandomState);

impl Default for TableHasher {
    fn default() -> Self {
        Self(next_table_state())
    }
}

impl BuildHasher for TableHasher {
    type Hasher = FoldHasher<'static>;

    fn build_hasher(&self) -> Self::Hasher {
        self.0.build_hasher()
    }
}

/// The key of the process, drawn on first use and kept for the life of the
/// worker.
fn process_key() -> &'static SharedSeed {
    static KEY: OnceLock<SharedSeed> = OnceLock::new();
    KEY.get_or_init(|| draw_key(getrandom::fill))
}

/// A key from `fill`, or foldhash's own random key when `fill` fails.
///
/// A failed entropy source must not stop the worker: the user would lose the
/// session. foldhash's key still differs from its fixed key wherever the
/// platform randomizes an address, so the fallback degrades and never panics.
fn draw_key<E>(fill: impl FnOnce(&mut [u8]) -> Result<(), E>) -> SharedSeed {
    let mut bytes = [0_u8; 8];
    match fill(&mut bytes) {
        Ok(()) => SharedSeed::from_u64(u64::from_le_bytes(bytes)),
        Err(_) => SharedSeed::global_random().clone(),
    }
}

/// The state of a new table: the key of the process, and a seed per table,
/// distinct on each call within a thread.
///
/// The seed need not be secret: the key of the process is, and foldhash mixes
/// the two. A counter keeps the cost to one thread-local read.
fn next_table_state() -> SeedableRandomState {
    thread_local! {
        static NEXT: Cell<u64> = const { Cell::new(0) };
    }
    let seed = NEXT.with(|next| {
        // The odd step is 2^64 divided by the golden ratio: successive seeds
        // then differ in their high bits as well as their low bits.
        let seed = next.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
        next.set(seed);
        seed
    });
    SeedableRandomState::with_seed(seed, process_key())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{BuildHasher, Hasher};

    /// The hash of `value` under a hasher of table seed 0 and key `key`.
    fn hash_under(key: &'static SharedSeed, value: u64) -> u64 {
        let mut h = FoldHasher::with_seed(0, key);
        h.write_u64(value);
        h.finish()
    }

    #[test]
    fn the_key_is_the_bytes_the_entropy_source_gives() {
        let a = draw_key(|b: &mut [u8]| {
            b.fill(1);
            Ok::<(), ()>(())
        });
        let b = draw_key(|b: &mut [u8]| {
            b.fill(2);
            Ok::<(), ()>(())
        });
        let a: &'static SharedSeed = Box::leak(Box::new(a));
        let b: &'static SharedSeed = Box::leak(Box::new(b));
        assert_ne!(
            hash_under(a, 42),
            hash_under(b, 42),
            "two draws of the entropy source give two keys"
        );
    }

    #[test]
    fn the_key_of_the_process_is_not_the_fixed_key() {
        assert_ne!(
            hash_under(process_key(), 42),
            hash_under(SharedSeed::global_fixed(), 42),
            "a peer knows the fixed key from the foldhash source"
        );
    }

    #[test]
    fn a_failed_entropy_source_falls_back_without_a_panic() {
        let key = draw_key(|_: &mut [u8]| Err(()));
        let key: &'static SharedSeed = Box::leak(Box::new(key));
        assert_eq!(
            hash_under(key, 42),
            hash_under(SharedSeed::global_random(), 42),
            "the fallback is foldhash's own random key"
        );
    }

    #[test]
    fn two_tables_iterate_in_two_orders() {
        let a = TableHasher::default();
        let b = TableHasher::default();
        assert_ne!(
            a.hash_one(42_u64),
            b.hash_one(42_u64),
            "each table takes a seed of its own"
        );
    }

    #[test]
    fn one_table_hashes_a_key_the_same_way_twice() {
        let t = TableHasher::default();
        assert_eq!(t.hash_one(42_u64), t.hash_one(42_u64));
    }
}

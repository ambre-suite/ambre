//! Media placeholder state machine + bounded local cache (`toile-media`).
//!
//! This is the **pure, host-testable** core of the media pipeline. It models the
//! `Loading -> Ready | Failed` lifecycle of a content-addressed blob, and the
//! local LRU cache that makes zoom/pan revisits refetch-free. It deliberately
//! does **not** hash, encrypt, compress, or touch the network. The real blake2b
//! hashing, and all crypto, runs in the Sync thread, which owns the plaintext.
//! The cache is keyed by a [`ContentHash`] the caller supplies, so nothing here
//! ever inspects plaintext bytes.
//!
//! The state machine is what Phase 0 needs standalone; Phase 1 wires it to the
//! `RestCommand::UploadMedia` / `DownloadMedia` service plane.

use ambre_core::Count;
use ambre_core::Validated;
use ambre_core::hash::HashMap;

/// The content address of a media blob. The hash itself is computed
/// Sync-side; interop only routes it.
pub use ambre_core::ContentHash;

/// Lifecycle of one cached blob. `Loading` is in-flight (a download was issued,
/// pixels not yet back) and `Ready` holds decoded pixels. `Failed` recorded a
/// fetch/decode error, so the UI can paint a broken-image affordance without
/// re-issuing the request in a tight loop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MediaState {
    Loading,
    Ready,
    Failed,
}

/// What a [`MediaCache::request`] resolved to. `Fetch` means this hash was not
/// known and the caller must issue a `DownloadMedia`. `CacheHit` means an entry
/// already exists (in-flight, ready, or failed) and **no** new fetch is due. That
/// is the content-addressing dedup that keeps identical images to a single blob.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RequestOutcome {
    Fetch,
    CacheHit(MediaState),
}

/// One cache slot: the lifecycle state plus, once `Ready`, the decoded pixels the
/// `toile-board-rendering` texture path uploads. Pixels stay `None` while `Loading`/`Failed`.
struct Entry {
    state: MediaState,
    pixels: Option<Vec<u8>>,
}

/// How many blobs a media cache holds.
pub enum CachedBlobs {}

impl ambre_core::CountKind for CachedBlobs {
    type Repr = u32;
    const NAME: &'static str = "CachedBlobCount";
}

/// The most blobs a media cache keeps resident. A bound is no count: it never
/// counts up, and the cache only compares its count against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaCacheCapacity(Count<CachedBlobs>);

impl MediaCacheCapacity {
    /// A bound of `blobs`. A bound of zero becomes one, so a single in-flight
    /// request always has a slot.
    pub fn new(blobs: Count<CachedBlobs>) -> Self {
        Self(blobs.max(Count::<CachedBlobs>::new(1)))
    }

    /// Whether a cache that holds `held` blobs is past this bound.
    pub fn is_exceeded_by(self, held: Count<CachedBlobs>) -> bool {
        held > self.0
    }
}

/// Bounded, content-addressed LRU cache with placeholder-then-load semantics.
/// Eviction is least-recently-used, but an in-flight `Loading` entry is never
/// evicted (its pending download would resolve into a dropped slot). The cache
/// never holds plaintext beyond the session -- it lives in Logic-thread memory
/// only, consistent with the confidentiality posture.
pub struct MediaCache {
    capacity: MediaCacheCapacity,
    entries: HashMap<Validated<ContentHash>, Entry>,
    /// Access order, least-recently-used at the front, most-recently at the back.
    order: Vec<Validated<ContentHash>>,
}

impl MediaCache {
    /// A cache bounded to `capacity` resident blobs.
    pub fn new(capacity: MediaCacheCapacity) -> Self {
        Self {
            capacity,
            entries: HashMap::default(),
            order: Vec::new(),
        }
    }

    /// Number of resident entries (any state).
    pub fn len(&self) -> Count<CachedBlobs> {
        Count::<CachedBlobs>::of(self.entries.len())
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Request a blob's pixels. On the first request for `hash` the slot is
    /// created `Loading` and [`RequestOutcome::Fetch`] is returned -- the caller
    /// issues a `DownloadMedia`. A repeated request (in-flight, ready, or failed)
    /// returns [`RequestOutcome::CacheHit`] and issues no fetch: the dedup that
    /// collapses identical images to one blob.
    pub fn request(&mut self, hash: &Validated<ContentHash>) -> RequestOutcome {
        if let Some(entry) = self.entries.get(hash) {
            let state = entry.state;
            self.touch(hash);
            return RequestOutcome::CacheHit(state);
        }
        self.insert(
            hash.clone(),
            Entry {
                state: MediaState::Loading,
                pixels: None,
            },
        );
        RequestOutcome::Fetch
    }

    /// Resolve an in-flight (or unknown) blob to `Ready`, storing its decoded
    /// pixels. Returns `true` if a `Loading` slot was fulfilled, `false` if it
    /// resolved an unknown or already-terminal slot. The transition is still
    /// applied: last write wins, matching a late-arriving download. Touches the
    /// entry as most-recently-used.
    pub fn resolve(&mut self, hash: &Validated<ContentHash>, pixels: Vec<u8>) -> bool {
        let was_loading = matches!(
            self.entries.get(hash).map(|e| e.state),
            Some(MediaState::Loading)
        );
        match self.entries.get_mut(hash) {
            Some(entry) => {
                entry.state = MediaState::Ready;
                entry.pixels = Some(pixels);
                self.touch(hash);
            }
            None => {
                self.insert(
                    hash.clone(),
                    Entry {
                        state: MediaState::Ready,
                        pixels: Some(pixels),
                    },
                );
            }
        }
        was_loading
    }

    /// Mark a blob's fetch/decode as failed. Idempotent; creates the slot
    /// if the failure races ahead of the request record.
    pub fn fail(&mut self, hash: &Validated<ContentHash>) {
        match self.entries.get_mut(hash) {
            Some(entry) => {
                entry.state = MediaState::Failed;
                entry.pixels = None;
                self.touch(hash);
            }
            None => {
                self.insert(
                    hash.clone(),
                    Entry {
                        state: MediaState::Failed,
                        pixels: None,
                    },
                );
            }
        }
    }

    /// The current lifecycle state of `hash`, or `None` if never requested.
    pub fn state(&self, hash: &Validated<ContentHash>) -> Option<MediaState> {
        self.entries.get(hash).map(|e| e.state)
    }

    /// The decoded pixels of a `Ready` blob, or `None` otherwise. A cache hit here
    /// is the "0 refetch" path.
    pub fn pixels(&self, hash: &Validated<ContentHash>) -> Option<&[u8]> {
        self.entries.get(hash).and_then(|e| e.pixels.as_deref())
    }

    // --- internal LRU bookkeeping -------------------------------------------

    /// Move `hash` to the most-recently-used position. Assumes it is resident.
    fn touch(&mut self, hash: &Validated<ContentHash>) {
        if let Some(pos) = self.order.iter().position(|h| h == hash) {
            let h = self.order.remove(pos);
            self.order.push(h);
        }
    }

    /// Insert a fresh entry as most-recently-used, evicting the LRU non-`Loading`
    /// entry if over capacity.
    fn insert(&mut self, hash: Validated<ContentHash>, entry: Entry) {
        self.entries.insert(hash.clone(), entry);
        self.order.push(hash);
        self.evict_if_needed();
    }

    /// Drop least-recently-used entries until within capacity, skipping any
    /// `Loading` slot so an in-flight download never resolves into a hole.
    fn evict_if_needed(&mut self) {
        while self.capacity.is_exceeded_by(self.len()) {
            let victim = self.order.iter().position(|h| {
                self.entries
                    .get(h)
                    .map(|e| e.state != MediaState::Loading)
                    .unwrap_or(false)
            });
            match victim {
                Some(pos) => {
                    let h = self.order.remove(pos);
                    self.entries.remove(&h);
                }
                // Every resident entry is in-flight: nothing is safe to evict, so
                // we exceed capacity transiently rather than drop a pending load.
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hash that repeats the hex digit `s` sixty-four times.
    fn h(s: &str) -> Validated<ContentHash> {
        Validated::<ContentHash>::parse(s.repeat(64)).expect("a hex digit")
    }

    #[test]
    fn first_request_fetches_then_dedups() {
        let mut cache = MediaCache::new(MediaCacheCapacity::new(Count::<CachedBlobs>::new(8)));
        assert_eq!(cache.request(&h("a")), RequestOutcome::Fetch);
        // Second request for the same content does not re-fetch (dedup,).
        assert_eq!(
            cache.request(&h("a")),
            RequestOutcome::CacheHit(MediaState::Loading)
        );
        assert_eq!(
            cache.len(),
            Count::<CachedBlobs>::new(1),
            "identical hashes collapse to one entry"
        );
    }

    #[test]
    fn loading_to_ready_carries_pixels() {
        let mut cache = MediaCache::new(MediaCacheCapacity::new(Count::<CachedBlobs>::new(8)));
        cache.request(&h("a"));
        assert!(
            cache.resolve(&h("a"), vec![1, 2, 3]),
            "a Loading slot was filled"
        );
        assert_eq!(cache.state(&h("a")), Some(MediaState::Ready));
        assert_eq!(cache.pixels(&h("a")), Some(&[1, 2, 3][..]));
        // Cache hit on a Ready blob is the 0-refetch path.
        assert_eq!(
            cache.request(&h("a")),
            RequestOutcome::CacheHit(MediaState::Ready)
        );
    }

    #[test]
    fn loading_to_failed() {
        let mut cache = MediaCache::new(MediaCacheCapacity::new(Count::<CachedBlobs>::new(8)));
        cache.request(&h("a"));
        cache.fail(&h("a"));
        assert_eq!(cache.state(&h("a")), Some(MediaState::Failed));
        assert_eq!(cache.pixels(&h("a")), None);
        assert_eq!(
            cache.request(&h("a")),
            RequestOutcome::CacheHit(MediaState::Failed)
        );
    }

    #[test]
    fn lru_evicts_least_recently_used_ready_entry() {
        let mut cache = MediaCache::new(MediaCacheCapacity::new(Count::<CachedBlobs>::new(2)));
        cache.resolve(&h("a"), vec![0]);
        cache.resolve(&h("b"), vec![0]);
        // Touch "a" so "b" becomes least-recently-used.
        assert!(cache.pixels(&h("a")).is_some());
        cache.request(&h("a"));
        cache.resolve(&h("c"), vec![0]); // over capacity -> evict LRU ("b")
        assert_eq!(cache.len(), Count::<CachedBlobs>::new(2));
        assert_eq!(cache.state(&h("b")), None, "b was evicted as LRU");
        assert_eq!(cache.state(&h("a")), Some(MediaState::Ready));
        assert_eq!(cache.state(&h("c")), Some(MediaState::Ready));
    }

    #[test]
    fn loading_entries_are_never_evicted() {
        let mut cache = MediaCache::new(MediaCacheCapacity::new(Count::<CachedBlobs>::new(1)));
        cache.request(&h("a")); // in-flight
        cache.request(&h("b")); // in-flight, over capacity but a is Loading
        // Neither in-flight download may be dropped -> capacity exceeded transiently.
        assert_eq!(cache.len(), Count::<CachedBlobs>::new(2));
        assert_eq!(cache.state(&h("a")), Some(MediaState::Loading));
        assert_eq!(cache.state(&h("b")), Some(MediaState::Loading));
        // Once one resolves, the next insert can evict it back to capacity.
        cache.resolve(&h("a"), vec![0]);
        cache.resolve(&h("c"), vec![0]);
        assert!(cache.len() <= Count::<CachedBlobs>::new(2));
    }

    #[test]
    fn capacity_zero_coerced_to_one() {
        let mut cache = MediaCache::new(MediaCacheCapacity::new(Count::<CachedBlobs>::ZERO));
        cache.resolve(&h("a"), vec![0]);
        assert_eq!(cache.len(), Count::<CachedBlobs>::new(1));
    }
}

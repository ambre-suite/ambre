//! The send window: the deltas of one document that leave for the relay as
//! one (`ambre-spec-sync.md` section 2, the send window).
//!
//! A delta offered with no window open leaves at once, and opens a window of
//! [`SEND_WINDOW`]. A delta offered while a window is open waits for its end.
//! At that end every delta held leaves as one, and that send opens the next
//! window. A lone key therefore pays nothing, and a held one pays at most the
//! window (`SYNC-193`). A document caps at one delta per window, whatever the
//! rate of its keys (`SYNC-192`). The deltas of one window leave merged into
//! one (`SYNC-194`).
//!
//! The window reads no clock of its own: the caller hands it the instant. A
//! test therefore drives it on a simulated clock. A state with no clock never
//! holds a delta, since nothing could end its window.

use ambre_core::{DeltaBytes, FracMs, Instant, Ms, PageLoad, Payload, TimeSpan};

/// How long a window stays open after a delta leaves.
///
/// The client share of the inter-user budget is 50 ms (README). One key
/// spends about 25 ms of it outside the window, on the targets of
/// `ambre-spec-budgets.md` section 1. The window takes the rest, and section 8
/// of that register holds the sum.
pub(crate) const SEND_WINDOW: TimeSpan<Ms> = TimeSpan::<Ms>::new(25);

/// The window of one document: when it opened, and what waits for its end.
#[derive(Debug, Default)]
pub(crate) struct SendWindow {
    /// The instant the last delta left, while its window may still be open.
    opened: Option<Instant<PageLoad>>,
    /// The deltas that wait for the window to end, oldest first.
    held: Vec<Payload<DeltaBytes>>,
}

impl SendWindow {
    /// Offer the deltas one pass wrote, in order, and return those that leave
    /// now: every delta held, once no window is open at `now`.
    pub(crate) fn offer(
        &mut self,
        now: Option<Instant<PageLoad>>,
        deltas: Vec<Payload<DeltaBytes>>,
    ) -> Vec<Payload<DeltaBytes>> {
        self.held.extend(deltas);
        self.release(now)
    }

    /// Return every delta held when the window is over at `now`, and open the
    /// next window on that send. An open window returns nothing. With no
    /// instant the window is over, since nothing would ever end it.
    pub(crate) fn release(&mut self, now: Option<Instant<PageLoad>>) -> Vec<Payload<DeltaBytes>> {
        if self.held.is_empty() {
            return Vec::new();
        }
        if let (Some(now), Some(end)) = (now, self.end())
            && now < end
        {
            return Vec::new();
        }
        self.opened = now;
        std::mem::take(&mut self.held)
    }

    /// Return every delta held, whatever the window: the document leaves
    /// memory, or the session ends, so no later pass would send them.
    pub(crate) fn flush(&mut self) -> Vec<Payload<DeltaBytes>> {
        std::mem::take(&mut self.held)
    }

    /// The instant the held deltas leave, or `None` when nothing waits.
    pub(crate) fn due(&self) -> Option<Instant<PageLoad>> {
        if self.held.is_empty() {
            return None;
        }
        self.end()
    }

    /// The end of the window opened by the last send.
    fn end(&self) -> Option<Instant<PageLoad>> {
        self.opened.map(|opened| opened.after(window_span()))
    }
}

/// The deltas that leave together, as one delta when they carry one run of one
/// peer's operations, read back from the document that wrote them.
///
/// The engine exports each command's operations past its cursor, so the
/// deltas of one window are one peer's run, end to end. Any other shape, and
/// an export that fails, leave the deltas apart and in order. They still
/// apply, at the cost of one relay write each.
pub(crate) fn merge(
    doc: &loro::LoroDoc,
    deltas: Vec<Payload<DeltaBytes>>,
) -> Vec<Payload<DeltaBytes>> {
    if deltas.len() < 2 {
        return deltas;
    }
    let Some(run) = one_run(&deltas) else {
        return deltas;
    };
    match doc.export(loro::ExportMode::updates_in_range(vec![run])) {
        Ok(bytes) if !bytes.is_empty() => vec![Payload::<DeltaBytes>::new(bytes)],
        Ok(_) => deltas,
        Err(e) => {
            tracing::warn!("Logic: the deltas of a window could not merge, they leave apart: {e}");
            deltas
        }
    }
}

/// The one run of one peer's operations the deltas carry, end to end. It is
/// `None` for two peers, a gap, or a blob no reader decodes.
fn one_run(deltas: &[Payload<DeltaBytes>]) -> Option<loro::IdSpan> {
    let mut run: Option<loro::IdSpan> = None;
    for delta in deltas {
        let meta = loro::LoroDoc::decode_import_blob_meta(delta.as_bytes(), false).ok()?;
        let mut peers = meta.partial_end_vv.iter();
        let (peer, end) = peers.next()?;
        if peers.next().is_some() || meta.partial_start_vv.iter().any(|(p, _)| p != peer) {
            return None;
        }
        let start = meta.partial_start_vv.get(peer).copied().unwrap_or(0);
        run = match run {
            None => Some(loro::IdSpan::new(*peer, start, *end)),
            Some(span) if span.peer == *peer && span.counter.end == start => {
                Some(loro::IdSpan::new(*peer, span.counter.start, *end))
            }
            Some(_) => return None,
        };
    }
    run
}

/// The window on the page clock, which counts fractional milliseconds.
fn window_span() -> TimeSpan<FracMs> {
    SEND_WINDOW.to_fractional()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: f64) -> Option<Instant<PageLoad>> {
        Some(Instant::<PageLoad>::new(ms))
    }

    fn delta(byte: u8) -> Payload<DeltaBytes> {
        Payload::<DeltaBytes>::new(vec![byte])
    }

    /// One key typed at each instant of `keys`, in milliseconds, with the
    /// thread waking at each due instant as the run loop does. Returns each
    /// send: its instant, and the instants of the keys it carries.
    fn typed(keys: &[f64]) -> Vec<(f64, Vec<f64>)> {
        let mut window = SendWindow::default();
        let mut sends = Vec::new();
        let mut waiting: Vec<f64> = Vec::new();
        let mut keys = keys.iter().copied().peekable();
        loop {
            let due = window.due().map(Instant::get);
            let next_key = keys.peek().copied();
            let (now, sent) = match (next_key, due) {
                (Some(key), Some(due)) if due <= key => (due, window.release(at(due))),
                (Some(key), _) => {
                    keys.next();
                    waiting.push(key);
                    (key, window.offer(at(key), vec![delta(0)]))
                }
                (None, Some(due)) => (due, window.release(at(due))),
                (None, None) => return sends,
            };
            if !sent.is_empty() {
                sends.push((now, std::mem::take(&mut waiting)));
            }
        }
    }

    /// The longest wait of any key between its pass and its send.
    fn longest_wait(sends: &[(f64, Vec<f64>)]) -> f64 {
        sends
            .iter()
            .flat_map(|(sent, keys)| keys.iter().map(move |key| sent - key))
            .fold(0.0, f64::max)
    }

    /// `SYNC-193`: a key typed with no window open leaves in its own pass.
    #[test]
    fn a_lone_key_leaves_at_once() {
        let sends = typed(&[0.0]);
        assert_eq!(sends, vec![(0.0, vec![0.0])]);
        let sends = typed(&[0.0, 500.0]);
        assert_eq!(sends, vec![(0.0, vec![0.0]), (500.0, vec![500.0])]);
    }

    /// `SYNC-192`: a burst of keys faster than the window leaves as one delta
    /// per window, and the first key of the burst leaves at once. Forty keys at
    /// one per 5 ms, key repeat or a fast typist, cost nine deltas, not forty.
    #[test]
    fn a_burst_leaves_once_per_window() {
        let keys: Vec<f64> = (0..40).map(|i| f64::from(i) * 5.0).collect();
        let sends = typed(&keys);
        assert_eq!(sends.len(), 9, "{sends:?}");
        assert_eq!(sends[0], (0.0, vec![0.0]), "the first key left alone");
        assert_eq!(
            sends.iter().map(|(_, keys)| keys.len()).sum::<usize>(),
            40,
            "every key left"
        );
        for pair in sends.windows(2) {
            assert!(
                pair[1].0 - pair[0].0 >= 25.0,
                "two sends inside one window: {pair:?}"
            );
        }
    }

    /// `SYNC-192`: at five keys a second the keys are farther apart than the
    /// window, so each leaves alone and none waits. The window caps a rate, and
    /// a slow typist never meets the cap.
    #[test]
    fn keys_farther_apart_than_the_window_leave_one_by_one() {
        let keys: Vec<f64> = (0..10).map(|i| f64::from(i) * 200.0).collect();
        let sends = typed(&keys);
        assert_eq!(sends.len(), 10);
        assert!(longest_wait(&sends) == 0.0, "{sends:?}");
    }

    /// `SYNC-193`: no key waits past the window, whatever the rhythm. Every
    /// spacing from 1 ms to 60 ms, with jitter, keeps the longest wait at or
    /// under 25 ms.
    #[test]
    fn no_key_waits_past_the_window() {
        for spacing in 1..=60_u32 {
            let mut keys: Vec<f64> = (0..50_u32)
                .map(|i| f64::from(i * spacing) + f64::from((i * 7) % 3))
                .collect();
            keys.sort_by(f64::total_cmp);
            let sends = typed(&keys);
            let wait = longest_wait(&sends);
            assert!(wait <= 25.0, "spacing {spacing} ms waits {wait} ms");
            assert_eq!(sends.iter().map(|(_, keys)| keys.len()).sum::<usize>(), 50);
        }
    }

    /// `SYNC-195`: a flush hands back every delta held, inside the window too.
    #[test]
    fn a_flush_takes_every_held_delta() {
        let mut window = SendWindow::default();
        assert_eq!(window.offer(at(0.0), vec![delta(1)]).len(), 1);
        assert!(window.offer(at(3.0), vec![delta(2), delta(3)]).is_empty());
        assert_eq!(window.due(), at(25.0));
        let flushed = window.flush();
        assert_eq!(flushed, vec![delta(2), delta(3)]);
        assert_eq!(window.due(), None, "nothing waits after a flush");
    }

    /// With no clock, nothing would end a window, so no delta waits.
    #[test]
    fn with_no_clock_no_delta_waits() {
        let mut window = SendWindow::default();
        assert_eq!(window.offer(None, vec![delta(1)]).len(), 1);
        assert_eq!(window.offer(None, vec![delta(2)]).len(), 1);
        assert_eq!(window.due(), None);
    }

    /// The deltas a Loro document of one peer writes, one per insert, each
    /// exported past the one before it as an engine does.
    fn typed_doc(peer: u64, keys: usize) -> (loro::LoroDoc, Vec<Payload<DeltaBytes>>) {
        let doc = loro::LoroDoc::new();
        doc.set_peer_id(peer).expect("a peer id");
        let text = doc.get_text("body");
        let mut deltas = Vec::new();
        for i in 0..keys {
            let before = doc.oplog_vv();
            text.insert(i, "a").expect("an insert");
            doc.commit();
            let bytes = doc
                .export(loro::ExportMode::updates(&before))
                .expect("an export");
            deltas.push(Payload::<DeltaBytes>::new(bytes));
        }
        (doc, deltas)
    }

    /// `SYNC-194`: the deltas of one window leave as one delta, and a peer
    /// that imports it holds every key.
    #[test]
    fn the_deltas_of_a_window_merge_into_one() {
        let (doc, deltas) = typed_doc(7, 5);
        let merged = merge(&doc, deltas);
        assert_eq!(merged.len(), 1, "one delta leaves");
        let reader = loro::LoroDoc::new();
        reader
            .import(merged[0].as_bytes())
            .expect("the merged delta imports");
        assert_eq!(reader.get_text("body").to_string(), "aaaaa");
    }

    /// `SYNC-194`: a run with a gap, or two peers, leaves apart and in order.
    #[test]
    fn deltas_that_are_no_single_run_leave_apart() {
        let (doc, mut deltas) = typed_doc(7, 3);
        deltas.remove(1);
        assert_eq!(merge(&doc, deltas.clone()), deltas, "a gap merges nothing");
        let (other, theirs) = typed_doc(8, 1);
        let (_, ours) = typed_doc(7, 1);
        let two = vec![ours[0].clone(), theirs[0].clone()];
        assert_eq!(merge(&other, two.clone()), two, "two peers merge nothing");
    }
}

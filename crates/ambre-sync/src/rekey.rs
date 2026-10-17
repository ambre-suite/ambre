// ambre-sync/src/rekey.rs
// ============================================================================
// What this client sent, and what it owes, around a rotation
// (`ambre-spec-key-rotation.md section 3`, a departure in steps).
//
// The relay refuses every frame of a document that owes a rotation, before it
// reads the clock (`SRV-123`, `SRV-125`). So a refused clock is still free, and
// so is every clock after it. This client keeps each update it sent until the
// relay echoes it back. It seals the refused ones again at the same clocks,
// under the next key (`SYNC-129`). An edit made while the rotation is owed waits
// for the new key unsealed (`SYNC-128`).
//
// Two sessions of one user share one clock at the relay. So this log knows a
// delta by its signature, never by its clock. The other session's echo at the
// same clock confirms nothing here (`SYNC-148`). A delta refused for its
// clock leaves again, with every later one, at the next clocks (`SYNC-147`).
//
// Pure state, no IO: the caller seals, sends and spawns.
// ============================================================================

use std::collections::VecDeque;

use ambre_core::KeyId;
use ambre_protocol::messages::Signature;

/// Updates kept per document until the relay echoes them back. Past the cap the
/// oldest is forgotten, and a refusal of it can no longer be answered
/// (`ambre-spec-budgets.md section 6`).
pub(crate) const UNCONFIRMED_LIMIT: usize = 1024;

/// One update sent and not yet echoed back by the relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SentFrame {
    pub clock: u64,
    /// The key the frame was last sealed under.
    pub key_id: KeyId,
    /// The signature of its last seal. The relay's echo and refusal carry it.
    pub signature: Signature,
    /// The plaintext body, its session nonce ahead of the delta, kept to seal
    /// it again under another key.
    pub payload: Vec<u8>,
}

/// Refused frames to seal again, as (clock, plaintext), in clock order.
pub(crate) type RefusedFrames = Vec<(u64, Vec<u8>)>;

/// What a refusal asks of the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A refusal of a frame already sealed again, or of one never sent.
    Stale,
    /// The rotation is owed under the current key: pay it, then resend.
    Settle,
    /// The rotation is owed and a payment is already running: wait for it.
    Wait,
    /// The current key is newer than the one refused: resend now.
    Resend,
}

/// The sent-frame log and the debt of one document.
#[derive(Debug, Default)]
pub(crate) struct Rekey {
    /// Updates sent and not yet echoed, in the order they left.
    sent: VecDeque<SentFrame>,
    /// The document owes a rotation this client pays before it sends.
    owed: bool,
    /// A payment is running.
    in_flight: bool,
    /// Lowest refused clock not yet sent again. Every frame from it on is free.
    refused_from: Option<u64>,
    /// Edits made while the rotation is owed, sealed once it lands.
    waiting: Vec<Vec<u8>>,
    /// The relay refused a delta by name. It holds an edit this client made,
    /// so no later delta leaves until the document reopens (`SYNC-156`).
    refused: bool,
}

impl Rekey {
    /// Records an update just sent. Returns `true` when the cap forgot one.
    pub(crate) fn record_sent(&mut self, sent: SentFrame) -> bool {
        self.sent.push_back(sent);
        let over = self.sent.len() > UNCONFIRMED_LIMIT;
        if over {
            self.sent.pop_front();
        }
        over
    }

    /// The relay echoed our update signed `signature`: it and every delta
    /// that left before it landed. An echo of a delta not here, such as
    /// another session's at the same clock, confirms nothing (`SYNC-148`).
    pub(crate) fn confirm(&mut self, signature: &Signature) {
        let Some((at, clock)) = self.position(signature) else {
            return;
        };
        self.sent.drain(..=at);
        if self.refused_from.is_some_and(|c| c <= clock) {
            self.refused_from = None;
        }
    }

    /// The relay refused the delta signed `signature` for its clock. Returns
    /// its body and the body of every delta that left after it, in order, to
    /// seal again at the next clocks. The relay dropped those later deltas
    /// behind its fence (`SRV-169`, `SYNC-147`). `None` when no delta here
    /// carries that signature: a refusal of a delta already sent again.
    pub(crate) fn clock_refused(&mut self, signature: &Signature) -> Option<Vec<Vec<u8>>> {
        let (at, from) = self.position(signature)?;
        if self.refused_from.is_some_and(|c| c >= from) {
            self.refused_from = None;
        }
        Some(self.sent.drain(at..).map(|f| f.payload).collect())
    }

    /// The relay refused the delta signed `signature` before sequencing it
    /// (`SYNC-155`). It and every delta that left after it are dropped: those
    /// build on it, and the relay took no clock for it. Returns whether this
    /// is the first refusal, the one to report.
    pub(crate) fn delta_refused(&mut self, signature: &Signature) -> bool {
        if let Some((at, _)) = self.position(signature) {
            self.sent.truncate(at);
        }
        self.refused_from = None;
        !std::mem::replace(&mut self.refused, true)
    }

    /// Whether a delta was refused by name, so nothing more may leave.
    pub(crate) fn holds_refused(&self) -> bool {
        self.refused
    }

    /// How many deltas wait for the relay's echo.
    #[cfg(test)]
    pub(crate) fn unconfirmed(&self) -> usize {
        self.sent.len()
    }

    /// The delta at `clock` was sealed again, under a new signature.
    pub(crate) fn resealed(&mut self, clock: u64, signature: Signature) {
        if let Some(frame) = self.sent.iter_mut().find(|f| f.clock == clock) {
            frame.signature = signature;
        }
    }

    /// The index and the clock of the delta signed `signature`.
    fn position(&self, signature: &Signature) -> Option<(usize, u64)> {
        self.sent
            .iter()
            .enumerate()
            .find(|(_, f)| f.signature == *signature)
            .map(|(at, f)| (at, f.clock))
    }

    /// The relay refused the frame at `clock`, sealed under `key_id`, while
    /// `current` is the key this client now holds.
    pub(crate) fn refused(&mut self, key_id: KeyId, clock: u64, current: KeyId) -> Refusal {
        let named = self
            .sent
            .iter()
            .any(|f| f.clock == clock && f.key_id == key_id);
        if !named {
            return Refusal::Stale;
        }
        self.refused_from = Some(self.refused_from.map_or(clock, |c| c.min(clock)));
        if key_id != current {
            return Refusal::Resend;
        }
        self.owed = true;
        self.begin_payment()
    }

    /// A members answer says the document owes a rotation (`SYNC-128`). The
    /// payment starts at the next edit, so a debt nobody writes after costs
    /// nothing.
    pub(crate) fn found_debt(&mut self) {
        self.owed = true;
    }

    /// Whether an edit must wait for the rotation.
    pub(crate) fn owes(&self) -> bool {
        self.owed
    }

    /// Whether every frame sent was acknowledged, and no edit waits on a
    /// payment (`RESID-009`). A refused delta never settles: the relay lacks
    /// an edit this client holds.
    pub(crate) fn settled(&self) -> bool {
        self.sent.is_empty() && self.waiting.is_empty() && !self.in_flight && !self.refused
    }

    /// Holds an edit made while the rotation is owed. Returns whether the
    /// caller starts a payment.
    pub(crate) fn hold(&mut self, payload: Vec<u8>) -> bool {
        self.waiting.push(payload);
        self.begin_payment() == Refusal::Settle
    }

    fn begin_payment(&mut self) -> Refusal {
        if self.in_flight {
            return Refusal::Wait;
        }
        self.in_flight = true;
        Refusal::Settle
    }

    /// The payment failed. The debt stands, and the next edit tries again.
    pub(crate) fn payment_failed(&mut self) {
        self.in_flight = false;
    }

    /// A key was installed: the debt is paid, by us or by someone else.
    /// Returns the refused frames to seal again under `current`, at their own
    /// clocks and in order, then the edits that waited.
    pub(crate) fn key_installed(&mut self, current: KeyId) -> (RefusedFrames, Vec<Vec<u8>>) {
        self.owed = false;
        self.in_flight = false;
        (
            self.take_refused(current),
            std::mem::take(&mut self.waiting),
        )
    }

    /// Every refused frame, marked as sealed under `current` from now on, so a
    /// late refusal of its earlier sealing reads as stale.
    pub(crate) fn take_refused(&mut self, current: KeyId) -> RefusedFrames {
        let Some(from) = self.refused_from.take() else {
            return Vec::new();
        };
        self.sent
            .iter_mut()
            .filter(|f| f.clock >= from)
            .map(|f| {
                f.key_id = current;
                (f.clock, f.payload.clone())
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn sent_len(&self) -> usize {
        self.sent.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> KeyId {
        KeyId::from_array([n; 16])
    }

    fn sig(n: u8) -> Signature {
        Signature([n; 64])
    }

    fn frame(clock: u64, signature: u8) -> SentFrame {
        SentFrame {
            clock,
            key_id: key(1),
            signature: sig(signature),
            payload: vec![clock as u8],
        }
    }

    /// A log with clocks 1 to 3 sent under key 1, each signed with its clock.
    fn three_sent() -> Rekey {
        let mut r = Rekey::default();
        for c in 1..=3 {
            r.record_sent(frame(c, c as u8));
        }
        r
    }

    /// A refusal under the current key pays the debt once and holds the edits
    /// made meanwhile. The refused frames are resent first, at their own clocks
    /// (`SYNC-128`, `SYNC-129`).
    #[test]
    fn a_refusal_under_the_current_key_pays_once_then_resends_at_the_same_clocks() {
        let mut r = three_sent();
        r.confirm(&sig(1));
        assert_eq!(r.refused(key(1), 2, key(1)), Refusal::Settle);
        assert_eq!(r.refused(key(1), 3, key(1)), Refusal::Wait);
        assert!(r.owes());
        assert!(!r.hold(vec![9]), "a payment already runs");

        let (resend, waiting) = r.key_installed(key(2));
        assert_eq!(resend, vec![(2, vec![2]), (3, vec![3])]);
        assert_eq!(waiting, vec![vec![9]]);
        assert!(!r.owes());
        // A late copy of a refusal names the key the frame no longer carries.
        assert_eq!(r.refused(key(1), 3, key(2)), Refusal::Stale);
    }

    /// A refusal of a key older than the current one resends at once: someone
    /// else already paid (`SYNC-129`).
    #[test]
    fn a_refusal_of_an_older_key_resends_without_paying() {
        let mut r = three_sent();
        assert_eq!(r.refused(key(1), 2, key(2)), Refusal::Resend);
        assert!(!r.owes());
        assert_eq!(r.take_refused(key(2)), vec![(2, vec![2]), (3, vec![3])]);
        assert_eq!(r.take_refused(key(2)), Vec::new(), "sent again once");
    }

    /// A debt found in a members answer holds the next edit and starts one
    /// payment. A failed payment lets the following edit try again (`SYNC-128`).
    #[test]
    fn a_debt_found_holds_the_next_edit_until_the_key_lands() {
        let mut r = Rekey::default();
        r.found_debt();
        assert!(r.hold(vec![1]));
        r.payment_failed();
        assert!(r.hold(vec![2]), "the next edit tries again");
        let (resend, waiting) = r.key_installed(key(2));
        assert!(resend.is_empty());
        assert_eq!(waiting, vec![vec![1], vec![2]]);
    }

    /// The echo of an update clears it and every earlier one, and the log never
    /// grows past its cap.
    #[test]
    fn the_sent_log_is_cleared_by_the_echo_and_bounded() {
        let mut r = three_sent();
        r.confirm(&sig(2));
        assert_eq!(r.sent_len(), 1);
        assert_eq!(r.refused(key(1), 1, key(1)), Refusal::Stale);
        let mut big = Rekey::default();
        let forgot = (0..=UNCONFIRMED_LIMIT as u64)
            .map(|c| big.record_sent(frame(c, 0)))
            .filter(|&f| f)
            .count();
        assert_eq!(forgot, 1);
        assert_eq!(big.sent_len(), UNCONFIRMED_LIMIT);
    }

    /// `SYNC-148`: the echo of another session's delta, at a clock this log
    /// holds and under the same key, confirms nothing. Only the signature of
    /// a delta sent here does.
    #[test]
    fn an_echo_at_the_same_clock_from_another_session_confirms_nothing() {
        let mut r = three_sent();
        r.confirm(&sig(99));
        assert_eq!(r.sent_len(), 3, "a foreign signature takes nothing");
        r.confirm(&sig(3));
        assert_eq!(r.sent_len(), 0);
    }

    /// `SYNC-147`: a delta refused for its clock comes back with every delta
    /// that left after it, in order. A second refusal of one of them, sent
    /// before the relay read the answer, is stale.
    #[test]
    fn a_clock_refusal_returns_the_delta_and_every_later_one() {
        let mut r = three_sent();
        assert_eq!(r.clock_refused(&sig(2)), Some(vec![vec![2], vec![3]]));
        assert_eq!(r.sent_len(), 1, "the delta before it stays unconfirmed");
        assert_eq!(r.clock_refused(&sig(3)), None, "already taken back");
    }
}

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
// Pure state, no IO: the caller seals, sends and spawns.
// ============================================================================

use std::collections::VecDeque;

use ambre_core::KeyId;

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
    /// The plaintext, kept to seal it again under another key.
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
    /// Updates sent and not yet echoed, in clock order.
    sent: VecDeque<SentFrame>,
    /// The document owes a rotation this client pays before it sends.
    owed: bool,
    /// A payment is running.
    in_flight: bool,
    /// Lowest refused clock not yet sent again. Every frame from it on is free.
    refused_from: Option<u64>,
    /// Edits made while the rotation is owed, sealed once it lands.
    waiting: Vec<Vec<u8>>,
}

impl Rekey {
    /// Records an update just sent. Returns `true` when the cap forgot one.
    pub(crate) fn record_sent(&mut self, clock: u64, key_id: KeyId, payload: Vec<u8>) -> bool {
        self.sent.push_back(SentFrame {
            clock,
            key_id,
            payload,
        });
        let over = self.sent.len() > UNCONFIRMED_LIMIT;
        if over {
            self.sent.pop_front();
        }
        over
    }

    /// The relay echoed our update at `clock`: it and every earlier one landed.
    pub(crate) fn confirm_through(&mut self, clock: u64) {
        while self.sent.front().is_some_and(|f| f.clock <= clock) {
            self.sent.pop_front();
        }
        if self.refused_from.is_some_and(|c| c <= clock) {
            self.refused_from = None;
        }
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

    /// A log with clocks 1 to 3 sent under key 1.
    fn three_sent() -> Rekey {
        let mut r = Rekey::default();
        for c in 1..=3 {
            r.record_sent(c, key(1), vec![c as u8]);
        }
        r
    }

    /// A refusal under the current key pays the debt once and holds the edits
    /// made meanwhile. The refused frames are resent first, at their own clocks
    /// (`SYNC-128`, `SYNC-129`).
    #[test]
    fn a_refusal_under_the_current_key_pays_once_then_resends_at_the_same_clocks() {
        let mut r = three_sent();
        r.confirm_through(1);
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
        r.confirm_through(2);
        assert_eq!(r.sent_len(), 1);
        assert_eq!(r.refused(key(1), 1, key(1)), Refusal::Stale);
        let mut big = Rekey::default();
        let forgot = (0..=UNCONFIRMED_LIMIT as u64)
            .map(|c| big.record_sent(c, key(1), Vec::new()))
            .filter(|&f| f)
            .count();
        assert_eq!(forgot, 1);
        assert_eq!(big.sent_len(), UNCONFIRMED_LIMIT);
    }
}

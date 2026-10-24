//! Counters that tell a late answer from the current one.
//!
//! A timer and a request both carry the number they were armed or asked
//! under. The answer is live only while that number is the latest one. Each
//! kind of timer counts on its own axis, so each counter is its own type. A
//! replay ticket compared to a resend ticket does not compile, and a listing
//! counts on one width wherever it travels.

ambre_core::define_generation! {
    /// The ticket of a replay timer, armed when the relay stopped a replay
    /// (`SYNC-153`).
    pub(crate) struct ReplayTicket;
    /// The ticket of a resend timer, armed when the relay failed to sequence
    /// a delta (`SYNC-159`).
    pub(crate) struct ResendTicket;
    /// The number of a listing asked of the relay. Two listings can cross on
    /// the way, and only the answer to the latest one counts (`SYNC-037`).
    pub(crate) struct RequestGeneration;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_counter_only_grows() {
        let first = ReplayTicket::ZERO.next();
        assert!(first > ReplayTicket::ZERO);
        assert!(first.next() > first);
        assert_eq!(ReplayTicket::default(), ReplayTicket::ZERO);
        assert_eq!(ResendTicket::default(), ResendTicket::ZERO);
        assert_eq!(ReplayTicket::nth(2), first.next());
        assert_eq!(ResendTicket::nth(1), ResendTicket::ZERO.next());
        assert_eq!(RequestGeneration::nth(1), RequestGeneration::ZERO.next());
    }
}

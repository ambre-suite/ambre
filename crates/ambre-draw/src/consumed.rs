// ambre-draw/src/consumed.rs
//! The answer of an input handler: whether it consumed the event.
//!
//! A view, an editor or a host returns [`Consumed::Yes`] when it took the
//! event, and the caller stops there: the host prevents the browser default.
//! [`Consumed::No`] leaves the event to the next handler, and at the end to
//! the browser (`ambre-spec-text-input.md` section 2). A bare `bool` said the
//! same, but a reader of `true` could not tell it from "changed" or "found".

/// Whether an input handler consumed the event.
///
/// The variant that was `false` comes first and is the default.
#[must_use = "a handler that consumed the event stops it; dropping the answer lets it propagate"]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Consumed {
    /// The handler left the event to the next one.
    #[default]
    No,
    /// The handler took the event, and nothing else sees it.
    Yes,
}

impl Consumed {
    /// The answer of a handler whose consumption is a fact it computed, such
    /// as "the scroll moved" or "the press landed on a control".
    pub const fn yes_if(taken: bool) -> Self {
        if taken { Self::Yes } else { Self::No }
    }

    /// Whether the handler took the event.
    #[must_use]
    pub const fn is_yes(self) -> bool {
        matches!(self, Self::Yes)
    }

    /// This answer when it is [`Consumed::Yes`], else the answer of the next
    /// handler. The next handler runs only when this one left the event.
    pub fn or_else(self, next: impl FnOnce() -> Self) -> Self {
        match self {
            Self::Yes => Self::Yes,
            Self::No => next(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Consumed;

    #[test]
    fn or_else_runs_the_next_handler_only_when_the_event_is_left() {
        let mut ran = false;
        assert_eq!(
            Consumed::Yes.or_else(|| {
                ran = true;
                Consumed::No
            }),
            Consumed::Yes
        );
        assert!(!ran);
        assert_eq!(Consumed::No.or_else(|| Consumed::Yes), Consumed::Yes);
        assert_eq!(Consumed::No.or_else(|| Consumed::No), Consumed::No);
    }

    #[test]
    fn yes_if_and_is_yes_round_trip() {
        for taken in [Consumed::No, Consumed::Yes].map(Consumed::is_yes) {
            assert_eq!(Consumed::yes_if(taken).is_yes(), taken);
        }
        assert_eq!(Consumed::default(), Consumed::No);
    }
}

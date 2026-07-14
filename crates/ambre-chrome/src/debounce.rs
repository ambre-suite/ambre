// ambre-chrome/src/debounce.rs
// ============================================================================
// A tiny debounce helper for chrome controls that must coalesce a burst of rapid
// edits into a single downstream action -- e.g. dragging the Toile style
// inspector's opacity slider should emit ONE `RestyleElements`, not one per
// pixel (`toile-ui sec.5quinquies.B`, "edits are debounced").
//
// It wraps leptos' `set_timeout_with_handle` (the same timer facility the toast
// host uses): each `call` cancels the still-pending timer and reschedules, so the
// closure runs only once the caller has been quiet for `delay`. The pending
// handle lives in a leptos `StoredValue`, so `Debouncer` is `Copy + Send + Sync`
// and drops straight into the `Callback` / event closures a component hands to
// `ColorPopover` and friends (leptos requires those be `Send + Sync`).
// ============================================================================

use std::time::Duration;

/// A rescheduling debouncer. `Copy`, so a component keeps one and hands it to
/// several event handlers.
#[derive(Clone, Copy)]
pub struct Debouncer {
    // The handle of the timer currently in flight, replaced on each call. Arena-
    // stored so the whole struct stays `Copy + Send + Sync`.
    #[cfg(target_arch = "wasm32")]
    pending: leptos::prelude::StoredValue<Option<leptos::prelude::TimeoutHandle>>,
}

impl Debouncer {
    /// A fresh debouncer with no timer pending. Must be called inside a reactive
    /// owner (a component body), like every leptos stored value.
    pub fn new() -> Self {
        Self {
            #[cfg(target_arch = "wasm32")]
            pending: leptos::prelude::StoredValue::new(None),
        }
    }

    /// Schedule `f` to run after `delay`, cancelling any still-pending call. On a
    /// non-wasm host (tests) there is no event loop, so `f` runs immediately -- the
    /// debounce is a wasm-only concern and host tests want the effect synchronously.
    #[cfg(target_arch = "wasm32")]
    pub fn call(&self, delay: Duration, f: impl FnOnce() + 'static) {
        use leptos::prelude::*;
        if let Some(h) = self.pending.get_value() {
            h.clear();
        }
        let handle = set_timeout_with_handle(f, delay).ok();
        self.pending.set_value(handle);
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn call(&self, _delay: Duration, f: impl FnOnce() + 'static) {
        f();
    }
}

impl Default for Debouncer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn host_call_runs_immediately() {
        // On the host the closure fires synchronously, so the flag flips at once.
        let d = Debouncer::new();
        let flag = Rc::new(RefCell::new(false));
        let f = flag.clone();
        d.call(Duration::from_millis(100), move || *f.borrow_mut() = true);
        assert!(*flag.borrow());
    }
}

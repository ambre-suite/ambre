// ambre-canvas/src/fatal.rs
// ============================================================================
// Process-global fatal latch.
//
// When the application suffers an unrecoverable error -- a panic on the Logic
// thread, or a Sync-worker death surfaced as a channel disconnect -- the UI
// (render) thread is usually still alive and pumping requestAnimationFrame. It
// reads this latch every frame and, once set, paints a terminal screen on the
// canvas instead of holding the last good frame (see `host::FrameHost`).
//
// The UI and Logic threads share one wasm linear memory (`wasm_thread`), so a
// plain process-global is visible across both. The Sync worker is a SEPARATE
// wasm instance and does NOT share this memory; its death reaches us only as a
// `UiEvent::FatalError` / channel `Disconnected`, which the view forwards here.
//
// The matching JS `window.ambreFatal` DOM overlay (index.html) stays as the
// last-resort fallback for a panic on the UI thread itself, where the rAF loop
// dies and the canvas can no longer repaint.
// ============================================================================

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

static FATAL: AtomicBool = AtomicBool::new(false);
static FATAL_MSG: Mutex<String> = Mutex::new(String::new());

/// Latch the process into the fatal state with a human-readable `message`.
/// Idempotent: the first message wins (the root cause), later calls are ignored.
/// Safe to call from any thread that shares this instance's memory (UI/Logic).
pub fn signal_fatal(message: &str) {
    // `swap` returns the previous value: if it was already `true`, a fatal has
    // been latched and we keep the original (root-cause) message.
    if FATAL.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Ok(mut slot) = FATAL_MSG.lock() {
        *slot = message.to_owned();
    }
}

/// The fatal message if the process has latched fatal, else `None`.
pub fn fatal_message() -> Option<String> {
    FATAL
        .load(Ordering::SeqCst)
        .then(|| FATAL_MSG.lock().ok().map(|m| m.clone()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_message_wins_and_latches() {
        assert_eq!(fatal_message(), None);
        signal_fatal("root cause");
        signal_fatal("later noise");
        assert_eq!(fatal_message().as_deref(), Some("root cause"));
    }
}

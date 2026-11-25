//! The Web Lock that orders the renewals of the kept session across the tabs
//! of an origin (`SRV-330`). A renewal dropped mid-way releases it
//! (`SRV-338`).
//!
//! The lock manager exists in a browser alone, so this test runs under
//! `wasm-bindgen-test-runner` in headless Chrome (`just test-wasm`). On the
//! host target the whole file is empty.
#![cfg(target_arch = "wasm32")]
#![expect(
    clippy::expect_used,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use futures_util::future::{Either, select};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

/// A promise that resolves after `ms` milliseconds of the page clock.
fn after(ms: u32) -> JsFuture {
    let timer =
        js_sys::Function::new_with_args("ms", "return new Promise(r => setTimeout(r, ms));");
    let promise = timer
        .call1(&JsValue::NULL, &JsValue::from(ms))
        .expect("the timer starts");
    JsFuture::from(promise.unchecked_into::<js_sys::Promise>())
}

/// `SRV-330`: two renewals that ask for the lock at once run one after the
/// other. The second starts only once the first released the lock, even
/// though the first waits on a timer while it holds it.
#[wasm_bindgen_test]
async fn two_renewals_hold_the_lock_in_turn() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let section = |name: &'static str, ms: u32| {
        let steps = Rc::clone(&steps);
        ambre_sync::strictly_exclusive(async move {
            steps.borrow_mut().push(format!("{name} in"));
            after(ms).await.expect("the timer fires");
            steps.borrow_mut().push(format!("{name} out"));
        })
    };
    let (first, second) = futures_util::join!(section("first", 50), section("second", 0));
    first.expect("the lock is granted");
    second.expect("the lock is granted");
    assert_eq!(
        *steps.borrow(),
        ["first in", "first out", "second in", "second out"]
    );
}

/// `SRV-338`: a renewal dropped while it holds the lock releases it. The
/// first section never ends, and the caller drops it once it holds the lock.
/// The second section is then granted the lock before its deadline. A lock
/// kept by the dropped section would hold the second until the worker ends.
#[wasm_bindgen_test]
async fn a_renewal_dropped_mid_way_releases_the_lock() {
    let entered = Rc::new(Cell::new(false));
    let mut first = {
        let entered = Rc::clone(&entered);
        Box::pin(ambre_sync::strictly_exclusive(async move {
            entered.set(true);
            std::future::pending::<()>().await;
        }))
    };
    let raced = select(first.as_mut(), after(50)).await;
    assert!(
        matches!(raced, Either::Right(_)),
        "the first section never ends"
    );
    assert!(entered.get(), "the first section holds the lock");
    drop(first);
    let second = Box::pin(ambre_sync::strictly_exclusive(async {}));
    let outcome = select(second, after(2_000)).await;
    assert!(
        matches!(outcome, Either::Left((Ok(()), _))),
        "the lock of the dropped section was released"
    );
}

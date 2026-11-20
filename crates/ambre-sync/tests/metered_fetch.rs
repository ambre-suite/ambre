//! The metered read of a `fetch` body, on a browser stream (`SYNC-278`).
//!
//! The read runs only on a `web_sys::ReadableStream`, so this test runs under
//! `wasm-bindgen-test-runner` in headless Chrome (`just test-wasm`). On the
//! host target the whole file is empty.
#![cfg(target_arch = "wasm32")]
#![expect(
    clippy::expect_used,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_core::{Count, Octets};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

/// A stream that yields `chunks` chunks of 4 bytes each, or never ends when
/// `chunks` is negative. The second value counts the pulls, and records
/// whether the reader cancelled the stream.
fn stream(chunks: i32) -> (web_sys::ReadableStream, js_sys::Object) {
    let make = js_sys::Function::new_with_args(
        "chunks",
        "const seen = { pulls: 0, cancelled: false };
         const stream = new ReadableStream({
           pull(controller) {
             if (chunks >= 0 && seen.pulls >= chunks) { controller.close(); return; }
             seen.pulls += 1;
             controller.enqueue(new Uint8Array(4));
           },
           cancel() { seen.cancelled = true; },
         }, { highWaterMark: 0 });
         return [stream, seen];",
    );
    let made: js_sys::Array = make
        .call1(&JsValue::NULL, &JsValue::from(chunks))
        .expect("the stream builds")
        .unchecked_into();
    (made.get(0).unchecked_into(), made.get(1).unchecked_into())
}

/// A field of the record [`stream`] returns.
fn seen(record: &js_sys::Object, field: &str) -> JsValue {
    js_sys::Reflect::get(record, &JsValue::from_str(field)).expect("the field reads")
}

/// `SYNC-278`: a body with no stated length that never ends stops at the
/// chunk past its ceiling. The reader cancels the stream and pulls no chunk
/// past that one. Before the metered read, the whole body was read first.
#[wasm_bindgen_test]
async fn a_body_past_its_ceiling_stops_at_the_chunk_that_passes_it() {
    let (body, record) = stream(-1);
    let read = ambre_sync::metered_body_len(None, body, Count::<Octets>::new(10)).await;
    assert_eq!(read, None, "the read stops");
    assert_eq!(seen(&record, "pulls").as_f64(), Some(3.0), "4 + 4 + 4 > 10");
    assert_eq!(seen(&record, "cancelled").as_bool(), Some(true));
}

/// `SYNC-278`: a stated length past the ceiling stops the read before any
/// chunk is pulled, though the body itself is short.
#[wasm_bindgen_test]
async fn a_stated_length_past_its_ceiling_reads_no_byte() {
    let (body, record) = stream(1);
    let read = ambre_sync::metered_body_len(
        Some(Count::<Octets>::new(11)),
        body,
        Count::<Octets>::new(10),
    )
    .await;
    assert_eq!(read, None);
    assert_eq!(seen(&record, "pulls").as_f64(), Some(0.0));
}

/// A body up to its ceiling reads whole, a lying stated length included.
#[wasm_bindgen_test]
async fn a_body_up_to_its_ceiling_reads_whole() {
    let (body, _) = stream(2);
    let read = ambre_sync::metered_body_len(
        Some(Count::<Octets>::new(1)),
        body,
        Count::<Octets>::new(8),
    )
    .await;
    assert_eq!(read, Some(Count::<Octets>::new(8)));
}

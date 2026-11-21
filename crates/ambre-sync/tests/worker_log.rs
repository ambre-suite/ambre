//! The log of the Sync worker reaches the browser itself (`SYNC-305`).
//!
//! The worker hands each message to `console.log` and to the performance
//! profile, with no `tracing` layer between. Both are browser APIs, so this
//! test runs under `wasm-bindgen-test-runner` in headless Chrome
//! (`just test-wasm`). On the host target the whole file is empty.
#![cfg(target_arch = "wasm32")]
#![expect(
    clippy::expect_used,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_bus::SyncCommand;
use ambre_core::{Id, Payload, id::Document};
use ambre_sync::{LogicSink, RelayHost, SyncState, Url};
use js_sys::{Array, Function, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

fn global(name: &str) -> JsValue {
    Reflect::get(&js_sys::global(), &name.into()).unwrap_or(JsValue::UNDEFINED)
}

/// An edit sent before the session is ready logs at INFO and buffers, with
/// no request: a log the worker writes on its own.
fn log_an_early_edit() {
    let (sink, _events) = LogicSink::channel();
    let mut state = SyncState::new(sink, Url::<RelayHost>::new("localhost".to_owned()));
    state.handle_command(SyncCommand::SendUpdate {
        document_id: Id::<Document>::from_bytes([41; 16]),
        payload: Payload::new(vec![1, 2, 3]),
        summary: None,
    });
}

#[wasm_bindgen_test]
fn a_worker_message_reaches_console_log_with_its_three_styles() {
    let console = global("console");
    let original = Reflect::get(&console, &"log".into()).expect("console.log");
    let calls = Array::new();
    let recorder = Function::new_with_args("...args", "this.push(args)").bind0(&calls);
    Reflect::set(&console, &"log".into(), &recorder).expect("the recorder");
    log_an_early_edit();
    Reflect::set(&console, &"log".into(), &original).expect("console.log back");

    let lines: Vec<Array> = calls.iter().map(JsCast::unchecked_into).collect();
    let line = lines
        .iter()
        .find(|args| {
            args.get(0)
                .as_string()
                .is_some_and(|text| text.contains("received SendUpdate"))
        })
        .expect("a line of the edit");
    let arg = |i| line.get(i).as_string().unwrap_or_default();
    assert_eq!(line.length(), 4);
    assert!(arg(0).starts_with("%cINFO%c "), "{}", arg(0));
    assert!(arg(0).contains(".rs:"), "{}", arg(0));
    assert_eq!(arg(1), "color: whitesmoke; background: #444");
    assert_eq!(arg(2), "color: gray; font-style: italic");
    assert_eq!(arg(3), "color: inherit");
}

#[wasm_bindgen_test]
fn a_worker_message_leaves_its_measure_in_the_browser_profile() {
    log_an_early_edit();
    let performance = global("performance");
    let by_type: Function = Reflect::get(&performance, &"getEntriesByType".into())
        .expect("getEntriesByType")
        .unchecked_into();
    let measures: Array = by_type
        .call1(&performance, &"measure".into())
        .expect("the measures")
        .unchecked_into();
    let names: Vec<String> = measures
        .iter()
        .filter_map(|entry| Reflect::get(&entry, &"name".into()).ok()?.as_string())
        .collect();
    assert!(
        names
            .iter()
            .any(|name| name.starts_with("INFO ") && name.contains("received SendUpdate")),
        "{names:?}"
    );
}

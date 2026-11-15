# Patched copy of wasm_thread 0.3.3

Upstream: https://github.com/chemicstry/wasm_thread (crate `wasm_thread` 0.3.3),
licensed Apache-2.0 OR MIT by Jurgis Balciunas. The workspace uses this copy
through `[patch.crates-io]` in the root `Cargo.toml`.

The copy holds `src/`, `Cargo.toml`, `LICENSE-APACHE`, `LICENSE-MIT` and
`README.md` of the published crate. The examples, the tests, the build scripts,
`rust-toolchain.toml`, `rustfmt.toml`, `Cargo.lock` and `Cargo.toml.orig` are
left out.

## Why

The page that loads the client carries a content policy that admits the
compile of a wasm module and refuses an evaluated string (`SRV-260`,
`specs/1-Foundations/ambre-spec-server.md`). The published crate calls
`js_sys::eval` four ways: to read `self`, to find the path of the glue from a
stack, and to install a polyfill of module workers before each spawn. Each call
throws under the policy, and the crate unwraps the result, so no thread starts.

The published crate also builds the script of each thread as a `blob:` address
even when the caller names a script, because it evaluates the default eagerly.
The page names a script it ships, so the policy admits workers of its own
origin alone.

## The change

Each change against the published crate, read on 2026-11-14 by a `diff -r`
against the crate from crates.io. Each changed site carries the words
"Patched in by Ambre" in a comment.

* `src/wasm32/mod.rs`:
  * `WorkerMessage::post` reads the global scope through `js_sys::global`;
  * `Builder::spawn_for_context` builds the default script only when the
    builder names none (`unwrap_or_else`);
  * the same function no longer evaluates the polyfill of module workers:
    every browser the client targets runs a module worker.
* `src/wasm32/utils.rs`:
  * `available_parallelism` and `is_web_worker_thread` read the global scope
    through `js_sys::global`;
  * `get_wasm_bindgen_shim_script_path` reads the stack of an error the glue
    constructs, and `script_url_in` matches the same frame as the regular
    expression of the upstream script.
* `src/wasm32/js/script_path.js` and `src/wasm32/js/module_workers_polyfill.min.js`
  are deleted: nothing reads them.

Drop the copy once the published crate reads its global scope without eval and
builds no default script for a builder that names one.

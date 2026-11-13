//! The loader: the one way the Logic thread runs a conversion.
//!
//! In the browser, [`Converter::load`] instantiates the module bytes the Sync
//! worker fetched and checked, one module per family. The loader reads no
//! format: a family that joins the list changes the request types and its
//! engine, never this file. Every import of the module is a trap, so the
//! module computes on its own memory and reaches nothing else (`ARCH-079`).
//! On the host the engine is linked in, and the loader calls it in place.
//! Both paths encode the request and decode the answer, so the host tier runs
//! the interface the browser runs.

use ambre_core::{ConverterFamily, ConverterModule, Payload};

use crate::{Conversion, ConvertError, Converted, EncodedAnswer, EncodedRequest};

/// A loaded converter module, ready to run the conversions of its family one
/// at a time.
pub struct Converter {
    family: ConverterFamily,
    inner: Inner,
}

/// Where the engine runs.
enum Inner {
    /// The engine is linked into this build: the host.
    #[cfg(all(not(target_arch = "wasm32"), feature = "engine"))]
    Linked,
    /// The engine runs in a module instantiated from fetched bytes.
    #[cfg(all(target_arch = "wasm32", feature = "loader"))]
    Fetched(wasm::Instance),
}

impl Converter {
    /// The converter of `family` that `module` holds. The bytes must already
    /// be checked against the build's digest of that family: the Sync worker
    /// checks them before they reach the Logic thread (`ARCH-080`).
    ///
    /// On the host every engine is linked in and the bytes are not read.
    ///
    /// # Errors
    ///
    /// [`ConvertError::Stopped`] when the browser refuses to compile or
    /// instantiate the bytes, or when the module imports anything but a
    /// function. [`ConvertError::Absent`] on a host build without the engine.
    pub fn load(
        family: ConverterFamily,
        module: &Payload<ConverterModule>,
    ) -> Result<Self, ConvertError> {
        #[cfg(all(target_arch = "wasm32", feature = "loader"))]
        {
            wasm::Instance::new(module).map(|instance| Self {
                family,
                inner: Inner::Fetched(instance),
            })
        }
        #[cfg(all(not(target_arch = "wasm32"), feature = "engine"))]
        {
            let _ = module;
            Ok(Self {
                family,
                inner: Inner::Linked,
            })
        }
        #[cfg(not(any(
            all(target_arch = "wasm32", feature = "loader"),
            all(not(target_arch = "wasm32"), feature = "engine")
        )))]
        {
            let _ = (family, module);
            Err(ConvertError::Absent)
        }
    }

    /// The family this converter serves.
    #[must_use]
    pub const fn family(&self) -> ConverterFamily {
        self.family
    }

    /// The bytes the module's memory spans now, or `None` on the host, where
    /// the engine runs in the process's own heap. A WebAssembly memory grows
    /// and never shrinks, so after a large decode the converter thread drops
    /// the instance and loads another (`ARCH-099`).
    #[must_use]
    pub fn footprint(&self) -> Option<ambre_core::Count<ambre_core::Octets>> {
        match self.inner {
            #[cfg(all(not(target_arch = "wasm32"), feature = "engine"))]
            Inner::Linked => None,
            #[cfg(all(target_arch = "wasm32", feature = "loader"))]
            Inner::Fetched(ref instance) => Some(instance.footprint()),
        }
    }

    /// Run one conversion and read its answer.
    ///
    /// # Errors
    ///
    /// | Error | When |
    /// | :--- | :--- |
    /// | [`ConvertError::Refused`] | the converter refused, in its own words |
    /// | [`ConvertError::Stopped`] | the module trapped |
    /// | [`ConvertError::Malformed`] | the answer does not decode |
    /// | [`ConvertError::OtherFamily`] | the conversion is of another family, and never reaches the module |
    pub fn convert(&self, conversion: &Conversion) -> Result<Converted, ConvertError> {
        if conversion.family() != self.family {
            return Err(ConvertError::OtherFamily {
                held: self.family,
                asked: conversion.family(),
            });
        }
        let request = crate::encode_request(conversion)?;
        crate::decode_answer(&self.answer(&request)?)
    }

    /// The answer bytes the engine wrote for `request`.
    fn answer(
        &self,
        request: &Payload<EncodedRequest>,
    ) -> Result<Payload<EncodedAnswer>, ConvertError> {
        // A build with neither half holds no `Inner` at all, and the match
        // below has no arm: such a converter cannot be built.
        #[cfg(not(any(
            all(target_arch = "wasm32", feature = "loader"),
            all(not(target_arch = "wasm32"), feature = "engine")
        )))]
        let _ = request;
        match self.inner {
            #[cfg(all(not(target_arch = "wasm32"), feature = "engine"))]
            Inner::Linked => Ok(crate::engine::run(request)),
            #[cfg(all(target_arch = "wasm32", feature = "loader"))]
            Inner::Fetched(ref instance) => instance.run(request),
        }
    }
}

#[cfg(all(target_arch = "wasm32", feature = "loader"))]
mod wasm {
    //! The module as a WebAssembly instance of the Logic worker.
    //!
    //! Compiled and instantiated synchronously. The Logic thread blocks on its
    //! queues and runs no browser event loop. A worker may compile a module of
    //! any size synchronously.

    use js_sys::{Array, Function, Object, Reflect, Uint8Array, WebAssembly};
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};

    use ambre_core::{ConverterModule, Payload};

    use crate::{ConvertError, EncodedAnswer, EncodedRequest};

    /// The instance, its memory, and the trap its imports call.
    pub(super) struct Instance {
        exports: Object,
        memory: WebAssembly::Memory,
        /// Held so the trap outlives every call into the module.
        _trap: Closure<dyn Fn() -> Result<(), JsValue>>,
    }

    /// The words of a JavaScript error, or a fixed text for a value that
    /// carries none.
    fn words(error: &JsValue) -> String {
        error
            .dyn_ref::<js_sys::Error>()
            .map(|e| String::from(e.message()))
            .or_else(|| error.as_string())
            .unwrap_or_else(|| "an error with no message".to_owned())
    }

    fn stopped(error: JsValue) -> ConvertError {
        ConvertError::Stopped(words(&error))
    }

    impl Instance {
        pub(super) fn new(bytes: &Payload<ConverterModule>) -> Result<Self, ConvertError> {
            let module = WebAssembly::Module::new(&Uint8Array::from(bytes.as_bytes()).into())
                .map_err(stopped)?;
            let trap: Closure<dyn Fn() -> Result<(), JsValue>> = Closure::new(|| {
                Err(js_sys::Error::new("the converter module may not call out").into())
            });
            let imports = Object::new();
            for import in WebAssembly::Module::imports(&module).iter() {
                let field =
                    |name: &str| Reflect::get(&import, &JsValue::from_str(name)).map_err(stopped);
                let (namespace, name, kind) = (field("module")?, field("name")?, field("kind")?);
                if kind.as_string().as_deref() != Some("function") {
                    return Err(ConvertError::Stopped(format!(
                        "the module imports a {} it may not hold",
                        kind.as_string().unwrap_or_default()
                    )));
                }
                let table = match Reflect::get(&imports, &namespace).map_err(stopped)? {
                    table if table.is_object() => table,
                    _ => {
                        let table: JsValue = Object::new().into();
                        Reflect::set(&imports, &namespace, &table).map_err(stopped)?;
                        table
                    }
                };
                Reflect::set(&table, &name, trap.as_ref()).map_err(stopped)?;
            }
            let instance = WebAssembly::Instance::new(&module, &imports).map_err(stopped)?;
            let exports = instance.exports();
            let memory = Reflect::get(&exports, &JsValue::from_str("memory"))
                .map_err(stopped)?
                .dyn_into::<WebAssembly::Memory>()
                .map_err(|_| ConvertError::Stopped("the module exports no memory".to_owned()))?;
            Ok(Self {
                exports,
                memory,
                _trap: trap,
            })
        }

        /// Call the export `name` with `args`, and read its result as an
        /// address or a length.
        fn call(&self, name: &str, args: &Array) -> Result<u32, ConvertError> {
            let function = Reflect::get(&self.exports, &JsValue::from_str(name))
                .map_err(stopped)?
                .dyn_into::<Function>()
                .map_err(|_| ConvertError::Stopped(format!("the module exports no `{name}`")))?;
            let value = function.apply(&JsValue::NULL, args).map_err(stopped)?;
            // An `i32` result reads as a negative number past 2 GiB; the
            // module's addresses and lengths are unsigned.
            Ok(value.as_f64().map_or(0, |n| n as i64 as u32))
        }

        /// The bytes the module's memory spans now.
        pub(super) fn footprint(&self) -> ambre_core::Count<ambre_core::Octets> {
            ambre_core::Count::of(Uint8Array::new(&self.memory.buffer()).length() as usize)
        }

        /// Write `request` into the module, run it, and copy the answer out.
        pub(super) fn run(
            &self,
            request: &Payload<EncodedRequest>,
        ) -> Result<Payload<EncodedAnswer>, ConvertError> {
            let request = request.as_bytes();
            let len = u32::try_from(request.len())
                .map_err(|_| ConvertError::Stopped("the request is past 4 GiB".to_owned()))?;
            let at = self.call("input_reserve", &Array::of1(&len.into()))?;
            // The view is taken after the call: a reserve may grow the
            // memory, which detaches an older view.
            self.window(at, len)?.copy_from(request);
            self.call("run", &Array::new())?;
            let at = self.call("output_ptr", &Array::new())?;
            let len = self.call("output_len", &Array::new())?;
            Ok(Payload::new(self.window(at, len)?.to_vec()))
        }

        /// The `len` bytes of the module's memory at `at`, refused past its
        /// end: a copy into a shorter view would panic the Logic thread.
        fn window(&self, at: u32, len: u32) -> Result<Uint8Array, ConvertError> {
            let memory = Uint8Array::new(&self.memory.buffer());
            match at.checked_add(len) {
                Some(end) if end <= memory.length() => Ok(memory.subarray(at, end)),
                _ => Err(ConvertError::Stopped(
                    "the module named bytes past its memory".to_owned(),
                )),
            }
        }
    }
}

/// The loader in the browser, on modules written by hand: what it refuses,
/// and what a call out of the module becomes (`ARCH-079`).
#[cfg(all(test, target_arch = "wasm32", feature = "loader"))]
mod wasm_tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// `(module (import "env" "m" (memory 1)))`: a module that asks for a
    /// memory of the page.
    const IMPORTS_A_MEMORY: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic, version
        0x02, 0x0a, 0x01, 0x03, b'e', b'n', b'v', 0x01, b'm', 0x02, 0x00, 0x01,
    ];

    /// A module whose `input_reserve` calls the function it imports, then
    /// answers 0. It exports its memory, as the converter does.
    const CALLS_OUT: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic, version
        // types: () -> (), (i32) -> (i32)
        0x01, 0x09, 0x02, 0x60, 0x00, 0x00, 0x60, 0x01, 0x7f, 0x01, 0x7f,
        // import env.f, type 0
        0x02, 0x09, 0x01, 0x03, b'e', b'n', b'v', 0x01, b'f', 0x00, 0x00,
        // one function, type 1
        0x03, 0x02, 0x01, 0x01, // one page of memory
        0x05, 0x03, 0x01, 0x00, 0x01, // exports: memory, input_reserve
        0x07, 0x1a, 0x02, 0x06, b'm', b'e', b'm', b'o', b'r', b'y', 0x02, 0x00, 0x0d, b'i', b'n',
        b'p', b'u', b't', b'_', b'r', b'e', b's', b'e', b'r', b'v', b'e', 0x00, 0x01,
        // code: call 0; i32.const 0; end
        0x0a, 0x08, 0x01, 0x06, 0x00, 0x10, 0x00, 0x41, 0x00, 0x0b,
    ];

    fn module(bytes: &[u8]) -> Payload<ConverterModule> {
        Payload::new(bytes.to_vec())
    }

    #[wasm_bindgen_test]
    fn a_module_that_imports_a_memory_is_refused() {
        assert!(matches!(
            Converter::load(ConverterFamily::Tables, &module(IMPORTS_A_MEMORY)),
            Err(ConvertError::Stopped(_))
        ));
    }

    #[wasm_bindgen_test]
    fn a_call_out_of_the_module_traps_into_an_error() {
        let converter = Converter::load(ConverterFamily::Tables, &module(CALLS_OUT))
            .expect("a function import loads");
        let answer = converter.convert(&crate::Conversion::WriteCsv {
            records: Vec::new(),
            delimiter: crate::CsvDelimiter::COMMA,
        });
        assert!(
            matches!(&answer, Err(ConvertError::Stopped(words)) if words.contains("may not call out")),
            "{answer:?}"
        );
    }

    #[wasm_bindgen_test]
    fn the_footprint_is_the_span_of_the_module_memory() {
        let converter = Converter::load(ConverterFamily::Tables, &module(CALLS_OUT))
            .expect("a function import loads");
        assert_eq!(
            converter.footprint(),
            Some(ambre_core::Count::new(64 * 1024))
        );
    }

    #[wasm_bindgen_test]
    fn bytes_that_are_no_module_are_refused() {
        assert!(matches!(
            Converter::load(ConverterFamily::Tables, &module(b"not wasm")),
            Err(ConvertError::Stopped(_))
        ));
    }
}

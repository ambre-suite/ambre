//! The module's exports, in the module build alone.
//!
//! Four numbers cross the boundary, and nothing else. The loader:
//!
//! 1. asks for an input buffer of a length;
//! 2. writes the request into the module's memory at the address it got;
//! 3. runs the conversion;
//! 4. reads the answer at the address and length the module gives.
//!
//! Every call takes or returns a `u32`, so no glue code is needed. The loader
//! calls the raw exports (`ambre-spec-architecture.md section 2`).
//!
//! The buffers live in thread-local cells of the module. The module runs one
//! conversion at a time. The loader writes the input only between two calls,
//! while no borrow of the cell is live.

use std::cell::RefCell;

use wasm_bindgen::prelude::wasm_bindgen;

thread_local! {
    /// The request the loader writes before it calls [`run`].
    static INPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// The answer [`run`] wrote, until the next conversion.
    static OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Size the input buffer to `len` bytes, zeroed, and return its address in
/// the module's memory.
#[wasm_bindgen]
pub fn input_reserve(len: u32) -> u32 {
    INPUT.with(|input| {
        let mut input = input.borrow_mut();
        input.clear();
        input.resize(len as usize, 0);
        input.as_ptr() as u32
    })
}

/// Run the conversion the input buffer holds, and keep its answer.
#[wasm_bindgen]
pub fn run() {
    let request =
        INPUT.with(|input| ambre_core::Payload::new(std::mem::take(&mut *input.borrow_mut())));
    let answer = crate::engine::run(&request).into_bytes();
    OUTPUT.with(|output| *output.borrow_mut() = answer);
}

/// The address of the last answer in the module's memory.
#[wasm_bindgen]
pub fn output_ptr() -> u32 {
    OUTPUT.with(|output| output.borrow().as_ptr() as u32)
}

/// The length of the last answer, in bytes.
#[wasm_bindgen]
pub fn output_len() -> u32 {
    OUTPUT.with(|output| output.borrow().len() as u32)
}

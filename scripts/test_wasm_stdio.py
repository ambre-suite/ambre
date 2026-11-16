"""Tests of the print gate of the Main instance (`ARCH-119`)."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import wasm_stdio  # noqa: E402


def leb(value: int) -> bytes:
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def vec(items: list[bytes]) -> bytes:
    return leb(len(items)) + b"".join(items)


def text(value: str) -> bytes:
    raw = value.encode()
    return leb(len(raw)) + raw


def section(kind: int, content: bytes) -> bytes:
    return bytes([kind]) + leb(len(content)) + content


def body(code: bytes) -> bytes:
    """A function body with no local, ended by `end`."""
    inner = leb(0) + code + b"\x0b"
    return leb(len(inner)) + inner


def module(bodies: list[bytes], names: dict[int, str] | None, imports: int = 1) -> bytes:
    """A module of one type `[] -> []`, `imports` imported functions and the bodies."""
    types = section(1, vec([b"\x60\x00\x00"]))
    imported = section(2, vec([text("env") + text(f"f{i}") + b"\x00\x00" for i in range(imports)]))
    functions = section(3, vec([b"\x00"] * len(bodies)))
    code = section(10, leb(len(bodies)) + b"".join(bodies))
    out = b"\0asm\x01\0\0\0" + types + imported + functions + code
    if names is not None:
        entries = vec([leb(index) + text(name) for index, name in sorted(names.items())])
        out += section(0, text("name") + b"\x01" + leb(len(entries)) + entries)
    return out


PRINT = "std[0123456789abcdef]::io::stdio::_print"
EPRINT = "std[0123456789abcdef]::io::stdio::_eprint"


class PrintCallers(unittest.TestCase):
    def test_a_direct_caller_of_each_print_function_is_found(self) -> None:
        wasm = module(
            [body(b""), body(b""), body(b"\x10\x01"), body(b"\x12\x02"), body(b"\x10\x00")],
            {0: "env::f0", 1: PRINT, 2: EPRINT, 3: "lyon[00aa]::log::<f32>",
             4: "loro::guard", 5: "ambre::quiet"},
        )
        self.assertEqual(wasm_stdio.print_callers(wasm), ["loro::guard", "lyon::log"])

    def test_a_call_byte_inside_an_immediate_is_no_call(self) -> None:
        # `i32.const 0x10` then a load whose offset is 1, then `drop`s.
        wasm = module([body(b""), body(b"\x41\x10\x1a\x41\x00\x28\x02\x01\x1a")],
                      {1: PRINT, 2: "std[01]::other"})
        self.assertEqual(wasm_stdio.print_callers(wasm), [])

    def test_a_call_inside_nested_blocks_and_atomics_is_found(self) -> None:
        code = b"\x02\x40\x41\x00\xfe\x10\x02\x00\x1a\x10\x01\x0b"
        wasm = module([body(b""), body(code)], {1: PRINT, 2: "std[01]::rt::helper"})
        self.assertEqual(wasm_stdio.print_callers(wasm), ["std::rt::helper"])

    def test_a_module_with_no_print_function_has_no_caller(self) -> None:
        wasm = module([body(b"\x10\x00")], {0: "std[01]::alloc", 1: "ambre::f"})
        self.assertEqual(wasm_stdio.print_callers(wasm), [])

    def test_a_module_with_no_name_section_fails(self) -> None:
        with self.assertRaises(wasm_stdio.WasmError):
            wasm_stdio.print_callers(module([body(b"")], None))

    def test_a_name_section_with_no_std_function_fails(self) -> None:
        with self.assertRaises(wasm_stdio.WasmError):
            wasm_stdio.print_callers(module([body(b"")], {1: "ambre::f"}))

    def test_a_mangled_name_fails(self) -> None:
        names = {1: PRINT, 2: "_RNvNtCs1_3std2io5print"}
        with self.assertRaises(wasm_stdio.WasmError):
            wasm_stdio.print_callers(module([body(b""), body(b"")], names))

    def test_an_unknown_instruction_fails(self) -> None:
        with self.assertRaises(wasm_stdio.WasmError):
            wasm_stdio.print_callers(module([body(b"\xff")], {1: PRINT}))

    def test_a_body_that_ends_inside_a_block_fails(self) -> None:
        with self.assertRaises(wasm_stdio.WasmError):
            wasm_stdio.print_callers(module([body(b"\x02\x40")], {1: PRINT}))


class Readable(unittest.TestCase):
    def test_a_name_drops_its_crate_hashes_generics_and_closure_numbers(self) -> None:
        name = ("<loro_internal[733855b123f716aa]::lock::LoroMutexGuardInner<core[679b]::option"
                "::Option<T>> as core[679b]::ops::drop::Drop>::drop")
        self.assertEqual(wasm_stdio.readable(name),
                         "<loro_internal::lock::LoroMutexGuardInner as core::ops::drop::Drop>::drop")

    def test_a_turbofish_and_a_closure_number_go(self) -> None:
        name = "<naga[aa]::Ctx>::resolve::<fn(u8) -> u8>::{closure#10}"
        self.assertEqual(wasm_stdio.readable(name), "<naga::Ctx>::resolve::{closure}")


class Allowlist(unittest.TestCase):
    def test_a_caller_no_line_names_is_refused(self) -> None:
        errors = wasm_stdio.verdict(["lyon::log"], [])
        self.assertEqual(len(errors), 1)
        self.assertIn("lyon::log", errors[0])

    def test_a_line_that_names_no_caller_is_refused(self) -> None:
        line = wasm_stdio.Line("gone::f", "a reason", 7)
        errors = wasm_stdio.verdict([], [line])
        self.assertEqual(len(errors), 1)
        self.assertIn("line 7", errors[0])

    def test_a_listed_caller_passes(self) -> None:
        line = wasm_stdio.Line("lyon::log", "a reason", 3)
        self.assertEqual(wasm_stdio.verdict(["lyon::log"], [line]), [])

    def test_a_line_without_a_reason_and_a_duplicate_are_refused(self) -> None:
        lines, errors = wasm_stdio.read_allowlist("caller\treason\na::f\t\nb::g\tr\nb::g\tr\n")
        self.assertEqual(len(errors), 2)
        self.assertEqual([line.caller for line in lines], ["b::g", "b::g"])

    def test_the_committed_allowlist_reads_with_no_error(self) -> None:
        lines, errors = wasm_stdio.read_allowlist(
            wasm_stdio.ALLOWLIST.read_text(encoding="utf-8"))
        self.assertEqual(errors, [])
        for line in lines:
            self.assertNotRegex(line.caller, r"::h[0-9a-f]{16}$")


if __name__ == "__main__":
    unittest.main()

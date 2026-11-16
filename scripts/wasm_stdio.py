#!/usr/bin/env python3
"""The Main instance links no caller of the print functions of std.

`print!`, `println!`, `eprint!`, `eprintln!` and `dbg!` expand to a call of
`std::io::stdio::_print` or `std::io::stdio::_eprint`. On the browser target,
std takes the reentrant lock of the stream before it drops the bytes. The Main
instance runs the page's main thread and the Logic thread over one shared
memory. When two threads print at once, the second one waits on the lock, and
a wait on the main thread is an `Atomics.wait` that the browser refuses with a
trap (`ARCH-119`).

The gate reads the debug Main wasm that `trunk build` writes into `dist/`, the
build the browser tier serves. It decodes the code of each function and finds
each function with a `call` or a `return_call` to one of the two print
functions. Each such caller is refused unless one line of
`specs/1-Foundations/ambre-stdio.tsv` names it with its reason. A line that
names no caller of the wasm is refused too, so the file only shrinks.

A function is named by the `name` section that the debug build keeps, which
wasm-bindgen writes demangled. A line drops the hash of each crate, the generic
arguments and the number of a closure, so it names one function across builds.
The gate fails closed on each form it cannot read:

* no `name` section, or one that names no function of std;
* a name still mangled, as the wasm of `cargo build` holds before wasm-bindgen;
* an instruction the decoder does not know, or a body that does not end where
  its size says.

A print reached through a function pointer, `call_indirect`, is out of reach of
the gate. The release build is not read: it links a subset of the debug build.

Run: `just stdio`, after `trunk build`. It writes nothing.
"""

from __future__ import annotations

import re
import struct
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ALLOWLIST = ROOT / "specs/1-Foundations/ambre-stdio.tsv"
DIST = ROOT / "dist"
MAIN_GLOB = "ambre-app*_bg.wasm"

# The two functions that the print macros of std call.
PRINTS = ("std::io::stdio::_print", "std::io::stdio::_eprint")

# The hash suffix of a legacy Rust symbol, the hash of a crate as wasm-bindgen
# writes it after the crate name, and the number of a closure or a shim.
HASH = re.compile(r"::h[0-9a-f]{16}$")
CRATE_HASH = re.compile(r"\[[0-9a-f]{1,16}\]")
CLOSURE_NUMBER = re.compile(r"#\d+\}")
# A symbol no demangler read: Rust v0, then legacy.
MANGLED = ("_R", "_ZN")


class WasmError(Exception):
    """A module the gate cannot read. The gate fails on it."""


@dataclass(frozen=True)
class Line:
    """One line of the allowlist: the caller it names and the reason."""

    caller: str
    reason: str
    number: int


class Reader:
    """A cursor over the bytes of a module."""

    def __init__(self, data: bytes, pos: int = 0, end: int | None = None) -> None:
        self.data = data
        self.pos = pos
        self.end = len(data) if end is None else end

    def byte(self) -> int:
        if self.pos >= self.end:
            raise WasmError(f"read past the end at byte {self.pos}")
        value = self.data[self.pos]
        self.pos += 1
        return value

    def u32(self) -> int:
        """An unsigned LEB128 value."""
        result = 0
        shift = 0
        while True:
            value = self.byte()
            result |= (value & 0x7F) << shift
            if value < 0x80:
                return result
            shift += 7
            if shift > 63:
                raise WasmError(f"LEB128 value too long at byte {self.pos}")

    def signed(self) -> None:
        """Skips a signed LEB128 value: the gate never reads one."""
        while self.byte() & 0x80:
            pass

    def skip(self, count: int) -> None:
        if self.pos + count > self.end:
            raise WasmError(f"read past the end at byte {self.pos}")
        self.pos += count

    def name(self) -> str:
        size = self.u32()
        start = self.pos
        self.skip(size)
        return self.data[start:self.pos].decode("utf-8", errors="replace")


def sections(data: bytes) -> list[tuple[int, int, int]]:
    """Each section of a module: its id, the offset of its content and its end."""
    if data[:4] != b"\0asm" or struct.unpack("<I", data[4:8])[0] != 1:
        raise WasmError("not a WebAssembly module of version 1")
    reader = Reader(data, 8)
    found = []
    while reader.pos < reader.end:
        section = reader.byte()
        size = reader.u32()
        found.append((section, reader.pos, reader.pos + size))
        reader.skip(size)
    return found


def skip_heap_type(reader: Reader) -> None:
    reader.signed()


def skip_value_type(reader: Reader) -> None:
    """Skips a value type, a reference type with a heap type included."""
    kind = reader.byte()
    if kind in (0x63, 0x64):
        skip_heap_type(reader)


def skip_limits(reader: Reader) -> None:
    flags = reader.byte()
    reader.u32()
    if flags & 0x01:
        reader.u32()


def imported_functions(data: bytes, start: int, end: int) -> int:
    """The count of functions the import section brings: they take the first indices."""
    reader = Reader(data, start, end)
    count = 0
    for _ in range(reader.u32()):
        reader.name()
        reader.name()
        kind = reader.byte()
        if kind == 0x00:
            reader.u32()
            count += 1
        elif kind == 0x01:
            skip_value_type(reader)
            skip_limits(reader)
        elif kind == 0x02:
            skip_limits(reader)
        elif kind == 0x03:
            skip_value_type(reader)
            reader.byte()
        elif kind == 0x04:
            reader.byte()
            reader.u32()
        else:
            raise WasmError(f"unknown import kind {kind:#x}")
    return count


def readable(name: str) -> str:
    """A function name as one line of the allowlist spells it.

    wasm-bindgen writes each name demangled, with the hash of each crate in
    brackets, the generic arguments and a number per closure. The line drops all
    three, so it names one function across builds and across its instances.
    """
    name = CRATE_HASH.sub("", HASH.sub("", name))
    name = CLOSURE_NUMBER.sub("}", name)
    out = []
    depth = 0
    for index, char in enumerate(name):
        previous = name[index - 1] if index else ""
        if char == "<" and (depth or previous.isalnum() or previous in ("_", ":")):
            depth += 1
            if depth == 1 and out[-2:] == [":", ":"]:
                del out[-2:]
            continue
        if depth:
            if char == ">" and previous != "-":
                depth -= 1
            continue
        out.append(char)
    return "".join(out)


def function_names(data: bytes, start: int, end: int) -> dict[int, str]:
    """The function names of a `name` section, as the symbols spell them."""
    reader = Reader(data, start, end)
    if reader.name() != "name":
        return {}
    names: dict[int, str] = {}
    while reader.pos < reader.end:
        sub = reader.byte()
        size = reader.u32()
        stop = reader.pos + size
        if sub == 1:
            inner = Reader(data, reader.pos, stop)
            for _ in range(inner.u32()):
                index = inner.u32()
                names[index] = inner.name()
        reader.pos = stop
    return names


# The instructions with no immediate: control, parametric and numeric.
NO_IMMEDIATE = frozenset(
    [0x00, 0x01, 0x05, 0x0A, 0x0B, 0x0F, 0x19, 0x1A, 0x1B, 0xD1, 0xD3, 0xD4]
    + list(range(0x45, 0xC5))
)
# The instructions whose immediate is one index: a label, a local, a global, a
# table, a tag, a type or a function.
ONE_INDEX = frozenset(
    [0x07, 0x08, 0x09, 0x0C, 0x0D, 0x14, 0x15, 0x18, 0x20, 0x21, 0x22, 0x23, 0x24,
     0x25, 0x26, 0xD2, 0xD5, 0xD6]
)
CALLS = (0x10, 0x12)
BLOCKS = frozenset([0x02, 0x03, 0x04, 0x06])


def skip_block_type(reader: Reader) -> None:
    kind = reader.data[reader.pos] if reader.pos < reader.end else -1
    if kind == 0x40 or kind in (0x7F, 0x7E, 0x7D, 0x7C, 0x7B, 0x70, 0x6F):
        reader.pos += 1
    elif kind in (0x63, 0x64):
        reader.pos += 1
        skip_heap_type(reader)
    else:
        reader.signed()


def skip_memarg(reader: Reader) -> None:
    align = reader.u32()
    if align & 0x40:
        reader.u32()
    reader.u32()


def skip_bulk(reader: Reader) -> None:
    """The `0xFC` prefix: saturating truncation, bulk memory and tables."""
    op = reader.u32()
    if op <= 7:
        return
    if op in (8, 10, 12, 14):
        reader.u32()
        reader.u32()
    elif op in (9, 11, 13, 15, 16, 17):
        reader.u32()
    else:
        raise WasmError(f"unknown instruction 0xfc {op}")


def skip_simd(reader: Reader) -> None:
    """The `0xFD` prefix: 128-bit SIMD, relaxed SIMD included."""
    op = reader.u32()
    if op <= 11 or op in (92, 93):
        skip_memarg(reader)
    elif op in (12, 13):
        reader.skip(16)
    elif 21 <= op <= 34:
        reader.skip(1)
    elif 84 <= op <= 91:
        skip_memarg(reader)
        reader.skip(1)
    elif op > 0x113:
        raise WasmError(f"unknown instruction 0xfd {op}")


def skip_atomic(reader: Reader) -> None:
    """The `0xFE` prefix: the threads proposal."""
    op = reader.u32()
    if op == 0x03:
        reader.skip(1)
    elif op <= 0x02 or 0x10 <= op <= 0x4E:
        skip_memarg(reader)
    else:
        raise WasmError(f"unknown instruction 0xfe {op}")


def calls_any(data: bytes, start: int, end: int, targets: frozenset[int]) -> bool:
    """Whether the body between `start` and `end` calls one of `targets`.

    The body is decoded whole, so a byte inside an immediate never reads as a
    call. A body that does not end on its last `end` fails.
    """
    reader = Reader(data, start, end)
    for _ in range(reader.u32()):
        reader.u32()
        skip_value_type(reader)
    depth = 1
    found = False
    while reader.pos < end:
        op = reader.byte()
        if op in NO_IMMEDIATE:
            if op == 0x0B:
                depth -= 1
        elif op in ONE_INDEX:
            reader.u32()
        elif op in CALLS:
            if reader.u32() in targets:
                found = True
        elif op in BLOCKS:
            depth += 1
            skip_block_type(reader)
        elif 0x28 <= op <= 0x3E:
            skip_memarg(reader)
        elif op == 0x41 or op == 0x42:
            reader.signed()
        elif op == 0x43:
            reader.skip(4)
        elif op == 0x44:
            reader.skip(8)
        elif op in (0x11, 0x13):
            reader.u32()
            reader.u32()
        elif op == 0x0E:
            for _ in range(reader.u32() + 1):
                reader.u32()
        elif op == 0x1C:
            for _ in range(reader.u32()):
                skip_value_type(reader)
        elif op in (0x3F, 0x40):
            reader.u32()
        elif op == 0xD0:
            skip_heap_type(reader)
        elif op == 0x1F:
            depth += 1
            skip_block_type(reader)
            for _ in range(reader.u32()):
                kind = reader.byte()
                if kind in (0x00, 0x01):
                    reader.u32()
                reader.u32()
        elif op == 0xFC:
            skip_bulk(reader)
        elif op == 0xFD:
            skip_simd(reader)
        elif op == 0xFE:
            skip_atomic(reader)
        else:
            raise WasmError(f"unknown instruction {op:#x} at byte {reader.pos - 1}")
    if depth != 0:
        raise WasmError(f"a body ends at depth {depth}, at byte {end}")
    return found


def print_callers(data: bytes) -> list[str]:
    """The name of each function of the module that calls a print function of std, sorted."""
    imports = 0
    names: dict[int, str] = {}
    code = None
    for section, start, end in sections(data):
        if section == 2:
            imports = imported_functions(data, start, end)
        elif section == 10:
            code = (start, end)
        elif section == 0:
            names.update(function_names(data, start, end))
    # The gate reads the names that can spell a print function, and the
    # callers alone, of some 170 000 names.
    if not any(name.startswith("std[") or name.startswith("std::") for name in names.values()):
        raise WasmError("the name section names no function of std: the debug build keeps it")
    mangled = next((name for name in names.values() if name.startswith(MANGLED)), None)
    if mangled is not None:
        raise WasmError(f"`{mangled[:80]}` is mangled: read the wasm wasm-bindgen writes")
    candidates = {index: readable(name) for index, name in names.items() if "print" in name}
    targets = frozenset(index for index, name in candidates.items() if name in PRINTS)
    if not targets or code is None:
        return []
    reader = Reader(data, code[0], code[1])
    callers = set()
    for offset in range(reader.u32()):
        size = reader.u32()
        start = reader.pos
        reader.skip(size)
        if calls_any(data, start, reader.pos, targets):
            index = imports + offset
            callers.add(readable(names[index]) if index in names else f"function {index}")
    return sorted(callers)


def read_allowlist(text: str) -> tuple[list[Line], list[str]]:
    """The lines of the allowlist, and the errors of its shape."""
    lines: list[Line] = []
    errors: list[str] = []
    header_seen = False
    for number, raw in enumerate(text.splitlines(), start=1):
        if not raw.strip() or raw.startswith("#"):
            continue
        if not header_seen:
            header_seen = True
            if raw != "caller\treason":
                errors.append(f"line {number}: the header reads `caller<TAB>reason`")
            continue
        fields = raw.split("\t")
        if len(fields) != 2 or not fields[0].strip() or not fields[1].strip():
            errors.append(f"line {number}: a caller, a tab and a reason")
            continue
        lines.append(Line(fields[0], fields[1], number))
    seen: set[str] = set()
    for line in lines:
        if line.caller in seen:
            errors.append(f"line {line.number}: `{line.caller}` is listed twice")
        seen.add(line.caller)
    return lines, errors


def verdict(callers: list[str], lines: list[Line]) -> list[str]:
    """The errors: each caller no line names, and each line that names no caller."""
    listed = {line.caller for line in lines}
    found = set(callers)
    errors = [
        f"`{caller}` calls a print function of std, and no line of the allowlist names it"
        for caller in callers if caller not in listed
    ]
    errors += [
        f"line {line.number}: `{line.caller}` calls no print function of std: delete the line"
        for line in lines if line.caller not in found
    ]
    return errors


def main(argv: list[str] | None = None) -> int:
    argv = sys.argv if argv is None else argv
    if len(argv) > 1:
        wasm = Path(argv[1])
    else:
        found = sorted(DIST.glob(MAIN_GLOB))
        if len(found) != 1:
            print(f"stdio: {len(found)} files match {MAIN_GLOB} in {DIST}, not one: run `trunk build`")
            return 1
        wasm = found[0]
    lines, errors = read_allowlist(ALLOWLIST.read_text(encoding="utf-8"))
    try:
        callers = print_callers(wasm.read_bytes())
    except WasmError as error:
        print(f"stdio: {wasm.name}: {error}")
        return 1
    errors += verdict(callers, lines)
    for error in errors:
        print(f"stdio: {error}")
    if errors:
        return 1
    print(f"stdio: {wasm.name}: {len(callers)} callers of a print function, each listed")
    return 0


if __name__ == "__main__":
    sys.exit(main())

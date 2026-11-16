#!/usr/bin/env python3
"""The number and date forms of each locale Ambre formats in, from CLDR.

`ambre-client-core` formats a number, a currency and a date in the forms of a
document's locale (`ambre-spec-locale.md`). The forms come from CLDR, never
from a hand: a group mark such as the narrow no-break space of French, or the
apostrophe of Swiss German, is the kind of value a person types wrong and a
reader never sees is wrong.

Three steps, each its own mode:

* `--fetch` downloads the two pinned CLDR packages from the npm registry,
  checks each against its pinned integrity, and writes the values Ambre reads
  into `SUBSET`. It is the one step that needs the network.
* `--write` writes the Rust table `TABLE` from `SUBSET`. `--fetch` writes it
  too, after the download.
* the default mode writes nothing, and fails when `TABLE` is not what `SUBSET`
  gives, or when `SUBSET` names another CLDR version or locale list than this
  file. CI runs it, offline, through `just locale-table-check`.

A locale the list leaves out formats in the neutral forms, as a document with
no locale does (`LOCALE-013`).
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import io
import json
import re
import sys
import tarfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SUBSET = ROOT / "scripts" / "data" / "cldr-locale-subset.json"
TABLE = ROOT / "crates" / "ambre-client-core" / "src" / "locale" / "table.rs"

CLDR_VERSION = "48.0.0"
# The npm `dist.integrity` of each package at `CLDR_VERSION`.
PACKAGES = {
    "cldr-numbers-full": "sha512-bPTCfVDc0KRdRCRES155EZX1U0ZjUnc9AgiyKPH7K+mXWgCPLx7CzxvYY0IXGTF8W6ig+2hbG5V7oHMenA8iGQ==",
    "cldr-dates-full": "sha512-RcJk/KQzmL9UlE5rAtS4Vxbi0lPGTXZ+s6ksqKY14HLQh3L3kewYlwK146l46uRS9JOhSlXhGV+V3KrUmKMioQ==",
}
REGISTRY = "https://registry.npmjs.org"
LOCALES = [
    "en",
    "en-GB",
    "fr",
    "fr-CA",
    "fr-CH",
    "de",
    "de-AT",
    "de-CH",
    "es",
    "it",
    "it-CH",
    "pt",
    "nl",
]
# CLDR keys the week from Sunday; a sheet day serial counts it from Saturday.
DAYS = ["sat", "sun", "mon", "tue", "wed", "thu", "fri"]


def fetch_package(name: str) -> tarfile.TarFile:
    """The tarball of `name` at `CLDR_VERSION`, checked against its pin."""
    url = f"{REGISTRY}/{name}/-/{name}-{CLDR_VERSION}.tgz"
    with urllib.request.urlopen(url) as response:  # noqa: S310 -- a pinned https URL
        data = response.read()
    digest = "sha512-" + base64.b64encode(hashlib.sha512(data).digest()).decode()
    if digest != PACKAGES[name]:
        raise SystemExit(f"{name}@{CLDR_VERSION}: integrity {digest} is not the pinned one")
    return tarfile.open(fileobj=io.BytesIO(data), mode="r:gz")


def member_json(archive: tarfile.TarFile, path: str) -> dict:
    """The JSON document at `path` inside `archive`."""
    member = archive.extractfile(f"package/{path}")
    if member is None:
        raise SystemExit(f"{path} is missing from the CLDR package")
    return json.load(member)


def extract(numbers: tarfile.TarFile, dates: tarfile.TarFile) -> dict:
    """The values Ambre reads, one record per locale of `LOCALES`."""
    records = {}
    for tag in LOCALES:
        nums = member_json(numbers, f"main/{tag}/numbers.json")["main"][tag]["numbers"]
        greg = member_json(dates, f"main/{tag}/ca-gregorian.json")["main"][tag]["dates"][
            "calendars"
        ]["gregorian"]
        system = nums["defaultNumberingSystem"]
        symbols = nums[f"symbols-numberSystem-{system}"]
        records[tag] = {
            "decimal": symbols["decimal"],
            "group": symbols["group"],
            "minimumGroupingDigits": int(nums["minimumGroupingDigits"]),
            "currency": nums[f"currencyFormats-numberSystem-{system}"]["standard"],
            "short": greg["dateFormats"]["short"],
            "long": greg["dateFormats"]["long"],
            "months": [greg["months"]["format"]["wide"][str(m)] for m in range(1, 13)],
            "days": [greg["days"]["format"]["wide"][day] for day in DAYS],
        }
    return {"cldr": CLDR_VERSION, "packages": PACKAGES, "locales": records}


def rust_str(text: str) -> str:
    """`text` as a Rust string literal in printable ASCII."""
    out = []
    for c in text:
        if c in '"\\':
            out.append("\\" + c)
        elif 32 <= ord(c) <= 126:
            out.append(c)
        else:
            out.append(f"\\u{{{ord(c):x}}}")
    return '"' + "".join(out) + '"'


def rust_char(c: str) -> str:
    """`c` as a Rust char literal in printable ASCII."""
    if c in "'\\":
        return "'\\" + c + "'"
    if 32 <= ord(c) <= 126:
        return f"'{c}'"
    return f"'\\u{{{ord(c):x}}}'"


def affixes(pattern: str) -> tuple[str, str]:
    """The text before and after the digits of one currency subpattern."""
    match = re.fullmatch(r"([^#0,.]*)[#0,.]+([^#0,.]*)", pattern)
    if match is None:
        raise SystemExit(f"currency pattern {pattern!r} has no digit run")
    return match.group(1), match.group(2)


def currency(pattern: str) -> tuple[tuple[str, str], tuple[str, str]]:
    """The affixes of a positive and a negative amount. CLDR's implicit
    negative subpattern is the positive one behind a minus sign."""
    positive, _, negative = pattern.partition(";")
    pos = affixes(positive)
    neg = affixes(negative) if negative else ("-" + pos[0], pos[1])
    return pos, neg


def short_date(pattern: str) -> tuple[str, str, str]:
    """The order, the mark and the padding of a numeric date pattern."""
    fields = re.findall(r"[dMy]+", pattern)
    marks = set(re.sub(r"[dMy]+", "", pattern))
    if len(fields) != 3 or len(marks) != 1:
        raise SystemExit(f"short date pattern {pattern!r} is no three-part numeric date")
    order = "".join(field[0] for field in fields)
    orders = {"yMd": "YearMonthDay", "dMy": "DayMonthYear", "Mdy": "MonthDayYear"}
    if order not in orders:
        raise SystemExit(f"short date pattern {pattern!r} has an order Ambre does not write")
    day = next(field for field in fields if field[0] == "d")
    padding = "TwoDigits" if len(day) == 2 else "Bare"
    return orders[order], marks.pop(), padding


def long_parts(pattern: str) -> list[str]:
    """The parts of a long date pattern, as Rust `LongPart` values."""
    parts = []
    for token in re.findall(r"'[^']*'|d+|M+|y+|[^'dMy]+", pattern):
        if token.startswith("'"):
            parts.append(f"LongPart::Text({rust_str(token[1:-1])})")
        elif token[0] == "d":
            parts.append("LongPart::Day")
        elif token[0] == "M":
            if len(token) != 4:
                raise SystemExit(f"long date pattern {pattern!r} names no full month")
            parts.append("LongPart::Month")
        elif token[0] == "y":
            parts.append("LongPart::Year")
        else:
            parts.append(f"LongPart::Text({rust_str(token)})")
    return parts


def render(subset: dict) -> str:
    """The Rust table `TABLE` holds for `subset`."""
    lines = [
        "// ambre-client-core/src/locale/table.rs",
        f"// @generated by scripts/locale_table.py from CLDR {subset['cldr']}.",
        "// Never edit it: `just locale-table` writes it, `just locale-table-check`",
        "// holds it.",
        "",
        "use super::{",
        "    Affix, CalendarNames, CurrencyForm, DateOrder, Grouping, LocaleData, LongPart, Padding,",
        "    ShortDate,",
        "};",
        "",
        "/// One record per locale, a language alone before its regions.",
        "// One line per field keeps a diff of a CLDR upgrade readable.",
        "#[rustfmt::skip]",
        "pub(super) static LOCALES: &[LocaleData] = &[",
    ]
    for tag, record in subset["locales"].items():
        (pp, ps), (np, ns) = currency(record["currency"])
        order, mark, padding = short_date(record["short"])
        # CLDR's minimum grouping digits: 1 groups a four-digit whole, 2 waits
        # for five digits.
        grouping = {1: "Grouping::Grouped", 2: "Grouping::Ungrouped"}.get(
            record["minimumGroupingDigits"]
        )
        if grouping is None:
            raise SystemExit(f"{tag}: a minimum grouping past 2 is no form Ambre writes")
        lines += [
            "    LocaleData {",
            f"        tag: {rust_str(tag.lower())},",
            f"        decimal: {rust_char(record['decimal'])},",
            f"        group: {rust_char(record['group'])},",
            f"        four_digits: {grouping},",
            "        currency: CurrencyForm {",
            f"            positive: Affix {{ prefix: {rust_str(pp)}, suffix: {rust_str(ps)} }},",
            f"            negative: Affix {{ prefix: {rust_str(np)}, suffix: {rust_str(ns)} }},",
            "        },",
            "        short: ShortDate {",
            f"            order: DateOrder::{order},",
            f"            mark: {rust_char(mark)},",
            f"            padding: Padding::{padding},",
            "        },",
            f"        long: &[{', '.join(long_parts(record['long']))}],",
            "        names: CalendarNames {",
            f"            months: [{', '.join(rust_str(m) for m in record['months'])}],",
            f"            days: [{', '.join(rust_str(d) for d in record['days'])}],",
            "        },",
            "    },",
        ]
    lines += ["];", ""]
    return "\n".join(lines)


def read_subset() -> dict:
    """The committed subset, refused when it names another pin than this file."""
    subset = json.loads(SUBSET.read_text(encoding="utf-8"))
    if subset.get("cldr") != CLDR_VERSION or subset.get("packages") != PACKAGES:
        raise SystemExit(f"{SUBSET.name} names another CLDR pin: run `just locale-table-fetch`")
    if list(subset.get("locales", {})) != LOCALES:
        raise SystemExit(f"{SUBSET.name} names another locale list: run `just locale-table-fetch`")
    return subset


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--fetch", action="store_true", help="refresh the subset from CLDR")
    mode.add_argument("--write", action="store_true", help="write the table from the subset")
    args = parser.parse_args(argv)
    if args.fetch:
        subset = extract(fetch_package("cldr-numbers-full"), fetch_package("cldr-dates-full"))
        SUBSET.parent.mkdir(parents=True, exist_ok=True)
        SUBSET.write_text(json.dumps(subset, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")
    subset = read_subset()
    table = render(subset)
    if args.fetch or args.write:
        TABLE.write_text(table, encoding="utf-8")
        return 0
    if not TABLE.exists() or TABLE.read_text(encoding="utf-8") != table:
        print(f"{TABLE.relative_to(ROOT)} is stale: run `just locale-table`", file=sys.stderr)
        return 1
    print(f"locale table: {len(subset['locales'])} locales, CLDR {CLDR_VERSION}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

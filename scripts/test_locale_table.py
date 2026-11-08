#!/usr/bin/env python3
"""What `locale_table.py` reads out of a CLDR pattern, and what it refuses.

The patterns below are CLDR's own spellings. Fetching CLDR is left to
`just locale-table-fetch`; the committed table is held by
`just locale-table-check`.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import locale_table  # noqa: E402  -- the module under test


class Patterns(unittest.TestCase):
    def test_a_currency_pattern_splits_around_its_digits(self) -> None:
        self.assertEqual(
            locale_table.currency("#,##0.00 ¤"),
            (("", " ¤"), ("-", " ¤")),
        )

    def test_an_explicit_negative_subpattern_is_kept(self) -> None:
        self.assertEqual(
            locale_table.currency("¤ #,##0.00;¤-#,##0.00"),
            (("¤ ", ""), ("¤-", "")),
        )

    def test_a_short_date_names_its_order_mark_and_padding(self) -> None:
        self.assertEqual(locale_table.short_date("M/d/yy"), ("MonthDayYear", "/", "Bare"))
        self.assertEqual(locale_table.short_date("dd.MM.y"), ("DayMonthYear", ".", "TwoDigits"))
        self.assertEqual(locale_table.short_date("y-MM-dd"), ("YearMonthDay", "-", "TwoDigits"))

    def test_a_short_date_of_two_marks_is_refused(self) -> None:
        with self.assertRaises(SystemExit):
            locale_table.short_date("d.M/y")

    def test_a_long_date_keeps_its_quoted_words(self) -> None:
        self.assertEqual(
            locale_table.long_parts("d 'de' MMMM 'de' y"),
            [
                "LongPart::Day",
                'LongPart::Text(" ")',
                'LongPart::Text("de")',
                'LongPart::Text(" ")',
                "LongPart::Month",
                'LongPart::Text(" ")',
                'LongPart::Text("de")',
                'LongPart::Text(" ")',
                "LongPart::Year",
            ],
        )

    def test_a_rust_literal_stays_printable_ascii(self) -> None:
        self.assertEqual(locale_table.rust_str("août"), '"ao\\u{fb}t"')
        self.assertEqual(locale_table.rust_char("'"), "'\\''")
        self.assertEqual(locale_table.rust_char(" "), "'\\u{202f}'")

    def test_the_committed_table_is_what_the_subset_gives(self) -> None:
        self.assertEqual(locale_table.main([]), 0)


if __name__ == "__main__":
    unittest.main()

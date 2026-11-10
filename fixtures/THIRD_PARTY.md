# THIRD-PARTY INTEROP FIXTURES

Each input fixture copied from another project, one row per file. The licence text of each source
is under `fixtures/licences/`, named after its repository. A file this repository wrote, by hand or
through a committed script, has its row in `fixtures/AUTHORED.md` instead. A file a test assembles
in memory has no row. The rules are `ambre-spec-interop.md section 1.4`, and
`just fixture-provenance` holds them.

| Path in Ambre | Upstream repository | Upstream commit | Upstream path | Licence | Copyright |
|---|---|---|---|---|---|
| `crates/ambre-xlsx/tests/fixtures/calamine/absolute_shared_formula.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/absolute_shared_formula.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/any_sheets.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/any_sheets.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/borders.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/borders.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/date.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/date.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/date_1904.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/date_1904.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/errors.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/errors.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/has_x000D_.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/has_x000D_.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/merge_cells.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/merge_cells.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/password_protected.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/password_protected.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/picture.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/picture.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/pivots.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/pivots.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/rich_text_shared_strings.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/rich_text_shared_strings.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/shared_formula_dragged.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/shared_formula_dragged.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/shared_formula_function_names.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/shared_formula_function_names.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/shared_formula_reversed.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/shared_formula_reversed.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/shared_formula_simple.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/shared_formula_simple.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/strict_iso_paths.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/strict_iso_paths.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/styles.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/styles.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-xlsx/tests/fixtures/calamine/table-multiple.xlsx` | https://github.com/tafia/calamine | `57011cf6b1430785d0c47e5ad6eb76081d7c0e22` | `tests/table-multiple.xlsx` | MIT | Copyright (c) 2016 Johann Tuffe |
| `crates/ambre-docx/tests/fixtures/pandoc/block_quotes.docx` | https://github.com/jgm/pandoc | `e07fd5c33a0f63a6241115e341c9efda72bf155d` | `test/docx/block_quotes.docx` | GPL-2.0-or-later | Copyright (C) 2006-2024 John MacFarlane |
| `crates/ambre-docx/tests/fixtures/pandoc/codeblock.docx` | https://github.com/jgm/pandoc | `e07fd5c33a0f63a6241115e341c9efda72bf155d` | `test/docx/codeblock.docx` | GPL-2.0-or-later | Copyright (C) 2006-2024 John MacFarlane |
| `crates/ambre-docx/tests/fixtures/pandoc/comments.docx` | https://github.com/jgm/pandoc | `e07fd5c33a0f63a6241115e341c9efda72bf155d` | `test/docx/comments.docx` | GPL-2.0-or-later | Copyright (C) 2006-2024 John MacFarlane |
| `crates/ambre-docx/tests/fixtures/pandoc/headers.docx` | https://github.com/jgm/pandoc | `e07fd5c33a0f63a6241115e341c9efda72bf155d` | `test/docx/headers.docx` | GPL-2.0-or-later | Copyright (C) 2006-2024 John MacFarlane |
| `crates/ambre-docx/tests/fixtures/pandoc/image.docx` | https://github.com/jgm/pandoc | `e07fd5c33a0f63a6241115e341c9efda72bf155d` | `test/docx/image.docx` | GPL-2.0-or-later | Copyright (C) 2006-2024 John MacFarlane |
| `crates/ambre-docx/tests/fixtures/pandoc/inline_formatting.docx` | https://github.com/jgm/pandoc | `e07fd5c33a0f63a6241115e341c9efda72bf155d` | `test/docx/inline_formatting.docx` | GPL-2.0-or-later | Copyright (C) 2006-2024 John MacFarlane |
| `crates/ambre-docx/tests/fixtures/pandoc/links.docx` | https://github.com/jgm/pandoc | `e07fd5c33a0f63a6241115e341c9efda72bf155d` | `test/docx/links.docx` | GPL-2.0-or-later | Copyright (C) 2006-2024 John MacFarlane |
| `crates/ambre-docx/tests/fixtures/pandoc/lists.docx` | https://github.com/jgm/pandoc | `e07fd5c33a0f63a6241115e341c9efda72bf155d` | `test/docx/lists.docx` | GPL-2.0-or-later | Copyright (C) 2006-2024 John MacFarlane |
| `crates/ambre-docx/tests/fixtures/pandoc/notes.docx` | https://github.com/jgm/pandoc | `e07fd5c33a0f63a6241115e341c9efda72bf155d` | `test/docx/notes.docx` | GPL-2.0-or-later | Copyright (C) 2006-2024 John MacFarlane |
| `crates/ambre-docx/tests/fixtures/pandoc/tables.docx` | https://github.com/jgm/pandoc | `e07fd5c33a0f63a6241115e341c9efda72bf155d` | `test/docx/tables.docx` | GPL-2.0-or-later | Copyright (C) 2006-2024 John MacFarlane |
| `crates/ambre-docx/tests/fixtures/pandoc/track_changes_deletion.docx` | https://github.com/jgm/pandoc | `e07fd5c33a0f63a6241115e341c9efda72bf155d` | `test/docx/track_changes_deletion.docx` | GPL-2.0-or-later | Copyright (C) 2006-2024 John MacFarlane |
| `crates/ambre-docx/tests/fixtures/pandoc/track_changes_insertion.docx` | https://github.com/jgm/pandoc | `e07fd5c33a0f63a6241115e341c9efda72bf155d` | `test/docx/track_changes_insertion.docx` | GPL-2.0-or-later | Copyright (C) 2006-2024 John MacFarlane |
| `crates/ambre-docx/tests/fixtures/pandoc/track_changes_move.docx` | https://github.com/jgm/pandoc | `e07fd5c33a0f63a6241115e341c9efda72bf155d` | `test/docx/track_changes_move.docx` | GPL-2.0-or-later | Copyright (C) 2006-2024 John MacFarlane |
| `crates/ambre-docx/tests/fixtures/pandoc/track_changes_scrubbed_metadata.docx` | https://github.com/jgm/pandoc | `e07fd5c33a0f63a6241115e341c9efda72bf155d` | `test/docx/track_changes_scrubbed_metadata.docx` | GPL-2.0-or-later | Copyright (C) 2006-2024 John MacFarlane |
| `crates/ambre-docx/tests/fixtures/open-xml-sdk/Comment004.docx` | https://github.com/dotnet/Open-XML-SDK | `431ab05cf160248cc3885a4a766026d4f8243792` | `test/DocumentFormat.OpenXml.Tests.Assets/assets/TestDataStorage/O15Conformance/WD/CommentExTest/Comments-Sample-15-12-01/Comment004.docx` | MIT | Copyright (c) .NET Foundation and Contributors |
| `crates/ambre-docx/tests/fixtures/open-xml-sdk/strict-numbering.docx` | https://github.com/dotnet/Open-XML-SDK | `431ab05cf160248cc3885a4a766026d4f8243792` | `test/DocumentFormat.OpenXml.Tests.Assets/assets/TestDataStorage/O14ISOStrict/Word/numFmt-Numbering Format-val-decimalEnclosedFullstop.docx` | MIT | Copyright (c) .NET Foundation and Contributors |

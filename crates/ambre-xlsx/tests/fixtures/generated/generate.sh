#!/usr/bin/env bash
# Regenerate the LibreOffice fixtures of `ambre-xlsx` from what this
# repository holds: no file comes from elsewhere.
#
# - `trame.xlsx` is LibreOffice's export of `trame.fods`, written by hand.
# - `ambre_resaved.xlsx` is LibreOffice's save of the workbook the codec
#   writes for `sample()` in `tests/fixtures.rs`.
# - `trame.ods` and `losses.ods` are LibreOffice's OpenDocument exports of
#   `trame.fods` and `losses.fods`, written by hand.
# - `ambre_resaved.ods` is LibreOffice's save of the spreadsheet the
#   OpenDocument codec writes for `sample()`.
#
# LibreOffice stamps the date of the run into each file, so a new run
# changes the bytes and nothing the readers read.
#
# Needs `soffice` (LibreOffice 24.2 wrote the committed files) and cargo.
# Run from the repository root: `crates/ambre-xlsx/tests/fixtures/generated/generate.sh`.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
soffice --headless --convert-to xlsx --outdir "$work" "$here/trame.fods" >/dev/null
cp "$work/trame.xlsx" "$here/trame.xlsx"
AMBRE_XLSX_SAMPLE="$work/sample.xlsx" cargo test -q -p ambre-xlsx --test fixtures the_sample_writes_where_asked
mkdir -p "$work/out"
soffice --headless --convert-to 'xlsx:Calc MS Excel 2007 XML' --outdir "$work/out" "$work/sample.xlsx" >/dev/null
cp "$work/out/sample.xlsx" "$here/ambre_resaved.xlsx"
soffice --headless --convert-to ods --outdir "$work" "$here/trame.fods" "$here/losses.fods" >/dev/null
cp "$work/trame.ods" "$here/trame.ods"
cp "$work/losses.ods" "$here/losses.ods"
AMBRE_ODS_SAMPLE="$work/sample.ods" cargo test -q -p ambre-xlsx --test fixtures the_ods_sample_writes_where_asked
mkdir -p "$work/ods"
soffice --headless --convert-to 'ods:calc8' --outdir "$work/ods" "$work/sample.ods" >/dev/null
cp "$work/ods/sample.ods" "$here/ambre_resaved.ods"

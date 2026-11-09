#!/usr/bin/env bash
# Regenerate the LibreOffice fixtures of `ambre-xlsx` from what this
# repository holds: no file comes from elsewhere.
#
# - `trame.xlsx` is LibreOffice's export of `trame.fods`, written by hand.
# - `ambre_resaved.xlsx` is LibreOffice's save of the workbook the codec
#   writes for `sample()` in `tests/fixtures.rs`.
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

#!/bin/sh
# Writes features.docx from features.fodt with LibreOffice, as the fixture
# was written: soffice --headless --convert-to docx. Run from this folder.
# LibreOffice stamps the date of the run into the package, so a new run
# changes the bytes and nothing the reader reads.
set -eu
cd "$(dirname "$0")"
soffice --headless --convert-to docx features.fodt

#!/bin/sh
# Writes features.odt and features.docx from features.fodt with LibreOffice:
# soffice --headless --convert-to odt, then docx. Run from this folder.
# The two files carry one source, so the OpenDocument import of the first
# is held to the Word import of the second. LibreOffice stamps the date of
# the run into each package, so a new run changes the bytes and nothing the
# readers read.
set -eu
cd "$(dirname "$0")"
soffice --headless --convert-to odt features.fodt
soffice --headless --convert-to docx features.fodt

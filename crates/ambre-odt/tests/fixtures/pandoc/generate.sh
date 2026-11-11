#!/bin/sh
# Writes inline.odt and structure.odt from inline.md and structure.md with
# pandoc 3.1, then inline.docx and structure.docx from each .odt with
# LibreOffice: soffice --headless --convert-to docx. Run from this folder.
# Each .docx is the same text saved as Word, so the OpenDocument import of
# the first is held to the Word import of the second.
set -eu
cd "$(dirname "$0")"
pandoc --from markdown inline.md -o inline.odt
pandoc --from markdown structure.md -o structure.odt
soffice --headless --convert-to docx inline.odt
soffice --headless --convert-to docx structure.odt

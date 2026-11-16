#!/usr/bin/env python3
"""Write the PowerPoint fixtures of `ambre-pptx` from this script alone.

- `features.pptx` is a deck python-pptx 1.0.2 builds on its default
  template, which PowerPoint wrote: a master, its layouts and its theme. The
  script adds one construct of each kind the reader maps or reports, with
  invented text. It adds a shape to the master, which every slide inherits.
  python-pptx has no API for a transition, an animation or a SmartArt
  diagram, so the script writes their markup into the slide itself.
- `libreoffice.pptx` is LibreOffice's save of `features.pptx` as PowerPoint:
  the same deck through a second writer.

Each picture is drawn here, a few pixels of one colour. The video is a few
bytes that name no real clip: the reader reads its poster frame alone.

Needs python-pptx 1.0.2, Pillow, XlsxWriter and `soffice` (LibreOffice 24.2
wrote the committed files). Run from the repository root:
`python3 crates/ambre-pptx/tests/fixtures/generated/generate.py`. Each run
stamps new dates into the core properties, which the reader never reads.
"""

import io
import shutil
import subprocess
import tempfile
from pathlib import Path

from lxml import etree
from PIL import Image
from pptx import Presentation
from pptx.chart.data import CategoryChartData
from pptx.dml.color import RGBColor
from pptx.enum.chart import XL_CHART_TYPE
from pptx.enum.shapes import MSO_CONNECTOR, MSO_SHAPE
from pptx.util import Emu, Pt

HERE = Path(__file__).resolve().parent
P = "http://schemas.openxmlformats.org/presentationml/2006/main"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"


def picture(fmt: str, colour: tuple) -> io.BytesIO:
    """A picture of 8 by 8 pixels of one colour, in `fmt`."""
    out = io.BytesIO()
    Image.new("RGB", (8, 8), colour).save(out, fmt)
    out.seek(0)
    return out


def xml(fragment: str):
    """The element `fragment` spells, under the three namespaces."""
    return etree.fromstring(
        f'<root xmlns:p="{P}" xmlns:a="{A}" xmlns:r="{R}">{fragment}</root>'
    )[0]


def features(path: Path) -> None:
    deck = Presentation()
    master = deck.slide_master

    # A shape of the master: every slide shows it, under its own shapes.
    master.shapes._spTree.append(
        xml(
            '<p:sp><p:nvSpPr><p:cNvPr id="99" name="Master mark"/><p:cNvSpPr/><p:nvPr/>'
            '</p:nvSpPr><p:spPr><a:xfrm><a:off x="8686800" y="6400800"/>'
            '<a:ext cx="304800" cy="304800"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/>'
            '</a:prstGeom><a:solidFill><a:srgbClr val="123456"/></a:solidFill></p:spPr></p:sp>'
        )
    )

    # Slide 1: the title layout, a transition and notes.
    first = deck.slides.add_slide(deck.slide_layouts[0])
    first.shapes.title.text = "Quarterly plan"
    first.placeholders[1].text = "An invented team"
    first.notes_slide.notes_text_frame.text = "Open with the plan."
    first._element.insert(2, xml("<p:transition><p:fade/></p:transition>"))

    # Slide 2: a title and a body, shapes, a connector, a text box, an
    # animation and notes of two lines.
    second = deck.slides.add_slide(deck.slide_layouts[1])
    second.shapes.title.text = "Three shapes"
    body = second.placeholders[1].text_frame
    body.text = "First point"
    run = body.add_paragraph().add_run()
    run.text = "Second point"
    run.font.bold = True
    box = second.shapes.add_shape(
        MSO_SHAPE.RECTANGLE, Emu(914_400), Emu(4_572_000), Emu(1_828_800), Emu(914_400)
    )
    box.name = "Box"
    disc = second.shapes.add_shape(
        MSO_SHAPE.OVAL, Emu(5_486_400), Emu(4_572_000), Emu(1_828_800), Emu(914_400)
    )
    disc.name = "Disc"
    disc.fill.solid()
    disc.fill.fore_color.rgb = RGBColor(0x00, 0x80, 0x00)
    second.shapes.add_shape(
        MSO_SHAPE.ROUNDED_RECTANGLE, Emu(3_200_400), Emu(5_715_000), Emu(1_828_800), Emu(457_200)
    )
    second.shapes.add_shape(
        MSO_SHAPE.STAR_5_POINT, Emu(7_772_400), Emu(1_371_600), Emu(457_200), Emu(457_200)
    )
    link = second.shapes.add_connector(MSO_CONNECTOR.STRAIGHT, 0, 0, 0, 0)
    link.begin_connect(box, 3)
    link.end_connect(disc, 1)
    note = second.shapes.add_textbox(Emu(914_400), Emu(6_172_200), Emu(3_657_600), Emu(457_200))
    note.text_frame.text = "A note in red"
    font = note.text_frame.paragraphs[0].runs[0].font
    font.size = Pt(24)
    font.italic = True
    font.color.rgb = RGBColor(0xC0, 0x00, 0x00)
    second.notes_slide.notes_text_frame.text = "Point at each shape.\nThen move on."
    second._element.append(
        xml(
            '<p:timing><p:tnLst><p:par><p:cTn id="1" dur="indefinite" nodeType="tmRoot"/>'
            "</p:par></p:tnLst></p:timing>"
        )
    )

    # Slide 3: the blank layout, two pictures, a group, a table, a chart, a
    # video and a SmartArt frame, over a background of its own.
    third = deck.slides.add_slide(deck.slide_layouts[6])
    third.background.fill.solid()
    third.background.fill.fore_color.rgb = RGBColor(0xF0, 0xF0, 0xE0)
    third.shapes.add_picture(picture("PNG", (200, 30, 30)), Emu(457_200), Emu(457_200))
    third.shapes.add_picture(picture("JPEG", (30, 30, 200)), Emu(1_828_800), Emu(457_200))
    group = third.shapes.add_group_shape()
    group.shapes.add_shape(MSO_SHAPE.RECTANGLE, Emu(457_200), Emu(2_286_000), Emu(457_200), Emu(457_200))
    group.shapes.add_shape(MSO_SHAPE.OVAL, Emu(1_143_000), Emu(2_286_000), Emu(457_200), Emu(457_200))
    table = third.shapes.add_table(2, 2, Emu(3_657_600), Emu(457_200), Emu(2_743_200), Emu(914_400))
    table.table.cell(0, 0).text = "A"
    data = CategoryChartData()
    data.categories = ["North", "South"]
    data.add_series("Units", (3, 5))
    third.shapes.add_chart(
        XL_CHART_TYPE.COLUMN_CLUSTERED, Emu(3_657_600), Emu(1_828_800), Emu(2_743_200), Emu(1_828_800), data
    )
    third.shapes.add_movie(
        io.BytesIO(b"not a real clip"),
        Emu(457_200),
        Emu(3_657_600),
        Emu(1_828_800),
        Emu(1_371_600),
        poster_frame_image=picture("PNG", (30, 160, 30)),
        mime_type="video/mp4",
    )
    third.shapes._spTree.append(
        xml(
            '<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="90" name="Diagram"/>'
            "<p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>"
            '<p:xfrm><a:off x="6858000" y="457200"/><a:ext cx="1828800" cy="1828800"/></p:xfrm>'
            '<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"/>'
            "</a:graphic></p:graphicFrame>"
        )
    )
    deck.core_properties.author = "Fixture"
    deck.core_properties.title = "Fixture deck"
    deck.save(path)


def main() -> None:
    features(HERE / "features.pptx")
    with tempfile.TemporaryDirectory() as work:
        subprocess.run(
            ["soffice", "--headless", "--convert-to", "pptx:Impress MS PowerPoint 2007 XML",
             "--outdir", work, str(HERE / "features.pptx")],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        shutil.copy(Path(work) / "features.pptx", HERE / "libreoffice.pptx")


if __name__ == "__main__":
    main()

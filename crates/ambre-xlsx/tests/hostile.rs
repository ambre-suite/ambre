//! The OpenDocument spreadsheet reader on hostile files the test assembles
//! in memory: no hostile file is committed (pre-audit `W72`, `W305` to
//! `W307`).

#![expect(
    clippy::panic,
    clippy::expect_used,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_client_core::FileBytes;
use ambre_core::Payload;
use ambre_xlsx::opc::PartName;
use ambre_xlsx::opc::odf::{OdfType, OdfWriter};

/// An `.ods` package whose content part holds the tables `tables`, under a
/// prologue `prologue` written before the root.
fn ods(prologue: &str, tables: &str) -> Payload<FileBytes> {
    let content = format!(
        concat!(
            "{}",
            r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "#,
            r#"xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" "#,
            r#"xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">"#,
            r#"<office:body><office:spreadsheet>{}</office:spreadsheet></office:body>"#,
            r#"</office:document-content>"#
        ),
        prologue, tables
    );
    let mut writer = OdfWriter::new(OdfType::SPREADSHEET);
    writer.part(
        PartName::new("content.xml").expect("a part name"),
        OdfType::XML,
        Payload::new(content.into_bytes()),
    );
    writer.finish().expect("the archive").into_role()
}

/// `W307`: a content part that declares a document type is refused, as the
/// four other codecs refuse it.
#[test]
fn a_content_part_that_declares_a_document_type_is_refused() {
    let tables = r#"<table:table table:name="S"><table:table-row><table:table-cell/></table:table-row></table:table>"#;
    assert!(ambre_xlsx::read_ods(&ods("", tables)).is_ok());
    let typed = ods(
        r#"<?xml version="1.0"?><!DOCTYPE d [<!ENTITY e "x">]>"#,
        tables,
    );
    assert!(ambre_xlsx::read_ods(&typed).is_err());
}

/// `W306`: a matrix span the file spells at the top of the width reads as
/// no range, and the read never panics on the sum.
#[test]
fn a_matrix_span_at_the_top_of_the_width_reads_as_no_range() {
    let tables = concat!(
        r#"<table:table table:name="S"><table:table-row>"#,
        r#"<table:table-cell table:formula="of:=1" "#,
        r#"table:number-matrix-columns-spanned="18446744073709551615" "#,
        r#"table:number-matrix-rows-spanned="18446744073709551615" "#,
        r#"office:value-type="float" office:value="1"/>"#,
        r#"</table:table-row></table:table>"#
    );
    let imported = ambre_xlsx::read_ods(&ods("", tables)).expect("the sheet reads");
    assert_eq!(imported.workbook.sheets.len(), 1);
}

/// `W305`: a row of many cells names no style over many column runs. Each
/// cell finds its column style by a binary search, and the sheet reads.
#[test]
fn many_column_runs_read_their_styles_by_search() {
    let count = 100_000;
    let tables = format!(
        r#"<table:table table:name="S">{}<table:table-row>{}</table:table-row></table:table>"#,
        r#"<table:table-column table:default-cell-style-name="c"/>"#.repeat(count),
        r#"<table:table-cell office:value-type="float" office:value="1"/>"#.repeat(count)
    );
    let imported = ambre_xlsx::read_ods(&ods("", &tables)).expect("the sheet reads");
    assert_eq!(imported.workbook.sheets.len(), 1);
}

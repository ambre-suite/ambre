use super::*;

fn name(text: &str) -> PartName {
    PartName::new(text).expect("a valid name")
}

const IMAGE: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/image");
const HYPERLINK: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink");
const LAYOUT: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout");

/// A package of `count` parts of `size` zero bytes each, `part0.xml` on.
/// Zeros deflate a thousandfold, so the archive stays small.
fn zeros(count: usize, size: Count<Octets>) -> Payload<PackageBytes> {
    let mut writer = PackageWriter::new();
    let size = usize::try_from(size.get()).expect("fits");
    for at in 0..count {
        writer.part(
            name(&format!("part{at}.xml")),
            None,
            Payload::new(vec![0; size]),
        );
    }
    writer.finish().expect("writes")
}

#[test]
fn a_target_resolves_against_its_source_folder() {
    let document = name("word/document.xml");
    assert_eq!(
        document.resolve("media/a.png"),
        Some(name("word/media/a.png"))
    );
    assert_eq!(
        document.resolve("../docProps/core.xml"),
        Some(name("docProps/core.xml"))
    );
    assert_eq!(
        document.resolve("/word/styles.xml"),
        Some(name("word/styles.xml"))
    );
    assert_eq!(
        document.resolve("media\\b.png"),
        Some(name("word/media/b.png"))
    );
    assert_eq!(document.resolve("../../../etc/passwd"), None);
    assert_eq!(root().resolve("word/document.xml"), Some(document.clone()));
    assert_eq!(
        document.relationships(),
        name("word/_rels/document.xml.rels")
    );
    assert_eq!(root().relationships(), name("_rels/.rels"));
}

#[test]
fn a_written_target_is_relative_to_its_source() {
    let slide = name("ppt/slides/slide1.xml");
    assert_eq!(
        relative_target(&slide, &name("ppt/slideLayouts/slideLayout1.xml")),
        "../slideLayouts/slideLayout1.xml"
    );
    assert_eq!(
        relative_target(&name("word/document.xml"), &name("word/media/a.png")),
        "media/a.png"
    );
    assert_eq!(
        relative_target(&root(), &name("xl/workbook.xml")),
        "xl/workbook.xml"
    );
}

#[test]
fn a_written_package_reads_back_with_its_types_and_relationships() {
    let mut writer = PackageWriter::new();
    let main = name("word/document.xml");
    writer.part(
        main.clone(),
        Some("application/main+xml"),
        Payload::new(b"<a/>".to_vec()),
    );
    writer.default_type("png", "image/png");
    writer.part(name("word/media/i.png"), None, Payload::new(vec![1, 2, 3]));
    let slide = name("ppt/slides/slide1.xml");
    let layout = name("ppt/slideLayouts/slideLayout1.xml");
    writer.part(slide.clone(), None, Payload::new(b"<s/>".to_vec()));
    writer.part(layout.clone(), None, Payload::new(b"<l/>".to_vec()));
    writer.relate(
        &root(),
        RelKind::OFFICE_DOCUMENT,
        RelTarget::Part(main.clone()),
    );
    let image = writer.relate(&main, IMAGE, RelTarget::Part(name("word/media/i.png")));
    writer.relate(
        &main,
        HYPERLINK,
        RelTarget::External("https://example.org/?a=1&b=2".to_owned()),
    );
    writer.relate(&slide, LAYOUT, RelTarget::Part(layout.clone()));
    let bytes = writer.finish().expect("writes");

    let mut package = Package::open(bytes.as_bytes()).expect("opens");
    assert_eq!(
        package
            .related(&root(), RelKind::OFFICE_DOCUMENT)
            .expect("rels"),
        Some(main.clone())
    );
    assert_eq!(package.content_type(&main), Some("application/main+xml"));
    assert_eq!(
        package.content_type(&name("WORD/media/i.PNG")),
        Some("image/png")
    );
    let rels = package.relationships(&main).expect("rels");
    assert_eq!(rels[0].id, image);
    assert!(rels[0].is(IMAGE));
    assert_eq!(
        rels[1].target,
        RelTarget::External("https://example.org/?a=1&b=2".to_owned())
    );
    assert_eq!(package.related(&slide, LAYOUT).expect("rels"), Some(layout));
    assert_eq!(
        package.part(&name("word/media/i.png")).expect("reads"),
        Some(Payload::new(vec![1, 2, 3]))
    );
    assert!(package.part_names().contains(&main));
}

#[test]
fn a_strict_relationship_type_reads_as_its_transitional_kind() {
    let kind = RelType("http://purl.oclc.org/ooxml/officeDocument/relationships/image".to_owned());
    assert!(kind.is(IMAGE));
    assert!(!kind.is(HYPERLINK));
}

#[test]
fn the_same_parts_write_the_same_bytes() {
    let write = || {
        let mut writer = PackageWriter::new();
        writer.part(name("a.xml"), None, Payload::new(b"<a/>".to_vec()));
        writer.finish().expect("writes")
    };
    assert_eq!(write(), write());
}

#[test]
fn a_part_added_twice_keeps_its_last_bytes() {
    let mut writer = PackageWriter::new();
    writer.part(
        name("a.xml"),
        Some("one/type"),
        Payload::new(b"<a/>".to_vec()),
    );
    writer.part(
        name("A.xml"),
        Some("two/type"),
        Payload::new(b"<b/>".to_vec()),
    );
    let bytes = writer.finish().expect("writes");
    let mut package = Package::open(bytes.as_bytes()).expect("opens");
    assert_eq!(package.content_type(&name("a.xml")), Some("two/type"));
    assert_eq!(
        package.part(&name("a.xml")).expect("reads"),
        Some(Payload::new(b"<b/>".to_vec()))
    );
}

#[test]
fn a_compound_file_and_noise_are_refused() {
    let mut compound = COMPOUND_FILE.to_vec();
    compound.extend_from_slice(&[0; 64]);
    assert_eq!(Package::open(&compound).err(), Some(OpcError::CompoundFile));
    assert_eq!(Package::open(b"not a zip").err(), Some(OpcError::NotAZip));
}

#[test]
fn a_package_without_content_types_is_refused() {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("a.xml", zip::write::SimpleFileOptions::default())
        .expect("starts");
    zip.write_all(b"<a/>").expect("writes");
    let bytes = zip.finish().expect("finishes").into_inner();
    assert_eq!(
        Package::open(&bytes).err(),
        Some(OpcError::MissingPart(CONTENT_TYPES))
    );
}

#[test]
fn a_document_type_in_the_content_types_is_refused() {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(CONTENT_TYPES, zip::write::SimpleFileOptions::default())
        .expect("starts");
    zip.write_all(b"<!DOCTYPE t [<!ENTITY e SYSTEM \"file:///etc/passwd\">]><Types>&e;</Types>")
        .expect("writes");
    let bytes = zip.finish().expect("finishes").into_inner();
    assert_eq!(
        Package::open(&bytes).err(),
        Some(OpcError::Malformed(CONTENT_TYPES))
    );
}

/// The bound on the part count: one part past [`MAX_PARTS`] refuses the
/// package before any part inflates.
#[test]
fn a_package_past_its_part_count_is_refused() {
    let past = usize::try_from(MAX_PARTS.get()).expect("fits") + 1;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for at in 0..past {
        zip.start_file(format!("p{at}"), options).expect("starts");
    }
    let bytes = zip.finish().expect("finishes").into_inner();
    assert_eq!(Package::open(&bytes).err(), Some(OpcError::TooManyParts));
}

/// The bound on a tree: a content types part holds more empty entries than
/// [`MAX_TREE_NODES`]. It reads under the part bound and deflates to a few
/// kilobytes, and the package is refused.
#[test]
fn a_content_types_part_past_the_node_bound_is_refused() {
    let past = usize::try_from(MAX_TREE_NODES.get()).expect("fits") + 1;
    let part = format!("<Types>{}</Types>", "<a/>".repeat(past));
    assert!(Count::<Octets>::of(part.len()) < MAX_PART_SIZE);
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(CONTENT_TYPES, zip::write::SimpleFileOptions::default())
        .expect("starts");
    zip.write_all(part.as_bytes()).expect("writes");
    let bytes = zip.finish().expect("finishes").into_inner();
    assert!(bytes.len() < 1024 * 1024);
    assert_eq!(
        Package::open(&bytes).err(),
        Some(OpcError::Malformed(CONTENT_TYPES))
    );
}

/// The bound on one part: a part one byte past [`MAX_PART_SIZE`] fails while
/// it inflates, whatever its header claims.
#[test]
fn a_part_that_inflates_past_its_bound_is_refused_while_it_inflates() {
    let bytes = zeros(1, MAX_PART_SIZE.plus(Count::new(1)));
    assert!(bytes.as_bytes().len() < 1024 * 1024);
    let mut package = Package::open(bytes.as_bytes()).expect("opens");
    assert_eq!(
        package.part(&name("part0.xml")).err(),
        Some(OpcError::TooLarge(MAX_PART_SIZE))
    );
}

/// The bound on the parts together: parts each under [`MAX_PART_SIZE`] fail
/// once their sum passes [`MAX_INFLATED`].
#[test]
fn parts_that_inflate_past_the_package_bound_are_refused() {
    // The content types part counts too, so each part leaves it room.
    let size = MAX_PART_SIZE.minus(Count::new(1024));
    let bytes = zeros(3, size);
    let mut package = Package::open(bytes.as_bytes()).expect("opens");
    for at in 0..2 {
        let part = package.part(&name(&format!("part{at}.xml")));
        assert_eq!(part.map(|p| p.map(|p| p.len())), Ok(Some(size)));
    }
    assert_eq!(
        package.part(&name("part2.xml")).err(),
        Some(OpcError::TooLarge(MAX_INFLATED))
    );
}

/// The three bounds sit under the client memory budget of 100 MB
/// (`ambre-spec-budgets.md section 6`).
#[test]
fn the_bounds_hold_under_the_memory_budget() {
    assert!(MAX_PART_SIZE < MAX_INFLATED);
    assert!(MAX_INFLATED.get() < 100 * 1000 * 1000);
}

#[test]
fn a_relationship_to_a_missing_part_is_refused() {
    let mut writer = PackageWriter::new();
    let main = name("ppt/presentation.xml");
    writer.part(main.clone(), None, Payload::new(b"<p/>".to_vec()));
    writer.relate(&main, LAYOUT, RelTarget::Part(name("ppt/missing.xml")));
    assert_eq!(writer.finish().err(), Some(OpcError::DanglingRelationship));

    let mut writer = PackageWriter::new();
    writer.relate(
        &name("ppt/absent.xml"),
        HYPERLINK,
        RelTarget::External("https://example.org".to_owned()),
    );
    assert_eq!(writer.finish().err(), Some(OpcError::DanglingRelationship));
}

#[test]
fn a_part_name_never_climbs_out_of_the_package() {
    assert_eq!(PartName::new(""), None);
    assert_eq!(PartName::new("/"), None);
    assert_eq!(PartName::new("../a.xml"), None);
    assert_eq!(PartName::new("/ppt/../a.xml"), Some(name("a.xml")));
    assert_eq!(
        PartName::new("/ppt//slides/./s.xml"),
        Some(name("ppt/slides/s.xml"))
    );
}

#[test]
fn an_opendocument_package_reads_back_its_parts_under_its_type() {
    let mut writer = odf::OdfWriter::new(odf::OdfType::SPREADSHEET);
    writer.part(
        name("content.xml"),
        odf::OdfType::XML,
        Payload::new(b"<a/>".to_vec()),
    );
    let bytes = writer.finish().expect("writes");
    // The type is the first entry, stored, at the offset a sniffer reads.
    assert_eq!(bytes.as_bytes().get(30..38), Some(b"mimetype".as_slice()));
    assert_eq!(
        bytes.as_bytes().get(38..84),
        Some(odf::OdfType::SPREADSHEET.as_str().as_bytes())
    );
    let mut package = odf::open(bytes.as_bytes(), odf::OdfType::SPREADSHEET).expect("opens");
    let content = package.part(&name("content.xml")).expect("reads");
    assert_eq!(content.map(Payload::into_bytes), Some(b"<a/>".to_vec()));
    let manifest = package
        .part(&name("META-INF/manifest.xml"))
        .expect("reads")
        .expect("a manifest");
    let text = String::from_utf8(manifest.into_bytes()).expect("UTF-8");
    assert!(text.contains("manifest:full-path=\"content.xml\""));
}

#[test]
fn an_opendocument_package_of_another_type_or_none_is_refused() {
    let text = odf::OdfWriter::new(odf::OdfType::XML)
        .finish()
        .expect("writes");
    assert!(matches!(
        odf::open(text.as_bytes(), odf::OdfType::SPREADSHEET),
        Err(OpcError::OtherDocument)
    ));
    let bare = PackageWriter::new().finish().expect("writes");
    assert!(matches!(
        odf::open(bare.as_bytes(), odf::OdfType::SPREADSHEET),
        Err(OpcError::MissingPart(_))
    ));
}

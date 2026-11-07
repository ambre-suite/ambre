//! The document locale register of a board (`ambre-spec-locale.md`).

use ambre_core::DocumentLocale;
use ambre_core::id::{Document, Id};
use kurbo::{Point, Size};

use super::ToileState;
use crate::logic::command::{InitContent, ToileCommand, ToileOutcome};
use crate::logic::model::{ElementKind, ShapeKind};

fn locale(text: &str) -> DocumentLocale {
    DocumentLocale::parse(text).unwrap()
}

fn set(st: &mut ToileState, to: DocumentLocale) -> ToileOutcome {
    st.handle_command(ToileCommand::SetLocale { locale: to })
}

/// Every root but `meta`, read whole: what the board stores besides its
/// registers.
fn content(st: &ToileState) -> Vec<(String, loro::LoroValue)> {
    let loro::LoroValue::Map(roots) = st.doc.get_deep_value() else {
        panic!("a board reads as a map");
    };
    let mut held: Vec<(String, loro::LoroValue)> = roots
        .iter()
        .filter(|(name, _)| name.as_str() != ambre_core::crdt::FoundationRoot::Meta.as_str())
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    held.sort_by(|a, b| a.0.cmp(&b.0));
    held
}

#[test]
fn a_new_board_names_no_locale() {
    assert_eq!(ToileState::new().locale(), DocumentLocale::none());
}

/// Setting the locale is one delta, the delta every edit sends, and no element
/// moves with it. The locale it already holds writes nothing.
#[test]
fn setting_the_locale_is_one_delta_and_rewrites_no_stored_value() {
    let mut st = ToileState::new();
    st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Shape(ShapeKind::Rect),
        at: Point::new(5.0, 5.0),
        size: Size::new(10.0, 10.0),
        content: InitContent::None,
        fill: None,
        correlate: None,
    });
    let before = content(&st);
    assert!(set(&mut st, locale("fr-FR")).update.is_some());
    assert_eq!(st.locale(), locale("fr-FR"));
    assert_eq!(content(&st), before);
    assert!(set(&mut st, locale("fr-FR")).update.is_none());
    assert!(set(&mut st, DocumentLocale::none()).update.is_some());
    assert_eq!(st.locale(), DocumentLocale::none());
    assert_eq!(content(&st), before);
}

/// Two peers set the locale at once: both read one tag. A later write, made
/// after the first was seen, stands on both.
#[test]
fn two_peers_setting_the_locale_converge_on_one_tag() {
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    let fr = set(&mut a, locale("fr")).update.unwrap();
    let de = set(&mut b, locale("de")).update.unwrap();
    a.apply_remote(&de);
    b.apply_remote(&fr);
    assert_eq!(a.locale(), b.locale());
    assert!(a.locale() == locale("fr") || a.locale() == locale("de"));

    let it = set(&mut b, locale("it")).update.unwrap();
    a.apply_remote(&it);
    assert_eq!(a.locale(), locale("it"));
}

#[test]
fn the_locale_survives_a_save_and_an_open() {
    let mut st = ToileState::new();
    set(&mut st, locale("pt-BR"));
    let file = st
        .export_ambre_plaintext(Id::<Document>::mint())
        .expect("the board exports");
    let mut opened = ToileState::new();
    opened.import_ambre_plaintext(&file).unwrap();
    assert_eq!(opened.locale(), locale("pt-BR"));
}

/// A version-1 board, written before the `locale` key, migrates to 2 and
/// opens with no language: the bytes it stored still read.
#[test]
fn a_version_1_board_opens_with_no_locale() {
    let doc = loro::LoroDoc::new();
    doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
        .insert(ambre_core::schema::MetaKey::SchemaVersion.as_str(), 1_i64)
        .unwrap();
    doc.commit();
    let bytes = ambre_core::Payload::<ambre_core::SnapshotBytes>::new(
        doc.export(loro::ExportMode::Snapshot).unwrap(),
    );
    let mut st = ToileState::new();
    st.import_snapshot(&bytes).unwrap();
    assert_eq!(st.schema_version(), crate::logic::schema::SCHEMA_VERSION);
    assert_eq!(st.locale(), DocumentLocale::none());
}

/// A board a peer wrote a malformed tag into reads as no language.
#[test]
fn a_malformed_stored_locale_reads_as_none() {
    let st = ToileState::new();
    st.doc
        .get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
        .insert(ambre_core::schema::MetaKey::Locale.as_str(), "fr_FR")
        .unwrap();
    assert_eq!(st.locale(), DocumentLocale::none());
}

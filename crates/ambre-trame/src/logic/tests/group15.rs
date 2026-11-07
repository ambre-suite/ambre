// The document locale register of a workbook (`ambre-spec-locale.md`).

use super::*;
use ambre_core::DocumentLocale;
use ambre_core::id::Document;

fn locale(text: &str) -> DocumentLocale {
    DocumentLocale::parse(text).unwrap()
}

fn set(state: &mut TrameState, to: DocumentLocale) -> TrameOutcome {
    state.handle_command(TrameCommand::SetLocale { locale: to })
}

/// Every root but `meta`, read whole: what the workbook stores besides its
/// registers.
fn content(state: &TrameState) -> Vec<(String, loro::LoroValue)> {
    let loro::LoroValue::Map(roots) = state.doc.get_deep_value() else {
        panic!("a workbook reads as a map");
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
fn a_new_workbook_names_no_locale() {
    assert_eq!(TrameState::new().locale(), DocumentLocale::none());
}

/// Setting the locale is one delta, the delta every edit sends, and the cells,
/// the styles and the lines stay as they were.
#[test]
fn setting_the_locale_is_one_delta_and_rewrites_no_stored_value() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
        text: "1234.5".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(1),
        col: Pos::<Col>::new(0),
        text: "=A1*2".to_owned(),
    });
    let before = content(&state);

    let outcome = set(&mut state, locale("fr-FR"));
    assert!(outcome.update.is_some(), "the write leaves as a delta");
    assert_eq!(state.locale(), locale("fr-FR"));
    assert_eq!(content(&state), before);

    let outcome = set(&mut state, DocumentLocale::none());
    assert!(outcome.update.is_some());
    assert_eq!(state.locale(), DocumentLocale::none());
    assert_eq!(content(&state), before);
    assert_eq!(read_raw(&mut state, 0, 0), "1234.5");
    assert_eq!(read_raw(&mut state, 1, 0), "=A1*2");
}

#[test]
fn setting_the_locale_it_holds_writes_nothing() {
    let mut state = TrameState::new();
    set(&mut state, locale("de-CH"));
    assert!(set(&mut state, locale("de-CH")).update.is_none());
    assert!(
        set(&mut TrameState::new(), DocumentLocale::none())
            .update
            .is_none()
    );
}

/// Two peers set the locale at once: both read one tag. A later write, made
/// after the first was seen, stands on both.
#[test]
fn two_peers_setting_the_locale_converge_on_one_tag() {
    let mut a = TrameState::new();
    let mut b = TrameState::new();
    b.import_snapshot(&a.export_snapshot()).unwrap();
    set(&mut a, locale("fr"));
    set(&mut b, locale("de"));
    let (sa, sb) = (a.export_snapshot(), b.export_snapshot());
    a.import_snapshot(&sb).unwrap();
    b.import_snapshot(&sa).unwrap();
    assert_eq!(a.locale(), b.locale());
    assert!(a.locale() == locale("fr") || a.locale() == locale("de"));

    set(&mut b, locale("it"));
    a.import_snapshot(&b.export_snapshot()).unwrap();
    assert_eq!(a.locale(), locale("it"));
}

#[test]
fn the_locale_survives_a_save_and_an_open() {
    let mut state = TrameState::new();
    set(&mut state, locale("pt-BR"));
    let file = state
        .export_ambre_plaintext(Id::<Document>::mint())
        .expect("the workbook exports");
    let mut opened = TrameState::new();
    opened.import_ambre_plaintext(&file).unwrap();
    assert_eq!(opened.locale(), locale("pt-BR"));
}

/// A workbook a peer wrote a malformed tag into reads as no language, never
/// as the text it holds.
#[test]
fn a_malformed_stored_locale_reads_as_none() {
    let state = TrameState::new();
    state
        .doc
        .get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
        .insert(ambre_core::schema::MetaKey::Locale.as_str(), "fr_FR")
        .unwrap();
    assert_eq!(state.locale(), DocumentLocale::none());
}

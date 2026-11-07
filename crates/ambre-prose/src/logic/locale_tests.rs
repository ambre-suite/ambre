//! The document locale register of a Prose document (`ambre-spec-locale.md`).

use ambre_core::DocumentLocale;
use ambre_core::id::{Document, Id};

use super::ProseState;
use crate::events::{ProseCommand, ProseOutcome};

fn locale(text: &str) -> DocumentLocale {
    DocumentLocale::parse(text).unwrap()
}

fn set(state: &mut ProseState, to: DocumentLocale) -> ProseOutcome {
    state.handle_command(ProseCommand::SetLocale { locale: to })
}

/// Every root but `meta`, read whole: what the document stores besides its
/// registers.
fn content(state: &ProseState) -> Vec<(String, loro::LoroValue)> {
    let loro::LoroValue::Map(roots) = state.doc.get_deep_value() else {
        panic!("a document reads as a map");
    };
    let mut held: Vec<(String, loro::LoroValue)> = roots
        .iter()
        .filter(|(name, _)| name.as_str() != ambre_core::crdt::FoundationRoot::Meta.as_str())
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    held.sort_by(|a, b| a.0.cmp(&b.0));
    held
}

fn new_document() -> ProseState {
    ProseState::new().expect("a fresh document")
}

#[test]
fn a_new_document_names_no_locale() {
    assert_eq!(
        ambre_crdt::read_locale(&new_document().doc),
        DocumentLocale::none()
    );
}

/// Setting the locale is one delta, the delta every edit sends, and no block
/// moves with it. The locale it already holds writes nothing.
#[test]
fn setting_the_locale_is_one_delta_and_rewrites_no_stored_value() {
    let mut state = new_document();
    let before = content(&state);
    assert!(set(&mut state, locale("fr-FR")).update.is_some());
    assert_eq!(ambre_crdt::read_locale(&state.doc), locale("fr-FR"));
    assert_eq!(content(&state), before);
    assert!(set(&mut state, locale("fr-FR")).update.is_none());
    assert!(set(&mut state, DocumentLocale::none()).update.is_some());
    assert_eq!(ambre_crdt::read_locale(&state.doc), DocumentLocale::none());
    assert_eq!(content(&state), before);
}

/// Two peers set the locale at once: both read one tag. A later write, made
/// after the first was seen, stands on both.
#[test]
fn two_peers_setting_the_locale_converge_on_one_tag() {
    let mut a = new_document();
    let mut b = new_document();
    let fr = set(&mut a, locale("fr")).update.unwrap();
    let de = set(&mut b, locale("de")).update.unwrap();
    a.apply_remote(&de);
    b.apply_remote(&fr);
    let merged = ambre_crdt::read_locale(&a.doc);
    assert_eq!(merged, ambre_crdt::read_locale(&b.doc));
    assert!(merged == locale("fr") || merged == locale("de"));

    let it = set(&mut b, locale("it")).update.unwrap();
    a.apply_remote(&it);
    assert_eq!(ambre_crdt::read_locale(&a.doc), locale("it"));
}

#[test]
fn the_locale_survives_a_save_and_an_open() {
    let mut state = new_document();
    set(&mut state, locale("pt-BR"));
    let file = state
        .export_ambre_plaintext(Id::<Document>::mint())
        .expect("the document exports");
    let mut opened = new_document();
    opened.import_ambre_plaintext(&file).unwrap();
    assert_eq!(ambre_crdt::read_locale(&opened.doc), locale("pt-BR"));
}

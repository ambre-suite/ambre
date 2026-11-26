// ambre-chrome/src/passphrase.rs
// ============================================================================
// The passphrase prompt of a sealed `.ambre` file (ambre-spec-docfile.md
// section 5).
//
// App-agnostic, like the container it serves. One operation seals a board and a
// workbook, so one dialog asks for the passphrase. The text never leaves this
// component except through `on_submit` -- not into a store, not into a URL, and
// not into a log line.
// ============================================================================

use ambre_core::{Confidential, Passphrase};
use leptos::prelude::*;

use crate::Dialog;

/// What the prompt is for, which decides how many fields it shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PassphraseIntent {
    /// Sealing a file: the prompt takes the passphrase twice, and the two must
    /// match. A typo here is unrecoverable, because nothing else can open the
    /// file. So the confirmation is a rule of the surface, not a courtesy
    /// (`DOCFILE-015`).
    Seal,
    /// Opening a file: one field, and the attempt reports a wrong answer.
    Open,
}

/// Whether the prompt may submit what the user typed, and the message to show
/// when it may not.
///
/// A pure function rather than three lines inside the click handler. It is the
/// only rule this component makes, and the handler around it is DOM glue. A
/// prompt that lets a mismatched confirmation through seals the file under a
/// passphrase the user never meant. Both spellings derive a key, both seal, and
/// nothing downstream can notice (`DOCFILE-015`). An empty confirmation is the same
/// mistake as a mismatched one, and the message says so rather than counting
/// characters at the user. The account setup of the launcher holds its two
/// fields to the same rule (`CHROME-098`).
///
/// # Errors
///
/// The refusal the prompt shows: an empty first field, or a seal whose
/// confirmation differs.
pub fn vet(
    first: &Confidential<Passphrase>,
    second: &Confidential<Passphrase>,
    intent: PassphraseIntent,
) -> Result<(), PassphraseRefusal> {
    if first.is_empty() {
        return Err(PassphraseRefusal::Empty);
    }
    if intent == PassphraseIntent::Seal && !first.matches(second) {
        return Err(PassphraseRefusal::Mismatch);
    }
    Ok(())
}

/// Why the prompt holds a submission back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PassphraseRefusal {
    /// Nothing typed in the first field.
    Empty,
    /// A seal whose confirmation differs from the first field.
    Mismatch,
}

impl PassphraseRefusal {
    /// The line the prompt shows under its fields.
    pub fn message(self) -> &'static str {
        match self {
            Self::Empty => {
                ambre_commands::tr!("Type a passphrase.", "Saisissez une phrase de passe.")
            }
            Self::Mismatch => ambre_commands::tr!(
                "The two passphrases do not match.",
                "Les deux phrases de passe ne correspondent pas."
            ),
        }
    }
}

/// The line under the fields: the prompt's own refusal, else the weakness
/// the Sync worker named for the last passphrase, else nothing.
///
/// The prompt's own refusal wins, because it answers what is typed now. The
/// worker's refusal answers the passphrase already sent (`DOCFILE-022`).
pub fn prompt_line(
    own: Option<PassphraseRefusal>,
    worker: Option<ambre_core::PassphraseWeakness>,
) -> &'static str {
    match (own, worker) {
        (Some(refusal), _) => refusal.message(),
        (None, Some(weakness)) => weakness.message(),
        (None, None) => "",
    }
}

/// A modal that asks for one passphrase.
///
/// Submitting hands the text to `on_submit` and closes; cancelling (Escape, the
/// backdrop, the button) closes and emits nothing. Both clear the fields, so a
/// reopened dialog never shows the last one's text. The Sync worker may
/// refuse a passphrase. A caller that reopens the prompt then sets
/// `weakness`, and the line says why until the person types or cancels.
#[component]
pub fn PassphraseDialog(
    /// Visibility, owned by the caller (the command that opens it).
    open: RwSignal<crate::Openness>,
    /// Seal or open: the two differ only by the confirmation field and the
    /// wording.
    intent: PassphraseIntent,
    /// The passphrase the user settled on. Fired once per submission.
    #[prop(into)]
    on_submit: Callback<Confidential<Passphrase>>,
    /// The weakness the Sync worker named for the last passphrase sent, if
    /// the caller tracks one (`DOCFILE-022`).
    #[prop(optional)]
    weakness: Option<RwSignal<Option<ambre_core::PassphraseWeakness>>>,
) -> impl IntoView {
    let first = RwSignal::new(String::new());
    let second = RwSignal::new(String::new());
    let error = RwSignal::new(None::<PassphraseRefusal>);
    let sealing = intent == PassphraseIntent::Seal;
    let forget_weakness = move || {
        if let Some(weakness) = weakness {
            weakness.set(None);
        }
    };

    let clear = move || {
        first.set(String::new());
        second.set(String::new());
        error.set(None);
        forget_weakness();
    };

    let submit = move || {
        let typed = Confidential::<Passphrase>::new(first.get_untracked());
        let confirmed = Confidential::<Passphrase>::new(second.get_untracked());
        if let Err(refusal) = vet(&typed, &confirmed, intent) {
            error.set(Some(refusal));
            return;
        }
        on_submit.run(typed);
        clear();
        open.set(crate::Openness::Closed);
    };

    // Enter submits from either field: the second field is the natural place to
    // press it, and the first is where a one-field prompt ends.
    let on_key = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Enter" {
            submit();
        }
    };

    let heading = if sealing {
        ambre_commands::tr!(
            "Save to an encrypted file",
            "Enregistrer dans un fichier chiffr\u{e9}"
        )
    } else {
        ambre_commands::tr!("Open an encrypted file", "Ouvrir un fichier chiffr\u{e9}")
    };
    let note = if sealing {
        "This passphrase is the only way to open the file. Ambre cannot recover it, \
         and it is not your account passphrase."
    } else {
        ambre_commands::tr!(
            "Type the passphrase this file was sealed with.",
            "Saisissez la phrase de passe qui a scell\u{e9} ce fichier."
        )
    };

    view! {
        <Dialog open=open class={ambre_core::StaticKey::<ambre_core::CssClasses>::new("ambre-passphrase")}>
            <div class="ambre-passphrase-panel">
                <h2>{heading}</h2>
                <p class="ambre-passphrase-note">{note}</p>
                <input
                    type="password"
                    autocomplete="new-password"
                    placeholder=ambre_commands::tr!("Passphrase", "Phrase de passe")
                    prop:value=move || first.get()
                    on:input=move |ev| {
                        first.set(event_target_value(&ev));
                        error.set(None);
                        forget_weakness();
                    }
                    on:keydown=on_key
                />
                <Show when=move || sealing>
                    <input
                        type="password"
                        autocomplete="new-password"
                        placeholder=ambre_commands::tr!("Passphrase again", "Phrase de passe, \u{e0} nouveau")
                        prop:value=move || second.get()
                        on:input=move |ev| {
                            second.set(event_target_value(&ev));
                            error.set(None);
                            forget_weakness();
                        }
                        on:keydown=on_key
                    />
                </Show>
                <p class="ambre-passphrase-error" aria-live="polite">
                    {move || prompt_line(error.get(), weakness.and_then(|w| w.get()))}
                </p>
                <div class="ambre-dialog-actions">
                    <button on:click=move |_| {
                        clear();
                        open.set(crate::Openness::Closed);
                    }>{ambre_commands::tr!("Cancel", "Annuler")}</button>
                    <button on:click=move |_| submit()>
                        {if sealing { ambre_commands::tr!("Save", "Enregistrer") } else { ambre_commands::tr!("Open", "Ouverts") }}
                    </button>
                </div>
            </div>
        </Dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vetted(
        first: &str,
        second: &str,
        intent: PassphraseIntent,
    ) -> Result<(), PassphraseRefusal> {
        vet(
            &Confidential::<Passphrase>::new(first.to_owned()),
            &Confidential::<Passphrase>::new(second.to_owned()),
            intent,
        )
    }

    #[test]
    fn sealing_needs_the_two_fields_to_agree() {
        assert_eq!(
            vetted("correct horse", "correct horse", PassphraseIntent::Seal),
            Ok(())
        );
        assert_eq!(
            vetted("correct horse", "correct hors", PassphraseIntent::Seal),
            Err(PassphraseRefusal::Mismatch)
        );
        // The confirmation left empty is a mismatch, not a second empty-field
        // case: a submission on the first field alone would seal the file.
        assert_eq!(
            vetted("correct horse", "", PassphraseIntent::Seal),
            Err(PassphraseRefusal::Mismatch)
        );
    }

    #[test]
    fn opening_asks_once_and_ignores_the_second_field() {
        // Opening renders no confirmation field, so its signal keeps whatever
        // the last seal left in it. The rule must not read it.
        assert_eq!(vetted("correct horse", "", PassphraseIntent::Open), Ok(()));
        assert_eq!(
            vetted("correct horse", "stale", PassphraseIntent::Open),
            Ok(())
        );
    }

    /// `DOCFILE-022`: the weakness the Sync worker named reads on the prompt
    /// in the words of the floor, its reason included. The prompt's own
    /// refusal of what is typed now comes first.
    #[test]
    fn the_prompt_shows_the_weakness_the_worker_named() {
        use ambre_core::PassphraseWeakness::{CommonPassword, TooShort};
        assert_eq!(prompt_line(None, None), "");
        assert_eq!(prompt_line(None, Some(TooShort)), TooShort.message());
        assert!(prompt_line(None, Some(CommonPassword)).contains("common password"));
        assert_eq!(
            prompt_line(Some(PassphraseRefusal::Mismatch), Some(TooShort)),
            PassphraseRefusal::Mismatch.message()
        );
    }

    #[test]
    fn nothing_is_submitted_from_an_empty_prompt() {
        // Sealing under an empty passphrase derives a key like any other, so
        // the refusal is here rather than in the worker.
        assert_eq!(
            vetted("", "", PassphraseIntent::Seal),
            Err(PassphraseRefusal::Empty)
        );
        assert_eq!(
            vetted("", "", PassphraseIntent::Open),
            Err(PassphraseRefusal::Empty)
        );
    }
}

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
/// characters at the user.
fn vet(first: &str, second: &str, sealing: bool) -> Result<(), &'static str> {
    if first.is_empty() {
        return Err("Type a passphrase.");
    }
    if sealing && second != first {
        return Err("The two passphrases do not match.");
    }
    Ok(())
}

/// A modal that asks for one passphrase.
///
/// Submitting hands the text to `on_submit` and closes; cancelling (Escape, the
/// backdrop, the button) closes and emits nothing. Both clear the fields, so a
/// reopened dialog never shows the last one's text.
#[component]
pub fn PassphraseDialog(
    /// Visibility, owned by the caller (the command that opens it).
    open: RwSignal<bool>,
    /// Seal or open: the two differ only by the confirmation field and the
    /// wording.
    intent: PassphraseIntent,
    /// The passphrase the user settled on. Fired once per submission.
    #[prop(into)]
    on_submit: Callback<String>,
    /// Fired after the modal opens (the launcher suspends the canvas rAF).
    /// Absent where there is no canvas to suspend -- the landing screen.
    #[prop(optional, into)]
    on_open: Option<Callback<()>>,
    /// Fired after the modal closes (the launcher resumes it).
    #[prop(optional, into)]
    on_close: Option<Callback<()>>,
) -> impl IntoView {
    let on_open = on_open.unwrap_or_else(|| Callback::new(|()| {}));
    let on_close = on_close.unwrap_or_else(|| Callback::new(|()| {}));
    let first = RwSignal::new(String::new());
    let second = RwSignal::new(String::new());
    let error = RwSignal::new(None::<&'static str>);
    let sealing = intent == PassphraseIntent::Seal;

    let clear = move || {
        first.set(String::new());
        second.set(String::new());
        error.set(None);
    };

    let submit = move || {
        let typed = first.get_untracked();
        if let Err(message) = vet(&typed, &second.get_untracked(), sealing) {
            error.set(Some(message));
            return;
        }
        on_submit.run(typed);
        clear();
        open.set(false);
    };

    // Enter submits from either field: the second field is the natural place to
    // press it, and the first is where a one-field prompt ends.
    let on_key = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Enter" {
            submit();
        }
    };

    let heading = if sealing {
        "Save to an encrypted file"
    } else {
        "Open an encrypted file"
    };
    let note = if sealing {
        "This passphrase is the only way to open the file. Ambre cannot recover it, \
         and it is not your account passphrase."
    } else {
        "Type the passphrase this file was sealed with."
    };

    view! {
        <Dialog
            open=open
            on_open=on_open
            on_close=on_close
            class="ambre-passphrase"
        >
            <div class="ambre-passphrase-panel">
                <h2>{heading}</h2>
                <p class="ambre-passphrase-note">{note}</p>
                <input
                    type="password"
                    autocomplete="new-password"
                    placeholder="Passphrase"
                    prop:value=move || first.get()
                    on:input=move |ev| {
                        first.set(event_target_value(&ev));
                        error.set(None);
                    }
                    on:keydown=on_key
                />
                <Show when=move || sealing>
                    <input
                        type="password"
                        autocomplete="new-password"
                        placeholder="Passphrase again"
                        prop:value=move || second.get()
                        on:input=move |ev| {
                            second.set(event_target_value(&ev));
                            error.set(None);
                        }
                        on:keydown=on_key
                    />
                </Show>
                <p class="ambre-passphrase-error" aria-live="polite">
                    {move || error.get().unwrap_or("")}
                </p>
                <div class="ambre-dialog-actions">
                    <button on:click=move |_| {
                        clear();
                        open.set(false);
                    }>"Cancel"</button>
                    <button on:click=move |_| submit()>
                        {if sealing { "Save" } else { "Open" }}
                    </button>
                </div>
            </div>
        </Dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealing_needs_the_two_fields_to_agree() {
        assert_eq!(vet("correct horse", "correct horse", true), Ok(()));
        assert!(vet("correct horse", "correct hors", true).is_err());
        // The confirmation left empty is a mismatch, not a second empty-field
        // case: a submission on the first field alone would seal the file.
        assert!(vet("correct horse", "", true).is_err());
    }

    #[test]
    fn opening_asks_once_and_ignores_the_second_field() {
        // Opening renders no confirmation field, so its signal keeps whatever
        // the last seal left in it. The rule must not read it.
        assert_eq!(vet("correct horse", "", false), Ok(()));
        assert_eq!(vet("correct horse", "stale", false), Ok(()));
    }

    #[test]
    fn nothing_is_submitted_from_an_empty_prompt() {
        // Sealing under an empty passphrase derives a key like any other, so
        // the refusal is here rather than in the worker.
        assert!(vet("", "", true).is_err());
        assert!(vet("", "", false).is_err());
    }
}

// ambre-trame/src/chrome/formula_bar.rs
// ============================================================================
// The formula bar: a name box plus an editable HTML `<input>` bound to the
// active cell's raw value / live edit buffer. Caret, selection, IME and
// clipboard are handled natively by the input element.
// ============================================================================

use leptos::html;
use leptos::prelude::*;

use crate::render::{CommitDirection, FormulaAction};

/// The formula bar component. A name box shows the active cell reference,
/// such as `"B2"`; an HTML `<input>` binds to the cell's raw value or the
/// live edit buffer. The input is always editable; focus or typed characters
/// open an edit session.
///
/// `on_action` fires on Enter (commit Down), Tab (commit Right), or Escape
/// (cancel). The caret, selection, IME, and clipboard are handled natively by
/// the HTML input element.
#[component]
pub fn FormulaBar(
    cell_ref: RwSignal<String>,
    content: RwSignal<String>,
    on_action: Callback<FormulaAction>,
    /// The workbook's locale and the reader's half, which the date preview
    /// reads.
    locale: RwSignal<Option<crate::ui::locale::LocaleView>>,
) -> impl IntoView {
    let input_ref = NodeRef::<html::Input>::new();
    // A date whose day and month could swap shows its long form before the
    // commit (`LOCALE-022`).
    let preview = move || {
        let view = locale.get()?;
        let text = crate::ui::locale::date_preview(&content.get(), &view)?;
        Some(view! { <span class="formula-date-preview" role="status">{text}</span> })
    };

    // When the signal changes from outside, push the new value into the DOM
    // input without resetting the caret. A new cell was selected, or an edit
    // was seeded. The comparison avoids fighting the user's typing.
    Effect::new(move |_| {
        let val = content.get();
        if let Some(input) = input_ref.get()
            && val != input.value()
        {
            input.set_value(&val);
        }
    });

    let on_input = move |ev: web_sys::Event| {
        content.set(event_target_value(&ev));
    };

    let on_keydown = move |ev: web_sys::KeyboardEvent| match ev.key().as_str() {
        "Enter" => {
            ev.prevent_default();
            // Ctrl/Cmd+Enter fills the whole selection with the value.
            if ev.ctrl_key() || ev.meta_key() {
                on_action.run(FormulaAction::CommitFill);
            } else {
                on_action.run(FormulaAction::Commit(CommitDirection::Down));
            }
        }
        "Tab" => {
            ev.prevent_default();
            on_action.run(FormulaAction::Commit(CommitDirection::Right));
        }
        "Escape" => {
            ev.prevent_default();
            on_action.run(FormulaAction::Cancel);
        }
        _ => {}
    };

    view! {
        <span class="name-box">{move || cell_ref.get()}</span>
        <input
            type="text"
            id="ambre-formula-input"
            class="formula-input"
            node_ref=input_ref
            on:input=on_input
            on:keydown=on_keydown
        />
        {preview}
    }
}

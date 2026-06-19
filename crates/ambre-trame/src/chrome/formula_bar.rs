// ambre-trame/src/chrome/formula_bar.rs
// ============================================================================
// The formula bar: a name box plus an editable HTML `<input>` bound to the
// active cell's raw value / live edit buffer. Caret, selection, IME and
// clipboard are handled natively by the input element.
// ============================================================================

use leptos::html;
use leptos::prelude::*;

use crate::render::{CommitDirection, FormulaAction};

/// The formula bar component: a name box showing the active cell reference
/// (e.g. `"B2"`) and an HTML `<input>` bound to the cell's raw value or the
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
) -> impl IntoView {
    let input_ref = NodeRef::<html::Input>::new();

    // When the signal changes from outside (e.g. a new cell was selected or an
    // edit was seeded), push the new value into the DOM input without resetting
    // the caret. The comparison avoids fighting the user's typing.
    create_effect(move |_| {
        let val = content.get();
        if let Some(input) = input_ref.get() {
            if val != input.value() {
                input.set_value(&val);
            }
        }
    });

    let on_input = move |ev: web_sys::Event| {
        content.set(event_target_value(&ev));
    };

    let on_keydown = {
        let content = content;
        move |ev: web_sys::KeyboardEvent| {
            match ev.key().as_str() {
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
            }
        }
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
    }
}

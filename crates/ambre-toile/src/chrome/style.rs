// ambre-toile/src/chrome/style.rs
// ============================================================================
// The style / property inspector (toile-ui SS5quinquies.B), wasm-only. A DOM side
// panel shown only when the selection is non-empty; it edits the selected
// elements' `ElementStyle` -- fill, stroke, stroke width, opacity -- plus the
// z-order and Lock / Unlock actions.
//
// Style edits are DEBOUNCED and emitted as ONE `RestyleElements` over the whole
// selection (the style register is written whole, SS2.E): the panel keeps a local
// working `ElementStyle` seeded from the published common style, mutates it on
// each control, and (after a quiet interval) writes it to the shared `restyle`
// signal the canvas view drains. Selection is UI-only state the view owns, so the
// view supplies the ids -- the inspector never names them.
//
// The z-order / lock buttons are pure `CommandId`s already resolved by the view
// against its selection (`ui::menu`), so they reuse the shared `on_command`
// channel verbatim -- identical to the context menu and top menu bar. Values that
// differ across the selection show a "Mixed" hint (the mask from `ui::style`).
//
// Colour editing uses the shared `ambre_chrome::ColorPopover` in ARGB mode
// (`alpha=true`), the very control Trame's cell fg/bg picker uses (SS5quinquies.C).
// Every control does `on:mousedown prevent_default` to keep DOM focus on the
// canvas text pipe -- the palette / FormatBar hazard.
// ============================================================================

use std::time::Duration;

use ambre_bus::app::toile::ElementStyle;
use ambre_chrome::commands::CommandId;
use ambre_chrome::{ColorPopover, Debouncer, FontPicker};
use leptos::prelude::*;

/// Map a stored `FontSpec.family` string to the picker's family label (`None` =
/// Default): only the offered labels round-trip; anything else (e.g. the raw
/// default "sans-serif") reads as Default.
fn family_label(family: &str) -> Option<String> {
    matches!(family, "Sans" | "Serif" | "Monospace").then(|| family.to_owned())
}

use crate::ui::menu;
use crate::ui::menu::CtxScope;
use crate::ui::style::StyleModel;

/// How long the inspector waits after the last edit before emitting the coalesced
/// `RestyleElements` (SS5quinquies.B, "edits are debounced"). Short enough to feel
/// live, long enough to fold a slider drag into a single command.
const RESTYLE_DEBOUNCE: Duration = Duration::from_millis(120);

/// The style / property inspector. `model` is the published common style (`None`
/// hides the panel); `scope` gates the lock/unlock buttons; `on_command` is the
/// shared command sink (z-order / lock); `restyle` is the whole-style channel the
/// view turns into `RestyleElements` against the live selection.
#[component]
pub fn StyleInspector(
    model: RwSignal<Option<StyleModel>>,
    scope: RwSignal<CtxScope>,
    on_command: Callback<CommandId>,
    restyle: RwSignal<Option<ElementStyle>>,
) -> impl IntoView {
    // The local working copy of the whole style register: seeded from the common
    // style and mutated in place by every control. The four edited fields also get
    // their own signals so `ColorPopover` / the inputs can bind directly.
    let working = RwSignal::new(ElementStyle::default());
    let fill = RwSignal::new(None::<u32>);
    let stroke = RwSignal::new(None::<u32>);
    let stroke_w = RwSignal::new(1.0_f32);
    let opacity = RwSignal::new(1.0_f32);
    // Text (foreground) colour + font controls (§5quinquies.B / §6).
    let text_color = RwSignal::new(None::<u32>);
    let font_family = RwSignal::new(None::<String>);
    let font_size = RwSignal::new(0.0_f32);
    let bold = RwSignal::new(false);
    let italic = RwSignal::new(false);

    // Re-seed the local controls whenever the view publishes a new common style
    // (selection change / external restyle). This only reads `model`, so pushing
    // the values back into the local signals never re-triggers it.
    Effect::new(move |_| {
        if let Some(m) = model.get() {
            working.set(m.base.clone());
            fill.set(m.base.fill);
            stroke.set(m.base.stroke);
            stroke_w.set(m.base.stroke_w);
            opacity.set(m.base.opacity);
            text_color.set(m.base.text_color);
            font_family.set(family_label(&m.base.font.family));
            font_size.set(m.base.font.size);
            bold.set(m.base.font.weight >= 700);
            italic.set(m.base.font.italic);
        }
    });

    let debouncer = Debouncer::new();

    // Fold the current control values into `working` and schedule one debounced
    // `RestyleElements`. Called after every edit; the timer coalesces bursts.
    let commit = move || {
        working.update(|s| {
            s.fill = fill.get_untracked();
            s.stroke = stroke.get_untracked();
            s.stroke_w = stroke_w.get_untracked();
            s.opacity = opacity.get_untracked();
            s.text_color = text_color.get_untracked();
            // A `None` family (Default) stores the default sans label, so the
            // renderer resolves it to the bundled sans face.
            s.font.family = font_family
                .get_untracked()
                .unwrap_or_else(|| "Sans".to_owned());
            s.font.size = font_size.get_untracked().max(0.0);
            s.font.weight = if bold.get_untracked() { 700 } else { 400 };
            s.font.italic = italic.get_untracked();
        });
        let style = working.get_untracked();
        debouncer.call(RESTYLE_DEBOUNCE, move || restyle.set(Some(style)));
    };

    // Colour picks: update the local signal (so the swatch strip tracks) and commit.
    let on_fill = Callback::new(move |c: Option<u32>| {
        fill.set(c);
        commit();
    });
    let on_stroke = Callback::new(move |c: Option<u32>| {
        stroke.set(c);
        commit();
    });
    let on_text_color = Callback::new(move |c: Option<u32>| {
        text_color.set(c);
        commit();
    });
    let on_font = Callback::new(move |f: Option<String>| {
        font_family.set(f);
        commit();
    });

    // A "Mixed" hint shown beside a control whose value differs across selection.
    let mixed_hint = move |differs: bool| {
        differs.then(|| view! { <span class="ambre-inspector-mixed">"Mixed"</span> })
    };

    // Reactive per-field mixed flags (default when no model, i.e. panel hidden).
    let mixed = move || model.get().map(|m| m.mixed).unwrap_or_default();

    // A z-order / lock command button routing through the shared sink.
    let cmd_button = move |command: CommandId, label: &'static str, icon: icondata::Icon| {
        view! {
            <button
                aria-label=label
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| on_command.run(command)
            >
                <leptos_icons::Icon icon=icon />
            </button>
        }
    };

    view! {
        // The panel renders only for a non-empty selection.
        <Show when=move || model.get().is_some() fallback=|| ()>
            <div class="ambre-inspector" role="region" aria-label="Style inspector">
                // -- Appearance: fill / stroke / stroke width / opacity --
                <div class="ambre-inspector-section">
                    <div class="ambre-inspector-row">
                        <ColorPopover
                            title="Fill"
                            icon=icondata::MdiFormatColorFill
                            current=fill
                            default=Signal::derive(|| 0xffff_ffffu32)
                            reset_label="No fill"
                            on_pick=on_fill
                            alpha=true
                        />
                        {move || mixed_hint(mixed().fill)}
                    </div>
                    <div class="ambre-inspector-row">
                        <ColorPopover
                            title="Stroke"
                            icon=icondata::MdiBorderColor
                            current=stroke
                            default=Signal::derive(|| 0xff33_3333u32)
                            reset_label="No stroke"
                            on_pick=on_stroke
                            alpha=true
                        />
                        {move || mixed_hint(mixed().stroke)}
                    </div>
                    <div class="ambre-inspector-row">
                        <label>"Width"</label>
                        <input
                            type="number"
                            min="0"
                            step="0.5"
                            prop:value=move || format!("{:.1}", stroke_w.get())
                            on:mousedown=move |ev| ev.prevent_default()
                            on:input=move |ev| {
                                if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                                    stroke_w.set(v.max(0.0));
                                    commit();
                                }
                            }
                        />
                        {move || mixed_hint(mixed().stroke_w)}
                    </div>
                    <div class="ambre-inspector-row">
                        <label>"Opacity"</label>
                        <input
                            type="range"
                            min="0"
                            max="100"
                            prop:value=move || format!("{:.0}", opacity.get() * 100.0)
                            on:mousedown=move |ev| ev.prevent_default()
                            on:input=move |ev| {
                                if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                                    opacity.set((v / 100.0).clamp(0.0, 1.0));
                                    commit();
                                }
                            }
                        />
                        {move || mixed_hint(mixed().opacity)}
                    </div>
                </div>

                // -- Text: colour / font / size / bold / italic (§6) --
                <div class="ambre-inspector-section">
                    <div class="ambre-inspector-row">
                        <ColorPopover
                            title="Text colour"
                            icon=icondata::MdiFormatColorText
                            current=text_color
                            default=Signal::derive(|| 0xff1a_1a1au32)
                            reset_label="Default"
                            on_pick=on_text_color
                            alpha=true
                        />
                        {move || mixed_hint(mixed().text_color)}
                    </div>
                    <div class="ambre-inspector-row">
                        <FontPicker current=font_family.into() on_pick=on_font />
                        {move || mixed_hint(mixed().font_family)}
                    </div>
                    <div class="ambre-inspector-row">
                        <label>"Size"</label>
                        <input
                            type="number"
                            min="0"
                            step="1"
                            placeholder="Auto"
                            prop:value=move || {
                                let s = font_size.get();
                                if s > 0.0 { format!("{s:.0}") } else { String::new() }
                            }
                            on:mousedown=move |ev| ev.prevent_default()
                            on:input=move |ev| {
                                // Empty clears to the auto sentinel (0); a positive
                                // value overrides (a sticky then stops auto-sizing).
                                let raw = event_target_value(&ev);
                                let v = raw.trim().parse::<f32>().unwrap_or(0.0);
                                font_size.set(v.max(0.0));
                                commit();
                            }
                        />
                        {move || mixed_hint(mixed().font_size)}
                    </div>
                    <div
                        class="ambre-inspector-section ambre-commandbar"
                        role="toolbar"
                        aria-label="Text style"
                    >
                        <button
                            aria-label="Bold"
                            aria-pressed=move || bold.get().then_some("true")
                            on:mousedown=move |ev| ev.prevent_default()
                            on:click=move |_| {
                                bold.update(|b| *b = !*b);
                                commit();
                            }
                        >
                            <leptos_icons::Icon icon=icondata::MdiFormatBold />
                        </button>
                        <button
                            aria-label="Italic"
                            aria-pressed=move || italic.get().then_some("true")
                            on:mousedown=move |ev| ev.prevent_default()
                            on:click=move |_| {
                                italic.update(|i| *i = !*i);
                                commit();
                            }
                        >
                            <leptos_icons::Icon icon=icondata::MdiFormatItalic />
                        </button>
                        {move || mixed_hint(mixed().bold || mixed().italic)}
                    </div>
                </div>

                // -- Z-order (Bring to Front / Forward / Backward / Send to Back) --
                <div class="ambre-inspector-section ambre-commandbar" role="toolbar" aria-label="Order">
                    {cmd_button(menu::BRING_FRONT, "Bring to front", icondata::MdiArrangeBringToFront)}
                    {cmd_button(menu::FORWARD, "Bring forward", icondata::MdiArrangeBringForward)}
                    {cmd_button(menu::BACKWARD, "Send backward", icondata::MdiArrangeSendBackward)}
                    {cmd_button(menu::SEND_BACK, "Send to back", icondata::MdiArrangeSendToBack)}
                </div>

                // -- Lock / Unlock (shown per the selection's lock state) --
                <div class="ambre-inspector-section ambre-commandbar" role="toolbar" aria-label="Lock">
                    <Show when=move || scope.get().any_unlocked fallback=|| ()>
                        {cmd_button(menu::LOCK, "Lock", icondata::MdiLock)}
                    </Show>
                    <Show when=move || scope.get().any_locked fallback=|| ()>
                        {cmd_button(menu::UNLOCK, "Unlock", icondata::MdiLockOpenVariant)}
                    </Show>
                </div>
            </div>
        </Show>
    }
}

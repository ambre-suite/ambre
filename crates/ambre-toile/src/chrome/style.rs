// ambre-toile/src/chrome/style.rs
// ============================================================================
// The style / property inspector (`toile-styles`), wasm-only. A DOM side
// panel shown only when the selection is non-empty. It edits the selected
// elements' `ElementStyle` -- fill, stroke, stroke width, opacity -- plus the
// z-order and Lock / Unlock actions.
//
// Style edits are DEBOUNCED and emitted as ONE `RestyleElements` over the whole
// selection, the style register being written whole (`TOILE-018`). The panel
// keeps a local working `ElementStyle` seeded from the published common style
// and mutates it on each control. After a quiet interval it writes that style to
// the shared `restyle` signal the canvas view drains. Selection is UI-only state
// the view owns, so the view supplies the ids -- the inspector never names them.
// Each control writes its one field through `StyleEdit`, so a field nobody
// touched keeps the selection's value, a font family the picker cannot name too.
//
// The z-order / lock buttons are pure `CommandId`s already resolved by the view
// against its selection (`ui::menu`). So they reuse the shared `on_command`
// path verbatim -- identical to the context menu and top menu bar. Values that
// differ across the selection show a "Mixed" hint (the mask from `ui::style`).
//
// Colour editing uses the shared `ambre_chrome::ColorPopover` with its opacity
// slider, the very control Trame's cell fg/bg picker uses (`toile-styles`).
// Every control does `on:mousedown prevent_default` to keep DOM focus on the
// canvas text pipe -- the palette / FormatBar hazard.
// ============================================================================

use std::time::Duration;

use ambre_bus::app::toile::{ElementStyle, FontFamily, FontSizeChoice};
use ambre_chrome::commands::CommandId;
use ambre_chrome::{ColorPopover, Debouncer, FontPicker, FontSizeSelect, FormatToggle};
use ambre_core::{FontSize, Rgba};
use leptos::prelude::*;

use crate::ui::floatbar::StyleEdit;
use crate::ui::menu;
use crate::ui::menu::CtxScope;
use crate::ui::style::{Mix, StyleModel};

/// How long the inspector waits after the last edit before emitting the coalesced
/// `RestyleElements` (`toile-styles`, "edits are debounced"). Short enough to feel
/// live, long enough to fold a slider drag into a single command.
const RESTYLE_DEBOUNCE: Duration = Duration::from_millis(120);

/// The style / property inspector. `model` is the published common style (`None`
/// hides the panel), and `scope` gates the lock/unlock buttons. `on_command` is
/// the shared command sink (z-order / lock). `restyle` is the whole-style callback
/// the view turns into `RestyleElements` against the live selection.
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
    let fill = RwSignal::new(None::<Rgba>);
    let stroke = RwSignal::new(None::<Rgba>);
    let stroke_w = RwSignal::new(1.0_f32);
    let opacity = RwSignal::new(ambre_core::Opacity::OPAQUE);
    // Text (foreground) colour + font controls.
    let text_color = RwSignal::new(None::<Rgba>);
    let font_family = RwSignal::new(None::<FontFamily>);
    let font_size = RwSignal::new(FontSizeChoice::DEFAULT);
    let bold = RwSignal::new(false);
    let italic = RwSignal::new(false);
    let underline = RwSignal::new(false);
    let strikethrough = RwSignal::new(false);

    // Re-seed the local controls whenever the view publishes a new common style
    // (selection change / external restyle). This only reads `model`, so pushing
    // the values back into the local signals never re-triggers it.
    Effect::new(move |_| {
        if let Some(m) = model.get() {
            working.set(m.base.clone());
            fill.set(m.base.fill);
            stroke.set(m.base.stroke);
            stroke_w.set(m.base.stroke_w.get());
            opacity.set(m.base.opacity);
            text_color.set(m.base.text_color);
            // A family the picker does not offer shows the "Default" entry.
            font_family.set(
                m.base
                    .font
                    .family
                    .clone()
                    .filter(|f| f.picker_label().is_some()),
            );
            font_size.set(m.base.font.size);
            bold.set(m.base.font.weight.is_bold());
            italic.set(m.base.font.italic.as_bool());
            underline.set(m.base.font.underline.as_bool());
            strikethrough.set(m.base.font.strikethrough.as_bool());
        }
    });

    let debouncer = Debouncer::new();

    // Write the one edited field into `working` and schedule one debounced
    // `RestyleElements`. Every other field keeps the value the selection had: a
    // control the user did not touch never rewrites its field (`TOILE-UI-172`).
    // The timer coalesces bursts.
    let commit = move |edit: StyleEdit| {
        working.update(|s| edit.apply_to(s));
        let style = working.get_untracked();
        debouncer.call(RESTYLE_DEBOUNCE, move || restyle.set(Some(style)));
    };

    // Colour picks: update the local signal (so the swatch strip tracks) and commit.
    let on_fill = Callback::new(move |c: Option<Rgba>| {
        fill.set(c);
        commit(StyleEdit::Fill(c));
    });
    let on_stroke = Callback::new(move |c: Option<Rgba>| {
        stroke.set(c);
        commit(StyleEdit::Stroke(c));
    });
    let on_text_color = Callback::new(move |c: Option<Rgba>| {
        text_color.set(c);
        commit(StyleEdit::TextColor(c));
    });
    let on_font = Callback::new(move |f: Option<FontFamily>| {
        font_family.set(f.clone());
        // The picker's "Default" reset is `None`: the element then names no
        // family, which keeps it apart from an explicit Sans.
        commit(StyleEdit::Font(f));
    });
    // Size: the shared dropdown speaks sizes (`Option<FontSize>`, `None` =
    // auto), which is the choice the style holds.
    let size_current = Signal::derive(move || font_size.get().as_fixed());
    let on_size = Callback::new(move |p: Option<FontSize>| {
        let size = FontSizeChoice::picked(p);
        font_size.set(size);
        commit(StyleEdit::Size(size));
    });
    // The four format toggles flip their signal and commit.
    let on_bold = Callback::new(move |()| {
        bold.update(|b| *b = !*b);
        commit(StyleEdit::Bold(ambre_core::Weight::from_bool(
            bold.get_untracked(),
        )));
    });
    let on_italic = Callback::new(move |()| {
        italic.update(|i| *i = !*i);
        commit(StyleEdit::Italic(ambre_core::Slant::from_bool(
            italic.get_untracked(),
        )));
    });
    let on_underline = Callback::new(move |()| {
        underline.update(|u| *u = !*u);
        commit(StyleEdit::Underline(ambre_core::Underline::from_bool(
            underline.get_untracked(),
        )));
    });
    let on_strike = Callback::new(move |()| {
        strikethrough.update(|s| *s = !*s);
        commit(StyleEdit::Strikethrough(
            ambre_core::Strikethrough::from_bool(strikethrough.get_untracked()),
        ));
    });

    // A "Mixed" hint shown beside a control whose value differs across selection.
    let mixed_hint = move |field: Mix| {
        field
            .is_mixed()
            .then(|| view! { <span class="ambre-inspector-mixed">"Mixed"</span> })
    };

    // Reactive per-field mixed flags (default when no model, that is, panel hidden).
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
                            default=Signal::derive(|| Rgba::from_bits(0xffff_ffff))
                            reset_label="No fill"
                            on_pick=on_fill
                        />
                        {move || mixed_hint(mixed().fill)}
                    </div>
                    <div class="ambre-inspector-row">
                        <ColorPopover
                            title="Stroke"
                            icon=icondata::MdiBorderColor
                            current=stroke
                            default=Signal::derive(|| Rgba::from_bits(0x3333_33ff))
                            reset_label="No stroke"
                            on_pick=on_stroke
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
                                    commit(StyleEdit::StrokeWidth(ambre_bus::app::toile::StyleLength::new(v)));
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
                            prop:value=move || format!("{:.0}", opacity.get().get() * 100.0)
                            on:mousedown=move |ev| ev.prevent_default()
                            on:input=move |ev| {
                                if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                                    let picked = ambre_core::Opacity::clamped(v / 100.0);
                                    opacity.set(picked);
                                    commit(StyleEdit::Opacity(picked));
                                }
                            }
                        />
                        {move || mixed_hint(mixed().opacity)}
                    </div>
                </div>

                // -- Text: colour / font / size / bold / italic --
                <div class="ambre-inspector-section">
                    <div class="ambre-inspector-row">
                        <ColorPopover
                            title="Text colour"
                            icon=icondata::MdiFormatColorText
                            current=text_color
                            default=Signal::derive(|| Rgba::from_bits(0x1a1a_1aff))
                            reset_label="Default"
                            on_pick=on_text_color
                        />
                        {move || mixed_hint(mixed().text_color)}
                    </div>
                    <div class="ambre-inspector-row">
                        <FontPicker current=font_family.into() on_pick=on_font />
                        {move || mixed_hint(mixed().font_family)}
                    </div>
                    <div class="ambre-inspector-row">
                        <label>"Size"</label>
                        <FontSizeSelect current=size_current on_pick=on_size auto_label="Auto" />
                        {move || mixed_hint(mixed().font_size)}
                    </div>
                    // Bold / Italic / Underline / Strikethrough -- shared toggles, same
                    // order as Trame's toolbar (`ambre_chrome::FormatToggle`).
                    <div
                        class="ambre-inspector-section ambre-commandbar"
                        role="toolbar"
                        aria-label="Text style"
                    >
                        <FormatToggle
                            label="Bold"
                            icon=icondata::BsTypeBold
                            pressed=Signal::derive(move || ambre_chrome::Pressed::from_bool(bold.get()))
                            on_toggle=on_bold
                        />
                        <FormatToggle
                            label="Italic"
                            icon=icondata::BsTypeItalic
                            pressed=Signal::derive(move || ambre_chrome::Pressed::from_bool(italic.get()))
                            on_toggle=on_italic
                        />
                        <FormatToggle
                            label="Underline"
                            icon=icondata::BsTypeUnderline
                            pressed=Signal::derive(move || ambre_chrome::Pressed::from_bool(underline.get()))
                            on_toggle=on_underline
                        />
                        <FormatToggle
                            label="Strikethrough"
                            icon=icondata::BsTypeStrikethrough
                            pressed=Signal::derive(move || ambre_chrome::Pressed::from_bool(strikethrough.get()))
                            on_toggle=on_strike
                        />
                        {move || mixed_hint(
                            mixed().bold | mixed().italic | mixed().underline
                                | mixed().strikethrough,
                        )}
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

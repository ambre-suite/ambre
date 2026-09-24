// ambre-app/src/shell/app_root.rs
// The single root view (AppRoot): persistent chrome + canvas, landing overlay
// on `/`. Reads its wiring from context; the document binding is driven outside
// this tree by the per-frame route poll in `main`.

use super::super::*;

/// Resolve what the user typed into an address the board may hold, and place it
/// (`TOILE-087`, `TOILE-088`).
///
/// A refusal stays in the dialog. The user reads it beside the field they typed
/// into and fixes it there, rather than finding a box on the board that never
/// paints. `normalize_web_address` is the gate -- only `https` survives it, so
/// nothing that could run in the board's stead reaches a frame.
fn place_web_embed(
    typed: RwSignal<String>,
    error: RwSignal<Option<&'static str>>,
    open: RwSignal<bool>,
    placed: RwSignal<Option<toile_chrome::PlacedEmbed>>,
) {
    match toile_chrome::normalize_web_address(&typed.get_untracked()) {
        Ok(url) => {
            placed.set(Some(toile_chrome::PlacedEmbed {
                target: toile_chrome::EmbedTarget::Web(url),
                at: None,
            }));
            open.set(false);
        }
        Err(e) => error.set(Some(e.message())),
    }
}

/// Take the typed address, put it through the same gate an embed's address goes
/// through, and hand the board a link to it (`TOILE-066`). Only `https`
/// survives, and a refusal stays in the dialog where the user can still fix it.
fn set_web_link(
    typed: RwSignal<String>,
    error: RwSignal<Option<&'static str>>,
    open: RwSignal<bool>,
    edit: RwSignal<Option<toile_chrome::LinkEdit>>,
) {
    match toile_chrome::normalize_web_address(&typed.get_untracked()) {
        Ok(url) => {
            edit.set(Some(toile_chrome::LinkEdit(Some(
                ambre_bus::app::toile::LinkTarget::Url(url),
            ))));
            open.set(false);
        }
        Err(e) => error.set(Some(e.message())),
    }
}

/// Perform the half of a followed link that leaves the board (`TOILE-065`,
/// `TOILE-114`). An address opens in a fresh tab with `noopener,noreferrer`, so
/// the page can neither reach back into this window nor learn where it came
/// from. The board itself never fetches it (`TOILE-110`). A document opens the
/// same way the launcher opens one, reloading only when the target's render kind
/// differs from the booted one.
fn follow_shell_nav(
    nav: toile_chrome::ShellNav,
    navigate: &impl Fn(&str, leptos_router::NavigateOptions),
    booted: ambre_bus::DocType,
) {
    match nav {
        toile_chrome::ShellNav::Url(url) => {
            if let Some(window) = web_sys::window() {
                let _ = window.open_with_url_and_target_and_features(
                    &url,
                    "_blank",
                    "noopener,noreferrer",
                );
            }
        }
        toile_chrome::ShellNav::Document {
            document_id,
            doc_type,
        } => {
            let path = crate::route::doc_path(doc_type, document_id);
            if crate::route::needs_reload(doc_type, booted) {
                if let Some(window) = web_sys::window() {
                    let _ = window.location().set_href(&path);
                }
            } else {
                navigate(&path, Default::default());
            }
        }
    }
}

/// The single root view, rendered inside `<Router>` so it can read the client
/// route. Renders the persistent chrome + canvas and overlays the landing on the
/// `/` route. The document binding (open/close, canvas swap, rAF gate) is driven
/// OUTSIDE this tree by the per-frame route poll (see `pre_frame` in `main`).
/// The canvas handle is `!Send`, and the `<Router>` subtree must be Send.
#[component]
pub(crate) fn AppRoot() -> impl IntoView {
    let Wiring {
        to_logic,
        clipboard_paste,
        raf,
        theme_ctl,
        toggles,
        toile_toggles,
        style_state,
        anchor_pos,
        toasts,
        username,
        docs,
        members,
        dev_mode,
        current_doc,
        filter_popover,
        filter_values,
        find_state,
        find_result,
        find_state_toile,
        find_result_toile,
        boot_is_toile,
        boot_is_prose,
        prose_figures,
        prose_signals,
        formula_ref,
        formula_text,
        formula_action,
        tofu_prompt,
        tofu_confirm,
        invite_role,
        sel_stats,
        sel_stats_toile,
        present_toile,
        presenter_index,
        presenter_count,
        presenter_notes,
        presenter_frame,
        broadcasting_toile,
        following_toile,
        presenter_peer_toile,
        perf_snap,
        zoom,
        ctx_scope,
        filter_active,
        sel_bounds,
        toile_ctx_scope,
        toile_menu_cmd,
        toile_active_tool,
        toile_cursor,
        toile_style_model,
        toile_restyle,
        toile_float_bar,
        toile_quick_style,
        toile_comments,
        toile_deck,
        toile_dropped_file,
        toile_placed_embed,
        toile_set_link,
        toile_followed_link,
        sort_dialog,
        chrome_status,
        sheets,
        active_sheet,
    } = use_context::<Wiring>().expect("Wiring provided by mount_shell");

    // The reactive client route, used only to show/hide the landing overlay.
    let location = use_location();
    let is_landing = move || {
        matches!(
            route::parse(&location.pathname.get()),
            route::Route::Landing
        )
    };

    // Theme toggle drives only the button glyph; `theme_ctl` (canvas) and the
    // document `data-theme` (CSS custom properties) are the real switches.
    let theme_sig = RwSignal::new(theme_ctl.get());

    // About modal: opening it suspends the canvas rAF, closing it resumes.
    let about_open = RwSignal::new(false);
    let on_open = Callback::new({
        let raf = raf.clone();
        move |()| raf.pause()
    });
    let on_close = Callback::new({
        let raf = raf.clone();
        move |()| raf.resume()
    });

    // Sealed-save modal: the passphrase of a `.ambre` file the user is about to
    // write (`DOCFILE-014`). Same rAF suspend/resume as every other modal.
    let seal_open = RwSignal::new(false);

    // Share modal: same rAF suspend/resume. Opening it requests the current
    // member list so the dialog renders it; the relay fills `members` on the
    // `DocumentMembers` response.
    let share_open = RwSignal::new(false);
    let share_on_open = Callback::new({
        let raf = raf.clone();
        let to_logic = to_logic.clone();
        move |()| {
            raf.pause();
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = to_logic.try_send(LogicCommand::Rest {
                    id: RequestId::next(),
                    cmd: RestCommand::ListDocumentMembers { document_id },
                });
            }
        }
    });
    let share_on_close = Callback::new({
        let raf = raf.clone();
        move |()| raf.resume()
    });

    // Embed a web page (`TOILE-087`). The shell owns the dialog because it owns
    // the DOM. The board is handed an address that has already been through
    // `normalize_web_address`. A refusal is read while the dialog is still on
    // screen, rather than after a box lands on the board.
    let embed_web_open = RwSignal::new(false);
    let embed_web_text = RwSignal::new(String::new());
    let embed_web_error = RwSignal::new(None::<&'static str>);
    // Embed another Ambre document (`TOILE-086`). Only the launcher knows which
    // documents this user may open, so only it can offer the list. It reads the
    // names the document list already carries and opens nothing, so placing an
    // embed never touches the document it points at (`TOILE-225`).
    let embed_doc_open = RwSignal::new(false);
    // Put a link on the selected element (`toile-links`). The chooser is the
    // shell's for the same two reasons as the embed dialogs. An address is
    // validated where the user can still read the refusal, and only the launcher
    // knows which documents this user may open. The frame list is the deck's,
    // which already holds every frame on the board -- including the off-screen
    // ones a link exists to reach.
    let link_open = RwSignal::new(false);
    // Follow the outbound half of a link the board handed back (`toile-links`).
    // The board decided; this only performs, which is why nothing here reads a
    // target or writes one.
    {
        let navigate = leptos_router::hooks::use_navigate();
        Effect::new(move |_| {
            let Some(nav) = toile_followed_link.get() else {
                return;
            };
            toile_followed_link.set(None);
            follow_shell_nav(
                nav,
                &navigate,
                crate::route::booted_kind(boot_is_toile, boot_is_prose),
            );
        });
    }
    // A Prose link followed with the modifier leaves through the same shell
    // path (`PROSE-UI-071`): the page hands the target over and never opens it.
    if boot_is_prose {
        let navigate = leptos_router::hooks::use_navigate();
        Effect::new(move |_| {
            let Some(url) = prose_signals.followed.get() else {
                return;
            };
            prose_signals.followed.set(None);
            follow_shell_nav(
                toile_chrome::ShellNav::Url(url),
                &navigate,
                crate::route::booted_kind(boot_is_toile, boot_is_prose),
            );
        });
        // The link cursor over a link's text, the text cursor elsewhere
        // (`PROSE-UI-153`).
        Effect::new(move |_| {
            let cursor = if prose_signals.over_link.get() {
                "pointer"
            } else {
                "text"
            };
            if let Some(doc) = web_sys::window().and_then(|w| w.document())
                && let Some(el) = doc.get_element_by_id("ambre-canvas")
                && let Some(html) = el.dyn_ref::<web_sys::HtmlElement>()
            {
                let _ = html.style().set_property("cursor", cursor);
            }
        });
    }
    let link_text = RwSignal::new(String::new());
    let link_error = RwSignal::new(None::<&'static str>);

    // TOFU verification modal (`ambre-spec-key-verification.md`). `tofu_open` mirrors
    // `tofu_prompt` (Some => open): the relay sets the prompt on a New/Changed
    // key, and this effect drives the native `<dialog>` accordingly. Same rAF
    // suspend/resume as the other modals.
    let tofu_open = RwSignal::new(false);
    Effect::new(move |_| tofu_open.set(tofu_prompt.get().is_some()));
    let tofu_on_open = Callback::new({
        let raf = raf.clone();
        move |()| raf.pause()
    });
    let tofu_on_close = Callback::new({
        let raf = raf.clone();
        // A native close (Escape / backdrop / Cancel) is a refusal: clear the
        // pending prompt so nothing is pinned and phase 2 never fires.
        move |()| {
            raf.resume();
            tofu_prompt.set(None);
        }
    });

    // Invite form state: the typed invitee (email, or a bare username in dev
    // mode). The role to grant lives in `invite_role` (shared via `Wiring`, since
    // the two-phase TOFU invite dispatches phase 2 outside this component).
    let invitee_input = RwSignal::new(String::new());

    // Canvas right-click -> context menu (the launcher owns the listener since
    // the canvas is created by the AppShell).
    let ctx_menu = RwSignal::new(None::<(f64, f64)>);
    install_canvas_context_menu(ctx_menu);
    // A file dropped on a board reaches the importer at the point it landed on
    // (`toile-import-export`). Same reason the listener lives here: the canvas
    // belongs to the AppShell, not to the view that paints into it.
    if boot_is_toile {
        install_canvas_file_drop(toile_dropped_file, toasts);
    }

    // Toile cursor feedback (toile-tools): reflect the active tool on the
    // canvas element as a CSS cursor. That is crosshair while a creation
    // tool is armed, grab for Hand, arrow for Select. A pure function of the
    // shared active-tool signal, applied here in the chrome -- it never
    // touches the canvas render pass. Mounted only for a Toile board.
    if boot_is_toile {
        Effect::new(move |_| {
            // A position-dependent override wins: a directional resize or grab
            // cursor over a selection edge or corner, or during a transform.
            // Otherwise the cursor is the pure function of the active tool.
            let cursor = toile_cursor.get().unwrap_or_else(|| {
                std::borrow::Cow::Borrowed(ambre_toile::ui::palette::cursor_css(
                    toile_active_tool.get(),
                ))
            });
            if let Some(doc) = web_sys::window().and_then(|w| w.document())
                && let Some(el) = doc.get_element_by_id("ambre-canvas")
                && let Some(html) = el.dyn_ref::<web_sys::HtmlElement>()
            {
                let _ = html.style().set_property("cursor", cursor.as_ref());
            }
        });
    }

    // The single command path shared by every trigger (toolbar, selection bar,
    // context menu): a `CommandId` -> the routed document. `Callback` is `Copy`,
    // so it is handed to each app-chrome component below.
    // The command palette is a launcher-level overlay: no relay drives it, so
    // the signal lives here rather than in the wiring.
    let palette_open = RwSignal::new(false);

    let on_command = Callback::new({
        let tx = to_logic.clone();
        move |id: CommandId| {
            // The palette is a UI-only intent in both applications: it opens an
            // overlay and reaches no Logic thread (`CHROME-008`).
            if id == trame_chrome::PALETTE
                || id == toile_chrome::PALETTE
                || id == ambre_prose::commands::PALETTE
            {
                palette_open.set(true);
                return;
            }
            // Prose declares one command beside the palette: the native save.
            // It maps to one typed command, so it is built here rather than
            // through a table (`PROSE-IO-020`).
            if boot_is_prose {
                if id == ambre_prose::commands::SAVE_FILE
                    && let Some(document_id) = current_doc.get_untracked()
                {
                    let _ = tx.try_send(LogicCommand::App {
                        document_id,
                        command: AppCommand::Prose(
                            ambre_bus::app::prose::ProseCommand::ExportDocument {
                                format: ambre_bus::app::prose::ExportFormat::Ambre,
                            },
                        ),
                    });
                }
                return;
            }
            // The sealed save is a UI-only intent in both applications: a
            // `CommandId` carries no passphrase. So the dialog collects one and
            // emits the typed command itself (`DOCFILE-015`). One arm takes the
            // two ids because what follows them is identical: the container,
            // and the operation on it, are one for every application
            // (`DOCFILE-007`).
            if id == trame_chrome::SAVE_SEALED_FILE || id == toile_chrome::SAVE_SEALED_FILE {
                seal_open.set(true);
                return;
            }
            // Toile board: every context-menu id is forwarded into the canvas view,
            // which resolves it against its (UI-only) selection (`toile-menus`).
            // The Toile Find command still opens the Toile panel, handled below.
            if boot_is_toile {
                if id == toile_chrome::FIND {
                    find_state_toile.set(true);
                } else if id == toile_chrome::IMPORT_FILE {
                    // The file dialog is the shell's (`TOILE-IO-020`): the board
                    // learns of the file only once its bytes are read.
                    open_board_import_picker(toile_dropped_file, toasts);
                } else if id == toile_chrome::EMBED_WEB {
                    embed_web_text.set(String::new());
                    embed_web_error.set(None);
                    embed_web_open.set(true);
                } else if id == toile_chrome::EMBED_DOC {
                    // Refresh the list rather than trust whatever the landing
                    // screen last fetched: a document shared since then is one
                    // the user expects to find here.
                    let _ = tx.try_send(LogicCommand::Rest {
                        id: RequestId::next(),
                        cmd: RestCommand::ListDocuments,
                    });
                    embed_doc_open.set(true);
                } else if id == toile_chrome::LINK_SET {
                    link_text.set(String::new());
                    link_error.set(None);
                    // Both lists are refreshed rather than trusted. A frame may
                    // have been added since the panel last read the deck. A
                    // document may have been shared since the landing screen
                    // fetched. The user expects to find both here.
                    if let Some(document_id) = current_doc.get_untracked() {
                        let _ = tx.try_send(LogicCommand::App {
                            document_id,
                            command: AppCommand::Toile(
                                ambre_bus::app::toile::ToileCommand::ReadDeck,
                            ),
                        });
                    }
                    let _ = tx.try_send(LogicCommand::Rest {
                        id: RequestId::next(),
                        cmd: RestCommand::ListDocuments,
                    });
                    link_open.set(true);
                } else {
                    toile_menu_cmd.set(Some(id));
                }
                return;
            }
            // `FILTER_COLUMN` is a UI-only intent: open the popover anchored under
            // the active cell and ask the Logic thread for the column's distinct
            // values. Logic resolves the active column itself. Everything else
            // flows through the shared CommandId -> AppCommand path.
            if id == trame_chrome::FIND || id == trame_chrome::REPLACE {
                // UI-only intents (`trame-find-replace`): open the find/replace panel in the
                // matching mode. The panel emits FindNext/ReplaceAll itself.
                let mode = if id == trame_chrome::FIND {
                    trame_chrome::FindMode::Find
                } else {
                    trame_chrome::FindMode::Replace
                };
                find_state.set(Some(trame_chrome::FindReplaceState { mode }));
            } else if id == trame_chrome::SORT_RANGE_MULTI {
                // UI-only intent (`trame-sort`): open the sort dialog over
                // the current selection (published by the canvas view). The dialog
                // emits the typed `SortRangeMulti` itself via `on_sort_apply`.
                let (r1, c1, r2, c2) = sel_bounds.get_untracked().unwrap_or((0, 0, 0, 0));
                sort_dialog.set(Some(trame_chrome::SortDialogState { r1, c1, r2, c2 }));
            } else if id == trame_chrome::FILTER_COLUMN {
                let (x, y) = client_anchor(anchor_pos.get_untracked());
                // `col` is provisional; the ColumnValues reply carries the real one.
                filter_popover.set(Some(trame_chrome::FilterPopoverState { col: 0, x, y }));
                filter_values.set(Vec::new());
                if let Some(document_id) = current_doc.get_untracked() {
                    let _ = tx.try_send(LogicCommand::App {
                        document_id,
                        command: AppCommand::Trame(TrameCommand::RequestActiveColumnValues),
                    });
                }
            } else if id == trame_chrome::BOLD || id == trame_chrome::ITALIC {
                // Bold/Italic are part of the single style channel: toggle the
                // toolbar state and emit a `SetCellStyle` (`trame-cell-styles`).
                let is_bold = id == trame_chrome::BOLD;
                let cur = if is_bold {
                    style_state.bold.get_untracked()
                } else {
                    style_state.italic.get_untracked()
                };
                let mut patch = ambre_bus::StylePatch::default();
                if is_bold {
                    patch.bold = Some(!cur);
                } else {
                    patch.italic = Some(!cur);
                }
                if let Some(document_id) = current_doc.get_untracked() {
                    let _ = tx.try_send(LogicCommand::App {
                        document_id,
                        command: AppCommand::Trame(TrameCommand::SetCellStyle { patch }),
                    });
                }
            } else if id == trame_chrome::COPY || id == trame_chrome::CUT {
                // Copy/cut (`trame-clipboard`): focus the canvas text pipe and ask the
                // browser to fire the `copy`/`cut` DOM event the host listeners
                // service. They write both the TSV and the typed payload.
                if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                    if let Ok(Some(ta)) = doc.query_selector(".ambre-input")
                        && let Some(el) = ta.dyn_ref::<web_sys::HtmlElement>()
                    {
                        let _ = el.focus();
                    }
                    let cmd = if id == trame_chrome::COPY {
                        "copy"
                    } else {
                        "cut"
                    };
                    if let Ok(html_doc) = doc.dyn_into::<web_sys::HtmlDocument>() {
                        let _ = html_doc.exec_command(cmd);
                    }
                }
            } else if id == trame_chrome::PASTE {
                // Paste (`trame-clipboard`): `execCommand("paste")` is blocked by browsers,
                // so read the OS clipboard via the async Clipboard API (allowed in this
                // click gesture). The read is decoded and handed to the host, which
                // routes it to the active view through the same ladder as Ctrl+V
                // (`UI-THREAD-024`, `UI-THREAD-025`).
                let clipboard = web_sys::window().map(|w| w.navigator().clipboard());
                // `navigator.clipboard` is undefined outside a secure context, such as
                // http:// on a LAN IP. There the button cannot read at all, so tell
                // the user instead of failing silently.
                let available = clipboard
                    .as_ref()
                    .map(|c| {
                        let v: &wasm_bindgen::JsValue = c.as_ref();
                        !v.is_undefined() && !v.is_null()
                    })
                    .unwrap_or(false);
                if let Some(clipboard) = clipboard.filter(|_| available) {
                    let paste_tx = clipboard_paste.clone();
                    // Call `read()` synchronously, still inside the click's user
                    // gesture: Firefox shows its mandatory "paste" confirmation
                    // right away instead of deferring it to the next click.
                    let read_promise = clipboard.read();
                    wasm_bindgen_futures::spawn_local(paste_from_clipboard(
                        read_promise,
                        clipboard,
                        paste_tx,
                        toasts,
                    ));
                } else {
                    toasts.warning("Clipboard unavailable here (needs HTTPS); use Ctrl+V to paste");
                }
            } else if id == trame_chrome::IMPORT_CSV {
                // Interop (`trame-import-export`): open a file picker, read the chosen
                // CSV bytes, and send them to the Logic thread. It creates a new sheet
                // and converts client-side (Zero-Knowledge).
                if let Some(document_id) = current_doc.get_untracked() {
                    open_csv_picker(document_id, tx.clone(), toasts);
                }
            } else {
                dispatch_to_current(id, current_doc, &tx);
            }
        }
    });

    // Formatting toolbar: a control's `StylePatch` -> `SetCellStyle` over the
    // tracked selection (`trame-cell-styles`). The Logic thread resolves the
    // geometry from the selection it keeps via `SetActiveCell`.
    let on_style = Callback::new({
        let tx = to_logic.clone();
        move |patch: ambre_bus::StylePatch| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::SetCellStyle { patch }),
                });
            }
        }
    });

    // Apply the popover's chosen filter to the active column. The Logic thread
    // resolves the sheet/column from the tracked active cell.
    let on_filter_apply = Callback::new({
        let tx = to_logic.clone();
        move |kind: ambre_bus::FilterKind| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::ApplyFilterToActiveColumn { kind }),
                });
            }
        }
    });
    // Multi-column sort (`trame-sort`): the dialog emits the levels. Forward them
    // as the parameterless `SortSelectionMulti`, like the single-column sort
    // intents. The Logic thread resolves the range and active sheet from the
    // tracked selection. The dialog's snapshotted bounds are used only to label
    // its column picker, so the keys are range-relative and match Logic's view.
    let on_sort_apply = Callback::new({
        let tx = to_logic.clone();
        move |req: trame_chrome::SortApply| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::SortSelectionMulti {
                        sort_keys: req.sort_keys,
                        has_header: req.has_header,
                    }),
                });
            }
        }
    });
    // Find & Replace (`trame-find-replace`): the panel emits these requests; the Logic thread
    // resolves the search origin from the tracked active cell.
    let on_find = Callback::new({
        let tx = to_logic.clone();
        move |q: trame_chrome::FindQuery| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::FindNext {
                        query: q.query,
                        match_case: q.match_case,
                        whole_cell: q.whole_cell,
                    }),
                });
            }
        }
    });
    // Toile Find (`toile-find`): the panel emits a `Find` with the query,
    // options and resume point. Logic scans the whole board (off-screen matches
    // included) and replies with `FindResults`. Find-only -- no Replace on a
    // free-form canvas.
    let on_find_toile = Callback::new({
        let tx = to_logic.clone();
        move |q: toile_chrome::FindQuery| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Toile(ambre_bus::app::toile::ToileCommand::Find {
                        query: q.query,
                        options: q.options,
                        from: q.from,
                        backward: q.backward,
                    }),
                });
            }
        }
    });
    // Presenter overlay panel intents (toile-presentation): navigation maps to
    // the local playback commands. Note edits target the active slide's frame,
    // captured from the last SlideChanged into `presenter_frame`.
    let on_present_intent = Callback::new({
        let tx = to_logic.clone();
        move |intent: toile_chrome::PresenterIntent| {
            let Some(document_id) = current_doc.get_untracked() else {
                return;
            };
            use ambre_bus::app::toile::ToileCommand;
            let command = match intent {
                toile_chrome::PresenterIntent::Prev => ToileCommand::PrevSlide,
                toile_chrome::PresenterIntent::Next => ToileCommand::NextSlide,
                toile_chrome::PresenterIntent::Exit => ToileCommand::ExitPresent,
                toile_chrome::PresenterIntent::EditNotes(delta) => {
                    let Some(frame) = presenter_frame.get_untracked() else {
                        return;
                    };
                    ToileCommand::EditNotes { frame, delta }
                }
                toile_chrome::PresenterIntent::ToggleBroadcast(on) => {
                    ToileCommand::SetPresentBroadcast(on)
                }
                toile_chrome::PresenterIntent::Follow(who) => ToileCommand::FollowPresenter(who),
            };
            let _ = tx.try_send(LogicCommand::App {
                document_id,
                command: AppCommand::Toile(command),
            });
        }
    });
    let on_replace = Callback::new({
        let tx = to_logic.clone();
        move |r: trame_chrome::ReplaceRequest| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::ReplaceOne {
                        query: r.query,
                        replacement: r.replacement,
                        match_case: r.match_case,
                        whole_cell: r.whole_cell,
                        in_formulas: r.in_formulas,
                    }),
                });
            }
        }
    });
    let on_replace_all = Callback::new({
        let tx = to_logic.clone();
        move |r: trame_chrome::ReplaceRequest| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::ReplaceAll {
                        query: r.query,
                        replacement: r.replacement,
                        match_case: r.match_case,
                        whole_cell: r.whole_cell,
                        in_formulas: r.in_formulas,
                        all_sheets: r.all_sheets,
                    }),
                });
            }
        }
    });

    // Sheet-tab strip (`trame-sheets`): each DOM intent maps onto its sheet
    // `TrameCommand`. The Logic thread mints ids / resolves geometry and replies
    // with a fresh `SheetList` that re-feeds the `sheets`/`active_sheet` signals.
    let on_sheet_activate = Callback::new({
        let tx = to_logic.clone();
        move |sheet_id: String| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::ActivateSheet { sheet_id }),
                });
            }
        }
    });
    let on_sheet_create = Callback::new({
        let tx = to_logic.clone();
        move |()| {
            if let Some(document_id) = current_doc.get_untracked() {
                let name = format!("Sheet{}", sheets.get_untracked().len() + 1);
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::CreateSheet { name }),
                });
            }
        }
    });
    let on_sheet_rename = Callback::new({
        let tx = to_logic.clone();
        move |(sheet_id, name): (String, String)| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::RenameSheet { sheet_id, name }),
                });
            }
        }
    });
    let on_sheet_delete = Callback::new({
        let tx = to_logic.clone();
        move |sheet_id: String| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::DeleteSheet { sheet_id }),
                });
            }
        }
    });
    let on_sheet_reorder = Callback::new({
        let tx = to_logic.clone();
        move |req: trame_chrome::ReorderSheet| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::MoveSheet {
                        sheet_id: req.id,
                        to_index: req.to_index as u32,
                    }),
                });
            }
        }
    });

    // The menu layout follows the right-click scope the view published
    // (`trame-context-menu`). A cell right-click gets the full menu, a header the
    // header-scoped one (no clipboard / merge). A Toile board builds its menu
    // from the selection scope the canvas view publishes (`toile-menus`); a Trame
    // sheet from the cell/header scope (`trame-context-menu`). Both return
    // `ambre_chrome::MenuEntry` rows, so one `<ContextMenu>` renders either.
    let ctx_entries = Memo::new(move |_| {
        if boot_is_toile {
            toile_chrome::context_entries_for(&toile_ctx_scope.get())
        } else {
            trame_chrome::context_entries_for(ctx_scope.get(), filter_active.get())
        }
    });

    // Per-slot sender clones. Each slot's children is a `Fn`, so it captures its
    // own clone and re-clones inside the handler rather than moving the sender
    // out. Moving it out would make the children `FnOnce`.
    let tx_invite = to_logic.clone();
    // The member-row handlers live inside a `<For>` whose item template is a
    // re-runnable `Fn`. A plain `Sender` clone would be moved into it, making it
    // `FnOnce`. A `StoredValue` is `Copy`, so each row reads its own clone.
    let tx_member = StoredValue::new(to_logic.clone());
    let landing_signals = LandingSignals {
        docs,
        username,
        dev_mode,
        booted: crate::route::booted_kind(boot_is_toile, boot_is_prose),
        toasts,
    };
    let landing_tx = to_logic.clone();

    view! {
        <AppShell>
            <Header slot>
                <span class="title">"Ambre"</span>
                <span class="spacer"></span>
                <span class="ambre-sync">
                    <span class=move || chrome_status.get().dot_class()></span>
                    <span class="label">{move || chrome_status.get().label()}</span>
                </span>
                <button
                    class="icon"
                    title="Toggle theme"
                    aria-label="Toggle colour theme"
                    on:click={
                        let theme_ctl = theme_ctl.clone();
                        move |_| {
                            let next = theme_ctl.toggle();
                            theme_sig.set(next);
                            apply_theme_attr(next);
                            let label = if next == ambre_canvas::Theme::Dark {
                                "Dark theme"
                            } else {
                                "Light theme"
                            };
                            toasts.info(label);
                        }
                    }
                >
                    {move || {
                        let icon = if theme_sig.get() == ambre_canvas::Theme::Dark {
                            icondata::AiMoonFilled
                        } else {
                            icondata::AiSunFilled
                        };
                        view! { <Icon icon=icon /> }
                    }}
                </button>
                <Show when=move || current_doc.get().is_some()>
                    <button
                        class="icon"
                        title="Share"
                        aria-label="Share document"
                        aria-haspopup="dialog"
                        aria-expanded=move || share_open.get().then_some("true")
                        on:click=move |_| share_open.set(true)
                    >
                        <Icon icon=icondata::AiShareAltOutlined />
                    </button>
                </Show>
                <button
                    class="icon"
                    title="About"
                    aria-label="About Ambre"
                    aria-haspopup="dialog"
                    aria-expanded=move || about_open.get().then_some("true")
                    on:click=move |_| about_open.set(true)
                >
                    "?"
                </button>
                <span
                    class="ambre-avatar"
                    title=move || username.get().unwrap_or_else(|| "Signed out".to_owned())
                >
                    {move || initials(username.get().as_deref())}
                </span>
            </Header>
            // The toolbar / formula bar / sheet tabs / status bar are Trame chrome
            // (`trame-cell-styles`, `trame-cell-editing`, `trame-sheets`,
            // `trame-status-bar`). A Toile board renders none of them. Its tool palette,
            // status bar and panels are separate roadmap items (`toile-tools`,
            // `toile-status-bar`). Gated on the booted app kind, fixed per page load.
            <Toolbar slot>
                {(!boot_is_toile).then(|| view! {
                    <trame_chrome::FormatBar
                        toggles=toggles
                        style=style_state
                        theme=theme_sig
                        on_command=on_command
                        on_style=on_style
                    />
                })}
            </Toolbar>
            <FormulaBar slot>
                {(!boot_is_toile).then(|| view! {
                    <trame_chrome::FormulaBar
                        cell_ref=formula_ref
                        content=formula_text
                        on_action=Callback::new(move |action| formula_action.set(Some(action)))
                    />
                })}
            </FormulaBar>
            <Tabs slot>
                {(!boot_is_toile).then(|| view! {
                    <trame_chrome::SheetTabs
                        sheets=sheets
                        active=active_sheet
                        on_activate=on_sheet_activate
                        on_create=on_sheet_create
                        on_rename=on_sheet_rename
                        on_delete=on_sheet_delete
                        on_reorder=on_sheet_reorder
                    />
                })}
            </Tabs>
            <StatusBar slot>
                {if boot_is_prose {
                    // The five figures of the Prose document
                    // (`PROSE-FEAT-040`), in the shared status band.
                    view! {
                        <ambre_chrome::StatusLabel label="Prose" />
                        <span class="statusbar-stats">
                            {move || {
                                prose_figures.get().map(|figures| {
                                    figures
                                        .into_iter()
                                        .map(|figure| view! { <span class="stat">{figure}</span> })
                                        .collect_view()
                                })
                            }}
                        </span>
                        <span class="spacer"></span>
                    }
                    .into_any()
                } else if boot_is_toile {
                    // Toile selection status bar: count/bounds + shared
                    // zoom & fps chips, styled identically to Trame's band.
                    view! {
                        <toile_chrome::StatusBar
                            stats=sel_stats_toile
                            perf=perf_snap
                            zoom=zoom
                        />
                    }
                    .into_any()
                } else {
                    view! {
                        <trame_chrome::StatusBar stats=sel_stats perf=perf_snap zoom=zoom />
                    }
                    .into_any()
                }}
            </StatusBar>
        </AppShell>

        <ambre_chrome::PassphraseDialog
            open=seal_open
            intent=ambre_chrome::PassphraseIntent::Seal
            on_open=on_open
            on_close=on_close
            on_submit=Callback::new({
                let tx = to_logic.clone();
                move |passphrase: String| {
                    let Some(document_id) = current_doc.get_untracked() else {
                        return;
                    };
                    // The typed text becomes a `SecretBytes` here and goes
                    // nowhere else. Every hop that drops it wipes it, and the
                    // Sync worker is the only thread that reads it
                    // (`ARCH-004`).
                    let passphrase = ambre_bus::SecretBytes::from(passphrase.into_bytes());
                    let command = if boot_is_toile {
                        AppCommand::Toile(
                            ambre_bus::app::toile::ToileCommand::ExportAmbreFileSealed {
                                passphrase,
                            },
                        )
                    } else {
                        AppCommand::Trame(TrameCommand::ExportAmbreFileSealed { passphrase })
                    };
                    let _ = tx.try_send(LogicCommand::App {
                        document_id,
                        command,
                    });
                }
            })
        />

        <Dialog open=about_open on_open=on_open on_close=on_close class="ambre-about">
            <div class="ambre-about-panel">
                <h2>"Ambre"</h2>
                <p>"Collaborative office suite, 100% Rust, E2EE encrypted."</p>
                <div class="ambre-dialog-actions">
                    <button on:click=move |_| about_open.set(false)>"Close"</button>
                </div>
            </div>
        </Dialog>

        <Dialog
            open=share_open
            on_open=share_on_open
            on_close=share_on_close
            class="ambre-share"
        >
            <div class="ambre-share-panel">
                <h2>"Share document"</h2>

                // Current members.
                <ul class="ambre-share-members">
                    <For
                        each=move || members.get()
                        key=|m| m.user_id
                        let:member
                    >
                        {
                            let tx_role = tx_member.get_value();
                            let tx_revoke = tx_member.get_value();
                            let uid = member.user_id;
                            let cur_role = member.role;
                            view! {
                                <li>
                                    <span class="ambre-share-member-name">
                                        {member.display_name.clone()}
                                    </span>
                                    <span class="ambre-share-member-email">
                                        {member.email.clone()}
                                    </span>
                                    <select
                                        prop:value=cur_role.as_str()
                                        on:change={
                                            move |ev| {
                                                let Some(document_id) = current_doc.get_untracked() else {
                                                    return;
                                                };
                                                if let Ok(role) = event_target_value(&ev).parse::<Role>() {
                                                    let _ = tx_role.try_send(LogicCommand::Rest {
                                                        id: RequestId::next(),
                                                        cmd: RestCommand::UpdateDocumentMemberRole {
                                                            document_id,
                                                            user_id: uid,
                                                            role,
                                                        },
                                                    });
                                                }
                                            }
                                        }
                                    >
                                        <option value="owner" selected=cur_role == Role::Owner>"Owner"</option>
                                        <option value="editor" selected=cur_role == Role::Editor>"Editor"</option>
                                        <option value="viewer" selected=cur_role == Role::Viewer>"Viewer"</option>
                                    </select>
                                    <button
                                        class="ambre-share-revoke"
                                        title="Revoke access"
                                        aria-label="Revoke access"
                                        on:click={
                                            move |_| {
                                                let Some(document_id) = current_doc.get_untracked() else {
                                                    return;
                                                };
                                                let _ = tx_revoke.try_send(LogicCommand::Rest {
                                                    id: RequestId::next(),
                                                    cmd: RestCommand::RevokeFromDocument {
                                                        document_id,
                                                        user_id: uid,
                                                    },
                                                });
                                            }
                                        }
                                    >
                                        "Revoke"
                                    </button>
                                </li>
                            }
                        }
                    </For>
                </ul>

                // Invite a new collaborator.
                <form
                    class="ambre-share-invite"
                    on:submit={
                        let tx = tx_invite.clone();
                        move |ev| {
                            ev.prevent_default();
                            let Some(document_id) = current_doc.get_untracked() else {
                                return;
                            };
                            let raw = invitee_input.get_untracked();
                            let raw = raw.trim();
                            if raw.is_empty() {
                                return;
                            }
                            // Dev convention: a bare username maps to the
                            // deterministic `<username>@dev.local` email the dev
                            // login assigns (server resolves invitees by email).
                            let invitee_email = if dev_mode.get_untracked() == Some(true)
                                && !raw.contains('@')
                            {
                                format!("{raw}@dev.local")
                            } else {
                                raw.to_owned()
                            };
                            // TOFU phase 1 (`ambre-spec-key-verification.md`): fetch the
                            // invitee's key + fingerprint without wrapping. The role
                            // is kept in `invite_role`; the relay dispatches phase 2
                            // (the actual share) once the trust decision is made.
                            let _ = tx.try_send(LogicCommand::Rest {
                                id: RequestId::next(),
                                cmd: RestCommand::FetchInviteeKey {
                                    document_id,
                                    invitee_email,
                                },
                            });
                            invitee_input.set(String::new());
                        }
                    }
                >
                    <input
                        r#type="text"
                        class="ambre-share-input"
                        aria-label="Invitee"
                        placeholder=move || {
                            if dev_mode.get() == Some(true) {
                                "username or email"
                            } else {
                                "user@example.com"
                            }
                        }
                        prop:value=move || invitee_input.get()
                        on:input=move |ev| invitee_input.set(event_target_value(&ev))
                    />
                    <select
                        aria-label="Role"
                        on:change=move |ev| {
                            if let Ok(role) = event_target_value(&ev).parse::<Role>() {
                                invite_role.set(role);
                            }
                        }
                    >
                        <option value="editor" selected=true>"Editor"</option>
                        <option value="viewer">"Viewer"</option>
                        <option value="owner">"Owner"</option>
                    </select>
                    <button r#type="submit">"Invite"</button>
                </form>

                <div class="ambre-dialog-actions">
                    <button on:click=move |_| share_open.set(false)>"Close"</button>
                </div>
            </div>
        </Dialog>

        // Embed a web page (`TOILE-087`). The address is normalized here, at the
        // one place the user can still read the refusal (`TOILE-088`).
        <Dialog
            open=embed_web_open
            on_open=on_open
            on_close=on_close
            class="ambre-embed-web"
        >
            <div class="ambre-embed-panel">
                <h2>"Embed a web page"</h2>
                <input
                    type="url"
                    placeholder="example.com"
                    prop:value=move || embed_web_text.get()
                    on:input=move |ev| {
                        embed_web_text.set(event_target_value(&ev));
                        embed_web_error.set(None);
                    }
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if ev.key() == "Enter" {
                            place_web_embed(
                                embed_web_text,
                                embed_web_error,
                                embed_web_open,
                                toile_placed_embed,
                            );
                        }
                    }
                />
                <p class="ambre-embed-error" aria-live="polite">
                    {move || embed_web_error.get().unwrap_or("")}
                </p>
                <div class="ambre-dialog-actions">
                    <button on:click=move |_| embed_web_open.set(false)>"Cancel"</button>
                    <button on:click=move |_| {
                        place_web_embed(
                            embed_web_text,
                            embed_web_error,
                            embed_web_open,
                            toile_placed_embed,
                        );
                    }>"Embed"</button>
                </div>
            </div>
        </Dialog>

        // Embed another Ambre document (`TOILE-086`). The list is the one the
        // launcher already holds: this opens no document, so it touches none
        // (`TOILE-225`). The board itself is not offered -- a board embedding
        // itself is a session that opens a session that opens a session.
        <Dialog
            open=embed_doc_open
            on_open=on_open
            on_close=on_close
            class="ambre-embed-doc"
        >
            <div class="ambre-embed-panel">
                <h2>"Embed a document"</h2>
                <ul class="ambre-embed-docs">
                    <For
                        each=move || {
                            let here = current_doc.get();
                            docs.get()
                                .into_iter()
                                .filter(|d| Some(d.document_id) != here)
                                .collect::<Vec<_>>()
                        }
                        key=|d| d.document_id
                        let:doc
                    >
                        {
                            let (document_id, doc_type) = (doc.document_id, doc.doc_type);
                            let label = doc
                                .name
                                .clone()
                                .unwrap_or_else(|| "Untitled".to_owned());
                            view! {
                                <li>
                                    <button on:click=move |_| {
                                        toile_placed_embed
                                            .set(
                                                Some(toile_chrome::PlacedEmbed {
                                                    target: toile_chrome::EmbedTarget::Document {
                                                        document_id,
                                                        doc_type,
                                                    },
                                                    at: None,
                                                }),
                                            );
                                        embed_doc_open.set(false);
                                    }>{label}</button>
                                </li>
                            }
                        }
                    </For>
                </ul>
                <div class="ambre-dialog-actions">
                    <button on:click=move |_| embed_doc_open.set(false)>"Cancel"</button>
                </div>
            </div>
        </Dialog>

        // Put a link on the selected element (`toile-links`). Three targets and
        // no more: an address, a frame on this board, or another document. The
        // frame list is the deck's, which holds every frame including the
        // off-screen ones a link exists to reach.
        <Dialog
            open=link_open
            on_open=on_open
            on_close=on_close
            class="ambre-link-chooser"
        >
            <div class="ambre-embed-panel">
                <h2>"Link this element"</h2>
                <h3>"To a web page"</h3>
                <input
                    type="url"
                    placeholder="example.com"
                    prop:value=move || link_text.get()
                    on:input=move |ev| {
                        link_text.set(event_target_value(&ev));
                        link_error.set(None);
                    }
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if ev.key() == "Enter" {
                            set_web_link(link_text, link_error, link_open, toile_set_link);
                        }
                    }
                />
                <p class="ambre-embed-error" aria-live="polite">
                    {move || link_error.get().unwrap_or("")}
                </p>
                <h3>"To a frame on this board"</h3>
                <ul class="ambre-embed-docs">
                    <For
                        each=move || {
                            let deck = toile_deck.view.get();
                            deck.slides.into_iter().chain(deck.off_deck).collect::<Vec<_>>()
                        }
                        key=|f| f.id.clone()
                        let:frame
                    >
                        {
                            let id = frame.id.clone();
                            let label = if frame.name.is_empty() {
                                "Untitled frame".to_owned()
                            } else {
                                frame.name.clone()
                            };
                            view! {
                                <li>
                                    <button on:click=move |_| {
                                        toile_set_link
                                            .set(
                                                Some(
                                                    toile_chrome::LinkEdit(
                                                        Some(
                                                            ambre_bus::app::toile::LinkTarget::Frame(id.clone()),
                                                        ),
                                                    ),
                                                ),
                                            );
                                        link_open.set(false);
                                    }>{label}</button>
                                </li>
                            }
                        }
                    </For>
                </ul>
                <h3>"To another document"</h3>
                <ul class="ambre-embed-docs">
                    <For
                        each=move || {
                            let here = current_doc.get();
                            docs.get()
                                .into_iter()
                                .filter(|d| Some(d.document_id) != here)
                                .collect::<Vec<_>>()
                        }
                        key=|d| d.document_id
                        let:doc
                    >
                        {
                            let (document_id, doc_type) = (doc.document_id, doc.doc_type);
                            let label = doc.name.clone().unwrap_or_else(|| "Untitled".to_owned());
                            view! {
                                <li>
                                    <button on:click=move |_| {
                                        toile_set_link
                                            .set(
                                                Some(
                                                    toile_chrome::LinkEdit(
                                                        Some(
                                                            ambre_bus::app::toile::LinkTarget::Document {
                                                                document_id,
                                                                doc_type,
                                                            },
                                                        ),
                                                    ),
                                                ),
                                            );
                                        link_open.set(false);
                                    }>{label}</button>
                                </li>
                            }
                        }
                    </For>
                </ul>
                <div class="ambre-dialog-actions">
                    <button on:click=move |_| link_open.set(false)>"Cancel"</button>
                    <button on:click=move |_| {
                        set_web_link(link_text, link_error, link_open, toile_set_link);
                    }>"Link"</button>
                </div>
            </div>
        </Dialog>

        // TOFU key verification (`ambre-spec-key-verification.md`). A modal `Dialog`,
        // never a toast: a possible server MITM must not auto-dismiss. Shown only
        // for a New key (confirm before the first share) or a Changed key (a
        // blocking alert whose default action is Cancel).
        <Dialog
            open=tofu_open
            on_open=tofu_on_open
            on_close=tofu_on_close
            class="ambre-tofu"
        >
            {move || {
                tofu_prompt
                    .get()
                    .map(|p| {
                        let changed = p.changed;
                        let fp_copy = p.fingerprint.clone();
                        let confirm_p = p.clone();
                        view! {
                            <div class="ambre-tofu-panel">
                                <h2>
                                    {if changed {
                                        "Recipient key changed"
                                    } else {
                                        "Verify recipient key"
                                    }}
                                </h2>
                                <p class="ambre-tofu-who">
                                    <span class="ambre-tofu-name">{p.display_name.clone()}</span>
                                    <span class="ambre-tofu-email">{p.email.clone()}</span>
                                </p>
                                {changed
                                    .then(|| {
                                        view! {
                                            <p class="ambre-tofu-warning" role="alert">
                                                "This user's key differs from the one you trusted before. \
                                                 A legitimate cause exists (they reinstalled and lost their \
                                                 keys), but a compromised server can also substitute keys. \
                                                 Re-verify out of band before proceeding."
                                            </p>
                                        }
                                    })}
                                <p class="ambre-tofu-fingerprint">{p.fingerprint.clone()}</p>
                                <button
                                    r#type="button"
                                    class="ambre-tofu-copy"
                                    on:click=move |_| {
                                        if let Some(win) = web_sys::window() {
                                            let _ = win.navigator().clipboard().write_text(&fp_copy);
                                        }
                                    }
                                >
                                    "Copy fingerprint"
                                </button>
                                <p class="ambre-tofu-limit">
                                    "Without out-of-band verification (in person, phone, SMS), this \
                                     protection reduces to trusting the server. Compare this fingerprint \
                                     with the recipient over a channel the server does not control."
                                </p>
                                <div class="ambre-dialog-actions">
                                    // Cancel is the default action: closing
                                    // the modal pins nothing and sends no key.
                                    <button
                                        r#type="button"
                                        class="ambre-tofu-cancel"
                                        on:click=move |_| tofu_prompt.set(None)
                                    >
                                        "Cancel"
                                    </button>
                                    <button
                                        r#type="button"
                                        class="ambre-tofu-confirm"
                                        on:click=move |_| {
                                            // Hand the pin + phase-2 dispatch to the
                                            // relay (it owns the trusted_keys store).
                                            tofu_confirm.set(Some(confirm_p.clone()));
                                            tofu_prompt.set(None);
                                        }
                                    >
                                        {if changed { "Re-pin and invite" } else { "Confirm and invite" }}
                                    </button>
                                </div>
                            </div>
                        }
                    })
            }}
        </Dialog>

        <ContextMenu pos=ctx_menu entries=ctx_entries on_command=on_command />

        // Shared filter popover (`trame-shared-filter`), under the active column.
        <trame_chrome::FilterPopover
            state=filter_popover
            values=filter_values
            on_apply=on_filter_apply
            on_command=on_command
        />

        // The command palette (`CHROME-008`): every command the booted
        // application declares, searchable, whether or not a menu shows it. One
        // overlay for both, fed by the app's own table.
        <ambre_chrome::CommandPalette
            open=palette_open
            table=if boot_is_toile {
                ambre_toile::ui::commands::COMMANDS
            } else if boot_is_prose {
                ambre_prose::commands::COMMANDS
            } else {
                trame_chrome::COMMANDS
            }
            on_command=on_command
        />
        // Find panel, pinned top-right. Trame ships `trame-find-replace`;
        // Toile ships a find-only panel (`toile-find`). The booted app kind
        // is fixed per page load, so exactly one is mounted.
        {(!boot_is_toile).then(|| view! {
            <trame_chrome::FindReplace
                state=find_state
                result=find_result
                on_find=on_find
                on_replace=on_replace
                on_replace_all=on_replace_all
            />
        })}
        {boot_is_toile.then(|| view! {
            <toile_chrome::FindPanel
                state=find_state_toile
                result=find_result_toile
                on_find=on_find_toile
            />
        })}
        // Toile presenter overlay panel (toile-presentation): speaker notes +
        // navigation, shown while presentation mode is active. Visibility is
        // driven by the `present_toile` signal (mirrored from PresentModeChanged);
        // the slide payload feeds the counter + notes editor.
        {boot_is_toile.then(|| view! {
            <toile_chrome::PresenterPanel
                open=present_toile
                index=presenter_index
                count=presenter_count
                notes=presenter_notes
                broadcasting=broadcasting_toile
                following=following_toile
                presenter_peer=presenter_peer_toile
                on_intent=on_present_intent
            />
        })}
        // Toile tool palette / primary toolbar (toile-tools): a floating
        // vertical bar, mounted only for a Toile board. It writes the shared active-
        // tool signal the canvas view applies; the keyboard chords write the same one.
        {boot_is_toile.then(|| view! {
            <toile_chrome::ToolPalette active=toile_active_tool />
        })}
        // Toile top menu bar + zoom control (toile-menus): the low- frequency menus
        // (File/Edit/View/Object/Present) and the zoom cluster. Every entry routes
        // through the shared `on_command` path, the same channel the context menu
        // uses. The zoom readout mirrors the `zoom` signal the canvas view
        // publishes each frame.
        {boot_is_toile.then(|| view! {
            <toile_chrome::TopMenuBar zoom=zoom on_command=on_command toggles=toile_toggles />
        })}
        // Toile style / property inspector (toile-styles): a side panel shown only for
        // a non-empty selection. It reads the common style the canvas view publishes.
        // It reuses `toile_ctx_scope` for the lock buttons and the shared `on_command`
        // path for z-order / lock. It writes debounced whole-style edits into
        // `toile_restyle`, which the view turns into `RestyleElements`.
        {boot_is_toile.then(|| view! {
            <toile_chrome::StyleInspector
                model=toile_style_model
                scope=toile_ctx_scope
                on_command=on_command
                restyle=toile_restyle
            />
        })}
        // Toile content-anchored floating selection toolbar (toile-styles): follows
        // the selection as the board pans/zooms (the canvas view republishes its
        // client anchor into `toile_float_bar` each frame). Its command buttons
        // reuse the shared `on_command` path, a strict subset of the context menu.
        // The style controls (fill, text colour, font, size, bold, italic) seed from
        // the inspector `toile_style_model`. They write single-field `StyleEdit`
        // edits the view turns into a live-based `RestyleElements` over the
        // selection.
        {boot_is_toile.then(|| view! {
            <toile_chrome::SelectionToolbar
                bar=toile_float_bar
                model=toile_style_model
                on_command=on_command
                on_style=Callback::new(move |s: toile_chrome::StyleEdit| {
                    toile_quick_style.set(Some(s))
                })
            />
        })}

        // Toile comments (`toile-comments`): the board-wide thread list with the
        // one thread it shows whole, and the composer the comment tool opens
        // where the user clicked. Both hold nothing of their own
        // (`TOILE-UI-023`). They render what the canvas view read back from the
        // board, and hand every action to it as one intent.
        {boot_is_toile.then(|| view! {
            <toile_chrome::CommentsPanel
                state=toile_comments.panel
                list=toile_comments.list
                open=toile_comments.open
                on_intent=Callback::new(move |i: toile_chrome::CommentIntent| {
                    toile_comments.intent.set(Some(i))
                })
            />
            <toile_chrome::CommentComposer
                draft=toile_comments.draft
                on_intent=Callback::new(move |i: toile_chrome::CommentIntent| {
                    toile_comments.intent.set(Some(i))
                })
            />
        })}

        // Prose annotations: the comment panel and the suggestion popover,
        // both DOM chrome (`PROSE-UI-050`, `PROSE-UI-052`).
        {boot_is_prose.then(|| view! {
            <ambre_prose::chrome::ProseCommentsPanel signals=prose_signals />
            <ambre_prose::chrome::ProseSuggestionPopover signals=prose_signals />
            <ambre_prose::chrome::ProseFormatBar signals=prose_signals />
        })}

        // The deck panel (`toile-presentation`): the surface that adds a frame
        // to the presentation, reorders it and removes it (`TOILE-FEAT-026`).
        // It holds no order of its own. It renders the frames the canvas view
        // read back from the board, and hands each edit to it as one intent.
        {boot_is_toile.then(|| view! {
            <toile_chrome::DeckPanel
                state=toile_deck.panel
                view=toile_deck.view
                on_intent=Callback::new(move |i: toile_chrome::DeckIntent| {
                    toile_deck.intent.set(Some(i))
                })
            />
        })}

        // Multi-column sort dialog (`trame-sort`), from "Sort range...".
        <trame_chrome::SortDialog state=sort_dialog on_apply=on_sort_apply />

        // Live-region toast host (announces transient messages politely).
        <ToastHost toasts=toasts />

        // Document-loading overlay (`-chrome.md` 2.G): covers the canvas while a
        // freshly opened document hydrates (snapshot fetch + decrypt), dismissed
        // on `UiEvent::DocumentReady`. Reuses the boot loader's spinner visual.
        // Never shown on the landing route -- `hydrating` is cleared there.
        <Show when=move || chrome_status.get().hydrating>
            <div class="ambre-doc-loading">
                <div class="spinner"></div>
                <p>"Loading..."</p>
            </div>
        </Show>

        // Landing overlay, shown on `/` over the (frozen, hidden) canvas.
        <Show when=is_landing>
            {
                let landing_tx = landing_tx.clone();
                move || landing::landing_view(landing_signals, landing_tx.clone())
            }
        </Show>
    }
}

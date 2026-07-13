// ambre-app/src/shell/app_root.rs
// The single root view (AppRoot): persistent chrome + canvas, landing overlay
// on `/`. Reads its wiring from context; the document binding is driven outside
// this tree by the per-frame route poll in `main`.

use super::super::*;

/// The single root view, rendered inside `<Router>` so it can read the client
/// route. Renders the persistent chrome + canvas and overlays the landing on the
/// `/` route. The document binding (open/close, canvas swap, rAF gate) is driven
/// OUTSIDE this tree by the per-frame route poll (see `pre_frame` in `main`),
/// because the canvas handle is `!Send` and the `<Router>` subtree must be Send.
#[component]
pub(crate) fn AppRoot() -> impl IntoView {
    let Wiring {
        to_logic,
        raf,
        theme_ctl,
        toggles,
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
        formula_ref,
        formula_text,
        formula_action,
        sel_stats,
        perf_snap,
        zoom,
        ctx_scope,
        filter_active,
        sel_bounds,
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
    // Invite form state: the typed invitee (email, or a bare username in dev
    // mode) and the role to grant.
    let invitee_input = RwSignal::new(String::new());
    let invite_role = RwSignal::new(Role::Editor);

    // Canvas right-click -> context menu (the launcher owns the listener since
    // the canvas is created by the AppShell).
    let ctx_menu = RwSignal::new(None::<(f64, f64)>);
    install_canvas_context_menu(ctx_menu);

    // The single command path shared by every trigger (toolbar, selection bar,
    // context menu): a `CommandId` -> the routed document. `Callback` is `Copy`,
    // so it is handed to each app-chrome component below.
    let on_command = Callback::new({
        let tx = to_logic.clone();
        move |id: CommandId| {
            // `FILTER_COLUMN` is a UI-only intent: open the popover anchored under
            // the active cell and ask the Logic thread for the column's distinct
            // values (it resolves the active column itself). Everything else flows
            // through the shared CommandId -> AppCommand path.
            if id == trame_chrome::FIND || id == trame_chrome::REPLACE {
                // UI-only intents (sec.5.G): open the find/replace panel in the
                // matching mode. The panel emits FindNext/ReplaceAll itself.
                let mode = if id == trame_chrome::FIND {
                    trame_chrome::FindMode::Find
                } else {
                    trame_chrome::FindMode::Replace
                };
                find_state.set(Some(trame_chrome::FindReplaceState { mode }));
            } else if id == trame_chrome::SORT_RANGE_MULTI {
                // UI-only intent (sec.5.C): open the multi-column sort dialog over
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
                // tracked toolbar state and emit a `SetCellStyle` patch (sec.5bis).
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
                // Copy/cut (sec.5.E): focus the canvas text pipe and ask the
                // browser to fire the `copy`/`cut` DOM event the host listeners
                // service (they write both the TSV and the typed payload).
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
                // Paste (sec.5.E): `execCommand("paste")` is blocked by browsers,
                // so read the OS clipboard via the async Clipboard API (allowed in
                // this click gesture). `read()` recovers the private Ambre payload
                // (full styled / formula-reindexed paste, like Ctrl+V) when
                // present, else the plain text; the Logic thread resolves the
                // target from the tracked active cell.
                if let Some(document_id) = current_doc.get_untracked() {
                    let clipboard = web_sys::window().map(|w| w.navigator().clipboard());
                    // `navigator.clipboard` is undefined outside a secure context
                    // (http:// on a LAN IP, etc.): there the button cannot read at
                    // all -- tell the user instead of failing silently.
                    let available = clipboard
                        .as_ref()
                        .map(|c| {
                            let v: &wasm_bindgen::JsValue = c.as_ref();
                            !v.is_undefined() && !v.is_null()
                        })
                        .unwrap_or(false);
                    if let Some(clipboard) = clipboard.filter(|_| available) {
                        let tx = tx.clone();
                        // Call `read()` synchronously, still inside the click's user
                        // gesture: Firefox shows its mandatory "paste" confirmation
                        // right away instead of deferring it to the next click.
                        let read_promise = clipboard.read();
                        wasm_bindgen_futures::spawn_local(paste_from_clipboard(
                            read_promise,
                            clipboard,
                            document_id,
                            tx,
                            toasts,
                        ));
                    } else {
                        toasts.warning(
                            "Clipboard unavailable here (needs HTTPS); use Ctrl+V to paste",
                        );
                    }
                }
            } else if id == trame_chrome::IMPORT_CSV {
                // Interop (`-interop.md` §4.A): open a file picker, read the chosen
                // CSV bytes, and send them to the Logic thread, which creates a new
                // sheet and converts client-side (Zero-Knowledge).
                if let Some(document_id) = current_doc.get_untracked() {
                    open_csv_picker(document_id, tx.clone(), toasts);
                }
            } else {
                dispatch_to_current(id, current_doc, &tx);
            }
        }
    });

    // Formatting toolbar: a control's `StylePatch` -> `SetCellStyle` over the
    // tracked selection (`-ui.md` sec.5bis). The Logic thread resolves the
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
    // Multi-column sort (sec.5.C): the dialog emits the chosen levels; forward
    // them as the parameterless `SortSelectionMulti`, like the single-column sort
    // intents -- the Logic thread resolves the range and active sheet from the
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
    // Find & Replace (sec.5.G): the panel emits these requests; the Logic thread
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
    // Toile Find (`toile-ui §5quater`): the panel emits a `Find` with the query,
    // options and resume point; Logic scans the whole board (off-screen matches
    // included) and replies with `FindResults`. Find-only -- no Replace on a
    // free-form canvas (§12).
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

    // Sheet-tab strip (sec.13): each DOM `SheetTabs` intent maps onto its sheet
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

    // The menu layout follows the right-click scope the view published (`-ui.md`
    // sec.5.C): a cell right-click gets the full menu, a header right-click the
    // header-scoped one (no clipboard / merge).
    let ctx_entries =
        Memo::new(move |_| trame_chrome::context_entries_for(ctx_scope.get(), filter_active.get()));

    // Per-slot sender clones. Each slot's children is a `Fn` (re-runnable), so it
    // captures its own clone and re-clones inside the handler rather than moving
    // the sender out (which would make the children `FnOnce`).
    let tx_invite = to_logic.clone();
    // The member-row handlers live inside a `<For>` whose item template is a
    // re-runnable `Fn`; a plain `Sender` clone would be moved into it (making it
    // `FnOnce`). A `StoredValue` is `Copy`, so each row reads its own clone.
    let tx_member = StoredValue::new(to_logic.clone());
    let landing_signals = LandingSignals {
        docs,
        username,
        dev_mode,
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
            <Toolbar slot>
                <trame_chrome::FormatBar
                    toggles=toggles
                    style=style_state
                    theme=theme_sig
                    on_command=on_command
                    on_style=on_style
                />
            </Toolbar>
            <FormulaBar slot>
                <trame_chrome::FormulaBar
                    cell_ref=formula_ref
                    content=formula_text
                    on_action=Callback::new(move |action| formula_action.set(Some(action)))
                />
            </FormulaBar>
            <Tabs slot>
                <trame_chrome::SheetTabs
                    sheets=sheets
                    active=active_sheet
                    on_activate=on_sheet_activate
                    on_create=on_sheet_create
                    on_rename=on_sheet_rename
                    on_delete=on_sheet_delete
                    on_reorder=on_sheet_reorder
                />
            </Tabs>
            <StatusBar slot>
                <trame_chrome::StatusBar stats=sel_stats perf=perf_snap zoom=zoom />
            </StatusBar>
        </AppShell>

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
                            let _ = tx.try_send(LogicCommand::Rest {
                                id: RequestId::next(),
                                cmd: RestCommand::InviteToDocument {
                                    document_id,
                                    invitee_email,
                                    role: invite_role.get_untracked(),
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

        <ContextMenu pos=ctx_menu entries=ctx_entries on_command=on_command />

        // Shared filter popover (sec.5.D), anchored under the active column.
        <trame_chrome::FilterPopover
            state=filter_popover
            values=filter_values
            on_apply=on_filter_apply
            on_command=on_command
        />

        // Find panel, pinned top-right. Trame ships Find & Replace (sec.5.G);
        // Toile ships a find-only panel (`toile-ui §5quater`). The booted app kind
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

        // Multi-column sort dialog (sec.5.C), opened from "Sort range...".
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

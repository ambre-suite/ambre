// ambre-toile/src/ui/menu.rs
// ============================================================================
// Command surface for the Toile right-click context menu (`toile-menus`). The
// host-visible half: the `CommandId`s the menu contributes, the pure selection
// scope descriptor the canvas view publishes to the DOM chrome, and the pure
// `CommandId -> Vec<ToileCommand>` resolver the view applies against its local
// selection.
//
// Kept in the always-compiled `ui` module (not the wasm-only `chrome`) so it is
// host-unit-tested, exactly like `ui::find`. The wasm-only entry builder
// (icons + `MenuEntry` + separators) that consumes these lives in `chrome::menu`.
//
// Unlike Trame -- whose context commands are parameterless because the Logic
// thread tracks the active cell -- Toile selection is UI-only state living in
// `ToileView` (`toile-selection`, and `ambre-spec-ui.md`). The Logic thread never sees it, so a menu
// `CommandId` cannot be turned into a `ToileCommand { ids: .. }` in the DOM
// shell. The shell forwards the bare `CommandId` into the view, which calls
// [`resolve_menu_command`] with its selection to build the real command(s).
//
// Everything here is pure and `cfg`-agnostic (no Leptos, no wasm), so it is
// host-unit-tested per `toile-menus`: each context-menu entry emits the expected
// command. The wasm-only entry builder (icons + separators) lives in `mod.rs`.
// ============================================================================

use ambre_bus::app::toile::{
    DisplayElement, DisplayNode, DisplayViewport, ElementId, ElementKind, NodeId, Reaction,
    ReactionKind, ToileCommand, ZTarget,
};
use ambre_commands::CommandId;
use kurbo::{Rect, Vec2};

/// Diagonal offset applied to a menu Duplicate so the copy is visibly distinct
/// from its source (`toile-clipboard`), matching the `Ctrl/Cmd+D` nudge in the view.
const DUPLICATE_NUDGE: f64 = 10.0;

// -- Command identities --------------------------------------------------------
// String-keyed ids (`ambre-commands`), one per menu entry. The identity is the
// string, so the shell can round-trip an id through a signal without carrying the
// `Command` table.

/// Cut the selection (copy + delete).
pub const CUT: CommandId = CommandId("toile.clipboard.cut");
/// Copy the selection to the shared clipboard.
pub const COPY: CommandId = CommandId("toile.clipboard.copy");
/// Paste the clipboard at the cursor / viewport center.
pub const PASTE: CommandId = CommandId("toile.clipboard.paste");
/// Duplicate the selection in place with a small nudge.
pub const DUPLICATE: CommandId = CommandId("toile.edit.duplicate");
/// Delete the selection.
pub const DELETE: CommandId = CommandId("toile.edit.delete");
/// Bind the selection into a group.
pub const GROUP: CommandId = CommandId("toile.group.group");
/// Drop the selected group's binding.
pub const UNGROUP: CommandId = CommandId("toile.group.ungroup");
/// Raise the selection to the top of the z-order.
pub const BRING_FRONT: CommandId = CommandId("toile.z.front");
/// Lower the selection to the bottom of the z-order.
pub const SEND_BACK: CommandId = CommandId("toile.z.back");
/// Raise the selection one step.
pub const FORWARD: CommandId = CommandId("toile.z.forward");
/// Lower the selection one step.
pub const BACKWARD: CommandId = CommandId("toile.z.backward");
/// Lock the selection (skips hit-test / handles).
pub const LOCK: CommandId = CommandId("toile.lock.lock");
/// Unlock the selection.
pub const UNLOCK: CommandId = CommandId("toile.lock.unlock");
/// Add the selection to the frame that contains it.
pub const ADD_TO_FRAME: CommandId = CommandId("toile.frame.add");
/// Remove the selection from its containing frame.
pub const REMOVE_FROM_FRAME: CommandId = CommandId("toile.frame.remove");
/// Select every element on the board (local, never synced).
pub const SELECT_ALL: CommandId = CommandId("toile.selection.all");

// -- Reactions ----------------------------------------------------------
// A minimal, fixed emoji set for the reaction affordance: each id sets the local
// user's reaction on every selected element (one `React` per target, LWW on the
// user's own key). `REACT_CLEAR` removes it. The picker is intentionally small
// and hardcoded for the MVP; a full emoji browser is a later concern.
pub const REACT_THUMBSUP: CommandId = CommandId("toile.react.thumbsup");
pub const REACT_HEART: CommandId = CommandId("toile.react.heart");
pub const REACT_CHECK: CommandId = CommandId("toile.react.check");
pub const REACT_PARTY: CommandId = CommandId("toile.react.party");
/// Remove the local user's own reaction from the selection.
pub const REACT_CLEAR: CommandId = CommandId("toile.react.clear");

/// The fixed emoji entries of the reaction picker, id + glyph, in display order.
pub const REACTION_EMOJI: &[(CommandId, &str)] = &[
    (REACT_THUMBSUP, "\u{1F44D}"),     // thumbs up
    (REACT_HEART, "\u{2764}\u{FE0F}"), // red heart
    (REACT_CHECK, "\u{2705}"),         // check mark button
    (REACT_PARTY, "\u{1F389}"),        // party popper
];

/// Every command the Toile context menu contributes to the registry.
pub const COMMANDS: &[CommandId] = &[
    CUT,
    COPY,
    PASTE,
    DUPLICATE,
    DELETE,
    GROUP,
    UNGROUP,
    BRING_FRONT,
    SEND_BACK,
    FORWARD,
    BACKWARD,
    LOCK,
    UNLOCK,
    ADD_TO_FRAME,
    REMOVE_FROM_FRAME,
    SELECT_ALL,
    REACT_THUMBSUP,
    REACT_HEART,
    REACT_CHECK,
    REACT_PARTY,
    REACT_CLEAR,
];

/// Pure descriptor of the current selection, published by the canvas view on a
/// right-click and consumed by the entry builder (`mod.rs`) to omit inapplicable
/// rows (the "greyed out when inapplicable" of `toile-menus`, realized as omission --
/// the Trame pattern). `Default` is the empty-canvas scope (Paste + Select-All).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct CtxScope {
    /// True when the right-click landed on a non-empty selection. False on empty
    /// canvas: only Paste + Select-All apply.
    pub over_selection: bool,
    /// Number of selected elements.
    pub count: usize,
    /// At least two elements selected -> Group applies.
    pub multi: bool,
    /// At least one selected element is a group node -> Ungroup applies.
    pub has_group: bool,
    /// At least one selected element is locked -> Unlock applies.
    pub any_locked: bool,
    /// At least one selected element is unlocked -> Lock applies.
    pub any_unlocked: bool,
    /// The frame that geometrically contains the whole selection (if any and not
    /// itself selected) -> Add to / Remove from frame apply.
    pub containing_frame: Option<ElementId>,
}

/// Find a displayed element by id (linear over the culled set, which is small).
fn find_element<'a>(display: &'a DisplayViewport, id: &ElementId) -> Option<&'a DisplayElement> {
    display.nodes.iter().find_map(|n| match n {
        DisplayNode::Element(e) if &e.id == id => Some(e),
        _ => None,
    })
}

/// True if `outer` fully contains `inner` (edges inclusive).
fn rect_contains_rect(outer: Rect, inner: Rect) -> bool {
    outer.x0 <= inner.x0 && outer.y0 <= inner.y0 && outer.x1 >= inner.x1 && outer.y1 >= inner.y1
}

/// Classify the current selection into a [`CtxScope`] the entry builder consumes.
/// Pure: reads only the culled `DisplayViewport` and the UI selection. `group_scope`
/// is accepted for symmetry with the rest of the resolution path (frame lookups may
/// later honor it); the empty-canvas case ignores everything else.
pub fn ctx_scope_for(
    selection: &[ElementId],
    _group_scope: Option<&ElementId>,
    display: &DisplayViewport,
) -> CtxScope {
    if selection.is_empty() {
        return CtxScope::default();
    }

    let mut count = 0usize;
    let mut has_group = false;
    let mut any_locked = false;
    let mut any_unlocked = false;
    // Union of the selected elements' bounds, to find an enclosing frame.
    let mut union: Option<Rect> = None;

    for id in selection {
        let Some(elem) = find_element(display, id) else {
            continue;
        };
        count += 1;
        if elem.kind == ElementKind::Group {
            has_group = true;
        }
        if elem.locked {
            any_locked = true;
        } else {
            any_unlocked = true;
        }
        union = Some(match union {
            Some(u) => u.union(elem.bounds),
            None => elem.bounds,
        });
    }

    let containing_frame = union.and_then(|u| containing_frame(display, selection, u));

    CtxScope {
        over_selection: true,
        count,
        multi: count >= 2,
        has_group,
        any_locked,
        any_unlocked,
        containing_frame,
    }
}

/// The top-most frame whose bounds fully contain `union` and which is not itself
/// part of the selection. `nodes` is back-to-front, so a reverse scan yields the
/// front-most (visually top) frame first.
fn containing_frame(
    display: &DisplayViewport,
    selection: &[ElementId],
    union: Rect,
) -> Option<ElementId> {
    display.nodes.iter().rev().find_map(|n| match n {
        DisplayNode::Element(e)
            if e.kind == ElementKind::Frame
                && !selection.contains(&e.id)
                && rect_contains_rect(e.bounds, union) =>
        {
            Some(e.id.clone())
        }
        _ => None,
    })
}

/// Translate a menu `CommandId` into the `ToileCommand`(s) to emit against the
/// current selection. The single place a Toile menu id becomes command(s); pure
/// and host-tested (`toile-menus`). `PASTE` and `SELECT_ALL` are UI-only intents
/// handled in the view (clipboard / local selection), so they return `vec![]`.
pub fn resolve_menu_command(
    id: CommandId,
    selection: &[ElementId],
    _group_scope: Option<&ElementId>,
    display: &DisplayViewport,
) -> Vec<ToileCommand> {
    let ids = || selection.to_vec();

    match id {
        COPY => vec![ToileCommand::CopyElements { ids: ids() }],
        // Cut = copy then delete; the channel preserves order, so the copy
        // captures the selection before the delete removes it (`toile-clipboard`).
        CUT => vec![
            ToileCommand::CopyElements { ids: ids() },
            ToileCommand::DeleteElements { ids: ids() },
        ],
        DELETE => vec![ToileCommand::DeleteElements { ids: ids() }],
        DUPLICATE => vec![ToileCommand::DuplicateElements {
            ids: ids(),
            delta: Vec2::new(DUPLICATE_NUDGE, DUPLICATE_NUDGE),
        }],
        GROUP => vec![ToileCommand::GroupElements { ids: ids() }],
        // One ungroup per selected group node (a non-group id is skipped here,
        // not left for Logic to reject).
        UNGROUP => selection
            .iter()
            .filter(|id| find_element(display, id).is_some_and(|e| e.kind == ElementKind::Group))
            .map(|id| ToileCommand::UngroupElements { group: id.clone() })
            .collect(),
        // One Reorder per selected element over the unified NodeId layer list.
        BRING_FRONT => reorder_each(selection, ZTarget::Front),
        SEND_BACK => reorder_each(selection, ZTarget::Back),
        FORWARD => reorder_each(selection, ZTarget::Forward),
        BACKWARD => reorder_each(selection, ZTarget::Backward),
        LOCK => vec![ToileCommand::LockElements {
            ids: ids(),
            locked: true,
        }],
        UNLOCK => vec![ToileCommand::LockElements {
            ids: ids(),
            locked: false,
        }],
        // Frame membership: the frame is the geometric container of the selection.
        // Logic stays authoritative on membership (the display exposes no
        // parent-frame edge), so Remove is a best-effort against that container.
        ADD_TO_FRAME => frame_children(selection, display, true),
        REMOVE_FROM_FRAME => frame_children(selection, display, false),
        // Reactions: set / clear the local user's reaction on each
        // selected target. One `React` per element; the emoji glyph is looked up
        // from the fixed picker set.
        REACT_CLEAR => react_each(selection, None),
        REACT_THUMBSUP | REACT_HEART | REACT_CHECK | REACT_PARTY => {
            let emoji = REACTION_EMOJI
                .iter()
                .find(|(cid, _)| *cid == id)
                .map(|(_, e)| (*e).to_string())
                .unwrap_or_default();
            react_each(
                selection,
                Some(Reaction {
                    kind: ReactionKind::Emoji(emoji),
                    ts: 0,
                }),
            )
        }
        // UI-only intents handled in the view.
        PASTE | SELECT_ALL => Vec::new(),
        _ => Vec::new(),
    }
}

/// One `React` command per selected element, setting (`Some`) or clearing
/// (`None`) the local user's reaction. The `Reaction` is cloned per target;
/// `ts` is left at 0 for the MVP (the derived tally ignores it -- ordering by
/// time is a later concern once a UI time source feeds the picker).
fn react_each(selection: &[ElementId], reaction: Option<Reaction>) -> Vec<ToileCommand> {
    selection
        .iter()
        .map(|id| ToileCommand::React {
            target: id.clone(),
            reaction: reaction.clone(),
        })
        .collect()
}

/// One `Reorder` command per selected element, targeting `to`.
fn reorder_each(selection: &[ElementId], to: ZTarget) -> Vec<ToileCommand> {
    selection
        .iter()
        .map(|id| ToileCommand::Reorder {
            node: NodeId::Element(id.clone()),
            to,
        })
        .collect()
}

/// Build the `SetFrameChildren` command adding (or removing) the selection to its
/// containing frame; empty when no enclosing frame exists.
fn frame_children(
    selection: &[ElementId],
    display: &DisplayViewport,
    add: bool,
) -> Vec<ToileCommand> {
    let union = selection
        .iter()
        .filter_map(|id| find_element(display, id).map(|e| e.bounds))
        .reduce(|a, b| a.union(b));
    let Some(union) = union else {
        return Vec::new();
    };
    let Some(frame) = containing_frame(display, selection, union) else {
        return Vec::new();
    };
    let (add_ids, remove_ids) = if add {
        (selection.to_vec(), Vec::new())
    } else {
        (Vec::new(), selection.to_vec())
    };
    vec![ToileCommand::SetFrameChildren {
        frame,
        add: add_ids,
        remove: remove_ids,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{
        DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, ElementStyle, Viewport,
    };
    use kurbo::Affine;

    fn eid(s: &str) -> ElementId {
        ElementId(s.to_owned())
    }

    fn elem(id: &str, kind: ElementKind, bounds: Rect, locked: bool) -> DisplayElement {
        DisplayElement {
            id: eid(id),
            kind,
            transform: Affine::IDENTITY,
            bounds,
            locked,
            style: ElementStyle::default(),
            geom: match kind {
                ElementKind::Group => DisplayGeom::Group,
                ElementKind::Frame => DisplayGeom::Frame,
                _ => DisplayGeom::Rect,
            },
            group: None,
        }
    }

    fn viewport(nodes: Vec<DisplayElement>) -> DisplayViewport {
        DisplayViewport {
            reactions: Default::default(),
            viewport: Viewport::default(),
            nodes: nodes.into_iter().map(DisplayNode::Element).collect(),
        }
    }

    fn r(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect::new(x0, y0, x1, y1)
    }

    #[test]
    fn react_emoji_resolves_to_one_react_per_target() {
        let dv = viewport(vec![
            elem("a", ElementKind::Sticky, r(0.0, 0.0, 10.0, 10.0), false),
            elem("b", ElementKind::Sticky, r(0.0, 0.0, 10.0, 10.0), false),
        ]);
        let cmds = resolve_menu_command(REACT_THUMBSUP, &[eid("a"), eid("b")], None, &dv);
        assert_eq!(cmds.len(), 2);
        for (cmd, want) in cmds.iter().zip(["a", "b"]) {
            match cmd {
                ToileCommand::React {
                    target,
                    reaction: Some(Reaction { kind, .. }),
                } => {
                    assert_eq!(target.as_str(), want);
                    assert_eq!(*kind, ReactionKind::Emoji("\u{1F44D}".to_string()));
                }
                other => panic!("expected React Some, got {other:?}"),
            }
        }
    }

    #[test]
    fn react_clear_resolves_to_none_reaction() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            r(0.0, 0.0, 10.0, 10.0),
            false,
        )]);
        let cmds = resolve_menu_command(REACT_CLEAR, &[eid("a")], None, &dv);
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            ToileCommand::React { reaction: None, .. }
        ));
    }

    #[test]
    fn empty_selection_is_empty_canvas_scope() {
        let dv = viewport(vec![]);
        let scope = ctx_scope_for(&[], None, &dv);
        assert_eq!(scope, CtxScope::default());
        assert!(!scope.over_selection);
    }

    #[test]
    fn single_unlocked_element_scope() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            r(0.0, 0.0, 10.0, 10.0),
            false,
        )]);
        let scope = ctx_scope_for(&[eid("a")], None, &dv);
        assert!(scope.over_selection);
        assert_eq!(scope.count, 1);
        assert!(!scope.multi);
        assert!(!scope.has_group);
        assert!(scope.any_unlocked);
        assert!(!scope.any_locked);
    }

    #[test]
    fn multi_selection_enables_group() {
        let dv = viewport(vec![
            elem("a", ElementKind::Sticky, r(0.0, 0.0, 10.0, 10.0), false),
            elem("b", ElementKind::Sticky, r(20.0, 0.0, 30.0, 10.0), false),
        ]);
        let scope = ctx_scope_for(&[eid("a"), eid("b")], None, &dv);
        assert!(scope.multi);
        assert_eq!(scope.count, 2);
    }

    #[test]
    fn group_and_locked_flags() {
        let dv = viewport(vec![
            elem("g", ElementKind::Group, r(0.0, 0.0, 10.0, 10.0), false),
            elem("l", ElementKind::Sticky, r(0.0, 0.0, 5.0, 5.0), true),
        ]);
        let scope = ctx_scope_for(&[eid("g"), eid("l")], None, &dv);
        assert!(scope.has_group);
        assert!(scope.any_locked);
        assert!(scope.any_unlocked);
    }

    #[test]
    fn containing_frame_detected() {
        let dv = viewport(vec![
            elem("f", ElementKind::Frame, r(0.0, 0.0, 100.0, 100.0), false),
            elem("a", ElementKind::Sticky, r(10.0, 10.0, 20.0, 20.0), false),
        ]);
        let scope = ctx_scope_for(&[eid("a")], None, &dv);
        assert_eq!(scope.containing_frame, Some(eid("f")));
    }

    #[test]
    fn no_frame_when_selection_outside() {
        let dv = viewport(vec![
            elem("f", ElementKind::Frame, r(0.0, 0.0, 30.0, 30.0), false),
            elem(
                "a",
                ElementKind::Sticky,
                r(100.0, 100.0, 110.0, 110.0),
                false,
            ),
        ]);
        let scope = ctx_scope_for(&[eid("a")], None, &dv);
        assert_eq!(scope.containing_frame, None);
    }

    #[test]
    fn copy_cut_delete_resolve() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            r(0.0, 0.0, 10.0, 10.0),
            false,
        )]);
        let sel = vec![eid("a")];
        assert!(matches!(
            resolve_menu_command(COPY, &sel, None, &dv).as_slice(),
            [ToileCommand::CopyElements { ids }] if ids == &sel
        ));
        assert!(matches!(
            resolve_menu_command(CUT, &sel, None, &dv).as_slice(),
            [
                ToileCommand::CopyElements { .. },
                ToileCommand::DeleteElements { .. }
            ]
        ));
        assert!(matches!(
            resolve_menu_command(DELETE, &sel, None, &dv).as_slice(),
            [ToileCommand::DeleteElements { ids }] if ids == &sel
        ));
    }

    #[test]
    fn duplicate_carries_nudge() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            r(0.0, 0.0, 10.0, 10.0),
            false,
        )]);
        let out = resolve_menu_command(DUPLICATE, &[eid("a")], None, &dv);
        assert!(matches!(
            out.as_slice(),
            [ToileCommand::DuplicateElements { delta, .. }]
                if delta.x == DUPLICATE_NUDGE && delta.y == DUPLICATE_NUDGE
        ));
    }

    #[test]
    fn group_ungroup_resolve() {
        let dv = viewport(vec![
            elem("g", ElementKind::Group, r(0.0, 0.0, 10.0, 10.0), false),
            elem("a", ElementKind::Sticky, r(0.0, 0.0, 5.0, 5.0), false),
        ]);
        assert!(matches!(
            resolve_menu_command(GROUP, &[eid("a"), eid("g")], None, &dv).as_slice(),
            [ToileCommand::GroupElements { .. }]
        ));
        // Ungroup emits only for the group node, not the plain sticky.
        let out = resolve_menu_command(UNGROUP, &[eid("a"), eid("g")], None, &dv);
        assert!(matches!(
            out.as_slice(),
            [ToileCommand::UngroupElements { group }] if group == &eid("g")
        ));
    }

    #[test]
    fn reorder_one_per_element() {
        let dv = viewport(vec![
            elem("a", ElementKind::Sticky, r(0.0, 0.0, 5.0, 5.0), false),
            elem("b", ElementKind::Sticky, r(6.0, 0.0, 10.0, 5.0), false),
        ]);
        let out = resolve_menu_command(BRING_FRONT, &[eid("a"), eid("b")], None, &dv);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|c| matches!(
            c,
            ToileCommand::Reorder {
                to: ZTarget::Front,
                ..
            }
        )));
    }

    #[test]
    fn lock_unlock_resolve() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            r(0.0, 0.0, 10.0, 10.0),
            false,
        )]);
        assert!(matches!(
            resolve_menu_command(LOCK, &[eid("a")], None, &dv).as_slice(),
            [ToileCommand::LockElements { locked: true, .. }]
        ));
        assert!(matches!(
            resolve_menu_command(UNLOCK, &[eid("a")], None, &dv).as_slice(),
            [ToileCommand::LockElements { locked: false, .. }]
        ));
    }

    #[test]
    fn add_to_frame_targets_container() {
        let dv = viewport(vec![
            elem("f", ElementKind::Frame, r(0.0, 0.0, 100.0, 100.0), false),
            elem("a", ElementKind::Sticky, r(10.0, 10.0, 20.0, 20.0), false),
        ]);
        let out = resolve_menu_command(ADD_TO_FRAME, &[eid("a")], None, &dv);
        assert!(matches!(
            out.as_slice(),
            [ToileCommand::SetFrameChildren { frame, add, remove }]
                if frame == &eid("f") && add == &vec![eid("a")] && remove.is_empty()
        ));
    }

    #[test]
    fn paste_and_select_all_are_ui_only() {
        let dv = viewport(vec![]);
        assert!(resolve_menu_command(PASTE, &[], None, &dv).is_empty());
        assert!(resolve_menu_command(SELECT_ALL, &[], None, &dv).is_empty());
    }
}

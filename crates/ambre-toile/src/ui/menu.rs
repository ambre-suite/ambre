// ambre-toile/src/ui/menu.rs
// ============================================================================ Command surface for
// the Toile right-click context menu (`toile-menus`). The host-visible half is three things. The
// `StaticKey<Commands>`s the menu contributes, and the pure selection scope descriptor the canvas view
// publishes to the DOM chrome. Then the pure `StaticKey<Commands> -> Vec<ToileCommand>` resolver the view
// applies against its local selection.
//
// Kept in the always-compiled `ui` module (not the wasm-only `chrome`) so it is host-unit-tested,
// exactly like `ui::find`. The wasm-only entry builder (icons + `MenuEntry` + separators) that
// consumes these lives in `chrome::menu`.
//
// Trame's context commands are parameterless, because the Logic thread tracks the active cell.
// Toile selection instead is UI-only state living in `ToileView` (`toile-selection`, and
// `ambre-spec-ui.md`). The Logic thread never sees it, so a menu `StaticKey<Commands>` cannot be turned into
// a `ToileCommand { ids: .. }` in the DOM shell. The shell forwards the bare `StaticKey<Commands>` into the
// view, which calls [`resolve_menu_command`] with its selection to build the real command(s).
//
// Everything here is pure and `cfg`-agnostic (no Leptos, no wasm), so it is host-unit-tested per
// `toile-menus`: each context-menu entry emits the expected command. The wasm-only entry builder
// (icons + separators) lives in `mod.rs`.
// ============================================================================

use crate::message::{
    AlignOp, DisplayElement, DisplayNode, DisplayViewport, ElementKind, Elements, LockState,
    NodeId, Reaction, ReactionKind, ToileCommand, ZTarget,
};
use ambre_bus::{Authored, Instant};
use ambre_core::Count;
use ambre_core::Flag as _;
use ambre_core::id::{Element, Id};
use ambre_core::{Board, Px};
use ambre_core::{Commands, StaticKey};
use kurbo::{Rect, Vec2};

/// Diagonal offset, on each axis, of a duplicate so the copy is visibly
/// distinct from its source (`toile-clipboard`). The menu Duplicate and the
/// `Ctrl/Cmd+D` of the view both read it.
const DUPLICATE_NUDGE: Px<Board> = Px::<Board>::new(10.0);

/// The move of a duplicate from its source: [`DUPLICATE_NUDGE`] on each axis.
pub(crate) fn duplicate_delta() -> Vec2 {
    Vec2::new(DUPLICATE_NUDGE.get(), DUPLICATE_NUDGE.get())
}

// -- Command identities --------------------------------------------------------
// String-keyed ids (`ambre-commands`), one per menu entry. The identity is the
// string, so the shell can round-trip an id through a signal without carrying the
// `Command` table.

// The three clipboard intents carry the shared identity, not one of Toile's.
// A cut is the same command in every application, whether a menu entry, a
// toolbar button or the browser's own event raised it
// (`ambre-spec-commands.md`, `UI-THREAD-011`).

/// Cut the selection (copy + delete).
pub const CUT: StaticKey<Commands> = ambre_commands::shared::CUT;
/// Copy the selection to the shared clipboard.
pub const COPY: StaticKey<Commands> = ambre_commands::shared::COPY;
/// Paste the clipboard at the cursor / viewport center.
pub const PASTE: StaticKey<Commands> = ambre_commands::shared::PASTE;
/// Duplicate the selection in place with a small nudge.
pub const DUPLICATE: StaticKey<Commands> = StaticKey::<Commands>::new("toile.edit.duplicate");
/// Delete the selection.
pub const DELETE: StaticKey<Commands> = StaticKey::<Commands>::new("toile.edit.delete");
/// Bind the selection into a group.
pub const GROUP: StaticKey<Commands> = StaticKey::<Commands>::new("toile.group.group");
/// Drop the selected group's binding.
pub const UNGROUP: StaticKey<Commands> = StaticKey::<Commands>::new("toile.group.ungroup");
/// Raise the selection to the top of the z-order.
pub const BRING_FRONT: StaticKey<Commands> = StaticKey::<Commands>::new("toile.z.front");
/// Lower the selection to the bottom of the z-order.
pub const SEND_BACK: StaticKey<Commands> = StaticKey::<Commands>::new("toile.z.back");
/// Raise the selection one step.
pub const FORWARD: StaticKey<Commands> = StaticKey::<Commands>::new("toile.z.forward");
/// Lower the selection one step.
pub const BACKWARD: StaticKey<Commands> = StaticKey::<Commands>::new("toile.z.backward");
/// Lock the selection (skips hit-test / handles).
pub const LOCK: StaticKey<Commands> = StaticKey::<Commands>::new("toile.lock.lock");
/// Unlock the selection.
pub const UNLOCK: StaticKey<Commands> = StaticKey::<Commands>::new("toile.lock.unlock");

// -- Align and distribute (`toile-elements section 1.7`) ----------------
// Eight one-shot batches over the selection: six put every box on an edge or
// centre line of the selection's union box, two equalize the gaps between three
// or more. Nothing is persisted -- the result is a set of coordinates, and the
// coordinates are what converges.

/// Put every selected box on the left edge of the selection.
pub const ALIGN_LEFT: StaticKey<Commands> = StaticKey::<Commands>::new("toile.align.left");
/// Centre every selected box on the vertical mid-line of the selection.
pub const ALIGN_HCENTER: StaticKey<Commands> = StaticKey::<Commands>::new("toile.align.hcenter");
/// Put every selected box on the right edge of the selection.
pub const ALIGN_RIGHT: StaticKey<Commands> = StaticKey::<Commands>::new("toile.align.right");
/// Put every selected box on the top edge of the selection.
pub const ALIGN_TOP: StaticKey<Commands> = StaticKey::<Commands>::new("toile.align.top");
/// Centre every selected box on the horizontal mid-line of the selection.
pub const ALIGN_VCENTER: StaticKey<Commands> = StaticKey::<Commands>::new("toile.align.vcenter");
/// Put every selected box on the bottom edge of the selection.
pub const ALIGN_BOTTOM: StaticKey<Commands> = StaticKey::<Commands>::new("toile.align.bottom");
/// Equalize the horizontal gaps between three or more selected boxes.
pub const DISTRIBUTE_H: StaticKey<Commands> =
    StaticKey::<Commands>::new("toile.align.distribute_h");
/// Equalize the vertical gaps between three or more selected boxes.
pub const DISTRIBUTE_V: StaticKey<Commands> =
    StaticKey::<Commands>::new("toile.align.distribute_v");
/// Add the selection to the frame that contains it.
pub const ADD_TO_FRAME: StaticKey<Commands> = StaticKey::<Commands>::new("toile.frame.add");
/// Remove the selection from its containing frame.
pub const REMOVE_FROM_FRAME: StaticKey<Commands> = StaticKey::<Commands>::new("toile.frame.remove");
/// Select every element on the board (local, never synced).
pub const SELECT_ALL: StaticKey<Commands> = StaticKey::<Commands>::new("toile.selection.all");

// -- Reactions ----------------------------------------------------------
// A minimal, fixed emoji set for the reaction affordance: each id sets the local
// user's reaction on every selected element (one `React` per target, LWW on the
// user's own key). `REACT_CLEAR` removes it. The picker is intentionally small
// and hardcoded for the MVP; a full emoji browser is a later concern.
pub const REACT_THUMBSUP: StaticKey<Commands> = StaticKey::<Commands>::new("toile.react.thumbsup");
pub const REACT_HEART: StaticKey<Commands> = StaticKey::<Commands>::new("toile.react.heart");
pub const REACT_CHECK: StaticKey<Commands> = StaticKey::<Commands>::new("toile.react.check");
pub const REACT_PARTY: StaticKey<Commands> = StaticKey::<Commands>::new("toile.react.party");
/// Remove the local user's own reaction from the selection.
pub const REACT_CLEAR: StaticKey<Commands> = StaticKey::<Commands>::new("toile.react.clear");

/// The fixed emoji entries of the reaction picker, id + glyph, in display order.
pub const REACTION_EMOJI: &[(StaticKey<Commands>, ambre_bus::Emoji)] = &[
    (REACT_THUMBSUP, ambre_bus::Emoji::from_static("\u{1F44D}")), // thumbs up
    (
        REACT_HEART,
        ambre_bus::Emoji::from_static("\u{2764}\u{FE0F}"),
    ), // red heart
    (REACT_CHECK, ambre_bus::Emoji::from_static("\u{2705}")),     // check mark button
    (REACT_PARTY, ambre_bus::Emoji::from_static("\u{1F389}")),    // party popper
];

/// Every command the Toile context menu contributes to the registry.
pub const COMMANDS: &[StaticKey<Commands>] = &[
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
    ALIGN_LEFT,
    ALIGN_HCENTER,
    ALIGN_RIGHT,
    ALIGN_TOP,
    ALIGN_VCENTER,
    ALIGN_BOTTOM,
    DISTRIBUTE_H,
    DISTRIBUTE_V,
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
/// rows. That is the "greyed out when inapplicable" of `toile-menus`, realized as
/// omission, the Trame pattern. `Default` is the empty-canvas scope (Paste +
/// Select-All).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct CtxScope {
    /// Where the right-click landed. On empty canvas only Paste + Select-All
    /// apply.
    pub site: ClickSite,
    /// Number of selected elements. Two or more -> Group applies.
    pub count: Count<Elements>,
    /// A selected element is a group node -> Ungroup applies.
    pub groups: GroupHeld,
    /// The lock states the selection holds: Unlock applies to a locked one,
    /// Lock to an unlocked one.
    pub locks: LockMix,
    /// The frame that geometrically contains the whole selection (if any and not
    /// itself selected) -> Add to frame applies.
    pub containing_frame: Option<Id<Element>>,
    /// A selected element is held by a frame, whatever its box -> Remove from
    /// frame applies (`TOILE-UI-384`).
    pub framed: FrameHeld,
    /// Where a key opened the menu, on the board's canvas, or `None` for a
    /// pointer, whose own event names its point (`TOILE-UI-422`). The shell
    /// opens the menu there, and clears it once the menu closes.
    pub keyed: Option<ambre_core::Point<Px<ambre_core::View>>>,
}

impl CtxScope {
    /// At least two elements selected -> Group applies.
    pub fn multi(&self) -> bool {
        self.count >= Count::<Elements>::new(2)
    }
}

ambre_core::define_flag! {
    /// Where a right-click landed: on empty canvas, or on a non-empty selection.
    pub enum ClickSite {
        Canvas,
        Selection,
    }

    /// Whether a selection holds a group node.
    pub enum GroupHeld {
        NoGroup,
        Group,
    }

    /// Whether a frame holds any element of a selection.
    pub enum FrameHeld {
        Unframed,
        Framed,
    }
}

/// The lock states a selection holds. A selection can hold both, and each
/// side offers its own command (`TOILE-UI-366`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LockMix {
    /// No element read yet.
    #[default]
    Empty,
    /// Every element read is unlocked.
    Unlocked,
    /// Every element read is locked.
    Locked,
    /// Some elements are locked and some are not.
    Both,
}

impl LockMix {
    /// The mix once one more element, in lock state `lock`, is read.
    #[must_use]
    pub fn with(self, lock: LockState) -> Self {
        let locked = lock == LockState::Locked;
        match (self, locked) {
            (Self::Empty | Self::Unlocked, false) => Self::Unlocked,
            (Self::Empty | Self::Locked, true) => Self::Locked,
            _ => Self::Both,
        }
    }

    /// At least one element is locked -> Unlock applies.
    pub fn any_locked(self) -> bool {
        matches!(self, Self::Locked | Self::Both)
    }

    /// At least one element is unlocked -> Lock applies.
    pub fn any_unlocked(self) -> bool {
        matches!(self, Self::Unlocked | Self::Both)
    }
}

/// Find a selected element by id (linear over the selection's data, which the
/// Logic thread sends whole, off-screen included -- `TOILE-UI-134`).
fn find_element<'a>(
    elements: &'a [DisplayElement],
    id: &Id<Element>,
) -> Option<&'a DisplayElement> {
    elements.iter().find(|e| &e.id == id)
}

/// True if `outer` fully contains `inner` (edges inclusive).
fn rect_contains_rect(outer: Rect, inner: Rect) -> bool {
    outer.x0 <= inner.x0 && outer.y0 <= inner.y0 && outer.x1 >= inner.x1 && outer.y1 >= inner.y1
}

/// Classify the current selection into a [`CtxScope`] the entry builder consumes.
/// Pure: reads the selection's whole data (`selected`, off-screen included), the
/// selected ids a frame holds (`framed`), and the culled `DisplayViewport` for
/// the enclosing-frame scan. `group_scope` is accepted for symmetry with the
/// rest of the resolution path (frame lookups may later honor it); the
/// empty-canvas case ignores everything else.
pub fn ctx_scope_for(
    selection: &[Id<Element>],
    _group_scope: Option<&Id<Element>>,
    selected: &[DisplayElement],
    framed: &[Id<Element>],
    display: &DisplayViewport,
) -> CtxScope {
    if selection.is_empty() {
        return CtxScope::default();
    }

    let mut count = Count::<Elements>::ZERO;
    let mut groups = GroupHeld::NoGroup;
    let mut locks = LockMix::Empty;
    // Union of the selected elements' bounds, to find an enclosing frame.
    let mut union: Option<Rect> = None;

    for id in selection {
        let Some(elem) = find_element(selected, id) else {
            continue;
        };
        count = count.incremented();
        if elem.kind == ElementKind::Group {
            groups = GroupHeld::Group;
        }
        locks = locks.with(elem.locked);
        union = Some(match union {
            Some(u) => u.union(elem.bounds),
            None => elem.bounds,
        });
    }

    let containing_frame = union.and_then(|u| containing_frame(display, selection, u));

    CtxScope {
        site: ClickSite::Selection,
        count,
        groups,
        locks,
        containing_frame,
        framed: FrameHeld::from_bool(selection.iter().any(|id| framed.contains(id))),
        keyed: None,
    }
}

/// The top-most frame whose bounds fully contain `union` and which is not itself
/// part of the selection. `nodes` is back-to-front, so a reverse scan yields the
/// front-most (visually top) frame first.
fn containing_frame(
    display: &DisplayViewport,
    selection: &[Id<Element>],
    union: Rect,
) -> Option<Id<Element>> {
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

/// The context-menu rows for `scope`, as an ordered id list. A `None` is a
/// separator. Pure and host-tested; the wasm entry builder maps each id to an
/// iconed `MenuEntry`. A locked selection omits the entries that would move or
/// remove it (`TOILE-UI-126`): cut, delete, duplicate and the four z-order moves.
pub fn context_menu_ids(scope: &CtxScope) -> Vec<Option<StaticKey<Commands>>> {
    let mut ids: Vec<Option<StaticKey<Commands>>> = Vec::new();

    if scope.site == ClickSite::Canvas {
        // Empty canvas: Paste + Select-All only.
        ids.extend([Some(PASTE), Some(SELECT_ALL)]);
        return ids;
    }
    let multi = scope.multi();
    let group = scope.groups == GroupHeld::Group;
    let framed = scope.framed == FrameHeld::Framed;
    let (any_locked, any_unlocked) = (scope.locks.any_locked(), scope.locks.any_unlocked());

    // Clipboard: a locked selection keeps copy and paste, and drops the three
    // that move or remove (`TOILE-UI-126`).
    if any_locked {
        ids.extend([Some(COPY), Some(PASTE)]);
    } else {
        ids.extend([
            Some(CUT),
            Some(COPY),
            Some(PASTE),
            Some(DUPLICATE),
            Some(DELETE),
        ]);
    }

    // Reactions.
    ids.push(None);
    ids.extend([
        Some(REACT_THUMBSUP),
        Some(REACT_HEART),
        Some(REACT_CHECK),
        Some(REACT_PARTY),
        Some(REACT_CLEAR),
    ]);

    // Grouping (only the applicable side).
    if multi || group {
        ids.push(None);
        if multi {
            ids.push(Some(GROUP));
        }
        if group {
            ids.push(Some(UNGROUP));
        }
    }

    // Z-order: a locked selection omits all four (`TOILE-UI-126`).
    if !any_locked {
        ids.push(None);
        ids.extend([
            Some(BRING_FRONT),
            Some(FORWARD),
            Some(BACKWARD),
            Some(SEND_BACK),
        ]);
    }

    // Lock / unlock (only the applicable side).
    if any_unlocked || any_locked {
        ids.push(None);
        if any_unlocked {
            ids.push(Some(LOCK));
        }
        if any_locked {
            ids.push(Some(UNLOCK));
        }
    }

    // Align and distribute: below two boxes there is no reference edge, below
    // three no interior gap.
    if multi {
        ids.push(None);
        ids.extend([
            Some(ALIGN_LEFT),
            Some(ALIGN_HCENTER),
            Some(ALIGN_RIGHT),
            Some(ALIGN_TOP),
            Some(ALIGN_VCENTER),
            Some(ALIGN_BOTTOM),
        ]);
        if scope.count >= Count::<Elements>::new(3) {
            ids.extend([Some(DISTRIBUTE_H), Some(DISTRIBUTE_V)]);
        }
    }

    // Frame membership. Adding needs a frame whose box holds the selection;
    // removing needs a frame that lists it, wherever its box now sits
    // (`TOILE-UI-384`).
    let add = scope.containing_frame.is_some();
    if add || framed {
        ids.push(None);
        if add {
            ids.push(Some(ADD_TO_FRAME));
        }
        if framed {
            ids.push(Some(REMOVE_FROM_FRAME));
        }
    }

    ids
}

/// The command `Ctrl+Shift+G` sends: one ungroup over the whole selection, so
/// the gesture is one undo item (`TOILE-094`). The chord does not filter the
/// groups, since the selection payload may lag a click. Logic reads the
/// document itself and answers nothing for a non-group beside a group
/// (`TOILE-264`). An empty selection sends nothing. Pure, so the chord's
/// shape is host-tested here rather than through the key handler.
pub fn ungroup_chord(selection: &[Id<Element>]) -> Option<ToileCommand> {
    (!selection.is_empty()).then(|| ToileCommand::UngroupElements {
        groups: selection.to_vec(),
    })
}

/// Translate a menu `StaticKey<Commands>` into the `ToileCommand`(s) to emit against the
/// current selection. The single place a Toile menu id becomes command(s); pure
/// and host-tested (`toile-menus`). `PASTE` and `SELECT_ALL` are UI-only intents
/// handled in the view, so they return `Some(vec![])`. An id the resolver does
/// not know returns `None`, refused rather than passing as a view-only intent
/// (`TOILE-UI-152`).
pub fn resolve_menu_command(
    id: StaticKey<Commands>,
    selection: &[Id<Element>],
    _group_scope: Option<&Id<Element>>,
    selected: &[DisplayElement],
    display: &DisplayViewport,
) -> Option<Vec<ToileCommand>> {
    let ids = || selection.to_vec();

    Some(match id {
        COPY => vec![ToileCommand::CopyElements { ids: ids() }],
        // Cut = copy then delete; the queue preserves order, so the copy
        // captures the selection before the delete removes it (`toile-clipboard`).
        CUT => vec![
            ToileCommand::CopyElements { ids: ids() },
            ToileCommand::DeleteElements { ids: ids() },
        ],
        DELETE => vec![ToileCommand::DeleteElements { ids: ids() }],
        DUPLICATE => vec![ToileCommand::DuplicateElements {
            ids: ids(),
            delta: duplicate_delta(),
        }],
        GROUP => vec![ToileCommand::GroupElements { ids: ids() }],
        // One ungroup over every selected group node, so one undo item
        // (`TOILE-094`). A non-group id is skipped here, not left for Logic to
        // reject; a selection with no group sends nothing.
        UNGROUP => {
            let groups: Vec<Id<Element>> = selection
                .iter()
                .filter(|id| {
                    find_element(selected, id).is_some_and(|e| e.kind == ElementKind::Group)
                })
                .cloned()
                .collect();
            if groups.is_empty() {
                Vec::new()
            } else {
                vec![ToileCommand::UngroupElements { groups }]
            }
        }
        // One Reorder per selected element over the unified NodeId layer list.
        BRING_FRONT => reorder_each(selection, ZTarget::Front),
        SEND_BACK => reorder_each(selection, ZTarget::Back),
        FORWARD => reorder_each(selection, ZTarget::Forward),
        BACKWARD => reorder_each(selection, ZTarget::Backward),
        LOCK => vec![ToileCommand::LockElements {
            ids: ids(),
            lock: crate::message::LockState::Locked,
        }],
        UNLOCK => vec![ToileCommand::LockElements {
            ids: ids(),
            lock: crate::message::LockState::Unlocked,
        }],
        // Align and distribute: one batch over the whole selection, whatever
        // the operation. Logic owns the arithmetic -- it holds the boxes, and
        // the culled display does not carry the ones off screen.
        ALIGN_LEFT => align(selection, AlignOp::Left),
        ALIGN_HCENTER => align(selection, AlignOp::HCenter),
        ALIGN_RIGHT => align(selection, AlignOp::Right),
        ALIGN_TOP => align(selection, AlignOp::Top),
        ALIGN_VCENTER => align(selection, AlignOp::VCenter),
        ALIGN_BOTTOM => align(selection, AlignOp::Bottom),
        DISTRIBUTE_H => align(selection, AlignOp::DistributeH),
        DISTRIBUTE_V => align(selection, AlignOp::DistributeV),
        // Frame membership. Add targets the frame that geometrically
        // holds the selection. Remove names no frame: the display carries no parent-frame
        // edge, so Logic takes each element out of the frame that really holds
        // it (`TOILE-UI-149`).
        ADD_TO_FRAME => add_to_frame(selection, selected, display),
        REMOVE_FROM_FRAME => vec![ToileCommand::RemoveFromFrames {
            ids: selection.to_vec(),
        }],
        // Reactions: set / clear the local user's reaction on each
        // selected target. One `React` per element; the emoji glyph is looked up
        // from the fixed picker set.
        REACT_CLEAR => react_each(selection, None),
        REACT_THUMBSUP | REACT_HEART | REACT_CHECK | REACT_PARTY => {
            let (_, emoji) = REACTION_EMOJI.iter().find(|(cid, _)| *cid == id)?;
            let emoji = emoji.clone();
            react_each(
                selection,
                Some(Reaction {
                    kind: ReactionKind::Emoji(emoji),
                    ts: Instant::<Authored>::new(0),
                }),
            )
        }
        // UI-only intents handled in the view.
        PASTE | SELECT_ALL => Vec::new(),
        // An id the resolver does not know is refused (`TOILE-UI-152`).
        _ => return None,
    })
}

/// One `React` command per selected element, setting (`Some`) or clearing
/// (`None`) the local user's reaction. The `Reaction` is cloned per target.
/// `ts` is left at 0 for the MVP, and the derived count ignores it. Ordering
/// by time is a later concern, once a UI time source feeds the picker.
fn react_each(selection: &[Id<Element>], reaction: Option<Reaction>) -> Vec<ToileCommand> {
    selection
        .iter()
        .map(|id| ToileCommand::React {
            target: id.clone(),
            reaction: reaction.clone(),
        })
        .collect()
}

/// The one align / distribute batch for the whole selection.
fn align(selection: &[Id<Element>], op: AlignOp) -> Vec<ToileCommand> {
    vec![ToileCommand::AlignElements {
        ids: selection.to_vec(),
        op,
    }]
}

/// One `Reorder` command per selected element, targeting `to`.
fn reorder_each(selection: &[Id<Element>], to: ZTarget) -> Vec<ToileCommand> {
    selection
        .iter()
        .map(|id| ToileCommand::Reorder {
            node: NodeId::Element(id.clone()),
            to,
        })
        .collect()
}

/// Build the `SetFrameChildren` command adding the selection to its containing
/// frame; empty when no enclosing frame exists.
fn add_to_frame(
    selection: &[Id<Element>],
    selected: &[DisplayElement],
    display: &DisplayViewport,
) -> Vec<ToileCommand> {
    let union = selection
        .iter()
        .filter_map(|id| find_element(selected, id).map(|e| e.bounds))
        .reduce(|a, b| a.union(b));
    let Some(union) = union else {
        return Vec::new();
    };
    let Some(frame) = containing_frame(display, selection, union) else {
        return Vec::new();
    };
    vec![ToileCommand::SetFrameChildren {
        frame,
        add: selection.to_vec(),
        remove: Vec::new(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{
        DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, ElementStyle, Viewport,
    };
    use kurbo::Affine;

    fn eid(s: &str) -> Id<Element> {
        Id::<Element>::from_text(s.to_owned())
    }

    fn elem(
        id: &str,
        kind: ElementKind,
        bounds: Rect,
        locked: crate::message::LockState,
    ) -> DisplayElement {
        DisplayElement {
            id: eid(id),
            kind,
            transform: Affine::IDENTITY,
            bounds,
            locked,
            style: ElementStyle::default(),
            geom: match kind {
                ElementKind::Group => DisplayGeom::Group,
                ElementKind::Frame => DisplayGeom::Frame(String::new()),
                _ => DisplayGeom::Rect,
            },
            link: None,
            group: None,
        }
    }

    fn viewport(nodes: Vec<DisplayElement>) -> DisplayViewport {
        DisplayViewport {
            version: ambre_core::Seq::ZERO,
            grid_unit: Px::<Board>::new(20.0),
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Viewport::default(),
            nodes: nodes.into_iter().map(DisplayNode::Element).collect(),
        }
    }

    /// The selection's whole display data, as Logic sends it (off-screen
    /// included). The tests build a viewport holding the whole selection, so the
    /// extracted elements are the same set.
    fn els(dv: &DisplayViewport) -> Vec<DisplayElement> {
        dv.nodes
            .iter()
            .filter_map(|n| match n {
                DisplayNode::Element(e) => Some(e.clone()),
                _ => None,
            })
            .collect()
    }

    fn r(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect::new(x0, y0, x1, y1)
    }

    /// `TOILE-UI-047`: an entry that does not apply is left out. One unlocked
    /// element in no frame gets no ungroup, unlock, align, distribute or frame
    /// entry, and the empty board gets paste and select all alone.
    #[test]
    fn an_entry_that_does_not_apply_is_left_out() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            r(0.0, 0.0, 10.0, 10.0),
            crate::message::LockState::Unlocked,
        )]);
        let scope = ctx_scope_for(&[eid("a")], None, &els(&dv), &[], &dv);
        let ids: Vec<StaticKey<Commands>> =
            context_menu_ids(&scope).into_iter().flatten().collect();
        for omitted in [
            UNGROUP,
            UNLOCK,
            ALIGN_LEFT,
            DISTRIBUTE_H,
            ADD_TO_FRAME,
            REMOVE_FROM_FRAME,
        ] {
            assert!(!ids.contains(&omitted), "{omitted:?} does not apply");
        }
        let empty: Vec<StaticKey<Commands>> = context_menu_ids(&CtxScope::default())
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(empty, [PASTE, SELECT_ALL]);
    }

    /// `TOILE-UI-150`: where two frames hold the selection, the entries
    /// target the front-most one, the later in the painted order.
    #[test]
    fn overlapping_frames_resolve_to_the_front_most() {
        let dv = viewport(vec![
            elem(
                "back",
                ElementKind::Frame,
                r(0.0, 0.0, 100.0, 100.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "front",
                ElementKind::Frame,
                r(5.0, 5.0, 50.0, 50.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "a",
                ElementKind::Sticky,
                r(10.0, 10.0, 20.0, 20.0),
                crate::message::LockState::Unlocked,
            ),
        ]);
        let scope = ctx_scope_for(&[eid("a")], None, &els(&dv), &[], &dv);
        assert_eq!(scope.containing_frame, Some(eid("front")));
    }

    #[test]
    fn react_emoji_resolves_to_one_react_per_target() {
        let dv = viewport(vec![
            elem(
                "a",
                ElementKind::Sticky,
                r(0.0, 0.0, 10.0, 10.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "b",
                ElementKind::Sticky,
                r(0.0, 0.0, 10.0, 10.0),
                crate::message::LockState::Unlocked,
            ),
        ]);
        let cmds =
            resolve_menu_command(REACT_THUMBSUP, &[eid("a"), eid("b")], None, &els(&dv), &dv)
                .unwrap();
        assert_eq!(cmds.len(), 2);
        for (cmd, want) in cmds.iter().zip(["a", "b"]) {
            match cmd {
                ToileCommand::React {
                    target,
                    reaction: Some(Reaction { kind, .. }),
                } => {
                    assert_eq!(target.as_str(), want);
                    assert_eq!(
                        *kind,
                        ReactionKind::Emoji(ambre_bus::Emoji::from("\u{1F44D}"))
                    );
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
            crate::message::LockState::Unlocked,
        )]);
        let cmds = resolve_menu_command(REACT_CLEAR, &[eid("a")], None, &els(&dv), &dv).unwrap();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            ToileCommand::React { reaction: None, .. }
        ));
    }

    #[test]
    fn empty_selection_is_empty_canvas_scope() {
        let dv = viewport(vec![]);
        let scope = ctx_scope_for(&[], None, &els(&dv), &[], &dv);
        assert_eq!(scope, CtxScope::default());
        assert_eq!(scope.site, ClickSite::Canvas);
    }

    #[test]
    fn single_unlocked_element_scope() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            r(0.0, 0.0, 10.0, 10.0),
            crate::message::LockState::Unlocked,
        )]);
        let scope = ctx_scope_for(&[eid("a")], None, &els(&dv), &[], &dv);
        assert_eq!(scope.site, ClickSite::Selection);
        assert_eq!(scope.count, Count::<Elements>::new(1));
        assert!(!scope.multi());
        assert_eq!(scope.groups, GroupHeld::NoGroup);
        assert_eq!(scope.locks, LockMix::Unlocked);
    }

    #[test]
    fn multi_selection_enables_group() {
        let dv = viewport(vec![
            elem(
                "a",
                ElementKind::Sticky,
                r(0.0, 0.0, 10.0, 10.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "b",
                ElementKind::Sticky,
                r(20.0, 0.0, 30.0, 10.0),
                crate::message::LockState::Unlocked,
            ),
        ]);
        let scope = ctx_scope_for(&[eid("a"), eid("b")], None, &els(&dv), &[], &dv);
        assert!(scope.multi());
        assert_eq!(scope.count, Count::<Elements>::new(2));
    }

    #[test]
    fn group_and_locked_flags() {
        let dv = viewport(vec![
            elem(
                "g",
                ElementKind::Group,
                r(0.0, 0.0, 10.0, 10.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "l",
                ElementKind::Sticky,
                r(0.0, 0.0, 5.0, 5.0),
                crate::message::LockState::Locked,
            ),
        ]);
        let scope = ctx_scope_for(&[eid("g"), eid("l")], None, &els(&dv), &[], &dv);
        assert_eq!(scope.groups, GroupHeld::Group);
        assert_eq!(scope.locks, LockMix::Both);
    }

    #[test]
    fn containing_frame_detected() {
        let dv = viewport(vec![
            elem(
                "f",
                ElementKind::Frame,
                r(0.0, 0.0, 100.0, 100.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "a",
                ElementKind::Sticky,
                r(10.0, 10.0, 20.0, 20.0),
                crate::message::LockState::Unlocked,
            ),
        ]);
        let scope = ctx_scope_for(&[eid("a")], None, &els(&dv), &[], &dv);
        assert_eq!(scope.containing_frame, Some(eid("f")));
    }

    #[test]
    fn no_frame_when_selection_outside() {
        let dv = viewport(vec![
            elem(
                "f",
                ElementKind::Frame,
                r(0.0, 0.0, 30.0, 30.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "a",
                ElementKind::Sticky,
                r(100.0, 100.0, 110.0, 110.0),
                crate::message::LockState::Unlocked,
            ),
        ]);
        let scope = ctx_scope_for(&[eid("a")], None, &els(&dv), &[], &dv);
        assert_eq!(scope.containing_frame, None);
    }

    /// `TOILE-UI-384`: an element a frame holds, dragged out of that frame's
    /// box, is still offered removal. Adding needs a box that holds it, so
    /// add is not offered.
    #[test]
    fn a_framed_element_outside_every_box_is_offered_removal() {
        let dv = viewport(vec![
            elem(
                "f",
                ElementKind::Frame,
                r(0.0, 0.0, 30.0, 30.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "a",
                ElementKind::Sticky,
                r(100.0, 100.0, 110.0, 110.0),
                crate::message::LockState::Unlocked,
            ),
        ]);
        let scope = ctx_scope_for(&[eid("a")], None, &els(&dv), &[eid("a")], &dv);
        assert_eq!(scope.framed, FrameHeld::Framed);
        let ids: Vec<StaticKey<Commands>> =
            context_menu_ids(&scope).into_iter().flatten().collect();
        assert!(ids.contains(&REMOVE_FROM_FRAME), "removal not offered");
        assert!(
            !ids.contains(&ADD_TO_FRAME),
            "add offered with no frame box"
        );
    }

    /// `TOILE-UI-047`: a box that holds an element no frame lists offers add,
    /// never a removal that would change nothing.
    #[test]
    fn an_unframed_element_inside_a_box_is_offered_add_only() {
        let dv = viewport(vec![
            elem(
                "f",
                ElementKind::Frame,
                r(0.0, 0.0, 100.0, 100.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "a",
                ElementKind::Sticky,
                r(10.0, 10.0, 20.0, 20.0),
                crate::message::LockState::Unlocked,
            ),
        ]);
        let scope = ctx_scope_for(&[eid("a")], None, &els(&dv), &[], &dv);
        let ids: Vec<StaticKey<Commands>> =
            context_menu_ids(&scope).into_iter().flatten().collect();
        assert!(ids.contains(&ADD_TO_FRAME));
        assert!(!ids.contains(&REMOVE_FROM_FRAME));
    }

    /// The culled viewport holds only `a`; `b` is off-screen but in the
    /// selection data Logic sent. The scope counts and flags both
    /// (`TOILE-UI-134`).
    #[test]
    fn off_screen_selection_counts_in_scope() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            r(0.0, 0.0, 10.0, 10.0),
            crate::message::LockState::Unlocked,
        )]);
        let selected = vec![
            elem(
                "a",
                ElementKind::Sticky,
                r(0.0, 0.0, 10.0, 10.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "b",
                ElementKind::Sticky,
                r(1000.0, 1000.0, 1010.0, 1010.0),
                crate::message::LockState::Locked,
            ),
        ];
        let scope = ctx_scope_for(&[eid("a"), eid("b")], None, &selected, &[], &dv);
        assert_eq!(scope.count, Count::<Elements>::new(2));
        assert!(scope.multi());
        assert_eq!(scope.locks, LockMix::Both);
    }

    /// A lock mix that has read both states stays `Both`, whatever it reads
    /// next, and an empty one offers neither command.
    #[test]
    fn a_lock_mix_keeps_both_states_once_read() {
        use LockState::{Locked, Unlocked};
        assert!(!LockMix::Empty.any_locked() && !LockMix::Empty.any_unlocked());
        let both = LockMix::Empty.with(Locked).with(Unlocked);
        assert_eq!(both, LockMix::Both);
        assert_eq!(both.with(Locked), LockMix::Both);
        assert_eq!(both.with(Unlocked), LockMix::Both);
        assert!(both.any_locked() && both.any_unlocked());
        assert_eq!(LockMix::Empty.with(Locked).with(Locked), LockMix::Locked);
        assert_eq!(LockMix::Empty.with(Unlocked), LockMix::Unlocked);
    }

    #[test]
    fn copy_cut_delete_resolve() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            r(0.0, 0.0, 10.0, 10.0),
            crate::message::LockState::Unlocked,
        )]);
        let sel = vec![eid("a")];
        assert!(matches!(
            resolve_menu_command(COPY, &sel, None, &els(&dv), &dv).unwrap().as_slice(),
            [ToileCommand::CopyElements { ids }] if ids == &sel
        ));
        assert!(matches!(
            resolve_menu_command(CUT, &sel, None, &els(&dv), &dv)
                .unwrap()
                .as_slice(),
            [
                ToileCommand::CopyElements { .. },
                ToileCommand::DeleteElements { .. }
            ]
        ));
        assert!(matches!(
            resolve_menu_command(DELETE, &sel, None, &els(&dv), &dv).unwrap().as_slice(),
            [ToileCommand::DeleteElements { ids }] if ids == &sel
        ));
    }

    #[test]
    fn duplicate_carries_nudge() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            r(0.0, 0.0, 10.0, 10.0),
            crate::message::LockState::Unlocked,
        )]);
        let out = resolve_menu_command(DUPLICATE, &[eid("a")], None, &els(&dv), &dv).unwrap();
        assert!(matches!(
            out.as_slice(),
            [ToileCommand::DuplicateElements { delta, .. }]
                if *delta == duplicate_delta()
        ));
    }

    #[test]
    fn group_ungroup_resolve() {
        let dv = viewport(vec![
            elem(
                "g",
                ElementKind::Group,
                r(0.0, 0.0, 10.0, 10.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "a",
                ElementKind::Sticky,
                r(0.0, 0.0, 5.0, 5.0),
                crate::message::LockState::Unlocked,
            ),
        ]);
        assert!(matches!(
            resolve_menu_command(GROUP, &[eid("a"), eid("g")], None, &els(&dv), &dv)
                .unwrap()
                .as_slice(),
            [ToileCommand::GroupElements { .. }]
        ));
        // Ungroup emits only for the group node, not the plain sticky.
        let out =
            resolve_menu_command(UNGROUP, &[eid("a"), eid("g")], None, &els(&dv), &dv).unwrap();
        assert!(matches!(
            out.as_slice(),
            [ToileCommand::UngroupElements { groups }] if groups == &[eid("g")]
        ));
    }

    /// `TOILE-094`: the menu entry and the chord each send one ungroup for a
    /// selection of several groups, never one per id.
    #[test]
    fn ungrouping_a_selection_sends_one_command() {
        let dv = viewport(vec![
            elem(
                "g",
                ElementKind::Group,
                r(0.0, 0.0, 10.0, 10.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "h",
                ElementKind::Group,
                r(20.0, 0.0, 30.0, 10.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "a",
                ElementKind::Sticky,
                r(0.0, 0.0, 5.0, 5.0),
                crate::message::LockState::Unlocked,
            ),
        ]);
        let sel = [eid("g"), eid("a"), eid("h")];
        let out = resolve_menu_command(UNGROUP, &sel, None, &els(&dv), &dv).unwrap();
        assert!(matches!(
            out.as_slice(),
            [ToileCommand::UngroupElements { groups }] if groups == &[eid("g"), eid("h")]
        ));
        assert!(matches!(
            ungroup_chord(&sel),
            Some(ToileCommand::UngroupElements { groups }) if groups == sel
        ));
        assert!(ungroup_chord(&[]).is_none());
    }

    #[test]
    fn reorder_one_per_element() {
        let dv = viewport(vec![
            elem(
                "a",
                ElementKind::Sticky,
                r(0.0, 0.0, 5.0, 5.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "b",
                ElementKind::Sticky,
                r(6.0, 0.0, 10.0, 5.0),
                crate::message::LockState::Unlocked,
            ),
        ]);
        let out =
            resolve_menu_command(BRING_FRONT, &[eid("a"), eid("b")], None, &els(&dv), &dv).unwrap();
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
            crate::message::LockState::Unlocked,
        )]);
        assert!(matches!(
            resolve_menu_command(LOCK, &[eid("a")], None, &els(&dv), &dv)
                .unwrap()
                .as_slice(),
            [ToileCommand::LockElements {
                lock: crate::message::LockState::Locked,
                ..
            }]
        ));
        assert!(matches!(
            resolve_menu_command(UNLOCK, &[eid("a")], None, &els(&dv), &dv)
                .unwrap()
                .as_slice(),
            [ToileCommand::LockElements {
                lock: crate::message::LockState::Unlocked,
                ..
            }]
        ));
    }

    #[test]
    fn add_to_frame_targets_container() {
        let dv = viewport(vec![
            elem(
                "f",
                ElementKind::Frame,
                r(0.0, 0.0, 100.0, 100.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "a",
                ElementKind::Sticky,
                r(10.0, 10.0, 20.0, 20.0),
                crate::message::LockState::Unlocked,
            ),
        ]);
        let out = resolve_menu_command(ADD_TO_FRAME, &[eid("a")], None, &els(&dv), &dv).unwrap();
        assert!(matches!(
            out.as_slice(),
            [ToileCommand::SetFrameChildren { frame, add, remove }]
                if frame == &eid("f") && add == &vec![eid("a")] && remove.is_empty()
        ));
    }

    /// `TOILE-UI-149`: the removal names the selection and no frame, even when
    /// a frame other than the element's parent contains its box. Logic resolves
    /// the parent.
    #[test]
    fn remove_from_frame_names_no_frame() {
        let dv = viewport(vec![
            elem(
                "f",
                ElementKind::Frame,
                r(0.0, 0.0, 100.0, 100.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "a",
                ElementKind::Sticky,
                r(10.0, 10.0, 20.0, 20.0),
                crate::message::LockState::Unlocked,
            ),
        ]);
        let out =
            resolve_menu_command(REMOVE_FROM_FRAME, &[eid("a")], None, &els(&dv), &dv).unwrap();
        assert!(matches!(
            out.as_slice(),
            [ToileCommand::RemoveFromFrames { ids }] if ids == &vec![eid("a")]
        ));
    }

    #[test]
    fn paste_and_select_all_are_ui_only() {
        let dv = viewport(vec![]);
        assert!(
            resolve_menu_command(PASTE, &[], None, &els(&dv), &dv)
                .unwrap()
                .is_empty()
        );
        assert!(
            resolve_menu_command(SELECT_ALL, &[], None, &els(&dv), &dv)
                .unwrap()
                .is_empty()
        );
    }

    /// `TOILE-UI-152`: an id the resolver does not know is refused, never passed
    /// as a view-only intent.
    #[test]
    fn an_unknown_id_is_refused_not_ui_only() {
        let dv = viewport(vec![]);
        assert!(
            resolve_menu_command(
                StaticKey::<Commands>::new("toile.unknown"),
                &[],
                None,
                &els(&dv),
                &dv
            )
            .is_none()
        );
    }

    /// Every align id resolves to one batch over the whole selection. The
    /// arithmetic is Logic's, because the culled display does not carry the
    /// boxes that sit off screen.
    #[test]
    fn an_align_id_resolves_to_one_batch_over_the_selection() {
        let dv = viewport(vec![
            elem(
                "a",
                ElementKind::Sticky,
                r(0.0, 0.0, 10.0, 10.0),
                crate::message::LockState::Unlocked,
            ),
            elem(
                "b",
                ElementKind::Sticky,
                r(40.0, 0.0, 10.0, 10.0),
                crate::message::LockState::Unlocked,
            ),
        ]);
        let sel = [eid("a"), eid("b")];
        for (id, want) in [
            (ALIGN_LEFT, AlignOp::Left),
            (ALIGN_HCENTER, AlignOp::HCenter),
            (ALIGN_RIGHT, AlignOp::Right),
            (ALIGN_TOP, AlignOp::Top),
            (ALIGN_VCENTER, AlignOp::VCenter),
            (ALIGN_BOTTOM, AlignOp::Bottom),
            (DISTRIBUTE_H, AlignOp::DistributeH),
            (DISTRIBUTE_V, AlignOp::DistributeV),
        ] {
            let out = resolve_menu_command(id, &sel, None, &els(&dv), &dv).unwrap();
            assert!(
                matches!(
                    out.as_slice(),
                    [ToileCommand::AlignElements { ids, op }]
                        if ids.as_slice() == sel && *op == want
                ),
                "{id:?} resolved to {out:?}"
            );
        }
    }

    /// `TOILE-UI-126`: a locked selection omits the entries that would move or
    /// remove it. Cut, delete, duplicate and the four z-order moves go; copy,
    /// paste and unlock stay.
    #[test]
    fn a_locked_selection_omits_move_and_remove_entries() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            r(0.0, 0.0, 10.0, 10.0),
            crate::message::LockState::Locked,
        )]);
        let scope = ctx_scope_for(&[eid("a")], None, &els(&dv), &[], &dv);
        let ids: Vec<StaticKey<Commands>> =
            context_menu_ids(&scope).into_iter().flatten().collect();
        for omitted in [
            CUT,
            DELETE,
            DUPLICATE,
            BRING_FRONT,
            FORWARD,
            BACKWARD,
            SEND_BACK,
        ] {
            assert!(
                !ids.contains(&omitted),
                "{omitted:?} must be omitted when locked"
            );
        }
        for kept in [COPY, PASTE, UNLOCK] {
            assert!(ids.contains(&kept), "{kept:?} must stay when locked");
        }
    }

    /// An unlocked selection keeps the full clipboard and z-order sets.
    #[test]
    fn an_unlocked_selection_keeps_the_full_set() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            r(0.0, 0.0, 10.0, 10.0),
            crate::message::LockState::Unlocked,
        )]);
        let scope = ctx_scope_for(&[eid("a")], None, &els(&dv), &[], &dv);
        let ids: Vec<StaticKey<Commands>> =
            context_menu_ids(&scope).into_iter().flatten().collect();
        for kept in [
            CUT,
            COPY,
            PASTE,
            DUPLICATE,
            DELETE,
            BRING_FRONT,
            FORWARD,
            BACKWARD,
            SEND_BACK,
        ] {
            assert!(ids.contains(&kept), "{kept:?} must stay when unlocked");
        }
    }
}

// ambre-ui/src/focus.rs
// ============================================================================
// FocusManager -- keyboard focus and Tab order.
//
// Tab order is rebuilt after each layout pass (DFS on the widget tree,
// filtered by Widget::focusable()). Between two layouts the order is stable.
// ============================================================================

use crate::widget::WidgetId;

/// Manages keyboard focus and Tab navigation order.
pub struct FocusManager {
    /// Tab order computed after each layout pass.
    /// DFS on the main widget tree, filtered by `widget.focusable()`.
    tab_order: Vec<WidgetId>,
    focused: Option<WidgetId>,
}

impl FocusManager {
    pub fn new() -> Self {
        Self {
            tab_order: Vec::new(),
            focused: None,
        }
    }

    /// Replace the tab order with a new list of focusable widget IDs.
    /// Called by the framework after layout when `dirty_layout` was true.
    ///
    /// The caller (framework integration code) is responsible for DFS
    /// traversal and filtering by `Widget::focusable()` -- the FocusManager
    /// only stores and navigates the flat list.
    pub fn set_tab_order(&mut self, order: Vec<WidgetId>) {
        self.tab_order = order;
        // If the currently focused widget was removed, clear focus.
        if let Some(id) = self.focused
            && !self.tab_order.contains(&id)
        {
            self.focused = None;
        }
    }

    /// Advance focus to the next focusable widget (Tab).
    pub fn focus_next(&mut self) -> Option<WidgetId> {
        if self.tab_order.is_empty() {
            return None;
        }
        let current_idx = self
            .focused
            .and_then(|id| self.tab_order.iter().position(|&x| x == id))
            .unwrap_or(usize::MAX);
        let next_idx = current_idx.wrapping_add(1) % self.tab_order.len();
        self.focused = self.tab_order.get(next_idx).copied();
        self.focused
    }

    /// Move focus to the previous focusable widget (Shift+Tab).
    pub fn focus_prev(&mut self) -> Option<WidgetId> {
        if self.tab_order.is_empty() {
            return None;
        }
        let current_idx = self
            .focused
            .and_then(|id| self.tab_order.iter().position(|&x| x == id))
            .unwrap_or(0);
        let prev_idx = current_idx
            .checked_sub(1)
            .unwrap_or(self.tab_order.len() - 1);
        self.focused = self.tab_order.get(prev_idx).copied();
        self.focused
    }

    pub fn focused(&self) -> Option<WidgetId> {
        self.focused
    }

    pub fn set_focus(&mut self, id: WidgetId) {
        self.focused = Some(id);
    }

    pub fn clear_focus(&mut self) {
        self.focused = None;
    }

    pub fn tab_order(&self) -> &[WidgetId] {
        &self.tab_order
    }
}

impl Default for FocusManager {
    fn default() -> Self {
        Self::new()
    }
}

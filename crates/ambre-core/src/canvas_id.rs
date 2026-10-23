//! The id of a canvas element the shell mounts.

/// The id of a `<canvas>` element in the page. The shell mounts the element
/// with this id, and the canvas host finds it by the same id. Each id is a
/// constant of the code, so a distinct type keeps it apart from page text.
/// `Debug` and `Display` print the bare id.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CanvasId(&'static str);

impl CanvasId {
    /// The content canvas, the central one that every application paints.
    pub const CONTENT: Self = Self("ambre-canvas");

    /// The canvas whose element carries `id`.
    #[must_use]
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }

    /// The id, as the DOM takes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for CanvasId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::fmt::Debug for CanvasId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_prints_its_bare_text() {
        assert_eq!(CanvasId::CONTENT.to_string(), "ambre-canvas");
        assert_eq!(format!("{:?}", CanvasId::new("x")), "\"x\"");
    }
}

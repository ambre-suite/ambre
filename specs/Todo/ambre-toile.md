# AMBRE — APP: TOILE (DESIGN & INFINITE COLLABORATION)
> Specifications for the graphics engine, spatial indexing, and presentation mode.

## 1. VISION & SYNERGY
Toile is Ambre's free creation space. It merges precision vector drawing (Figma), collaborative whiteboard (Miro), and sequential storytelling (Presentation).
- **Engine:** wgpy + Parley/Swash (High-fidelity GPU rendering).
- **Performance:** Support for 50,000+ objects with smooth zoom and instant selection.

---

## 2. CORE FEATURES

### A. Infinite Workspace
* **Infinite Canvas:** Unlimited 2D coordinates with adaptive magnetic grid.
* **Frames:** Logical containers to organize content (Artboards). A Frame can become a "Slide" in presentation mode.
* **Auto-Layout:** Dynamic constraint system to auto-align and space objects (inspired by Figma).

### B. Creation & Design Tools
* **Precision Vector:** Bezier paths, boolean operations (Union, Subtraction), and node manipulation.
* **Miro-style Objects:** Sticky notes, smart connectors (arrows that follow objects), and quick shapes.
* **Asset Library:** Drag & drop of images, SVG, and reusable components synced via CRDT.
* **wgpu Typography:** Rich text integrated via Parley, with vector text effect support.

### C. Real-time Co-creation
* **Multi-cursors:** Visualization of collaborators' movements and selections.
* **Highlighters:** Temporary drawing tools (laser pointer) for brainstorming sessions.
* **Votes & Reactions:** Placing emojis or voting on board elements.

---

## 3. PRESENTATION MODE
The presentation mode is not a separate application, but an orchestrated view of Toile.

* **Frame Sequencing:** A side panel allows ordering "Frames" to define the narrative thread.
* **Smooth Transitions (Smart Animate):** When moving from one slide to another, Toile interpolates (Lerp) properties (position, size, color) of objects with the same stable ID.
* **Focus Mode:** Hiding the editing interface, auto-centering on the active Frame.
* **Presenter Overlay:** Floating window to view speaker notes.

---

## 4. TECHNICAL ARCHITECTURE

Infinite canvas with freely positioned elements. Reuses the positional canvas from presentations, extended with continuous zoom/pan, spatial index, connectors, and freehand drawing.

### CRDT Structure

```
Loro::Doc
├── "board" (Map)                → {name, bg_color, grid_size, grid_snap}
├── "elements" (Map)             → all board elements
│   └── "elem_{id}" (Map)        → {type, x, y, w, h, rotation, z_index, locked, ...}
│       type = "sticky_note" | "shape" | "text" | "image" | "frame"
│             | "connector" | "drawing" | "embed"
├── "connectors" (Map)           → links between elements
│   └── "conn_{id}" (Map)        → {from_elem, from_anchor, to_elem, to_anchor, path_type, style}
├── "layers" (Array)             → global z-ordering
├── "frames" (Map)               → groups/frames (logical containers)
│   └── "frame_{id}" (Map)       → {x, y, w, h, children: [...elem_ids]}
├── "styles" (Map)
└── "media" (Map)
```

### Spatial index (culling)

With potentially thousands of elements, an R-tree filters those visible in the viewport in O(log N). Rebuilt on load, updated incrementally on each CRDT mutation.

```rust
struct SpatialIndex {
    tree: RTree<ElementBounds>,
}

struct Viewport {
    center: DVec2,
    zoom: f64,        // 0.1 .. 10.0
    size: DVec2,      // screen size in pixels
}

impl SpatialIndex {
    fn query_visible(&self, viewport: &Viewport) -> Vec<ElementId> {
        let world_rect = viewport.to_world_rect();
        self.tree.locate_in_envelope(&world_rect).map(|e| e.id).collect()
    }
}
```

### Freehand drawing

Each stroke = an element of type `"drawing"` with a `Vec<(f32, f32, f32)>` (x, y, pressure). No conflict possible: each stroke belongs to a single author. Catmull-Rom smoothing client-side at render time.

### Connectors

Connectors reference the source/target element IDs and an anchor point (top, bottom, left, right, center). When an element is moved, connectors are recalculated client-side (rendering only, no CRDT mutation on the connector). Path routing (straight, elbow, curved) is pure client compute.

### CRDT conflicts

Elements are independent → conflicts virtually non-existent. Two users moving the same element? Last-write-wins, acceptable for a whiteboard. Frames (groups) use an Array of child IDs, concurrent insertions are resolved by Loro.

---

## 5. MEDIA & ASSET MANAGEMENT
* **Asynchronous Loading:** Images and PDFs are stored on MinIO. Toile displays a low-resolution placeholder during download.
* **Caching:** Local caching of frequently used assets for instant display during zooms/pans.

---

## 6. INTEROPERABILITY (IMPORT / EXPORT)
* **Import:** SVG, PNG/JPG, Figma files (via JSON converter), and Miro diagrams.
* **Export:** Multi-page PDF, high-resolution SVG/PNG, and Web-Showcase (standalone WASM).

---

## 7. REJECTED TECHNICAL CHOICES
* **HTML5 Canvas API / SVG DOM:** Rejected (Insufficient performance for 10,000+ objects).
* **Linear presentation engine (PPT type):** Rejected in favor of the "Spatial Frames" concept.

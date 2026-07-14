//! The display list: absolutely-positioned drawing primitives produced by
//! layout and consumed by the renderer, plus the logical structure tree and
//! outline that make the rendered PDF tagged and accessible.
//!
//! Layout produces three things at once, bundled in [`Layout`]:
//!
//! * the [`Page`]s of drawing [`Element`]s (what to paint, and where);
//! * a [`StructNode`] tree describing the document's *logical* structure
//!   (headings, paragraphs, lists, tables) in reading order; and
//! * a flat list of [`OutlineEntry`] bookmarks, one per heading.
//!
//! Every text run carries a [`Tagging`] linking it to a structure node (or
//! marking it as a decorative artifact), so the renderer can wrap each piece of
//! drawn content in the marked-content sequence its structure node references.

use std::sync::Arc;

use krilla::{
    color::rgb,
    tagging::{ListNumbering, TableHeaderScope},
    text::KrillaGlyph,
};

use crate::fonts::Style;

/// Identifies a leaf node of the logical [structure tree](StructNode): the
/// renderer collects every marked-content sequence tagged with a given id under
/// that node.
pub type NodeId = usize;

/// The full output of a layout pass: the pages to paint plus the accessibility
/// metadata (logical structure and outline) needed to emit a tagged PDF.
///
/// Dereferences to the page slice, so existing code that only wants the pages
/// can treat a `Layout` as `&[Page]`.
#[derive(Debug, Default)]
pub struct Layout {
    /// The laid-out pages, in order.
    pub pages: Vec<Page>,
    /// The logical structure tree's top-level nodes, in reading order.
    pub structure: Vec<StructNode>,
    /// One bookmark per heading, in document order, for the PDF outline.
    pub outline: Vec<OutlineEntry>,
    /// The number of allocated [`NodeId`]s, i.e. the size of the id space the
    /// renderer must collect marked-content identifiers into.
    pub nodes: usize,
}

impl std::ops::Deref for Layout {
    type Target = [Page];

    fn deref(&self) -> &[Page] {
        &self.pages
    }
}

/// A single laid-out page: a flat list of drawing primitives.
#[derive(Debug, Default)]
pub struct Page {
    pub elements: Vec<Element>,
}

/// A drawing primitive positioned in page space.
///
/// A [`Text`](Element::Text) run belongs to the logical structure (its
/// [`Tagging`] says which node); rectangles and strokes are always decorative
/// and are emitted as artifacts, excluded from the structure tree.
#[derive(Debug, Clone)]
pub enum Element {
    /// A run of shaped glyphs drawn from `(x, baseline)`.
    Text(TextElement),
    /// A filled rectangle.
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: rgb::Color,
    },
    /// A stroked polyline; `closed` connects the last point back to the first.
    Stroke {
        points: Vec<(f32, f32)>,
        width: f32,
        color: rgb::Color,
        closed: bool,
    },
}

/// How a drawn text run maps into the tagged PDF's logical structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tagging {
    /// Real content belonging to the structure [leaf](StructNode::Leaf) with
    /// this id; the renderer collects its marked-content sequence there.
    Content(NodeId),
    /// Content excluded from the logical structure and emitted as an artifact,
    /// e.g. a table header repeated at the top of a continuation page (the
    /// header is tagged once, on its first appearance).
    Artifact,
}

/// A positioned run of shaped text.
#[derive(Debug, Clone)]
pub struct TextElement {
    pub x: f32,
    pub baseline: f32,
    pub size: f32,
    pub color: rgb::Color,
    pub style: Style,
    /// Shaped glyphs, shared with the shaping cache (see [`crate::fonts::Shaped`]).
    pub glyphs: Arc<[KrillaGlyph]>,
    pub text: String,
    /// Where this run belongs in the logical structure.
    pub tag: Tagging,
}

/// The semantic role of a [structure node](StructNode). Mapped to a concrete
/// krilla structure tag by the renderer.
#[derive(Debug, Clone)]
pub enum StructTag {
    /// A heading at the given level (1 = highest). `title` is the heading's
    /// plain text, required as the tag's title attribute for PDF/UA.
    Heading { level: u8, title: String },
    /// A paragraph of flowing text.
    Paragraph,
    /// A list, numbered in the given style (bullets use a bullet "numbering").
    List(ListNumbering),
    /// One item of a list.
    ListItem,
    /// A list item's label (its bullet or number).
    Label,
    /// A list item's body (its content).
    Body,
    /// A generic grouping element (used for boxed callouts).
    Div,
    /// A table.
    Table,
    /// A table row.
    TableRow,
    /// A header cell, scoped to its row and/or column.
    TableHeaderCell(TableHeaderScope),
    /// A data cell.
    TableCell,
}

/// A node in the logical structure tree, built during layout in reading order.
#[derive(Debug, Clone)]
pub enum StructNode {
    /// A grouping element with a semantic tag and structural children.
    Group {
        tag: StructTag,
        children: Vec<StructNode>,
    },
    /// A content leaf: the renderer wraps it around the marked-content
    /// sequences drawn with [`Tagging::Content(id)`](Tagging::Content).
    Leaf { tag: StructTag, id: NodeId },
}

/// A heading bookmark for the PDF outline: its level (for nesting) and the page
/// and vertical position it should jump to.
#[derive(Debug, Clone)]
pub struct OutlineEntry {
    pub level: u8,
    pub title: String,
    pub page_index: usize,
    /// The heading's top edge in page space (distance from the page top).
    pub y: f32,
}

//! The document model: a structured, layout-agnostic representation of a
//! document. The [`build`](crate::build) API produces this; the layout engine
//! consumes it.

use std::{collections::HashMap, sync::Arc};

use krilla::color::rgb;

use crate::{
    fonts::Style,
    theme::{BoxStyle, Palette, TableStyle, Theme},
};

/// A whole document ready to be laid out.
#[derive(Debug, Default, Clone)]
pub struct Document {
    /// Page header
    pub header: Chrome,
    /// Blocks that make up the body, in order
    pub blocks: Vec<Block>,
    /// Page footer
    pub footer: Chrome,
    /// Design tokens used to lay out and render the document.
    pub theme: Theme,
    /// The document title, written to the PDF metadata and shown by viewers.
    /// Required for the accessible (PDF/UA) output; when unset the renderer
    /// falls back to the first heading's text.
    pub title: Option<String>,
    /// The document's primary natural language as a BCP 47 / RFC 3066 tag (e.g.
    /// `"en"`, `"en-GB"`), written to the PDF metadata. Required for accessible
    /// output; defaults to `"en"` when unset.
    pub language: Option<String>,
}

impl Document {
    /// Resolve section numbering in place.
    ///
    /// Numbered headings (see [`Block::Heading::numbered`]) get a hierarchical
    /// number ("3", "3.1", …) built from one counter per heading level and
    /// prefixed to their text. Section references ([`Inline::section_ref`])
    /// are then replaced with the number of the section they point to, so
    /// forward references work. Headings inside [`Block::Box`] content
    /// participate as well.
    ///
    /// The builder's [`build`](crate::build::Textris::build) and
    /// [`render`](crate::build::Textris::render) call this automatically; the
    /// pass is idempotent because it clears the `numbered` flags and
    /// `section_ref` markers it resolves.
    pub fn resolve_sections(&mut self) {
        self.resolve_sections_impl(true, RefStyle::Number);
    }

    /// Resolve section references to the referenced section's title, in quotes,
    /// and leave the headings unnumbered.
    ///
    /// Like [`resolve_sections`](Self::resolve_sections) this replaces
    /// [`section_ref`](Inline::section_ref) placeholders, but instead of a
    /// section number it substitutes the referenced heading's title wrapped in
    /// double quotes (e.g. `"Vision"`), and it does not prefix numbers onto the
    /// headings. This suits output formats that number headings themselves, and
    /// so have no visible section numbers to reference, such as Markdown.
    pub fn resolve_sections_unnumbered(&mut self) {
        self.resolve_sections_impl(false, RefStyle::QuotedTitle);
    }

    /// Shared body of the resolution passes: walk the numbered headings to
    /// build the map from label to resolved reference text (a number or a
    /// quoted title, per `ref_style`) and substitute the references.
    /// `prefix_headings` controls whether section numbers are also prefixed
    /// onto the headings themselves.
    fn resolve_sections_impl(&mut self, prefix_headings: bool, ref_style: RefStyle) {
        let mut counters = [0usize; 7];
        let mut refs = HashMap::new();
        number_headings(
            &mut self.blocks,
            &mut counters,
            &mut refs,
            prefix_headings,
            ref_style,
        );
        resolve_refs_in_blocks(&mut self.blocks, &refs);
        for section in [&mut self.header, &mut self.footer] {
            for content in [&mut section.left, &mut section.center, &mut section.right]
                .into_iter()
                .flatten()
            {
                if let SectionContent::Spans(spans) = content {
                    resolve_refs_in_inlines(spans, &refs);
                }
            }
        }
    }
}

/// How a [`section_ref`](Inline::section_ref) placeholder is resolved.
#[derive(Debug, Clone, Copy)]
enum RefStyle {
    /// Substitute the section's hierarchical number (e.g. `3.1`).
    Number,
    /// Substitute the section's heading title, in double quotes (e.g.
    /// `"Vision"`).
    QuotedTitle,
}

/// Walk the numbered headings, recording each labeled section's resolved
/// reference text (a number or a quoted title, per `ref_style`) in `refs` and,
/// when `prefix` is set, prefixing the number to the heading text.
fn number_headings(
    blocks: &mut [Block],
    counters: &mut [usize; 7],
    refs: &mut HashMap<String, String>,
    prefix: bool,
    ref_style: RefStyle,
) {
    for block in blocks {
        match block {
            Block::Heading {
                level,
                content,
                numbered,
                label,
            } if *numbered => {
                let level = (*level).clamp(1, 6) as usize;
                counters[level] += 1;
                for deeper in counters[level + 1..].iter_mut() {
                    *deeper = 0;
                }
                let number = counters[1..=level]
                    .iter()
                    .filter(|&&c| c > 0)
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(".");
                // Record the reference text before any number prefix is added,
                // so a title reference is the clean heading text.
                if let Some(label) = label.take() {
                    let resolved = match ref_style {
                        RefStyle::Number => number.clone(),
                        RefStyle::QuotedTitle => format!("\"{}\"", plain_text(content)),
                    };
                    refs.insert(label, resolved);
                }
                if prefix {
                    content.insert(0, Inline::new(format!("{number}. ")));
                }
                *numbered = false;
            }
            Block::Box { content, .. } => {
                number_headings(content, counters, refs, prefix, ref_style)
            }
            _ => {}
        }
    }
}

/// Replace section-reference placeholder runs with their resolved text (a
/// number or a quoted title, per the pass that built `refs`).
fn resolve_refs_in_blocks(blocks: &mut [Block], refs: &HashMap<String, String>) {
    for block in blocks {
        match block {
            Block::Heading { content, .. } | Block::Paragraph(content) => {
                resolve_refs_in_inlines(content, refs)
            }
            Block::TaskList(items) => {
                for item in items {
                    resolve_refs_in_inlines(&mut item.content, refs);
                }
            }
            Block::BulletList(items) | Block::OrderedList { items, .. } => {
                for item in items {
                    resolve_refs_in_inlines(item, refs);
                }
            }
            Block::Table(table) => {
                for cell in table
                    .headers
                    .iter_mut()
                    .chain(table.rows.iter_mut().flatten())
                {
                    if let Cell::Text(inlines) = cell {
                        resolve_refs_in_inlines(inlines, refs);
                    }
                }
            }
            Block::Box { content, .. } => resolve_refs_in_blocks(content, refs),
            Block::PageBreak | Block::Spacer(_) => {}
        }
    }
}

fn resolve_refs_in_inlines(inlines: &mut [Inline], refs: &HashMap<String, String>) {
    for inline in inlines {
        if let Some(label) = &inline.section_ref
            && let Some(resolved) = refs.get(label)
        {
            inline.text = resolved.clone();
            inline.section_ref = None;
        }
    }
}

/// A top-level block element.
#[derive(Debug, Clone)]
pub enum Block {
    /// A heading at the given level (1 = largest, 3-5 = section headings).
    Heading {
        level: u8,
        content: Vec<Inline>,
        /// Assign this heading a section number (see
        /// [`Document::resolve_sections`]). The number is prefixed to the
        /// heading text and the flag cleared when the document is resolved.
        numbered: bool,
        /// A label other content can reference with
        /// [`Inline::section_ref`]; it resolves to the section number.
        label: Option<String>,
    },
    /// A paragraph of flowing text.
    Paragraph(Vec<Inline>),
    /// A table.
    Table(Table),
    /// A task list (`- [ ]` / `- [x]`), rendered with checkboxes.
    TaskList(Vec<TaskItem>),
    /// A plain bullet list.
    BulletList(Vec<Vec<Inline>>),
    /// An ordered (numbered or lettered) list. Items are numbered from 1 in the
    /// chosen [`ListMarker`] style.
    OrderedList {
        marker: ListMarker,
        items: Vec<Vec<Inline>>,
    },
    /// A boxed callout: child blocks drawn on a filled background with padding
    /// and margin (e.g. a highlighted warning note).
    Box {
        style: BoxStyle,
        content: Vec<Block>,
    },
    /// An explicit page break: the following content starts on a fresh page.
    PageBreak,
    /// Fixed vertical space of the given height in points. No inter-block gap
    /// is added around a spacer, so it *is* the distance between its
    /// neighbours.
    Spacer(f32),
}

/// How an [`OrderedList`](Block::OrderedList) numbers its items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListMarker {
    /// Arabic numerals: `1.`, `2.`, `3.`, …
    #[default]
    Decimal,
    /// Lowercase letters: `a.`, `b.`, `c.`, … (wrapping `aa.`, `ab.` past `z.`).
    LowerAlpha,
}

impl ListMarker {
    /// The label for the 1-based item at `index` (1 = the first item), including
    /// the trailing dot, e.g. `"3."` or `"c."`.
    pub fn label(&self, index: usize) -> String {
        match self {
            Self::Decimal => format!("{index}."),
            Self::LowerAlpha => format!("{}.", alpha(index)),
        }
    }
}

/// Convert a 1-based index into a bijective base-26 lowercase label: 1→`a`,
/// 26→`z`, 27→`aa`, …
fn alpha(index: usize) -> String {
    let mut n = index;
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(b'a' + (n % 26) as u8);
        n /= 26;
    }
    out.reverse();
    String::from_utf8(out).expect("ascii letters")
}

/// The color of an inline run: either a concrete value or a theme role that is
/// resolved against the document's [`Palette`] at layout time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InlineColor {
    /// A concrete color.
    Rgb(rgb::Color),
    /// The theme palette's muted (secondary) text color.
    Muted,
}

impl InlineColor {
    /// The concrete color this stands for under the given palette.
    pub fn resolve(self, palette: &Palette) -> rgb::Color {
        match self {
            Self::Rgb(color) => color,
            Self::Muted => palette.muted,
        }
    }
}

impl From<rgb::Color> for InlineColor {
    fn from(color: rgb::Color) -> Self {
        Self::Rgb(color)
    }
}

/// A run of text with emphasis flags. The layout engine resolves these against a
/// role-specific base style to pick a concrete font [`Style`].
#[derive(Debug, Clone, PartialEq)]
pub struct Inline {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    /// Override the color for this run. `None` uses the default text color.
    pub color: Option<InlineColor>,
    /// When set, this run is a placeholder for the number of the section
    /// labeled with this name: [`Document::resolve_sections`] replaces `text`
    /// with the section number. An unknown label leaves the placeholder text.
    pub section_ref: Option<String>,
    /// When set, this run draws no text but an inline horizontal fill-in line
    /// of this many points, along the text baseline (a blank to be written on).
    /// Its `text` is empty and its emphasis flags are ignored.
    pub fill_in: Option<f32>,
}

impl Inline {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            bold: false,
            italic: false,
            mono: false,
            color: None,
            section_ref: None,
            fill_in: None,
        }
    }

    /// Resolve this run's font style, given whether the surrounding context is
    /// itself bold and/or italic (e.g. a heading is bold).
    pub fn resolve_style(&self, base_bold: bool, base_italic: bool) -> Style {
        if self.mono {
            return Style::Mono;
        }
        match (self.bold || base_bold, self.italic || base_italic) {
            (false, false) => Style::Regular,
            (true, false) => Style::Bold,
            (false, true) => Style::Italic,
            (true, true) => Style::BoldItalic,
        }
    }
}

/// Concatenate the plain text of a run of inlines.
pub fn plain_text(inlines: &[Inline]) -> String {
    inlines.iter().map(|i| i.text.as_str()).collect()
}

/// A task-list entry.
#[derive(Debug, Clone)]
pub struct TaskItem {
    pub checked: bool,
    pub content: Vec<Inline>,
}

/// One table cell.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    /// Inline-formatted text.
    Text(Vec<Inline>),
    /// An empty cell with a fill-in line along its bottom (a blank form field).
    FillIn,
    /// A deliberately empty cell: nothing is drawn.
    Blank,
    /// An empty cell that forces its row to be at least this many points tall.
    Spacer(f32),
}

impl Cell {
    /// The cell's inline text, empty for the non-text variants.
    pub fn inlines(&self) -> &[Inline] {
        match self {
            Self::Text(inlines) => inlines,
            _ => &[],
        }
    }

    /// Whether the cell shows nothing: blank, a spacer, or text that is empty
    /// or whitespace. A fill-in cell draws its line, so it is not blank.
    pub fn is_blank(&self) -> bool {
        match self {
            Self::Text(inlines) => plain_text(inlines).trim().is_empty(),
            Self::FillIn => false,
            Self::Blank | Self::Spacer(_) => true,
        }
    }
}

/// A table of cells. Its presentation is decided by its [`TableStyle`], chosen
/// when the table is added to the document.
#[derive(Debug, Clone)]
pub struct Table {
    pub style: TableStyle,
    pub headers: Vec<Cell>,
    pub rows: Vec<Vec<Cell>>,
}

impl Table {
    /// The number of columns, taken from the widest row.
    pub fn columns(&self) -> usize {
        self.headers
            .len()
            .max(self.rows.iter().map(Vec::len).max().unwrap_or(0))
    }

    /// Whether the table shows a header row: its style asks for one and the
    /// header cells are not all blank. Shared by every output format (PDF,
    /// Markdown, docx) so they agree on when a header appears.
    pub fn has_header(&self) -> bool {
        self.style.header && !self.headers.iter().all(Cell::is_blank)
    }
}

/// Content for one section (left, center, or right) of the page header or footer.
pub enum SectionContent {
    /// Plain text, rendered in the normal body color.
    Text(String),
    /// A sequence of styled [`Inline`] runs (reuses the same type as body text).
    Spans(Vec<Inline>),
    /// Page counter: called with `(page, total)`, returns styled [`Inline`] runs.
    PageCounter(Arc<dyn Fn(usize, usize) -> Vec<Inline> + Send + Sync>),
}

impl SectionContent {
    /// Resolve the content for a concrete page into styled runs: plain text
    /// becomes a single regular run, and page counters are invoked with
    /// `(page, total)`.
    pub fn resolve(&self, page: usize, total: usize) -> Vec<Inline> {
        match self {
            Self::Text(s) => vec![Inline::new(s.clone())],
            Self::Spans(spans) => spans.clone(),
            Self::PageCounter(f) => f(page, total),
        }
    }
}

impl Clone for SectionContent {
    fn clone(&self) -> Self {
        match self {
            Self::Text(s) => Self::Text(s.clone()),
            Self::Spans(spans) => Self::Spans(spans.clone()),
            Self::PageCounter(f) => Self::PageCounter(Arc::clone(f)),
        }
    }
}

impl std::fmt::Debug for SectionContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(s) => f.debug_tuple("Text").field(s).finish(),
            Self::Spans(spans) => f.debug_tuple("Spans").field(spans).finish(),
            Self::PageCounter(_) => f.debug_tuple("PageCounter").field(&"<fn>").finish(),
        }
    }
}

impl From<String> for SectionContent {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

impl From<&str> for SectionContent {
    fn from(s: &str) -> Self {
        Self::Text(s.into())
    }
}

/// One row of page chrome (the running header or footer) with up to three
/// aligned sections.
#[derive(Debug, Default, Clone)]
pub struct Chrome {
    pub left: Option<SectionContent>,
    pub center: Option<SectionContent>,
    pub right: Option<SectionContent>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inline(text: &str, bold: bool, italic: bool, mono: bool) -> Inline {
        Inline {
            text: text.into(),
            bold,
            italic,
            mono,
            color: None,
            section_ref: None,
            fill_in: None,
        }
    }

    #[test]
    fn resolve_style_combines_context_and_emphasis() {
        // Plain text in a plain context.
        assert_eq!(Inline::new("x").resolve_style(false, false), Style::Regular);
        // Bold emphasis wins over a plain context.
        assert_eq!(
            inline("x", true, false, false).resolve_style(false, false),
            Style::Bold
        );
        // A bold context (e.g. a heading) makes plain text bold.
        assert_eq!(Inline::new("x").resolve_style(true, false), Style::Bold);
        // Bold context plus italic emphasis is bold-italic.
        assert_eq!(
            inline("x", false, true, false).resolve_style(true, false),
            Style::BoldItalic
        );
        // Monospace always wins, ignoring the context.
        assert_eq!(
            inline("x", true, true, true).resolve_style(true, true),
            Style::Mono
        );
    }

    #[test]
    fn ordered_list_marker_labels() {
        assert_eq!(ListMarker::Decimal.label(1), "1.");
        assert_eq!(ListMarker::Decimal.label(42), "42.");
        assert_eq!(ListMarker::LowerAlpha.label(1), "a.");
        assert_eq!(ListMarker::LowerAlpha.label(26), "z.");
        assert_eq!(ListMarker::LowerAlpha.label(27), "aa.");
        assert_eq!(ListMarker::LowerAlpha.label(28), "ab.");
    }

    #[test]
    fn columns_uses_widest_row() {
        let table = Table {
            style: TableStyle::data(),
            headers: vec![Cell::Blank, Cell::Text(vec![Inline::new("a")])],
            rows: vec![vec![
                Cell::Blank,
                Cell::FillIn,
                Cell::Text(vec![Inline::new("c")]),
            ]],
        };
        assert_eq!(table.columns(), 3);
    }

    #[test]
    fn cell_blankness_reflects_what_is_drawn() {
        assert!(Cell::Blank.is_blank());
        assert!(Cell::Spacer(20.0).is_blank());
        assert!(Cell::Text(vec![Inline::new("  ")]).is_blank());
        assert!(!Cell::FillIn.is_blank(), "a fill-in cell draws its line");
        assert!(!Cell::Text(vec![Inline::new("x")]).is_blank());
    }

    /// Build a heading block for resolution tests.
    fn heading(level: u8, text: &str, numbered: bool, label: Option<&str>) -> Block {
        Block::Heading {
            level,
            content: vec![Inline::new(text)],
            numbered,
            label: label.map(String::from),
        }
    }

    #[test]
    fn resolve_sections_numbers_headings_hierarchically() {
        let mut doc = Document {
            blocks: vec![
                heading(3, "Intro", true, None),
                heading(4, "Scope", true, None),
                heading(4, "Terms", true, None),
                heading(3, "Body", true, None),
                heading(4, "Detail", true, None),
                heading(6, "Deep", true, None),
                heading(3, "Plain", false, None),
            ],
            ..Document::default()
        };
        doc.resolve_sections();

        let texts: Vec<String> = doc
            .blocks
            .iter()
            .map(|b| match b {
                Block::Heading { content, .. } => plain_text(content),
                _ => panic!("expected headings"),
            })
            .collect();
        assert_eq!(
            texts,
            [
                "1. Intro",
                "1.1. Scope",
                "1.2. Terms",
                "2. Body",
                "2.1. Detail",
                "2.1.1. Deep",
                "Plain"
            ]
        );
    }

    #[test]
    fn resolve_sections_substitutes_forward_and_backward_references() {
        let reference = |label: &str| Inline {
            section_ref: Some(label.into()),
            ..Inline::new("??")
        };
        let mut doc = Document {
            blocks: vec![
                Block::Paragraph(vec![reference("later")]),
                heading(3, "First", true, Some("early")),
                heading(3, "Second", true, Some("later")),
                Block::Paragraph(vec![reference("early"), reference("missing")]),
            ],
            ..Document::default()
        };
        doc.resolve_sections();

        let paragraph = |i: usize| match &doc.blocks[i] {
            Block::Paragraph(inlines) => inlines.clone(),
            _ => panic!("expected a paragraph"),
        };
        assert_eq!(plain_text(&paragraph(0)), "2", "forward reference");
        let last = paragraph(3);
        assert_eq!(last[0].text, "1", "backward reference");
        assert_eq!(last[1].text, "??", "unknown labels keep the placeholder");
        assert!(last[1].section_ref.is_some());
    }

    #[test]
    fn resolve_sections_reaches_header_and_footer_chrome() {
        let mut doc = Document {
            blocks: vec![heading(3, "Intro", true, Some("intro"))],
            ..Document::default()
        };
        doc.footer.right = Some(SectionContent::Spans(vec![Inline {
            section_ref: Some("intro".into()),
            ..Inline::new("??")
        }]));
        doc.resolve_sections();

        let Some(SectionContent::Spans(spans)) = &doc.footer.right else {
            panic!("footer spans expected");
        };
        assert_eq!(spans[0].text, "1", "chrome references resolve too");
        assert!(spans[0].section_ref.is_none());
    }

    #[test]
    fn resolve_sections_is_idempotent() {
        let mut doc = Document {
            blocks: vec![
                heading(3, "Intro", true, Some("intro")),
                Block::Paragraph(vec![Inline {
                    section_ref: Some("intro".into()),
                    ..Inline::new("??")
                }]),
            ],
            ..Document::default()
        };
        doc.resolve_sections();
        let once = format!("{:?}", doc.blocks);
        // A second pass must not re-number ("1. 1. Intro") or re-substitute.
        doc.resolve_sections();
        assert_eq!(format!("{:?}", doc.blocks), once);
    }

    #[test]
    fn resolve_sections_unnumbered_leaves_headings_plain_and_refs_by_title() {
        let reference = |label: &str| Inline {
            section_ref: Some(label.into()),
            ..Inline::new("??")
        };
        let mut doc = Document {
            blocks: vec![
                heading(3, "First", true, Some("a")),
                heading(4, "Nested", true, None),
                heading(3, "Second", true, None),
                Block::Paragraph(vec![reference("a")]),
            ],
            ..Document::default()
        };
        doc.resolve_sections_unnumbered();

        let heading_text = |i: usize| match &doc.blocks[i] {
            Block::Heading { content, .. } => plain_text(content),
            _ => panic!("expected a heading"),
        };
        // Headings keep their plain text, with no number prefixed.
        assert_eq!(heading_text(0), "First");
        assert_eq!(heading_text(1), "Nested");
        assert_eq!(heading_text(2), "Second");
        // The reference repeats the referenced heading's title, in quotes.
        let Block::Paragraph(inlines) = &doc.blocks[3] else {
            panic!("expected a paragraph");
        };
        assert_eq!(plain_text(inlines), "\"First\"");
    }
}

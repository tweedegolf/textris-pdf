//! The document model: a structured, layout-agnostic representation of a
//! document. The [`build`](crate::build) API produces this; the layout engine
//! consumes it.

use std::sync::Arc;

use krilla::color::rgb;

use crate::{
    fonts::Style,
    theme::{BoxStyle, TableStyle, Theme},
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
}

/// A top-level block element.
#[derive(Debug, Clone)]
pub enum Block {
    /// A heading at the given level (1 = largest, 3-5 = section headings).
    Heading { level: u8, content: Vec<Inline> },
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

/// A run of text with emphasis flags. The layout engine resolves these against a
/// role-specific base style to pick a concrete font [`Style`].
#[derive(Debug, Clone, PartialEq)]
pub struct Inline {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    /// Override the color for this run. `None` uses the default text color.
    pub color: Option<rgb::Color>,
}

impl Inline {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            bold: false,
            italic: false,
            mono: false,
            color: None,
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

/// A table of inline-formatted cells. Its presentation is decided by its
/// [`TableStyle`], chosen when the table is added to the document.
#[derive(Debug, Clone)]
pub struct Table {
    pub style: TableStyle,
    pub headers: Vec<Vec<Inline>>,
    pub rows: Vec<Vec<Vec<Inline>>>,
}

impl Table {
    /// The number of columns, taken from the widest row.
    pub fn columns(&self) -> usize {
        self.headers
            .len()
            .max(self.rows.iter().map(Vec::len).max().unwrap_or(0))
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
    fn plain_text_concatenates_runs() {
        let runs = vec![
            inline("Hello ", false, false, false),
            inline("world", true, false, false),
        ];
        assert_eq!(plain_text(&runs), "Hello world");
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
            headers: vec![vec![], vec![Inline::new("a")]],
            rows: vec![vec![vec![], vec![], vec![Inline::new("c")]]],
        };
        assert_eq!(table.columns(), 3);
    }
}

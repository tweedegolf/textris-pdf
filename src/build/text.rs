//! Rich text for the builder API: the [`Text`] fragment builder and the
//! [`IntoText`] conversion trait that lets every builder method accept plain
//! strings and styled fragments alike.

use krilla::color::rgb;

use crate::model::{Cell, Inline, InlineColor, SectionContent};

/// A rich-text fragment: an ordered sequence of styled runs.
///
/// Build one by chaining, starting from [`Text::new`] or one of the free
/// constructors ([`text`], [`bold`], [`italic`], [`mono`]). Each method appends
/// a run in the requested style, so styles can be freely interleaved:
///
/// ```
/// use textris_pdf::build::Text;
/// let t = Text::new()
///     .normal("The strike of a ")
///     .bold("smasher")
///     .normal(".");
/// # let _ = t;
/// ```
#[derive(Debug, Clone, Default)]
pub struct Text {
    runs: Vec<Inline>,
}

impl Text {
    /// An empty fragment.
    pub fn new() -> Self {
        Self::default()
    }

    fn push_run(
        mut self,
        text: impl Into<String>,
        bold: bool,
        italic: bool,
        mono: bool,
        color: Option<InlineColor>,
    ) -> Self {
        self.runs.push(Inline {
            text: text.into(),
            bold,
            italic,
            mono,
            color,
            section_ref: None,
            fill_in: None,
        });
        self
    }

    /// Append a run in the regular (upright, non-bold) style.
    pub fn normal(self, text: impl Into<String>) -> Self {
        self.push_run(text, false, false, false, None)
    }

    /// Append a bold run.
    pub fn bold(self, text: impl Into<String>) -> Self {
        self.push_run(text, true, false, false, None)
    }

    /// Append an italic run.
    pub fn italic(self, text: impl Into<String>) -> Self {
        self.push_run(text, false, true, false, None)
    }

    /// Append a bold *and* italic run.
    pub fn bold_italic(self, text: impl Into<String>) -> Self {
        self.push_run(text, true, true, false, None)
    }

    /// Append a monospace run (rendered like inline code).
    pub fn mono(self, text: impl Into<String>) -> Self {
        self.push_run(text, false, false, true, None)
    }

    /// Append a run in a specific color.
    pub fn colored(self, text: impl Into<String>, color: rgb::Color) -> Self {
        self.push_run(text, false, false, false, Some(InlineColor::Rgb(color)))
    }

    /// Append a muted (secondary) run. The concrete color is the document
    /// theme's [`Palette::muted`](crate::theme::Palette::muted), resolved when
    /// the document is laid out, so it follows a custom theme automatically.
    pub fn muted(self, text: impl Into<String>) -> Self {
        self.push_run(text, false, false, false, Some(InlineColor::Muted))
    }

    /// Append an inline fill-in line: a horizontal rule `length` points long,
    /// drawn along the text baseline, so it flows with the surrounding words as
    /// a blank to be written on. For example, `text("My name is ")` followed by
    /// `.fill_in(120.0)` renders `My name is ____________`. See also
    /// [`fill_in`] for the table-cell form.
    pub fn fill_in(mut self, length: f32) -> Self {
        self.runs.push(Inline {
            fill_in: Some(length),
            ..Inline::new("")
        });
        self
    }

    /// Append a hard line break: the following text starts on a new line
    /// without ending the paragraph. Equivalent to a `'\n'` in the text, which
    /// is honored anywhere text is laid out (paragraphs, cells, lists).
    pub fn line_break(self) -> Self {
        self.normal("\n")
    }

    /// Append a placeholder run that resolves to the number of the section
    /// labeled `label` (see [`Textris::anchor`](crate::build::Textris::anchor))
    /// when the document is built. An unknown label renders as `??`.
    pub fn section_ref(mut self, label: impl Into<String>) -> Self {
        self.runs.push(Inline {
            section_ref: Some(label.into()),
            ..Inline::new("??")
        });
        self
    }
}

/// A single regular-weight fragment.
pub fn text(t: impl Into<String>) -> Text {
    Text::new().normal(t)
}

/// A single bold fragment. Continue chaining for mixed styles.
pub fn bold(t: impl Into<String>) -> Text {
    Text::new().bold(t)
}

/// A single italic fragment.
pub fn italic(t: impl Into<String>) -> Text {
    Text::new().italic(t)
}

/// A single monospace fragment.
pub fn mono(t: impl Into<String>) -> Text {
    Text::new().mono(t)
}

/// A single muted (secondary-color) fragment.
pub fn muted(t: impl Into<String>) -> Text {
    Text::new().muted(t)
}

/// A fragment that resolves to the number of the section labeled `label`
/// (see [`Textris::anchor`](crate::build::Textris::anchor)).
pub fn section_ref(label: impl Into<String>) -> Text {
    Text::new().section_ref(label)
}

/// Anything that can be turned into a run of styled [`Inline`]s.
///
/// Implemented for string types (a single regular run), for [`Text`] (its runs),
/// and for pre-built [`Inline`] values, so builder methods accept both the easy
/// `"plain string"` form and rich [`Text`] without overloads.
pub trait IntoText {
    fn into_inlines(self) -> Vec<Inline>;
}

impl IntoText for Text {
    fn into_inlines(self) -> Vec<Inline> {
        normalize(self.runs)
    }
}

impl IntoText for &str {
    fn into_inlines(self) -> Vec<Inline> {
        normalize(vec![Inline::new(self)])
    }
}

impl IntoText for String {
    fn into_inlines(self) -> Vec<Inline> {
        normalize(vec![Inline::new(self)])
    }
}

impl IntoText for Inline {
    fn into_inlines(self) -> Vec<Inline> {
        normalize(vec![self])
    }
}

impl IntoText for Vec<Inline> {
    fn into_inlines(self) -> Vec<Inline> {
        normalize(self)
    }
}

/// Anything that can be turned into a table [`Cell`].
///
/// Implemented for every [`IntoText`] type (a text cell) and for [`Cell`]
/// itself, so table rows accept plain strings, rich [`Text`], and the special
/// cells from [`cell`], [`fill_in`], [`blank`] and [`spacer`]. Because array
/// literals must be homogeneous, wrap every cell of a mixed row in a helper,
/// e.g. `[cell("Date"), fill_in()]`.
pub trait IntoCell {
    fn into_cell(self) -> Cell;
}

impl IntoCell for Cell {
    fn into_cell(self) -> Cell {
        self
    }
}

macro_rules! impl_into_cell_via_text {
    ($($ty:ty),+) => {$(
        impl IntoCell for $ty {
            fn into_cell(self) -> Cell {
                Cell::Text(self.into_inlines())
            }
        }
    )+};
}

impl_into_cell_via_text!(Text, &str, String, Inline, Vec<Inline>);

/// A text table cell. Useful to make a mixed row's element type uniform:
/// `[cell("Date"), fill_in()]`.
pub fn cell(t: impl IntoText) -> Cell {
    Cell::Text(t.into_inlines())
}

/// An empty table cell with a fill-in line along its bottom (a blank form
/// field).
pub fn fill_in() -> Cell {
    Cell::FillIn
}

/// A deliberately empty table cell: nothing is drawn.
pub fn blank() -> Cell {
    Cell::Blank
}

/// An empty table cell that forces its row to be at least `height` points
/// tall, e.g. a roomy signature field. See also
/// [`Textris::spacer`](crate::build::Textris::spacer) for vertical space
/// between blocks.
pub fn spacer(height: f32) -> Cell {
    Cell::Spacer(height)
}

/// Drop empty runs and merge adjacent runs that share styling, so the layout
/// engine sees clean, minimal input. Also used by the Markdown dialect parser
/// so parsed inlines match what the builder would produce.
pub(crate) fn normalize(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for run in inlines {
        // A fill-in run carries no text but must survive; every other empty
        // run is noise.
        if run.text.is_empty() && run.fill_in.is_none() {
            continue;
        }
        match out.last_mut() {
            Some(last)
                if last.bold == run.bold
                    && last.italic == run.italic
                    && last.mono == run.mono
                    && last.color == run.color
                    // Section references are placeholders whose text is
                    // replaced at resolution; never merge them.
                    && last.section_ref.is_none()
                    && run.section_ref.is_none()
                    // Fill-in lines are atomic markers, not text; never merge.
                    && last.fill_in.is_none()
                    && run.fill_in.is_none() =>
            {
                last.text.push_str(&run.text);
            }
            _ => out.push(run),
        }
    }
    out
}

impl From<Text> for SectionContent {
    fn from(t: Text) -> Self {
        Self::Spans(t.into_inlines())
    }
}

impl SectionContent {
    /// Create a page counter. The closure receives `(page, total)` and returns
    /// a [`Text`] value built with the same helpers as body text (`text()`,
    /// `muted()`, `mono()`, etc.).
    pub fn page_counter(f: impl Fn(usize, usize) -> Text + Send + Sync + 'static) -> Self {
        Self::PageCounter(std::sync::Arc::new(move |p, t| f(p, t).into_inlines()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_builder_interleaves_styles() {
        let runs = Text::new()
            .normal("a ")
            .bold("b")
            .italic("c")
            .into_inlines();
        assert_eq!(runs.len(), 3);
        assert!(runs[1].bold);
        assert!(runs[2].italic);
    }

    #[test]
    fn into_text_normalizes_and_merges() {
        // Two adjacent regular runs collapse into one; empty runs vanish.
        let runs = Text::new()
            .normal("a")
            .normal("")
            .normal("b")
            .into_inlines();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "ab");
    }
}

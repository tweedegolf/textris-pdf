//! Rich text for the builder API: the [`Text`] fragment builder and the
//! [`IntoText`] conversion trait that lets every builder method accept plain
//! strings and styled fragments alike.

use krilla::color::rgb;

use crate::{
    model::{Inline, SectionContent},
    theme::Palette,
};

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
        color: Option<rgb::Color>,
    ) -> Self {
        self.runs.push(Inline {
            text: text.into(),
            bold,
            italic,
            mono,
            color,
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
        self.push_run(text, false, false, false, Some(color))
    }

    /// Append a muted (gray) run, using the default palette's muted color. To
    /// match a custom [`Theme`](crate::theme::Theme)'s muted color, pass it
    /// explicitly via [`colored`](Self::colored).
    pub fn muted(self, text: impl Into<String>) -> Self {
        self.colored(text, Palette::default().muted)
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

/// A single muted (gray) fragment.
pub fn muted(t: impl Into<String>) -> Text {
    Text::new().muted(t)
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

/// Drop empty runs and merge adjacent runs that share styling, so the layout
/// engine sees clean, minimal input.
fn normalize(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for run in inlines {
        if run.text.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last)
                if last.bold == run.bold
                    && last.italic == run.italic
                    && last.mono == run.mono
                    && last.color == run.color =>
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

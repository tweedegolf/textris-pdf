//! The Markdown dialect parser, gated behind the `markdown-parser` cargo
//! feature: the inverse of the [`to_markdown`](super::to_markdown) exporter.
//!
//! [`parse_markdown`] turns a dialect string into document [`Block`]s, so a
//! host application can author documents as (templated) Markdown text instead
//! of builder calls. It produces **body blocks only**; to author document
//! chrome (title, language, headers, footers) in the same file, put it in a
//! [front-matter block](#front-matter) and use
//! [`Textris::push_markdown`](crate::build::Textris::push_markdown), which
//! applies the chrome to the document. The [`Theme`](crate::theme::Theme) and
//! section-number resolution stay on the builder API.
//!
//! The parser is **strict**: unknown attribute keys, malformed tables, unknown
//! directives, unterminated emphasis and unsupported constructs are errors
//! carrying a 1-based source line number, not best-effort text. Templates are
//! developer-authored and CI-rendered, so failing loudly beats guessing.
//!
//! # The dialect
//!
//! GitHub-flavored Markdown, restricted to what the [`Block`]/[`Inline`] model
//! can express, plus an optional front-matter block and two block extensions:
//! *attribute lines* and *directives*. The exporter's output is valid input.
//!
//! ## Front matter
//!
//! When the very first line is `+++`, everything up to the next `+++` line is
//! a front-matter block of `key = value` lines that sets document chrome. It
//! is applied only by
//! [`Textris::push_markdown`](crate::build::Textris::push_markdown) (which has
//! a document to set it on); [`parse_markdown`] rejects a source that carries
//! one, since it returns blocks only.
//!
//! ```markdown
//! +++
//! title = "The Mantis Shrimp: A Field Guide"
//! language = "en"
//! header_right = "Stomatopoda - Field Guide"
//! footer_left = "Revision: `3`"
//! footer_right = "Page {page} of {total}"
//! +++
//!
//! # The Mantis Shrimp
//! ```
//!
//! Recognized keys: `title`, `language`, and the six chrome slots
//! `header_left` / `header_center` / `header_right` and `footer_left` /
//! `footer_center` / `footer_right`. Each value is a double-quoted string.
//! `title` and `language` are taken as plain text; the chrome slots are parsed
//! as [inline](#inlines) content (so `` `3` `` is a mono run). A slot whose
//! value contains the placeholders `{page}` and/or `{total}` becomes a page
//! counter, evaluated per page. Unknown keys are an error. Inline colors
//! (e.g. muted text) have no dialect syntax, as elsewhere.
//!
//! ## Blocks
//!
//! Blocks are separated by blank lines. Lists do not nest; a table row is one
//! source line.
//!
//! | Syntax | Block |
//! | --- | --- |
//! | `# H1` … `###### H6` | [`Block::Heading`] (numbered per [`ParseOptions::numbered_heading_levels`], overridable by attribute) |
//! | flowing lines | [`Block::Paragraph`] |
//! | `> quoted lines` | [`Block::Box`] (default [`BoxStyle::callout`]) |
//! | GFM pipe table | [`Block::Table`] |
//! | `- item` | [`Block::BulletList`] |
//! | `- [ ]` / `- [x]` | [`Block::TaskList`] |
//! | `1. item` / `a. item` | [`Block::OrderedList`] (`Decimal` / `LowerAlpha`; a lettered marker is one letter) |
//! | `@pagebreak` | [`Block::PageBreak`] |
//! | `@spacer(20)`, `@spacer(1.5em)` | [`Block::Spacer`] (points; `em` converts via [`em`]) |
//!
//! A table whose second line is a delimiter row (`| --- | ---: |`) has a
//! header row and defaults to [`TableStyle::data`]; per-column alignment comes
//! from the delimiter row. A table written as rows only (no header, no
//! delimiter row) defaults to [`TableStyle::label`]. A cell consisting solely
//! of underscores is a [`Cell::FillIn`]; an empty cell is [`Cell::Blank`].
//!
//! ## Attribute lines
//!
//! A line of the form `{ key = value, key = value, flag }` on its own line
//! binds to the **next** block (an attribute line followed by a blank line is
//! an error). Values are TOML-like scalars: bare words, quoted strings,
//! numbers and booleans; a bare `flag` key means `true`. Dimension values are
//! points by default and accept `pt`/`em` suffixes (`"3.5em"`, `"120pt"`).
//!
//! Table attributes, mapped onto [`TableStyle`]:
//!
//! ```markdown
//! { widths = "auto 4 4 3 6", striped = false, row-height = "3.5em" }
//! |   | name | initials | postcode | town |
//! | - | ---- | -------- | -------- | ---- |
//! | 1 | Jansen | A.B. | 1234 AB | Leiden |
//! ```
//!
//! - `style = "data"` or `"label"` selects [`TableStyle::data`] /
//!   [`TableStyle::label`] explicitly.
//! - `widths` is a space-separated list per column: `auto`, a bare integer
//!   ([`ColumnWidth::Fraction`]), or a point value like `120pt`
//!   ([`ColumnWidth::Absolute`]; `em` also accepted).
//! - `striped`, `row-height`, `font-size` and `flush-first` override the
//!   corresponding [`TableStyle`] fields.
//!
//! Heading attributes: `{ numbered = false }` (or `true`) overrides
//! [`ParseOptions::numbered_heading_levels`]; `{ label = "vision" }` sets the
//! anchor that `[#vision]` resolves against.
//!
//! Box attributes: `{ background = "highlight" }` before a blockquote names a
//! palette role (`text`, `muted` or `highlight`); only palette roles, no
//! literal colors. The free [`parse_markdown`] resolves roles against
//! [`Palette::default`];
//! [`Textris::push_markdown`](crate::build::Textris::push_markdown) resolves
//! them against the document's theme.
//!
//! ## Inlines
//!
//! | Syntax | Inline |
//! | --- | --- |
//! | `**bold**`, `*italic*`, `***both***` | emphasis flags |
//! | `` `text` `` (fence-length aware) | [`Inline::mono`], content verbatim |
//! | `___` (run of 3+ underscores) | [`Inline::fill_in`], width = run length × [`ParseOptions::fill_in_char_width`] |
//! | `___(120)` | [`Inline::fill_in`] with an explicit width in points |
//! | two trailing spaces, or `\` at end of line | hard break (`'\n'` in the run) |
//! | `<br>` (inside table cells only) | hard break |
//! | `[#label]` | [`Inline::section_ref`] placeholder |
//! | `\` + any ASCII punctuation | that literal character |
//!
//! Not supported, and a parse error where detectable: links, images, HTML
//! other than `<br>` in cells, fenced code blocks, thematic breaks and setext
//! headings, strikethrough, footnotes, nested lists. Emphasis supports the
//! exporter's canonical forms (markers hugging non-space text) and errors on
//! anything unterminated. Stray text that merely *looks* like syntax (a
//! sentence starting `a. `, a lone `_`) must be backslash-escaped; error
//! messages say so.
//!
//! # Escaping
//!
//! Outside a code span, a backslash before **any** ASCII punctuation character
//! yields that literal character; a backslash before anything else is a
//! literal backslash (CommonMark's rule). Untrusted data interpolated into a
//! template must have every ASCII punctuation character escaped so it can
//! never change document structure: run every interpolated value through
//! [`escape`](super::escape) (flow contexts), [`escape_cell`](super::escape_cell)
//! (table cells), [`mono`](super::mono) or [`mono_cell`](super::mono_cell)
//! (verbatim spans), typically wired up as the template engine's auto-escaper
//! and filters. See the [`escape` module docs](super::escape) for the
//! guarantees.
//!
//! Do **not** use an HTML escaper (e.g. askama's default) for dialect
//! templates: HTML escaping corrupts the text and leaves Markdown syntax live.

use std::{fmt, sync::Arc};

use crate::{
    build::normalize,
    model::{
        Block, Cell, Chrome, Document, Inline, ListMarker, SectionContent, Table, TaskItem,
        plain_text,
    },
    theme::{Align, BoxStyle, Color, ColumnWidth, ColumnWidths, Palette, TableStyle, em},
};

/// Options for [`parse_markdown`] and
/// [`Textris::push_markdown`](crate::build::Textris::push_markdown).
#[derive(Debug, Clone)]
pub struct ParseOptions {
    /// Heading levels that get `numbered: true` (e.g. `vec![3]` to number the
    /// `###` sections). An explicit `{ numbered = … }` attribute overrides
    /// this per heading. Empty by default.
    pub numbered_heading_levels: Vec<u8>,
    /// Width one underscore contributes to an inline fill-in (`___`), in
    /// points. Defaults to half the base font size ([`em(0.5)`](em)), so the
    /// exporter's eight-underscore run parses back to roughly the width the
    /// underscores would render at body size.
    pub fill_in_char_width: f32,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            numbered_heading_levels: Vec::new(),
            fill_in_char_width: em(0.5),
        }
    }
}

/// A parse error: a message plus the 1-based source line it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownParseError {
    /// 1-based line number in the parsed source.
    pub line: usize,
    /// What went wrong, e.g. ``unknown attribute key `stripd` for a table``.
    pub message: String,
}

impl fmt::Display for MarkdownParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for MarkdownParseError {}

type Result<T> = std::result::Result<T, MarkdownParseError>;

fn err<T>(line: usize, message: impl Into<String>) -> Result<T> {
    Err(MarkdownParseError {
        line,
        message: message.into(),
    })
}

/// Parse a dialect string into body blocks.
///
/// Box `background` palette roles resolve against [`Palette::default`]; use
/// [`Textris::push_markdown`](crate::build::Textris::push_markdown) to resolve
/// them against a document's own theme.
///
/// A source that opens with a [front-matter block](#front-matter) is rejected:
/// front matter sets document chrome and there is nowhere to put it in a
/// `Vec<Block>`. Use `push_markdown` for such documents.
pub fn parse_markdown(source: &str, options: &ParseOptions) -> Result<Vec<Block>> {
    let (front_matter, blocks) = parse_document(source, options, &Palette::default())?;
    if front_matter.is_some() {
        return err(
            1,
            "front matter (`+++`) sets document chrome and is applied only by \
             `Textris::push_markdown`; `parse_markdown` returns body blocks only",
        );
    }
    Ok(blocks)
}

/// Document chrome parsed from a front-matter block: any field left `None` (or
/// slot left empty) was not set and is left untouched when [applied](Self::apply).
#[derive(Debug, Default)]
pub(crate) struct FrontMatter {
    title: Option<String>,
    language: Option<String>,
    header: Chrome,
    footer: Chrome,
}

impl FrontMatter {
    /// Apply the parsed chrome to `document`, overriding only the fields and
    /// slots the front matter actually set.
    pub(crate) fn apply(self, document: &mut Document) {
        if self.title.is_some() {
            document.title = self.title;
        }
        if self.language.is_some() {
            document.language = self.language;
        }
        set_slot(&mut document.header.left, self.header.left);
        set_slot(&mut document.header.center, self.header.center);
        set_slot(&mut document.header.right, self.header.right);
        set_slot(&mut document.footer.left, self.footer.left);
        set_slot(&mut document.footer.center, self.footer.center);
        set_slot(&mut document.footer.right, self.footer.right);
    }
}

/// Overwrite `dst` only when the front matter provided a value for the slot.
fn set_slot(dst: &mut Option<SectionContent>, src: Option<SectionContent>) {
    if src.is_some() {
        *dst = src;
    }
}

/// Parse a dialect string into optional front matter plus body blocks, using
/// `palette` to resolve box `background` roles. The builder's `push_markdown`
/// passes the document theme's palette and applies the front matter; the free
/// [`parse_markdown`] passes the default palette and rejects front matter.
pub(crate) fn parse_document(
    source: &str,
    options: &ParseOptions,
    palette: &Palette,
) -> Result<(Option<FrontMatter>, Vec<Block>)> {
    let lines: Vec<Line> = source
        .lines()
        .enumerate()
        .map(|(index, text)| Line {
            number: index + 1,
            text,
        })
        .collect();
    let parser = Parser { options, palette };
    let (front_matter, body_start) = parser.front_matter(&lines)?;
    let blocks = parser.blocks(&lines[body_start..])?;
    Ok((front_matter, blocks))
}

/// One source line, carrying its 1-based number for error reporting. Quote
/// content is re-sliced into new `Line`s with the original numbers.
#[derive(Debug, Clone, Copy)]
struct Line<'a> {
    number: usize,
    text: &'a str,
}

/// What a line looks like, decided by its first characters. Escaped leading
/// characters (`\#`, `\-`, …) fall through to `Text`, which is how escaped
/// data stays inert.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind<'a> {
    Blank,
    /// `{ key = value, … }`, binding to the next block.
    Attribute,
    /// `@pagebreak` / `@spacer(…)`.
    Directive,
    /// `#` … `######` headings.
    Heading,
    /// A `>` blockquote line.
    Quote,
    /// A `|`-delimited table row.
    TableRow,
    /// `- item`, carrying the text after the marker.
    Bullet(&'a str),
    /// `- [ ]` / `- [x]` item: checked flag and the text after the checkbox.
    Task(bool, &'a str),
    /// `1. item` / `a. item`: marker style and the text after the marker.
    Ordered(ListMarker, &'a str),
    /// Anything else: paragraph text or a list-item continuation.
    Text,
}

fn classify(text: &str) -> Kind<'_> {
    // Only the start is trimmed: a list item's `rest` must keep its trailing
    // spaces so a hard-break marker survives.
    let t = text.trim_start();
    if t.is_empty() {
        return Kind::Blank;
    }
    if t.starts_with('{') {
        return Kind::Attribute;
    }
    if t.starts_with('@') {
        return Kind::Directive;
    }
    if t.starts_with('#') {
        return Kind::Heading;
    }
    if t.starts_with('>') {
        return Kind::Quote;
    }
    if t.starts_with('|') {
        return Kind::TableRow;
    }
    if let Some(rest) = t.strip_prefix("- ") {
        for (marker, checked) in [("[ ]", false), ("[x]", true), ("[X]", true)] {
            if let Some(after) = rest.strip_prefix(marker)
                && (after.is_empty() || after.starts_with(' '))
            {
                return Kind::Task(checked, after.trim_start());
            }
        }
        return Kind::Bullet(rest);
    }
    if let Some(kind) = ordered_prefix(t) {
        return kind;
    }
    Kind::Text
}

/// Match an ordered-list marker: a run of digits (`1.`, `2.`, … `10.`) or a
/// *single* lowercase letter (`a.`, `b.`) followed by `. `. Lettered markers
/// are one letter only, so a wrapped prose line like `tail. The rest…` is not
/// mistaken for a list; prose that genuinely starts `a. ` must escape the dot
/// (`a\. `).
fn ordered_prefix(t: &str) -> Option<Kind<'_>> {
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && let Some(rest) = t[digits..].strip_prefix(". ")
    {
        return Some(Kind::Ordered(ListMarker::Decimal, rest.trim_start()));
    }
    let mut chars = t.chars();
    if chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && let Some(rest) = chars.as_str().strip_prefix(". ")
    {
        return Some(Kind::Ordered(ListMarker::LowerAlpha, rest.trim_start()));
    }
    None
}

/// The inline-parsing context: table cells additionally understand `<br>` and
/// decode `\|` / `\\` inside code spans (see [`super::mono_cell`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineContext {
    Flow,
    Cell,
}

/// A parsed `{ … }` attribute line, consumed key by key ([`Attrs::take`]) by
/// the block it binds to; leftovers are unknown-key errors ([`Attrs::finish`]).
struct Attrs {
    line: usize,
    entries: Vec<(String, AttrValue)>,
}

#[derive(Debug, Clone, PartialEq)]
enum AttrValue {
    Bool(bool),
    Number(f64),
    Str(String),
}

impl Attrs {
    fn empty(line: usize) -> Self {
        Self {
            line,
            entries: Vec::new(),
        }
    }

    fn take(&mut self, key: &str) -> Option<AttrValue> {
        self.entries
            .iter()
            .position(|(k, _)| k == key)
            .map(|index| self.entries.remove(index).1)
    }

    /// Error on any key no block consumed.
    fn finish(self, context: &str) -> Result<()> {
        match self.entries.first() {
            Some((key, _)) => err(
                self.line,
                format!("unknown attribute key `{key}` for {context}"),
            ),
            None => Ok(()),
        }
    }

    fn bool(&mut self, key: &str) -> Result<Option<bool>> {
        match self.take(key) {
            None => Ok(None),
            Some(AttrValue::Bool(b)) => Ok(Some(b)),
            Some(_) => err(
                self.line,
                format!("attribute `{key}` expects `true` or `false`"),
            ),
        }
    }

    fn string(&mut self, key: &str) -> Result<Option<String>> {
        match self.take(key) {
            None => Ok(None),
            Some(AttrValue::Str(s)) => Ok(Some(s)),
            Some(_) => err(self.line, format!("attribute `{key}` expects a string")),
        }
    }

    /// A dimension attribute: a bare number is points; a string may carry a
    /// `pt` or `em` suffix.
    fn dimension(&mut self, key: &str) -> Result<Option<f32>> {
        let invalid = |line| {
            err(
                line,
                format!(
                    "attribute `{key}` expects a positive dimension in points or `em`, e.g. `\"3.5em\"`"
                ),
            )
        };
        match self.take(key) {
            None => Ok(None),
            Some(AttrValue::Number(n)) => {
                let n = n as f32;
                if n.is_finite() && n > 0.0 {
                    Ok(Some(n))
                } else {
                    invalid(self.line)
                }
            }
            Some(AttrValue::Str(s)) => match dimension(&s) {
                Some(v) => Ok(Some(v)),
                None => invalid(self.line),
            },
            Some(_) => invalid(self.line),
        }
    }
}

/// Parse a dimension: `120`, `120pt` or `1.5em` (points, points, em). Returns
/// `None` unless the value is finite and positive.
fn dimension(s: &str) -> Option<f32> {
    let s = s.trim();
    let (number, scale) = if let Some(v) = s.strip_suffix("em") {
        (v, em(1.0))
    } else if let Some(v) = s.strip_suffix("pt") {
        (v, 1.0)
    } else {
        (s, 1.0)
    };
    let value: f32 = number.trim().parse().ok()?;
    (value.is_finite() && value > 0.0).then_some(value * scale)
}

/// Split a line into (content, ends-with-hard-break): a hard break is two or
/// more trailing spaces, or an odd run of trailing backslashes (the last
/// backslash is the break marker; an even run is escaped backslashes).
fn split_hard_break(raw: &str) -> (&str, bool) {
    let trimmed = raw.trim_end();
    if !trimmed.is_empty() && raw[trimmed.len()..].chars().filter(|&c| c == ' ').count() >= 2 {
        return (trimmed, true);
    }
    let backslashes = trimmed.chars().rev().take_while(|&c| c == '\\').count();
    if backslashes % 2 == 1 {
        return (&trimmed[..trimmed.len() - 1], true);
    }
    (trimmed, false)
}

/// Multi-line text under assembly (a paragraph or list item): lines join with
/// `'\n'` after a hard break and a single space otherwise.
struct FlowText {
    line: usize,
    text: String,
    pending_break: bool,
}

impl FlowText {
    fn new(line: usize, first: &str) -> Self {
        let (content, hard) = split_hard_break(first);
        Self {
            line,
            text: content.trim_start().to_string(),
            pending_break: hard,
        }
    }

    fn continue_with(&mut self, raw: &str) {
        let (content, hard) = split_hard_break(raw);
        self.text.push(if self.pending_break { '\n' } else { ' ' });
        self.text.push_str(content.trim_start());
        self.pending_break = hard;
    }

    /// The assembled text, trimmed: a dangling trailing break (or leading one
    /// from an empty first segment) folds away at the block edge.
    fn finish(self) -> (usize, String) {
        (self.line, self.text.trim().to_string())
    }
}

struct Parser<'a> {
    options: &'a ParseOptions,
    palette: &'a Palette,
}

impl Parser<'_> {
    /// If the source opens with a `+++` front-matter block, parse it and return
    /// it with the index of the first body line; otherwise `(None, 0)`.
    fn front_matter(&self, lines: &[Line]) -> Result<(Option<FrontMatter>, usize)> {
        let Some(first) = lines.first() else {
            return Ok((None, 0));
        };
        if first.text.trim() != "+++" {
            return Ok((None, 0));
        }
        let Some(close) = lines[1..]
            .iter()
            .position(|line| line.text.trim() == "+++")
            .map(|offset| offset + 1)
        else {
            return err(
                first.number,
                "unterminated front matter: missing a closing `+++`",
            );
        };

        let mut front_matter = FrontMatter::default();
        for line in &lines[1..close] {
            let text = line.text.trim();
            if text.is_empty() {
                continue;
            }
            let Some((key, raw)) = text.split_once('=') else {
                return err(
                    line.number,
                    "front matter needs `key = value` lines (or a closing `+++`)",
                );
            };
            let value = front_matter_value(raw.trim(), line.number)?;
            self.set_front_matter(&mut front_matter, key.trim(), &value, line.number)?;
        }
        Ok((Some(front_matter), close + 1))
    }

    /// Assign one front-matter `key = value` entry onto `front_matter`.
    fn set_front_matter(
        &self,
        front_matter: &mut FrontMatter,
        key: &str,
        value: &str,
        line: usize,
    ) -> Result<()> {
        match key {
            "title" => front_matter.title = Some(self.plain_value(value, line)?),
            "language" => front_matter.language = Some(self.plain_value(value, line)?),
            "header_left" => front_matter.header.left = Some(self.chrome_value(value, line)?),
            "header_center" => front_matter.header.center = Some(self.chrome_value(value, line)?),
            "header_right" => front_matter.header.right = Some(self.chrome_value(value, line)?),
            "footer_left" => front_matter.footer.left = Some(self.chrome_value(value, line)?),
            "footer_center" => front_matter.footer.center = Some(self.chrome_value(value, line)?),
            "footer_right" => front_matter.footer.right = Some(self.chrome_value(value, line)?),
            _ => {
                return err(
                    line,
                    format!(
                        "unknown front matter key `{key}` (use `title`, `language`, or \
                         `header_left` / `header_center` / `header_right` and their `footer_` forms)"
                    ),
                );
            }
        }
        Ok(())
    }

    /// A plain-text front-matter value (`title`, `language`): inline-parsed for
    /// escape handling, then flattened, since these are plain strings.
    fn plain_value(&self, value: &str, line: usize) -> Result<String> {
        Ok(plain_text(&self.inlines(
            value,
            line,
            InlineContext::Flow,
        )?))
    }

    /// A chrome-slot front-matter value: inline-parsed content, or a page
    /// counter when it carries `{page}` / `{total}` placeholders.
    fn chrome_value(&self, value: &str, line: usize) -> Result<SectionContent> {
        let runs = self.inlines(value, line, InlineContext::Flow)?;
        if runs
            .iter()
            .any(|run| run.text.contains("{page}") || run.text.contains("{total}"))
        {
            return Ok(page_counter(runs));
        }
        // A single unstyled run keeps the plain `SectionContent::Text` shape the
        // builder uses for string chrome; anything richer stays as spans.
        match runs.as_slice() {
            [run]
                if !run.bold
                    && !run.italic
                    && !run.mono
                    && run.section_ref.is_none()
                    && run.fill_in.is_none() =>
            {
                Ok(SectionContent::Text(run.text.clone()))
            }
            _ => Ok(SectionContent::Spans(runs)),
        }
    }

    /// Group lines into blocks: skip blank separators, bind attribute lines to
    /// the block that follows, and require a blank line between blocks.
    fn blocks(&self, lines: &[Line]) -> Result<Vec<Block>> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            if classify(lines[i].text) == Kind::Blank {
                i += 1;
                continue;
            }
            let attrs = if classify(lines[i].text) == Kind::Attribute {
                let parsed = self.attributes(&lines[i])?;
                i += 1;
                match lines.get(i).map(|l| classify(l.text)) {
                    None | Some(Kind::Blank) => {
                        return err(
                            parsed.line,
                            "an attribute line must be immediately followed by the block it applies to",
                        );
                    }
                    Some(Kind::Attribute) => {
                        return err(
                            lines[i].number,
                            "only one attribute line may precede a block",
                        );
                    }
                    Some(_) => {}
                }
                Some(parsed)
            } else {
                None
            };

            let (block, next) = self.block(lines, i, attrs)?;
            out.push(block);
            i = next;
            if let Some(line) = lines.get(i)
                && classify(line.text) != Kind::Blank
            {
                return err(line.number, "blocks must be separated by a blank line");
            }
        }
        Ok(out)
    }

    /// Parse one block starting at `lines[i]`, returning it and the index of
    /// the first line it did not consume.
    fn block(&self, lines: &[Line], i: usize, attrs: Option<Attrs>) -> Result<(Block, usize)> {
        let line = &lines[i];
        match classify(line.text) {
            Kind::Heading => Ok((self.heading(line, attrs)?, i + 1)),
            Kind::Directive => {
                no_attrs(attrs, "a directive")?;
                Ok((directive(line)?, i + 1))
            }
            Kind::Quote => self.quote(lines, i, attrs),
            Kind::TableRow => self.table(lines, i, attrs),
            Kind::Bullet(_) | Kind::Task(..) | Kind::Ordered(..) => {
                no_attrs(attrs, "a list")?;
                self.list(lines, i)
            }
            Kind::Text => {
                no_attrs(attrs, "a paragraph")?;
                self.paragraph(lines, i)
            }
            Kind::Blank | Kind::Attribute => unreachable!("handled by blocks()"),
        }
    }

    /// `# Title` through `###### Title`, with optional `numbered` / `label`
    /// attributes.
    fn heading(&self, line: &Line, attrs: Option<Attrs>) -> Result<Block> {
        let t = line.text.trim();
        let level = t.chars().take_while(|&c| c == '#').count();
        if level > 6 {
            return err(
                line.number,
                format!("heading level {level} is deeper than the maximum of 6"),
            );
        }
        let rest = &t[level..];
        if !rest.is_empty() && !rest.starts_with(' ') {
            return err(line.number, "expected a space between `#` and the heading");
        }
        let text = rest.trim();
        if text.is_empty() {
            return err(line.number, "empty heading");
        }

        let mut numbered = self
            .options
            .numbered_heading_levels
            .contains(&(level as u8));
        let mut label = None;
        if let Some(mut attrs) = attrs {
            if let Some(value) = attrs.bool("numbered")? {
                numbered = value;
            }
            if let Some(value) = attrs.string("label")? {
                label = Some(value);
            }
            attrs.finish("a heading")?;
        }

        Ok(Block::Heading {
            level: level as u8,
            content: self.inlines(text, line.number, InlineContext::Flow)?,
            numbered,
            label,
        })
    }

    /// `> quoted lines` become a box; the stripped content is parsed
    /// recursively with the same rules (and original line numbers).
    fn quote(&self, lines: &[Line], start: usize, attrs: Option<Attrs>) -> Result<(Block, usize)> {
        let mut inner = Vec::new();
        let mut i = start;
        while i < lines.len() {
            let t = lines[i].text.trim_start();
            let Some(stripped) = t.strip_prefix('>') else {
                break;
            };
            inner.push(Line {
                number: lines[i].number,
                text: stripped.strip_prefix(' ').unwrap_or(stripped),
            });
            i += 1;
        }

        let mut style = BoxStyle::callout();
        if let Some(mut attrs) = attrs {
            if let Some(role) = attrs.string("background")? {
                style.background = self.palette_color(&role, attrs.line)?;
            }
            attrs.finish("a box")?;
        }

        Ok((
            Block::Box {
                style,
                content: self.blocks(&inner)?,
            },
            i,
        ))
    }

    fn palette_color(&self, role: &str, line: usize) -> Result<Color> {
        match role {
            "text" => Ok(self.palette.text),
            "muted" => Ok(self.palette.muted),
            "highlight" => Ok(self.palette.highlight),
            _ => err(
                line,
                format!("unknown palette role `{role}` (use `text`, `muted` or `highlight`)"),
            ),
        }
    }

    /// A run of `- ` / `- [ ]` / `1. ` / `a. ` lines of one kind. `Text` lines
    /// continue the previous item (the exporter indents them after a hard
    /// break); mixing item kinds in one run is an error.
    fn list(&self, lines: &[Line], start: usize) -> Result<(Block, usize)> {
        let first = classify(lines[start].text);
        let mut items: Vec<(FlowText, bool)> = Vec::new();
        let mut i = start;
        while i < lines.len() {
            let line = &lines[i];
            let kind = classify(line.text);
            let (rest, checked) = match (first, kind) {
                (Kind::Bullet(_), Kind::Bullet(rest)) => (rest, false),
                (Kind::Task(..), Kind::Task(checked, rest)) => (rest, checked),
                (Kind::Ordered(want, _), Kind::Ordered(got, rest)) => {
                    if want != got {
                        return err(
                            line.number,
                            "cannot mix numbered and lettered items in one list",
                        );
                    }
                    (rest, false)
                }
                (_, Kind::Bullet(_) | Kind::Task(..) | Kind::Ordered(..)) => {
                    return err(
                        line.number,
                        "cannot mix list item kinds in one list; separate them with a blank line",
                    );
                }
                (_, Kind::Text) => {
                    let (item, _) = items.last_mut().expect("list starts with an item");
                    item.continue_with(line.text);
                    i += 1;
                    continue;
                }
                _ => break,
            };
            items.push((FlowText::new(line.number, rest), checked));
            i += 1;
        }

        let mut contents = Vec::with_capacity(items.len());
        let mut checkboxes = Vec::with_capacity(items.len());
        for (item, checked) in items {
            let (line, text) = item.finish();
            if text.is_empty() {
                return err(line, "empty list item");
            }
            contents.push(self.inlines(&text, line, InlineContext::Flow)?);
            checkboxes.push(checked);
        }

        let block = match first {
            Kind::Bullet(_) => Block::BulletList(contents),
            Kind::Task(..) => Block::TaskList(
                contents
                    .into_iter()
                    .zip(checkboxes)
                    .map(|(content, checked)| TaskItem { checked, content })
                    .collect(),
            ),
            Kind::Ordered(marker, _) => Block::OrderedList {
                marker,
                items: contents,
            },
            _ => unreachable!("list() is called on a list item"),
        };
        Ok((block, i))
    }

    /// Consecutive `Text` lines joined into one paragraph, honoring hard
    /// breaks. Lines that look like GFM constructs the model cannot hold are
    /// rejected here.
    fn paragraph(&self, lines: &[Line], start: usize) -> Result<(Block, usize)> {
        let mut flow: Option<FlowText> = None;
        let mut i = start;
        while i < lines.len() && classify(lines[i].text) == Kind::Text {
            let line = &lines[i];
            let t = line.text.trim();
            if t.len() >= 3 && t.chars().all(|c| c == '-') {
                return err(line.number, "thematic breaks (`---`) are not supported");
            }
            if t.len() >= 3 && t.chars().all(|c| c == '=') {
                return err(
                    line.number,
                    "setext headings are not supported: use `#` headings",
                );
            }
            match &mut flow {
                None => flow = Some(FlowText::new(line.number, line.text)),
                Some(flow) => flow.continue_with(line.text),
            }
            i += 1;
        }
        let (line, text) = flow.expect("paragraph starts with a text line").finish();
        Ok((
            Block::Paragraph(self.inlines(&text, line, InlineContext::Flow)?),
            i,
        ))
    }

    /// A run of `|`-delimited rows: an optional header + delimiter pair, then
    /// body rows, all with matching cell counts.
    fn table(&self, lines: &[Line], start: usize, attrs: Option<Attrs>) -> Result<(Block, usize)> {
        let mut raw_rows: Vec<(usize, Vec<String>)> = Vec::new();
        let mut i = start;
        while i < lines.len() && classify(lines[i].text) == Kind::TableRow {
            raw_rows.push((lines[i].number, split_row(&lines[i])?));
            i += 1;
        }

        let delimiter = |cells: &[String]| -> Option<Vec<Align>> {
            cells.iter().map(|c| delimiter_cell(c)).collect()
        };
        if delimiter(&raw_rows[0].1).is_some() {
            return err(raw_rows[0].0, "a table cannot start with a delimiter row");
        }
        let alignment = raw_rows.get(1).and_then(|(_, cells)| delimiter(cells));
        let has_header = alignment.is_some();

        // Column count comes from the first row; every other row must match.
        let columns = raw_rows[0].1.len();
        if let Some(align) = &alignment
            && align.len() != columns
        {
            return err(
                raw_rows[1].0,
                format!(
                    "the delimiter row has {} columns but the header has {columns}",
                    align.len()
                ),
            );
        }
        let body_start = if has_header { 2 } else { 0 };
        for (number, cells) in &raw_rows[body_start.min(raw_rows.len())..] {
            if delimiter(cells).is_some() {
                return err(*number, "unexpected table delimiter row");
            }
            if cells.len() != columns {
                return err(
                    *number,
                    format!(
                        "table row has {} cells but the table has {columns} columns",
                        cells.len()
                    ),
                );
            }
        }

        let headers = if has_header {
            let (number, cells) = &raw_rows[0];
            cells
                .iter()
                .map(|c| self.cell(c, *number))
                .collect::<Result<Vec<_>>>()?
        } else {
            vec![Cell::Blank; columns]
        };
        let mut rows = Vec::new();
        for (number, cells) in &raw_rows[body_start.min(raw_rows.len())..] {
            rows.push(
                cells
                    .iter()
                    .map(|c| self.cell(c, *number))
                    .collect::<Result<Vec<_>>>()?,
            );
        }

        let mut attrs = attrs.unwrap_or_else(|| Attrs::empty(raw_rows[0].0));
        let mut style = match attrs.string("style")? {
            Some(choice) => match choice.as_str() {
                "data" => TableStyle::data(),
                "label" => {
                    if has_header && !headers.iter().all(Cell::is_blank) {
                        return err(raw_rows[0].0, "a label table cannot have a header row");
                    }
                    TableStyle::label()
                }
                other => {
                    return err(
                        attrs.line,
                        format!("unknown table style `{other}` (use `data` or `label`)"),
                    );
                }
            },
            None if has_header => TableStyle::data(),
            None => TableStyle::label(),
        };
        if let Some(align) = alignment
            && align.iter().any(|&a| a != Align::Left)
        {
            style.align = align;
        }
        if let Some(spec) = attrs.string("widths")? {
            style.columns = column_widths(&spec, attrs.line)?;
        }
        if let Some(striped) = attrs.bool("striped")? {
            style.striped = striped;
        }
        if let Some(flush) = attrs.bool("flush-first")? {
            style.flush_first_column = flush;
        }
        if let Some(height) = attrs.dimension("row-height")? {
            style.row_min_height = Some(height);
        }
        if let Some(size) = attrs.dimension("font-size")? {
            style.font_size = Some(size);
        }
        attrs.finish("a table")?;

        Ok((
            Block::Table(Table {
                style,
                headers,
                rows,
            }),
            i,
        ))
    }

    /// One table cell: empty is blank, all underscores is a fill-in, anything
    /// else is inline-parsed text.
    fn cell(&self, raw: &str, line: usize) -> Result<Cell> {
        if raw.is_empty() {
            return Ok(Cell::Blank);
        }
        if raw.chars().all(|c| c == '_') {
            return Ok(Cell::FillIn);
        }
        let inlines = self.inlines(raw, line, InlineContext::Cell)?;
        Ok(if inlines.is_empty() {
            Cell::Blank
        } else {
            Cell::Text(inlines)
        })
    }

    /// Parse a `{ key = value, … }` line into its entries.
    fn attributes(&self, line: &Line) -> Result<Attrs> {
        let t = line.text.trim();
        let Some(inner) = t.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
            return err(
                line.number,
                "an attribute line must be enclosed in `{ … }` (escape a literal `{` as `\\{`)",
            );
        };
        let chars: Vec<char> = inner.chars().collect();
        let mut entries: Vec<(String, AttrValue)> = Vec::new();
        let mut i = 0;
        loop {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
            let key_start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '-' || chars[i] == '_')
            {
                i += 1;
            }
            if key_start == i {
                return err(
                    line.number,
                    format!("unexpected `{}` in attribute line", chars[i]),
                );
            }
            let key: String = chars[key_start..i].iter().collect();
            if entries.iter().any(|(k, _)| *k == key) {
                return err(line.number, format!("duplicate attribute key `{key}`"));
            }
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let value = if i < chars.len() && chars[i] == '=' {
                i += 1;
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                if i >= chars.len() {
                    return err(line.number, format!("missing value after `{key} =`"));
                }
                if chars[i] == '"' {
                    let close = chars[i + 1..]
                        .iter()
                        .position(|&c| c == '"')
                        .map(|offset| i + 1 + offset);
                    let Some(close) = close else {
                        return err(line.number, "unterminated string in attribute line");
                    };
                    let s: String = chars[i + 1..close].iter().collect();
                    i = close + 1;
                    AttrValue::Str(s)
                } else {
                    let value_start = i;
                    while i < chars.len() && chars[i] != ',' {
                        i += 1;
                    }
                    let raw: String = chars[value_start..i].iter().collect::<String>();
                    let raw = raw.trim();
                    if raw.is_empty() {
                        return err(line.number, format!("missing value after `{key} =`"));
                    }
                    if raw.contains(char::is_whitespace) {
                        return err(
                            line.number,
                            format!("expected `,` after the value of `{key}`"),
                        );
                    }
                    match raw {
                        "true" => AttrValue::Bool(true),
                        "false" => AttrValue::Bool(false),
                        _ => match raw.parse::<f64>() {
                            Ok(n) => AttrValue::Number(n),
                            Err(_) => AttrValue::Str(raw.to_string()),
                        },
                    }
                }
            } else {
                // A bare key is a boolean flag.
                AttrValue::Bool(true)
            };
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i < chars.len() {
                if chars[i] != ',' {
                    return err(
                        line.number,
                        format!("expected `,` between attributes, found `{}`", chars[i]),
                    );
                }
                i += 1;
            }
            entries.push((key, value));
        }
        Ok(Attrs {
            line: line.number,
            entries,
        })
    }

    /// Parse inline text into styled runs. Emphasis markers toggle their flag:
    /// an opening marker must be followed by non-space text, a closing one
    /// preceded by it: the exporter's canonical forms.
    fn inlines(&self, text: &str, line: usize, ctx: InlineContext) -> Result<Vec<Inline>> {
        let chars: Vec<char> = text.chars().collect();
        let mut runs: Vec<Inline> = Vec::new();
        let mut buf = String::new();
        let mut bold = false;
        let mut italic = false;
        let mut i = 0;

        fn flush(runs: &mut Vec<Inline>, buf: &mut String, bold: bool, italic: bool) {
            if !buf.is_empty() {
                runs.push(Inline {
                    bold,
                    italic,
                    ..Inline::new(std::mem::take(buf))
                });
            }
        }

        while i < chars.len() {
            match chars[i] {
                '\\' => {
                    if i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() {
                        buf.push(chars[i + 1]);
                        i += 2;
                    } else {
                        buf.push('\\');
                        i += 1;
                    }
                }
                '`' => {
                    let open = run_len(&chars, i, '`');
                    let mut j = i + open;
                    let mut close = None;
                    while j < chars.len() {
                        if chars[j] == '`' {
                            let m = run_len(&chars, j, '`');
                            if m == open {
                                close = Some(j);
                                break;
                            }
                            j += m;
                        } else {
                            j += 1;
                        }
                    }
                    let Some(close) = close else {
                        return err(line, "unterminated code span");
                    };
                    let content: String = chars[i + open..close].iter().collect();
                    let content = strip_code_padding(&content);
                    let content = match ctx {
                        InlineContext::Cell => decode_cell_code(content),
                        InlineContext::Flow => content.to_string(),
                    };
                    flush(&mut runs, &mut buf, bold, italic);
                    runs.push(Inline {
                        mono: true,
                        bold,
                        italic,
                        ..Inline::new(content)
                    });
                    i = close + open;
                }
                '*' => {
                    let n = run_len(&chars, i, '*');
                    if n > 3 {
                        return err(line, "emphasis runs longer than `***` are not supported");
                    }
                    // Text gathered so far belongs to the style before the toggle.
                    flush(&mut runs, &mut buf, bold, italic);
                    let toggle = [(&mut italic, n != 2), (&mut bold, n != 1)];
                    for (flag, toggles) in toggle {
                        if !toggles {
                            continue;
                        }
                        if !*flag {
                            // Opening: the marker must hug the text it styles.
                            if chars.get(i + n).is_none_or(|c| c.is_whitespace()) {
                                return err(
                                    line,
                                    "an opening `*` must be followed by text (escape a literal `*` as `\\*`)",
                                );
                            }
                        } else if i == 0 || chars[i - 1].is_whitespace() {
                            return err(
                                line,
                                "a closing `*` must directly follow text (escape a literal `*` as `\\*`)",
                            );
                        }
                        *flag = !*flag;
                    }
                    i += n;
                }
                '_' => {
                    let n = run_len(&chars, i, '_');
                    if n < 3 {
                        return err(
                            line,
                            "stray `_`: escape it as `\\_`, or use `___` (three or more) for a fill-in line",
                        );
                    }
                    i += n;
                    let width = if chars.get(i) == Some(&'(') {
                        let Some(offset) = chars[i + 1..].iter().position(|&c| c == ')') else {
                            return err(line, "unterminated fill-in width: missing `)`");
                        };
                        let arg: String = chars[i + 1..i + 1 + offset].iter().collect();
                        let Some(width) = dimension(&arg) else {
                            return err(line, format!("invalid fill-in width `{}`", arg.trim()));
                        };
                        i += offset + 2;
                        width
                    } else {
                        n as f32 * self.options.fill_in_char_width
                    };
                    flush(&mut runs, &mut buf, bold, italic);
                    runs.push(Inline {
                        fill_in: Some(width),
                        ..Inline::new("")
                    });
                }
                '[' => {
                    if chars.get(i + 1) == Some(&'#') {
                        let Some(offset) = chars[i + 2..].iter().position(|&c| c == ']') else {
                            return err(line, "unterminated section reference: missing `]`");
                        };
                        let label: String = chars[i + 2..i + 2 + offset].iter().collect();
                        if label.is_empty() {
                            return err(line, "empty section reference label");
                        }
                        flush(&mut runs, &mut buf, bold, italic);
                        runs.push(Inline {
                            bold,
                            italic,
                            section_ref: Some(label),
                            ..Inline::new("??")
                        });
                        i += offset + 3;
                    } else {
                        return err(
                            line,
                            "links and images are not supported: escape `[` as `\\[`",
                        );
                    }
                }
                '<' => {
                    let tag: String = chars[i..].iter().take(5).collect();
                    if ctx == InlineContext::Cell
                        && (tag.starts_with("<br>") || tag.starts_with("<br/>"))
                    {
                        buf.push('\n');
                        i += if tag.starts_with("<br>") { 4 } else { 5 };
                    } else if chars
                        .get(i + 1)
                        .is_some_and(|&c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'))
                    {
                        return err(
                            line,
                            match ctx {
                                InlineContext::Cell => {
                                    "HTML tags other than `<br>` are not supported: escape `<` as `\\<`"
                                }
                                InlineContext::Flow => {
                                    "HTML tags are not supported (`<br>` works in table cells only): escape `<` as `\\<`"
                                }
                            },
                        );
                    } else {
                        buf.push('<');
                        i += 1;
                    }
                }
                '~' => {
                    if chars.get(i + 1) == Some(&'~') {
                        return err(line, "strikethrough is not supported: escape `~` as `\\~`");
                    }
                    buf.push('~');
                    i += 1;
                }
                '!' => {
                    if chars.get(i + 1) == Some(&'[') {
                        return err(line, "images are not supported: escape `!` as `\\!`");
                    }
                    buf.push('!');
                    i += 1;
                }
                c => {
                    buf.push(c);
                    i += 1;
                }
            }
        }
        flush(&mut runs, &mut buf, bold, italic);
        if bold || italic {
            return err(line, "unterminated emphasis: missing closing `*`");
        }
        Ok(normalize(runs))
    }
}

fn no_attrs(attrs: Option<Attrs>, what: &str) -> Result<()> {
    match attrs {
        Some(attrs) => err(
            attrs.line,
            format!("{what} does not take an attribute line"),
        ),
        None => Ok(()),
    }
}

/// Unwrap a front-matter value: the inner text of a double-quoted string, or a
/// bare (unquoted) token as-is. The inner text still carries backslash escapes
/// for the inline parser to resolve.
fn front_matter_value(raw: &str, line: usize) -> Result<String> {
    if let Some(inner) = raw.strip_prefix('"') {
        match inner.strip_suffix('"') {
            Some(inner) => Ok(inner.to_string()),
            None => err(line, "unterminated string in front matter"),
        }
    } else {
        Ok(raw.to_string())
    }
}

/// The emphasis flags a counter placeholder inherits from its surrounding run.
#[derive(Debug, Clone, Copy)]
struct RunStyle {
    bold: bool,
    italic: bool,
    mono: bool,
}

impl RunStyle {
    fn of(run: &Inline) -> Self {
        Self {
            bold: run.bold,
            italic: run.italic,
            mono: run.mono,
        }
    }

    /// A styled run holding `text` (a substituted page or total number).
    fn run(&self, text: String) -> Inline {
        Inline {
            bold: self.bold,
            italic: self.italic,
            mono: self.mono,
            ..Inline::new(text)
        }
    }
}

/// One piece of a page-counter template: fixed content or a placeholder that
/// takes the current page or total, keeping the surrounding run's style.
enum CounterPart {
    Literal(Inline),
    Page(RunStyle),
    Total(RunStyle),
}

/// Build a [`SectionContent::PageCounter`] from inline runs carrying `{page}` /
/// `{total}` placeholders. The template is fixed at parse time; each render
/// substitutes the numbers and merges adjacent same-style runs.
fn page_counter(runs: Vec<Inline>) -> SectionContent {
    let parts = counter_parts(&runs);
    SectionContent::PageCounter(Arc::new(move |page, total| {
        let runs = parts
            .iter()
            .map(|part| match part {
                CounterPart::Literal(run) => run.clone(),
                CounterPart::Page(style) => style.run(page.to_string()),
                CounterPart::Total(style) => style.run(total.to_string()),
            })
            .collect();
        normalize(runs)
    }))
}

/// Split each run's text on `{page}` / `{total}`, turning those markers into
/// placeholder parts and the surrounding text into literal parts of the same
/// style. Runs that are section references or fill-ins are kept verbatim.
fn counter_parts(runs: &[Inline]) -> Vec<CounterPart> {
    let mut parts = Vec::new();
    for run in runs {
        if run.section_ref.is_some() || run.fill_in.is_some() {
            parts.push(CounterPart::Literal(run.clone()));
            continue;
        }
        let style = RunStyle::of(run);
        let mut rest = run.text.as_str();
        while !rest.is_empty() {
            let page = rest.find("{page}").map(|at| (at, "{page}".len(), true));
            let total = rest.find("{total}").map(|at| (at, "{total}".len(), false));
            let next = match (page, total) {
                (Some(p), Some(t)) => Some(if p.0 <= t.0 { p } else { t }),
                (found, None) | (None, found) => found,
            };
            let Some((at, len, is_page)) = next else {
                parts.push(CounterPart::Literal(style.run(rest.to_string())));
                break;
            };
            if at > 0 {
                parts.push(CounterPart::Literal(style.run(rest[..at].to_string())));
            }
            parts.push(if is_page {
                CounterPart::Page(style)
            } else {
                CounterPart::Total(style)
            });
            rest = &rest[at + len..];
        }
    }
    parts
}

/// `@pagebreak` or `@spacer(height)`.
fn directive(line: &Line) -> Result<Block> {
    let t = line.text.trim();
    let body = t.strip_prefix('@').expect("classified as a directive");
    let name_len = body
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .count();
    let (name, rest) = body.split_at(name_len);
    let rest = rest.trim();
    match name {
        "pagebreak" => {
            if !rest.is_empty() {
                return err(line.number, "`@pagebreak` takes no argument");
            }
            Ok(Block::PageBreak)
        }
        "spacer" => {
            let arg = rest
                .strip_prefix('(')
                .and_then(|r| r.strip_suffix(')'))
                .map(str::trim);
            let Some(arg) = arg else {
                return err(
                    line.number,
                    "`@spacer` expects a height, e.g. `@spacer(20)` or `@spacer(1.5em)`",
                );
            };
            let Some(height) = dimension(arg) else {
                return err(line.number, format!("invalid `@spacer` height `{arg}`"));
            };
            Ok(Block::Spacer(height))
        }
        _ => err(
            line.number,
            format!("unknown directive `@{name}` (escape a literal `@` as `\\@`)"),
        ),
    }
}

/// The length of the run of `ch` starting at `chars[i]`.
fn run_len(chars: &[char], i: usize, ch: char) -> usize {
    chars[i..].iter().take_while(|&&c| c == ch).count()
}

/// Undo the one-space padding [`super::escape::code_span`] adds when content
/// starts and ends with a space (CommonMark's strip rule).
fn strip_code_padding(content: &str) -> &str {
    if content.len() >= 2
        && content.starts_with(' ')
        && content.ends_with(' ')
        && !content.trim().is_empty()
    {
        &content[1..content.len() - 1]
    } else {
        content
    }
}

/// Decode the cell encoding of [`super::mono_cell`] inside a cell's code
/// span: `\\` is a backslash, `\|` a pipe; any other backslash is literal.
fn decode_cell_code(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut iter = content.chars().peekable();
    while let Some(c) = iter.next() {
        if c == '\\' {
            match iter.peek() {
                Some(&'\\') => {
                    out.push('\\');
                    iter.next();
                }
                Some(&'|') => {
                    out.push('|');
                    iter.next();
                }
                _ => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Split a `| a | b |` row into trimmed raw cell strings. A `|` directly
/// preceded by a backslash belongs to its cell (`\|`); escape pairs are kept
/// raw for the inline pass to decode.
fn split_row(line: &Line) -> Result<Vec<String>> {
    let t = line.text.trim();
    let chars: Vec<char> = t.chars().collect();
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut closed = false;
    let mut k = 1; // skip the leading `|` that classified the line
    while k < chars.len() {
        match chars[k] {
            '\\' if k + 1 < chars.len() => {
                current.push('\\');
                current.push(chars[k + 1]);
                k += 2;
            }
            '|' => {
                cells.push(current.trim().to_string());
                current = String::new();
                closed = k == chars.len() - 1;
                k += 1;
            }
            c => {
                current.push(c);
                k += 1;
            }
        }
    }
    if !closed {
        return err(line.number, "a table row must end with `|`");
    }
    if cells.is_empty() {
        return err(line.number, "empty table row");
    }
    Ok(cells)
}

/// A delimiter-row cell (`---`, `:---`, `---:`, `:---:`) and the alignment it
/// declares, or `None` if the cell is not delimiter-shaped.
fn delimiter_cell(cell: &str) -> Option<Align> {
    let left = cell.starts_with(':');
    let right = cell.len() > 1 && cell.ends_with(':');
    let dashes = &cell[left as usize..cell.len() - right as usize];
    (!dashes.is_empty() && dashes.chars().all(|c| c == '-')).then_some(match (left, right) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    })
}

/// Parse a `widths` attribute: space-separated `auto`, bare integers
/// (fractions) or `pt`/`em` values (absolute points).
fn column_widths(spec: &str, line: usize) -> Result<ColumnWidths> {
    let mut columns = Vec::new();
    for token in spec.split_whitespace() {
        if token == "auto" {
            columns.push(ColumnWidth::Auto);
        } else if let Ok(fraction) = token.parse::<u32>() {
            if fraction == 0 {
                return err(line, "a column width fraction must be positive");
            }
            columns.push(ColumnWidth::Fraction(fraction));
        } else if token.ends_with("pt") || token.ends_with("em") {
            let Some(points) = dimension(token) else {
                return err(line, format!("invalid column width `{token}`"));
            };
            columns.push(ColumnWidth::Absolute(points));
        } else {
            return err(
                line,
                format!(
                    "invalid column width `{token}` (use `auto`, a fraction like `3`, or `120pt`)"
                ),
            );
        }
    }
    if columns.is_empty() {
        return err(line, "empty `widths` list");
    }
    Ok(ColumnWidths::Custom(columns))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        build::{Textris, bold, cell, fill_in, text},
        markdown::{escape, escape_cell, mono, mono_cell},
        model::plain_text,
        theme::Theme,
    };

    fn parse(source: &str) -> Vec<Block> {
        parse_markdown(source, &ParseOptions::default())
            .unwrap_or_else(|e| panic!("parse failed for {source:?}: {e}"))
    }

    fn parse_err(source: &str) -> MarkdownParseError {
        parse_markdown(source, &ParseOptions::default())
            .expect_err(&format!("expected an error for {source:?}"))
    }

    fn paragraph_runs(block: &Block) -> &[Inline] {
        match block {
            Block::Paragraph(runs) => runs,
            other => panic!("expected a paragraph, got {other:?}"),
        }
    }

    fn table(block: &Block) -> &Table {
        match block {
            Block::Table(table) => table,
            other => panic!("expected a table, got {other:?}"),
        }
    }

    // --- Blocks -------------------------------------------------------------

    #[test]
    fn headings_parse_levels_and_inline_content() {
        let blocks = parse("## Profile\n\n# The *Mantis* Shrimp\n\n###### Deep\n");
        let Block::Heading { level: 2, .. } = &blocks[0] else {
            panic!("{blocks:?}");
        };
        let Block::Heading {
            level: 1, content, ..
        } = &blocks[1]
        else {
            panic!("{blocks:?}");
        };
        assert_eq!(plain_text(content), "The Mantis Shrimp");
        assert!(content[1].italic);
        let Block::Heading { level: 6, .. } = &blocks[2] else {
            panic!("{blocks:?}");
        };
    }

    #[test]
    fn heading_numbering_comes_from_options_and_attributes() {
        let options = ParseOptions {
            numbered_heading_levels: vec![3],
            ..ParseOptions::default()
        };
        let source = "### Auto\n\n{ numbered = false }\n### Opted out\n\n{ numbered = true, label = \"vision\" }\n#### Opted in\n";
        let blocks = parse_markdown(source, &options).unwrap();
        let numbered_and_label = |b: &Block| match b {
            Block::Heading {
                numbered, label, ..
            } => (*numbered, label.clone()),
            other => panic!("expected a heading, got {other:?}"),
        };
        assert_eq!(numbered_and_label(&blocks[0]), (true, None));
        assert_eq!(numbered_and_label(&blocks[1]), (false, None));
        assert_eq!(
            numbered_and_label(&blocks[2]),
            (true, Some("vision".to_string()))
        );
    }

    #[test]
    fn heading_errors() {
        assert_eq!(parse_err("####### Seven\n").line, 1);
        assert!(parse_err("#Title\n").message.contains("space"));
        assert!(parse_err("# \n").message.contains("empty heading"));
    }

    #[test]
    fn paragraph_lines_join_with_soft_and_hard_breaks() {
        let blocks = parse("one\ntwo\n");
        assert_eq!(plain_text(paragraph_runs(&blocks[0])), "one two");

        let blocks = parse("one  \ntwo\n");
        assert_eq!(plain_text(paragraph_runs(&blocks[0])), "one\ntwo");

        let blocks = parse("one\\\ntwo\n");
        assert_eq!(plain_text(paragraph_runs(&blocks[0])), "one\ntwo");

        // An even run of trailing backslashes is escaped backslashes, not a break.
        let blocks = parse("one\\\\\ntwo\n");
        assert_eq!(plain_text(paragraph_runs(&blocks[0])), "one\\ two");
    }

    #[test]
    fn blocks_must_be_separated_by_blank_lines() {
        let error = parse_err("# Title\nbody\n");
        assert_eq!(error.line, 2);
        assert!(error.message.contains("blank line"), "{error}");
    }

    #[test]
    fn quote_becomes_a_callout_box() {
        let blocks = parse("> **Note.**\n>\n> Body text.\n");
        let Block::Box { style, content } = &blocks[0] else {
            panic!("{blocks:?}");
        };
        assert_eq!(*style, BoxStyle::callout());
        assert_eq!(content.len(), 2);
        assert_eq!(plain_text(paragraph_runs(&content[0])), "Note.");
        assert!(paragraph_runs(&content[0])[0].bold);
        assert_eq!(plain_text(paragraph_runs(&content[1])), "Body text.");
    }

    #[test]
    fn box_background_names_a_palette_role() {
        let blocks = parse("{ background = \"highlight\" }\n> Warning.\n");
        let Block::Box { style, .. } = &blocks[0] else {
            panic!("{blocks:?}");
        };
        assert_eq!(style.background, Palette::default().highlight);

        let error = parse_err("{ background = \"pink\" }\n> Warning.\n");
        assert!(
            error.message.contains("unknown palette role `pink`"),
            "{error}"
        );
    }

    #[test]
    fn lists_parse_by_marker_kind() {
        let blocks = parse("- first\n- second\n");
        let Block::BulletList(items) = &blocks[0] else {
            panic!("{blocks:?}");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(plain_text(&items[1]), "second");

        let blocks = parse("- [x] done\n- [ ] todo\n");
        let Block::TaskList(items) = &blocks[0] else {
            panic!("{blocks:?}");
        };
        assert!(items[0].checked);
        assert!(!items[1].checked);
        assert_eq!(plain_text(&items[1].content), "todo");

        let blocks = parse("1. first\n2. second\n");
        let Block::OrderedList { marker, items } = &blocks[0] else {
            panic!("{blocks:?}");
        };
        assert_eq!(*marker, ListMarker::Decimal);
        assert_eq!(items.len(), 2);

        let blocks = parse("a. first\nb. second\n");
        let Block::OrderedList { marker, .. } = &blocks[0] else {
            panic!("{blocks:?}");
        };
        assert_eq!(*marker, ListMarker::LowerAlpha);
    }

    #[test]
    fn list_items_continue_after_a_hard_break() {
        // The exporter continues a broken item on an indented line.
        let blocks = parse("- one  \n  two\n- three\n");
        let Block::BulletList(items) = &blocks[0] else {
            panic!("{blocks:?}");
        };
        assert_eq!(plain_text(&items[0]), "one\ntwo");
        assert_eq!(plain_text(&items[1]), "three");
    }

    #[test]
    fn a_bullet_item_may_start_with_a_section_reference() {
        let blocks = parse("- [#vision] and more\n");
        let Block::BulletList(items) = &blocks[0] else {
            panic!("{blocks:?}");
        };
        assert_eq!(items[0][0].section_ref.as_deref(), Some("vision"));
    }

    #[test]
    fn mixed_list_kinds_are_an_error() {
        let error = parse_err("- bullet\n1. numbered\n");
        assert_eq!(error.line, 2);
        assert!(error.message.contains("mix"), "{error}");
        assert!(parse_err("1. one\nb. two\n").message.contains("lettered"));
    }

    #[test]
    fn directives_parse_and_reject_garbage() {
        let blocks = parse("@pagebreak\n\n@spacer(20)\n\n@spacer(2em)\n");
        assert!(matches!(blocks[0], Block::PageBreak));
        assert!(matches!(blocks[1], Block::Spacer(h) if h == 20.0));
        assert!(matches!(blocks[2], Block::Spacer(h) if h == 18.0));

        assert!(
            parse_err("@pagebreak now\n")
                .message
                .contains("no argument")
        );
        assert!(parse_err("@spacer\n").message.contains("expects a height"));
        assert!(parse_err("@spacer(x)\n").message.contains("invalid"));
        assert!(
            parse_err("@foo\n")
                .message
                .contains("unknown directive `@foo`")
        );
    }

    // --- Tables -------------------------------------------------------------

    #[test]
    fn header_table_defaults_to_data_style_with_alignment() {
        let blocks = parse("| a | b | c |\n| :-: | ---: | --- |\n| 1 | 2 | 3 |\n");
        let t = table(&blocks[0]);
        assert!(t.style.header);
        assert!(t.style.striped);
        assert_eq!(
            t.style.align,
            vec![Align::Center, Align::Right, Align::Left]
        );
        assert_eq!(plain_text(t.headers[0].inlines()), "a");
        assert_eq!(plain_text(t.rows[0][2].inlines()), "3");
    }

    #[test]
    fn rows_only_table_defaults_to_label_style() {
        let blocks = parse("| Observer | Costa |\n| Date | ________ |\n|  | x |\n");
        let t = table(&blocks[0]);
        assert_eq!(t.style, TableStyle::label());
        assert_eq!(t.headers, vec![Cell::Blank, Cell::Blank]);
        assert!(matches!(t.rows[1][1], Cell::FillIn));
        assert!(matches!(t.rows[2][0], Cell::Blank));
    }

    #[test]
    fn cells_support_br_escaped_pipes_and_mono() {
        let blocks = parse("| a<br>b | x \\| y | `p\\|q` |\n");
        let t = table(&blocks[0]);
        assert_eq!(plain_text(t.rows[0][0].inlines()), "a\nb");
        assert_eq!(plain_text(t.rows[0][1].inlines()), "x | y");
        let mono_run = &t.rows[0][2].inlines()[0];
        assert!(mono_run.mono);
        assert_eq!(mono_run.text, "p|q");
    }

    #[test]
    fn table_attributes_map_onto_the_style() {
        let source = "{ widths = \"auto 4 120pt\", striped = false, row-height = \"3.5em\", font-size = 8, flush-first }\n| a | b | c |\n| - | - | - |\n| 1 | 2 | 3 |\n";
        let blocks = parse(source);
        let style = &table(&blocks[0]).style;
        assert_eq!(
            style.columns,
            ColumnWidths::Custom(vec![
                ColumnWidth::Auto,
                ColumnWidth::Fraction(4),
                ColumnWidth::Absolute(120.0),
            ])
        );
        assert!(!style.striped);
        assert!(style.flush_first_column);
        assert_eq!(style.row_min_height, Some(31.5));
        assert_eq!(style.font_size, Some(8.0));
    }

    #[test]
    fn explicit_table_styles_are_honored() {
        let blocks = parse("{ style = \"data\" }\n| 1 | 2 |\n");
        assert!(table(&blocks[0]).style.striped, "data style requested");

        let blocks = parse("{ style = \"label\" }\n|  |  |\n| - | - |\n| Date | x |\n");
        assert_eq!(table(&blocks[0]).style.columns, ColumnWidths::Labels);

        let error = parse_err("{ style = \"label\" }\n| a | b |\n| - | - |\n");
        assert!(error.message.contains("label table"), "{error}");
    }

    #[test]
    fn malformed_tables_error_with_line_numbers() {
        let error = parse_err("| a | b |\n| - | - |\n| 1 |\n");
        assert_eq!(error.line, 3);
        assert!(
            error.message.contains("2 columns") || error.message.contains("1 cells"),
            "{error}"
        );

        assert_eq!(parse_err("| --- |\n").line, 1);
        assert!(parse_err("| a | b\n").message.contains("end with"));
        let error = parse_err("| a |\n| - |\n| 1 |\n| - |\n");
        assert_eq!(error.line, 4);
        assert!(error.message.contains("delimiter"), "{error}");
    }

    #[test]
    fn unknown_attribute_keys_are_errors_with_line_numbers() {
        let error = parse_err("some text\n\n{ stripd = false }\n| a |\n| - |\n| 1 |\n");
        assert_eq!(error.line, 3);
        assert_eq!(
            error.to_string(),
            "line 3: unknown attribute key `stripd` for a table"
        );

        let error = parse_err("{ striped = false }\nnot a table\n");
        assert!(error.message.contains("paragraph"), "{error}");

        let error = parse_err("{ striped = false }\n\n| a |\n");
        assert_eq!(error.line, 1);
        assert!(error.message.contains("immediately followed"), "{error}");
    }

    #[test]
    fn attribute_line_syntax_errors() {
        assert!(
            parse_err("{ key = }\n# T\n")
                .message
                .contains("missing value")
        );
        assert!(
            parse_err("{ key = \"x }\n# T\n")
                .message
                .contains("unterminated string")
        );
        assert!(
            parse_err("{ a = 1 b = 2 }\n# T\n")
                .message
                .contains("expected `,`")
        );
        assert!(
            parse_err("{ numbered = false, numbered = true }\n# T\n")
                .message
                .contains("duplicate")
        );
        assert!(
            parse_err("{ numbered = \"yes\" }\n# T\n")
                .message
                .contains("`true` or `false`")
        );
    }

    // --- Inlines ------------------------------------------------------------

    #[test]
    fn emphasis_parses_the_canonical_forms() {
        let blocks = parse("**bold** and *italic* and ***both***\n");
        let runs = paragraph_runs(&blocks[0]);
        assert!(runs[0].bold && !runs[0].italic);
        assert_eq!(runs[0].text, "bold");
        assert!(runs[2].italic && !runs[2].bold);
        assert!(runs[4].bold && runs[4].italic);

        // Adjacent runs as the exporter writes them: `**b***i*`.
        let blocks = parse("**b***i*\n");
        let runs = paragraph_runs(&blocks[0]);
        assert_eq!((runs[0].bold, runs[0].text.as_str()), (true, "b"));
        assert_eq!((runs[1].italic, runs[1].text.as_str()), (true, "i"));

        // Emphasis spans a hard break.
        let blocks = parse("**a  \nb**\n");
        let runs = paragraph_runs(&blocks[0]);
        assert_eq!(runs[0].text, "a\nb");
        assert!(runs[0].bold);
    }

    #[test]
    fn emphasis_errors() {
        assert!(
            parse_err("**oops\n")
                .message
                .contains("unterminated emphasis")
        );
        assert!(parse_err("a ** b **\n").message.contains("opening"));
        assert!(parse_err("****x****\n").message.contains("longer than"));
    }

    #[test]
    fn backslash_escapes_any_punctuation() {
        let blocks = parse("\\*not bold\\* \\| \\# \\{ \\@ and \\\\ done\n");
        assert_eq!(
            plain_text(paragraph_runs(&blocks[0])),
            "*not bold* | # { @ and \\ done"
        );
        // A backslash before a non-punctuation character is literal.
        let blocks = parse("a\\z\n");
        assert_eq!(plain_text(paragraph_runs(&blocks[0])), "a\\z");
    }

    #[test]
    fn code_spans_are_verbatim_and_fence_aware() {
        let blocks = parse("`a*b` and ``x`y`` and `` `z` ``\n");
        let runs = paragraph_runs(&blocks[0]);
        let monos: Vec<&str> = runs
            .iter()
            .filter(|r| r.mono)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(monos, vec!["a*b", "x`y", "`z`"]);

        assert!(
            parse_err("`oops\n")
                .message
                .contains("unterminated code span")
        );
    }

    #[test]
    fn fill_ins_parse_run_length_and_explicit_widths() {
        let options = ParseOptions {
            fill_in_char_width: 5.0,
            ..ParseOptions::default()
        };
        let blocks = parse_markdown("name ____ here ___(120)\n", &options).unwrap();
        let runs = match &blocks[0] {
            Block::Paragraph(runs) => runs,
            other => panic!("{other:?}"),
        };
        assert_eq!(runs[1].fill_in, Some(20.0));
        assert_eq!(runs[3].fill_in, Some(120.0));

        assert!(parse_err("stray _ here\n").message.contains("stray `_`"));
        assert!(
            parse_err("___(x)\n")
                .message
                .contains("invalid fill-in width")
        );
        assert!(parse_err("___(12\n").message.contains("missing `)`"));
    }

    #[test]
    fn section_references_parse_to_placeholders() {
        let blocks = parse("see [#vision] here\n");
        let runs = paragraph_runs(&blocks[0]);
        assert_eq!(runs[1].section_ref.as_deref(), Some("vision"));
        assert_eq!(runs[1].text, "??");

        assert!(
            parse_err("see [#]\n")
                .message
                .contains("empty section reference")
        );
        assert!(parse_err("see [#vision\n").message.contains("missing `]`"));
    }

    #[test]
    fn unsupported_constructs_error_where_detectable() {
        assert!(parse_err("a [link](x)\n").message.contains("links"));
        assert!(parse_err("an ![image](x)\n").message.contains("images"));
        assert!(parse_err("a <b>tag</b>\n").message.contains("HTML"));
        assert!(parse_err("~~strike~~\n").message.contains("strikethrough"));
        assert!(parse_err("---\n").message.contains("thematic"));
        assert!(parse_err("===\n").message.contains("setext"));
        // A fenced code block reads as an unclosed inline span.
        assert!(
            parse_err("```rust\n")
                .message
                .contains("unterminated code span")
        );
        // `<br>` outside a cell is HTML like any other.
        assert!(parse_err("a <br> b\n").message.contains("HTML"));
        // But comparisons stay plain text.
        let blocks = parse("a < 5 and b > 1\n");
        assert_eq!(plain_text(paragraph_runs(&blocks[0])), "a < 5 and b > 1");
    }

    #[test]
    fn crlf_sources_parse() {
        let blocks = parse("# Title\r\n\r\nbody\r\n");
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn error_display_carries_the_line_number() {
        let error = MarkdownParseError {
            line: 12,
            message: "unknown attribute key `stripd`".into(),
        };
        assert_eq!(error.to_string(), "line 12: unknown attribute key `stripd`");
    }

    // --- The builder entry point ---------------------------------------------

    #[test]
    fn push_markdown_appends_and_resolves_with_the_document() {
        let options = ParseOptions {
            numbered_heading_levels: vec![3],
            ..ParseOptions::default()
        };
        let mut doc = Textris::new();
        doc.paragraph("Intro.");
        doc.push_markdown(
            "{ label = \"vision\" }\n### Vision\n\nSee section [#vision] for details\n",
            &options,
        )
        .unwrap();
        let built = doc.build();
        let Block::Heading { content, .. } = &built.blocks[1] else {
            panic!("{:?}", built.blocks);
        };
        assert_eq!(plain_text(content), "1. Vision");
        let Block::Paragraph(runs) = &built.blocks[2] else {
            panic!("{:?}", built.blocks);
        };
        assert_eq!(plain_text(runs), "See section 1 for details");
    }

    #[test]
    fn push_markdown_resolves_palette_roles_against_the_theme() {
        let mut theme = Theme::default();
        theme.palette.highlight = Color::new(0x11, 0x22, 0x33);
        let mut doc = Textris::with_theme(theme);
        doc.push_markdown(
            "{ background = \"highlight\" }\n> Boxed.\n",
            &ParseOptions::default(),
        )
        .unwrap();
        let Block::Box { style, .. } = &doc.document().blocks[0] else {
            panic!("{:?}", doc.document().blocks);
        };
        assert_eq!(style.background, Color::new(0x11, 0x22, 0x33));
    }

    // --- Front matter --------------------------------------------------------

    /// Resolve a chrome slot for `(page, total)` into plain text.
    fn slot_text(slot: &Option<SectionContent>, page: usize, total: usize) -> String {
        plain_text(
            &slot
                .as_ref()
                .expect("slot should be set")
                .resolve(page, total),
        )
    }

    #[test]
    fn front_matter_sets_chrome_on_the_document() {
        let source = "\
+++
title = \"Observation form\"
language = \"nl\"
header_right = \"Field guide\"
footer_left = \"Revision: `3`\"
footer_right = \"Page {page} of {total}\"
+++

# Observation form

Body.
";
        let mut doc = Textris::new();
        doc.push_markdown(source, &ParseOptions::default()).unwrap();
        let doc = doc.build();

        assert_eq!(doc.title.as_deref(), Some("Observation form"));
        assert_eq!(doc.language.as_deref(), Some("nl"));
        assert_eq!(slot_text(&doc.header.right, 1, 1), "Field guide");
        // The mono span survives as a styled run.
        let Some(SectionContent::Spans(spans)) = &doc.footer.left else {
            panic!("footer.left should be spans: {:?}", doc.footer.left);
        };
        assert_eq!(plain_text(spans), "Revision: 3");
        assert!(spans.iter().any(|run| run.mono && run.text == "3"));
        // The placeholder value became a live page counter.
        assert_eq!(slot_text(&doc.footer.right, 4, 9), "Page 4 of 9");
        // The body still parses after the front matter.
        assert!(matches!(doc.blocks[0], Block::Heading { .. }));
        assert!(matches!(doc.blocks[1], Block::Paragraph(_)));
    }

    #[test]
    fn front_matter_only_overrides_the_fields_it_sets() {
        let mut doc = Textris::new();
        doc.title("Kept title");
        doc.header_left("kept header");
        doc.push_markdown(
            "+++\nheader_right = \"added\"\n+++\n\n# Body\n",
            &ParseOptions::default(),
        )
        .unwrap();
        let doc = doc.document();
        // Untouched fields survive; the new slot is added.
        assert_eq!(doc.title.as_deref(), Some("Kept title"));
        assert_eq!(slot_text(&doc.header.left, 1, 1), "kept header");
        assert_eq!(slot_text(&doc.header.right, 1, 1), "added");
    }

    #[test]
    fn parse_markdown_rejects_front_matter() {
        let error = parse_err("+++\ntitle = \"x\"\n+++\n\n# Body\n");
        assert_eq!(error.line, 1);
        assert!(error.message.contains("front matter"), "{error}");
        assert!(error.message.contains("push_markdown"), "{error}");
    }

    #[test]
    fn front_matter_errors() {
        let unterminated = |source| {
            let mut doc = Textris::new();
            doc.push_markdown(source, &ParseOptions::default())
                .expect_err("expected an error")
        };
        assert!(
            unterminated("+++\ntitle = \"x\"\n")
                .message
                .contains("unterminated front matter")
        );
        assert_eq!(
            unterminated("+++\ntitle \"x\"\n+++\n\n# B\n").message,
            "front matter needs `key = value` lines (or a closing `+++`)"
        );
        assert!(
            unterminated("+++\nsubtitle = \"x\"\n+++\n\n# B\n")
                .message
                .contains("unknown front matter key `subtitle`")
        );
        assert!(
            unterminated("+++\ntitle = \"x\n+++\n\n# B\n")
                .message
                .contains("unterminated string")
        );
    }

    #[test]
    fn front_matter_counter_keeps_placeholder_styling() {
        let mut doc = Textris::new();
        doc.push_markdown(
            "+++\nfooter_center = \"page **{page}**/**{total}**\"\n+++\n\n# B\n",
            &ParseOptions::default(),
        )
        .unwrap();
        let Some(SectionContent::PageCounter(counter)) = &doc.document().footer.center else {
            panic!("footer.center should be a page counter");
        };
        let runs = counter(2, 5);
        assert_eq!(plain_text(&runs), "page 2/5");
        // The page and total numbers keep the bold styling of their placeholders.
        assert!(runs.iter().any(|run| run.text == "2" && run.bold));
        assert!(runs.iter().any(|run| run.text == "5" && run.bold));
    }

    #[test]
    fn without_front_matter_the_first_line_is_body() {
        // A document that does not open with `+++` is all body.
        let blocks = parse("# Title\n\nBody.\n");
        assert_eq!(blocks.len(), 2);
    }

    // --- Round-tripping the exporter ------------------------------------------

    /// Structural equivalence for the exporter round-trip: kinds, text and
    /// cell shapes match; styles, widths and colors are known-lossy.
    fn assert_blocks_equivalent(expected: &[Block], parsed: &[Block]) {
        assert_eq!(
            expected.len(),
            parsed.len(),
            "\n{expected:#?}\nvs\n{parsed:#?}"
        );
        for (e, p) in expected.iter().zip(parsed) {
            match (e, p) {
                (
                    Block::Heading {
                        level: el,
                        content: ec,
                        ..
                    },
                    Block::Heading {
                        level: pl,
                        content: pc,
                        ..
                    },
                ) => {
                    assert_eq!(el, pl);
                    assert_eq!(plain_text(ec), plain_text(pc));
                }
                (Block::Paragraph(ec), Block::Paragraph(pc)) => {
                    assert_eq!(plain_text(ec), plain_text(pc));
                }
                (Block::BulletList(ei), Block::BulletList(pi)) => {
                    let texts = |items: &[Vec<Inline>]| -> Vec<String> {
                        items.iter().map(|i| plain_text(i)).collect()
                    };
                    assert_eq!(texts(ei), texts(pi));
                }
                (Block::OrderedList { items: ei, .. }, Block::OrderedList { items: pi, .. }) => {
                    // The marker style is lossy: the exporter numbers even
                    // lettered lists with `1.`, `2.`, …
                    assert_eq!(ei.len(), pi.len());
                    for (a, b) in ei.iter().zip(pi) {
                        assert_eq!(plain_text(a), plain_text(b));
                    }
                }
                (Block::TaskList(ei), Block::TaskList(pi)) => {
                    assert_eq!(ei.len(), pi.len());
                    for (a, b) in ei.iter().zip(pi) {
                        assert_eq!(a.checked, b.checked);
                        assert_eq!(plain_text(&a.content), plain_text(&b.content));
                    }
                }
                (Block::Box { content: ec, .. }, Block::Box { content: pc, .. }) => {
                    assert_blocks_equivalent(ec, pc);
                }
                (Block::Table(et), Block::Table(pt)) => {
                    assert_eq!(et.columns(), pt.columns());
                    assert_eq!(et.rows.len(), pt.rows.len());
                    let cells = |t: &Table| -> Vec<Vec<Cell>> {
                        std::iter::once(t.headers.clone())
                            .chain(t.rows.iter().cloned())
                            .collect()
                    };
                    for (erow, prow) in cells(et).iter().zip(cells(pt).iter()) {
                        for (ecell, pcell) in erow.iter().zip(prow) {
                            match ecell {
                                Cell::FillIn => assert!(matches!(pcell, Cell::FillIn)),
                                _ => {
                                    assert_eq!(
                                        plain_text(ecell.inlines()),
                                        plain_text(pcell.inlines())
                                    );
                                }
                            }
                        }
                    }
                    // Alignment survives via the delimiter row.
                    if et.style.align.iter().any(|&a| a != Align::Left) {
                        assert_eq!(et.style.align, pt.style.align);
                    }
                }
                _ => panic!("block kind mismatch:\n{e:#?}\nvs\n{p:#?}"),
            }
        }
    }

    #[test]
    fn exporter_output_parses_back_to_the_same_structure() {
        let mut doc = Textris::new();
        doc.h2("Marine species profile");
        doc.h1("The Mantis Shrimp");
        doc.paragraph(
            text("Mantis shrimp are ")
                .bold("marine crustaceans")
                .normal(": ambush predators (see section ")
                .section_ref("strike")
                .normal(")."),
        );
        doc.h3_numbered("The predatory strike").anchor("strike");
        doc.paragraph(
            text("Speeds reach ")
                .mono("23 m/s")
                .normal(".")
                .line_break()
                .normal("A second line, with a * to escape."),
        );
        doc.bullet_list(["stalked eyes;", "raptorial appendages."]);
        doc.ordered_list_with(ListMarker::LowerAlpha, ["first", "second"]);
        doc.task_list([(true, "Photograph"), (false, "Record temperature")]);
        doc.boxed(|b| {
            b.paragraph(bold("Handle with care."));
            b.paragraph("Never pick one up.");
        });
        let style = TableStyle {
            align: vec![Align::Left, Align::Center, Align::Right],
            ..TableStyle::data()
        };
        doc.table_styled(
            &style,
            ["", "name", "length"],
            [
                [cell("1"), cell("Peacock mantis"), cell("18 cm")],
                [cell("2"), cell("Zebra | mantis"), cell("40 cm")],
            ],
        );
        doc.label_table([
            [cell("Observer"), cell("Costa, R.")],
            [cell("Date"), fill_in()],
        ]);
        doc.page_break();
        doc.spacer(20.0);
        doc.paragraph(text("I, ").fill_in(160.0).normal(", certify this."));

        let mut resolved = doc.document().clone();
        resolved.resolve_sections_unnumbered();
        let expected: Vec<Block> = resolved
            .blocks
            .into_iter()
            .filter(|b| !matches!(b, Block::PageBreak | Block::Spacer(_)))
            .collect();

        let markdown = doc.to_markdown();
        let parsed = parse_markdown(&markdown, &ParseOptions::default())
            .unwrap_or_else(|e| panic!("exporter output failed to parse: {e}\n---\n{markdown}"));
        assert_blocks_equivalent(&expected, &parsed);
    }

    // --- Escape guarantees (property tests) -----------------------------------

    /// A small xorshift PRNG, so the property tests need no dependencies and
    /// stay deterministic.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Random text biased toward punctuation, backslash runs, backticks,
    /// pipes and newlines, per the plan's escape guarantee.
    fn random_text(rng: &mut Rng) -> String {
        const CHARS: &[char] = &[
            '\\', '\\', '`', '`', '*', '*', '_', '_', '|', '|', '[', ']', '<', '>', '~', '{', '}',
            '@', '#', '-', '.', '"', '\'', '(', ')', ':', '!', '+', '=', 'a', 'b', 'Z', '9', ' ',
            ' ', '\n', '\n', '\t', '\u{1}', 'é', '日',
        ];
        let len = rng.below(40);
        (0..len).map(|_| CHARS[rng.below(CHARS.len())]).collect()
    }

    /// The newline normalization the escape functions apply: control
    /// characters drop, whitespace runs containing a newline become one `\n`.
    fn folded(s: &str) -> String {
        let mut out = String::new();
        let mut whitespace = String::new();
        let mut break_pending = false;
        for ch in s.chars() {
            if ch == '\n' {
                break_pending = true;
                whitespace.clear();
            } else if ch.is_control() {
            } else if ch.is_whitespace() {
                if !break_pending {
                    whitespace.push(ch);
                }
            } else {
                if break_pending {
                    out.push('\n');
                    break_pending = false;
                } else {
                    out.push_str(&whitespace);
                }
                whitespace.clear();
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn escaped_text_round_trips_as_one_plain_run() {
        let mut rng = Rng(0x5EED_CAFE);
        let options = ParseOptions::default();
        for _ in 0..500 {
            let s = random_text(&mut rng);
            let source = escape(&s);
            let expected = folded(&s).trim().to_string();
            let blocks = parse_markdown(&source, &options)
                .unwrap_or_else(|e| panic!("escape({s:?}) = {source:?} failed: {e}"));
            if expected.is_empty() {
                let text = match blocks.as_slice() {
                    [] => String::new(),
                    [Block::Paragraph(runs)] => plain_text(runs),
                    other => panic!("escape({s:?}) = {source:?} parsed to {other:?}"),
                };
                assert_eq!(text, expected, "for {s:?} via {source:?}");
                continue;
            }
            let [Block::Paragraph(runs)] = blocks.as_slice() else {
                panic!("escape({s:?}) = {source:?} parsed to {blocks:?}");
            };
            assert_eq!(runs.len(), 1, "for {s:?} via {source:?}: {runs:?}");
            let run = &runs[0];
            assert!(
                !run.bold && !run.italic && !run.mono,
                "styling leaked for {s:?}: {run:?}"
            );
            assert!(run.section_ref.is_none() && run.fill_in.is_none());
            assert_eq!(run.text, expected, "for {s:?} via {source:?}");
        }
    }

    #[test]
    fn escaped_cells_round_trip_as_one_plain_run() {
        let mut rng = Rng(0xC0FF_EE11);
        let options = ParseOptions::default();
        for _ in 0..500 {
            let s = random_text(&mut rng);
            let source = format!("| {} |\n", escape_cell(&s));
            let expected = folded(&s)
                .trim_matches(|c: char| c.is_whitespace() && c != '\n')
                .to_string();
            let blocks = parse_markdown(&source, &options)
                .unwrap_or_else(|e| panic!("escape_cell({s:?}) = {source:?} failed: {e}"));
            let [Block::Table(t)] = blocks.as_slice() else {
                panic!("escape_cell({s:?}) = {source:?} parsed to {blocks:?}");
            };
            assert_eq!(t.rows.len(), 1);
            assert_eq!(t.rows[0].len(), 1);
            match &t.rows[0][0] {
                Cell::Blank => assert_eq!(expected, "", "for {s:?} via {source:?}"),
                Cell::Text(runs) => {
                    assert_eq!(runs.len(), 1, "for {s:?} via {source:?}: {runs:?}");
                    assert!(!runs[0].bold && !runs[0].italic && !runs[0].mono);
                    assert_eq!(runs[0].text, expected, "for {s:?} via {source:?}");
                }
                other => panic!("escape_cell({s:?}) parsed to {other:?}"),
            }
        }
    }

    #[test]
    fn mono_round_trips_as_one_mono_run() {
        let mut rng = Rng(0xDEAD_BEEF);
        let options = ParseOptions::default();
        for _ in 0..500 {
            let s = random_text(&mut rng);
            let flow = mono(&s);
            if !flow.is_empty() {
                let blocks = parse_markdown(&flow, &options)
                    .unwrap_or_else(|e| panic!("mono({s:?}) = {flow:?} failed: {e}"));
                let [Block::Paragraph(runs)] = blocks.as_slice() else {
                    panic!("mono({s:?}) = {flow:?} parsed to {blocks:?}");
                };
                assert_eq!(runs.len(), 1, "for {s:?} via {flow:?}");
                assert!(runs[0].mono);
                assert_eq!(runs[0].text, folded_mono(&s), "for {s:?} via {flow:?}");
            }

            let in_cell = mono_cell(&s);
            let source = format!("| {in_cell} |\n");
            let blocks = parse_markdown(&source, &options)
                .unwrap_or_else(|e| panic!("mono_cell({s:?}) = {source:?} failed: {e}"));
            let [Block::Table(t)] = blocks.as_slice() else {
                panic!("mono_cell({s:?}) = {source:?} parsed to {blocks:?}");
            };
            match &t.rows[0][0] {
                Cell::Blank => assert_eq!(folded_mono(&s), "", "for {s:?}"),
                Cell::Text(runs) => {
                    assert_eq!(runs.len(), 1, "for {s:?} via {source:?}");
                    assert!(runs[0].mono);
                    assert_eq!(runs[0].text, folded_mono(&s), "for {s:?} via {source:?}");
                }
                other => panic!("mono_cell({s:?}) parsed to {other:?}"),
            }
        }
    }

    /// [`folded`] with newlines becoming spaces: what `mono`/`mono_cell` do.
    fn folded_mono(s: &str) -> String {
        folded(s).replace('\n', " ")
    }

    #[test]
    fn parser_never_panics_on_arbitrary_input() {
        let mut rng = Rng(0xFEED_F00D);
        let options = ParseOptions::default();
        for _ in 0..1000 {
            let s = random_text(&mut rng);
            let _ = parse_markdown(&s, &options);
        }
    }
}

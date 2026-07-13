//! An imperative builder API for constructing documents from Rust.
//!
//! This module lets you assemble a [`Document`] directly in code, one block at a
//! time, with ordinary control flow (loops, `if`, `match`) deciding what goes in.
//! The result feeds the layout → render pipeline.
//!
//! ```no_run
//! use textris_pdf::build::{Textris, mono, bold};
//! use textris_pdf::fonts::Fonts;
//!
//! let fonts = Fonts::from_variable_files("regular.ttf", "italic.ttf", "mono.ttf").unwrap();
//!
//! let mut doc = Textris::new();
//! doc.h2("Marine species profile");
//! doc.h1("The Mantis Shrimp");
//! doc.paragraph("A profile of the order Stomatopoda …");
//! doc.h3_numbered("Classification").anchor("classification");
//! doc.paragraph(bold("Order Stomatopoda"));
//! doc.table(
//!     ["", "common name", "species"],
//!     [["1", "Peacock mantis shrimp", ""]],
//! );
//!
//! let pdf = doc.render(&fonts);
//! # let _ = pdf;
//! ```
//!
//! ## Rich text
//!
//! Anywhere a method takes text it accepts [`IntoText`]: a plain `&str`/`String`
//! becomes a single regular run, while the [`Text`] builder (and the free
//! helpers [`text`], [`bold`], [`italic`], [`mono`]) compose mixed emphasis:
//!
//! ```
//! use textris_pdf::build::{text, bold, mono};
//! // "Length: " in normal weight, the value in bold.
//! let line = text("Length: ").bold("18 cm");
//! // A single monospace run.
//! let code = mono("STO-2026");
//! # let _ = (line, code);
//! ```

mod text;

pub use text::{
    IntoCell, IntoText, Text, blank, bold, cell, fill_in, italic, mono, muted, section_ref, spacer,
    text,
};

use std::{io, path::Path};

use crate::{
    fonts::Fonts,
    model::{Block, Cell, Chrome, Document, Inline, ListMarker, SectionContent, Table, TaskItem},
    render::RenderError,
    theme::{BoxStyle, TableStyle, Theme},
};

/// Collect a list of items, each of which is any [`IntoText`], into inline runs.
fn items<I, C>(items: I) -> Vec<Vec<Inline>>
where
    I: IntoIterator<Item = C>,
    C: IntoText,
{
    items.into_iter().map(IntoText::into_inlines).collect()
}

/// Collect a table row of cells, each of which is any [`IntoCell`].
fn cells<I, C>(cells: I) -> Vec<Cell>
where
    I: IntoIterator<Item = C>,
    C: IntoCell,
{
    cells.into_iter().map(IntoCell::into_cell).collect()
}

/// The document builder.
///
/// Create one with [`Textris::new`], append blocks with the `add`-style methods
/// (each returns `&mut Self`, so calls may be chained or written as statements),
/// then produce output with [`Textris::render`] / [`Textris::render_to_file`], or take
/// the underlying [`Document`] with [`Textris::build`].
#[derive(Debug, Default, Clone)]
pub struct Textris {
    doc: Document,
}

impl Textris {
    /// Start an empty document styled with the default [`Theme`].
    pub fn new() -> Self {
        Self {
            doc: Document::default(),
        }
    }

    /// Start an empty document with a custom [`Theme`]. Build a `Theme` from
    /// [`Theme::default`] and override the fields you want to re-skin.
    pub fn with_theme(theme: Theme) -> Self {
        Self {
            doc: Document {
                theme,
                ..Document::default()
            },
        }
    }

    /// The theme this document will be laid out with.
    pub fn theme(&self) -> &Theme {
        &self.doc.theme
    }

    /// Mutable access to the theme, for tweaking it after construction.
    pub fn theme_mut(&mut self) -> &mut Theme {
        &mut self.doc.theme
    }

    /// Set the left section of the header.
    pub fn header_left(&mut self, content: impl Into<SectionContent>) -> &mut Self {
        self.doc.header.left = Some(content.into());
        self
    }

    /// Set the center section of the header.
    pub fn header_center(&mut self, content: impl Into<SectionContent>) -> &mut Self {
        self.doc.header.center = Some(content.into());
        self
    }

    /// Set the right section of the header.
    pub fn header_right(&mut self, content: impl Into<SectionContent>) -> &mut Self {
        self.doc.header.right = Some(content.into());
        self
    }

    /// Add a heading at the given level. Level 1 is the largest (document
    /// title), level 2 is a subtitle, levels 3-5 are section headings of
    /// decreasing size (levels beyond 5 render at the level-5 size).
    /// Use the shorthand [`h1`](Self::h1) / [`h2`](Self::h2) / [`h3`](Self::h3)
    /// / [`h4`](Self::h4) / [`h5`](Self::h5)
    /// methods for the common cases.
    pub fn heading(&mut self, level: u8, text: impl IntoText) -> &mut Self {
        self.push_heading(level, text, false)
    }

    fn push_heading(&mut self, level: u8, text: impl IntoText, numbered: bool) -> &mut Self {
        self.doc.blocks.push(Block::Heading {
            level,
            content: text.into_inlines(),
            numbered,
            label: None,
        });
        self
    }

    /// Add a heading that carries an automatic section number ("3", "3.1", …),
    /// prefixed to its text when the document is built. One counter runs per
    /// level, and a deeper numbered heading nests under the last shallower
    /// one, so numbered `h3`/`h4` headings yield `1.`, `1.1.`, `1.2.`, `2.`, …
    /// Use the [`h3_numbered`](Self::h3_numbered) /
    /// [`h4_numbered`](Self::h4_numbered) / [`h5_numbered`](Self::h5_numbered)
    /// shorthands for the common levels, and [`anchor`](Self::anchor) to make
    /// the section referenceable.
    pub fn heading_numbered(&mut self, level: u8, text: impl IntoText) -> &mut Self {
        self.push_heading(level, text, true)
    }

    /// Add a numbered level-3 heading (section). See
    /// [`heading_numbered`](Self::heading_numbered).
    pub fn h3_numbered(&mut self, text: impl IntoText) -> &mut Self {
        self.heading_numbered(3, text)
    }

    /// Add a numbered level-4 heading (subsection).
    pub fn h4_numbered(&mut self, text: impl IntoText) -> &mut Self {
        self.heading_numbered(4, text)
    }

    /// Add a numbered level-5 heading.
    pub fn h5_numbered(&mut self, text: impl IntoText) -> &mut Self {
        self.heading_numbered(5, text)
    }

    /// Label the heading just added, so other text can reference its section
    /// number with [`section_ref`] (forward references included):
    ///
    /// ```
    /// # use textris_pdf::build::{Textris, text};
    /// let mut doc = Textris::new();
    /// doc.h3_numbered("Vision").anchor("vision");
    /// doc.paragraph(text("As shown in section ").section_ref("vision").normal("."));
    /// ```
    ///
    /// # Panics
    ///
    /// Panics when the last block added is not a heading.
    pub fn anchor(&mut self, label: impl Into<String>) -> &mut Self {
        match self.doc.blocks.last_mut() {
            Some(Block::Heading { label: slot, .. }) => *slot = Some(label.into()),
            _ => panic!("anchor() must directly follow a heading"),
        }
        self
    }

    /// Add a level-1 heading (document title).
    pub fn h1(&mut self, text: impl IntoText) -> &mut Self {
        self.heading(1, text)
    }

    /// Add a level-2 heading (subtitle / label).
    pub fn h2(&mut self, text: impl IntoText) -> &mut Self {
        self.heading(2, text)
    }

    /// Add a level-3 heading (section heading).
    pub fn h3(&mut self, text: impl IntoText) -> &mut Self {
        self.heading(3, text)
    }

    /// Add a level-4 heading (subsection heading).
    pub fn h4(&mut self, text: impl IntoText) -> &mut Self {
        self.heading(4, text)
    }

    /// Add a level-5 heading.
    pub fn h5(&mut self, text: impl IntoText) -> &mut Self {
        self.heading(5, text)
    }

    /// Add a paragraph of flowing text.
    pub fn paragraph(&mut self, text: impl IntoText) -> &mut Self {
        self.doc.blocks.push(Block::Paragraph(text.into_inlines()));
        self
    }

    /// Shorthand for [`paragraph`](Self::paragraph).
    pub fn p(&mut self, text: impl IntoText) -> &mut Self {
        self.paragraph(text)
    }

    /// Force a page break: the following content starts on a fresh page. A
    /// break at the top of an already-empty page does nothing.
    pub fn page_break(&mut self) -> &mut Self {
        self.doc.blocks.push(Block::PageBreak);
        self
    }

    /// Add fixed vertical space of `height` points. No inter-block gap is
    /// added around a spacer, so it *is* the distance between its neighbours —
    /// handy for extra air between sections. Express theme-relative heights
    /// with [`em`](crate::theme::em). For vertical space inside a table cell,
    /// see the [`spacer`] cell helper.
    pub fn spacer(&mut self, height: f32) -> &mut Self {
        self.doc.blocks.push(Block::Spacer(height));
        self
    }

    /// Add a data table ([`TableStyle::data`]): a header row followed by body
    /// rows, with an italic header and zebra-striped body.
    ///
    /// Each header and cell is any [`IntoCell`]: plain strings and rich
    /// [`Text`] become text cells, and the [`cell`], [`fill_in`], [`blank`]
    /// and [`spacer`] helpers build the special cells. Because array literals
    /// must be homogeneous, mix kinds by wrapping every cell in a helper so
    /// the element type is uniform, e.g. `[text("1"), mono("18 cm")]` or
    /// `[cell("Date"), fill_in()]`.
    ///
    /// To use a different [`TableStyle`], call [`table_styled`](Self::table_styled).
    ///
    /// ```
    /// # use textris_pdf::build::{Textris, text, mono};
    /// let mut doc = Textris::new();
    /// doc.table(
    ///     ["", "common name", "max. length"],
    ///     [
    ///         [text("1"), text("Peacock mantis shrimp"), mono("18 cm")],
    ///         [text("2"), text("Zebra mantis shrimp"), mono("40 cm")],
    ///     ],
    /// );
    /// ```
    pub fn table<H, HC, R, RC>(&mut self, headers: H, rows: R) -> &mut Self
    where
        H: IntoIterator<Item = HC>,
        HC: IntoCell,
        R: IntoIterator<Item = RC>,
        RC: IntoIterator,
        RC::Item: IntoCell,
    {
        self.table_styled(&TableStyle::data(), headers, rows)
    }

    /// Add a table with an explicit [`TableStyle`]. Define your styles up front
    /// and reference one here.
    ///
    /// ```
    /// # use textris_pdf::build::{Textris, text};
    /// # use textris_pdf::theme::TableStyle;
    /// // A data table without zebra striping.
    /// let plain = TableStyle { striped: false, ..TableStyle::data() };
    ///
    /// let mut doc = Textris::new();
    /// doc.table_styled(&plain, ["a", "b"], [[text("1"), text("2")]]);
    /// ```
    pub fn table_styled<H, HC, R, RC>(
        &mut self,
        style: &TableStyle,
        headers: H,
        rows: R,
    ) -> &mut Self
    where
        H: IntoIterator<Item = HC>,
        HC: IntoCell,
        R: IntoIterator<Item = RC>,
        RC: IntoIterator,
        RC::Item: IntoCell,
    {
        let headers = cells(headers);
        let rows: Vec<Vec<Cell>> = rows.into_iter().map(cells).collect();
        self.doc.blocks.push(Block::Table(Table {
            style: style.clone(),
            headers,
            rows,
        }));
        self
    }

    /// Build a table imperatively, adding rows in a loop or under conditionals.
    /// Ideal when the rows come from data rather than a literal.
    ///
    /// ```
    /// # use textris_pdf::build::{Textris, text, mono};
    /// # struct Species { name: &'static str, length: &'static str }
    /// # let species = [Species { name: "Peacock mantis shrimp", length: "18 cm" }];
    /// let mut doc = Textris::new();
    /// doc.table_with(|t| {
    ///     t.headers(["", "common name", "max. length"]);
    ///     for (i, s) in species.iter().enumerate() {
    ///         t.row([text((i + 1).to_string()), text(s.name), mono(s.length)]);
    ///     }
    /// });
    /// ```
    pub fn table_with(&mut self, build: impl FnOnce(&mut TableBuilder)) -> &mut Self {
        let mut builder = TableBuilder::default();
        build(&mut builder);
        self.doc.blocks.push(Block::Table(builder.finish()));
        self
    }

    /// Add a two-column label table: no header row, left column holds labels, no
    /// zebra striping. Convenience for the common `key / value` form. Use
    /// [`fill_in`] for value cells that should render as a blank line to fill
    /// in:
    ///
    /// ```
    /// # use textris_pdf::build::{Textris, cell, fill_in};
    /// let mut doc = Textris::new();
    /// doc.label_table([
    ///     [cell("Observer"), cell("Costa, R.")],
    ///     [cell("Date"), fill_in()],
    /// ]);
    /// ```
    pub fn label_table<R, RC>(&mut self, rows: R) -> &mut Self
    where
        R: IntoIterator<Item = RC>,
        RC: IntoIterator,
        RC::Item: IntoCell,
    {
        let rows: Vec<Vec<Cell>> = rows.into_iter().map(cells).collect();
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        self.doc.blocks.push(Block::Table(Table {
            style: TableStyle::label(),
            headers: vec![Cell::Blank; columns],
            rows,
        }));
        self
    }

    /// Build a two-column label table imperatively, one field per line. Reads
    /// more clearly than a [`label_table`](Self::label_table) literal when the
    /// rows mix filled values, blank fill-in lines, and roomy spaces, since each
    /// method names the kind of value:
    ///
    /// ```
    /// # use textris_pdf::build::Textris;
    /// # use textris_pdf::theme::em;
    /// let mut doc = Textris::new();
    /// doc.label_table_with(|t| {
    ///     t.value("Observer", "Costa, R.");
    ///     t.value("Location", "Lembeh Strait, Indonesia");
    ///     t.fill_in("Date");
    ///     t.fill_in("Depth");
    ///     t.spacer("Field notes", em(6.0));
    ///     t.fill_in("Signature");
    /// });
    /// ```
    pub fn label_table_with(&mut self, build: impl FnOnce(&mut LabelTableBuilder)) -> &mut Self {
        let mut builder = LabelTableBuilder::default();
        build(&mut builder);
        self.doc.blocks.push(Block::Table(builder.finish()));
        self
    }

    /// Add a plain bullet list. Each item is any [`IntoText`].
    pub fn bullet_list<I, C>(&mut self, list: I) -> &mut Self
    where
        I: IntoIterator<Item = C>,
        C: IntoText,
    {
        self.doc.blocks.push(Block::BulletList(items(list)));
        self
    }

    /// Add a numbered ordered list (`1.`, `2.`, `3.`, …). Each item is any
    /// [`IntoText`]. Use [`ordered_list_with`](Self::ordered_list_with) for a
    /// different [`ListMarker`] (e.g. `a.`, `b.`, `c.`).
    pub fn ordered_list<I, C>(&mut self, items: I) -> &mut Self
    where
        I: IntoIterator<Item = C>,
        C: IntoText,
    {
        self.ordered_list_with(ListMarker::Decimal, items)
    }

    /// Add an ordered list with an explicit [`ListMarker`].
    ///
    /// ```
    /// # use textris_pdf::build::Textris;
    /// # use textris_pdf::model::ListMarker;
    /// let mut doc = Textris::new();
    /// doc.ordered_list_with(ListMarker::LowerAlpha, ["first", "second"]);
    /// ```
    pub fn ordered_list_with<I, C>(&mut self, marker: ListMarker, list: I) -> &mut Self
    where
        I: IntoIterator<Item = C>,
        C: IntoText,
    {
        self.doc.blocks.push(Block::OrderedList {
            marker,
            items: items(list),
        });
        self
    }

    /// Add a boxed callout: a filled background with padding and margin wrapping
    /// child blocks, built with the same API as the document itself.
    ///
    /// The closure receives a nested [`Textris`] (sharing this document's theme);
    /// every block you add to it becomes the box's content.
    ///
    /// ```
    /// # use textris_pdf::build::{Textris, bold};
    /// let mut doc = Textris::new();
    /// doc.boxed(|b| {
    ///     b.paragraph(bold("Handle with care."));
    ///     b.paragraph("A large smasher can crack aquarium glass …");
    /// });
    /// ```
    pub fn boxed(&mut self, build: impl FnOnce(&mut Textris)) -> &mut Self {
        self.boxed_styled(&BoxStyle::callout(), build)
    }

    /// Add a boxed callout with an explicit [`BoxStyle`]. See [`boxed`](Self::boxed)
    /// for the content closure.
    pub fn boxed_styled(
        &mut self,
        style: &BoxStyle,
        build: impl FnOnce(&mut Textris),
    ) -> &mut Self {
        let mut inner = Textris::with_theme(self.doc.theme.clone());
        build(&mut inner);
        self.doc.blocks.push(Block::Box {
            style: *style,
            content: inner.doc.blocks,
        });
        self
    }

    /// Add a task list of `(checked, content)` items, rendered with checkboxes.
    ///
    /// ```
    /// # use textris_pdf::build::Textris;
    /// let mut doc = Textris::new();
    /// doc.task_list([
    ///     (true, "Photograph the raptorial appendages"),
    ///     (false, "Record the water temperature"),
    /// ]);
    /// ```
    pub fn task_list<I, C>(&mut self, items: I) -> &mut Self
    where
        I: IntoIterator<Item = (bool, C)>,
        C: IntoText,
    {
        let items = items
            .into_iter()
            .map(|(checked, content)| TaskItem {
                checked,
                content: content.into_inlines(),
            })
            .collect();
        self.doc.blocks.push(Block::TaskList(items));
        self
    }

    /// Set the left section of the footer.
    pub fn footer_left(&mut self, content: impl Into<SectionContent>) -> &mut Self {
        self.doc.footer.left = Some(content.into());
        self
    }

    /// Set the center section of the footer.
    pub fn footer_center(&mut self, content: impl Into<SectionContent>) -> &mut Self {
        self.doc.footer.center = Some(content.into());
        self
    }

    /// Set the right section of the footer.
    pub fn footer_right(&mut self, content: impl Into<SectionContent>) -> &mut Self {
        self.doc.footer.right = Some(content.into());
        self
    }

    /// Replace the whole header at once.
    pub fn set_header(&mut self, header: Chrome) -> &mut Self {
        self.doc.header = header;
        self
    }

    /// Replace the whole footer at once.
    pub fn set_footer(&mut self, footer: Chrome) -> &mut Self {
        self.doc.footer = footer;
        self
    }

    /// Escape hatch: push a pre-built [`Block`] for cases the typed methods don't
    /// cover.
    pub fn push_block(&mut self, block: Block) -> &mut Self {
        self.doc.blocks.push(block);
        self
    }

    /// Consume the builder and return the assembled [`Document`], with section
    /// numbering and references resolved (see [`Document::resolve_sections`]).
    pub fn build(self) -> Document {
        let mut doc = self.doc;
        doc.resolve_sections();
        doc
    }

    /// A borrowed view of the document built so far. Section numbering and
    /// references are still unresolved here; [`build`](Self::build) resolves
    /// them.
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// Lay out and render the document to PDF/A-2b bytes.
    pub fn render(&self, fonts: &Fonts) -> Result<Vec<u8>, RenderError> {
        let mut doc = self.doc.clone();
        doc.resolve_sections();
        let pages = crate::layout::layout(&doc, fonts);
        crate::render::render(&pages, &doc, fonts)
    }

    /// Render the document and write the PDF to `path`.
    pub fn render_to_file(&self, path: impl AsRef<Path>, fonts: &Fonts) -> io::Result<()> {
        let pdf = self.render(fonts).map_err(io::Error::other)?;
        std::fs::write(path, pdf)
    }

    /// Export the document as a Word `.docx` file, returning its bytes.
    ///
    /// This is a structural export (headings, paragraphs, tables, lists, boxes
    /// and chrome) rather than a faithful reproduction of the PDF's styling;
    /// see the [`docx`](crate::docx) module for the shortcuts taken. Section
    /// numbering and references are resolved first, as in [`render`](Self::render).
    ///
    /// Requires the `docx` cargo feature.
    #[cfg(feature = "docx")]
    pub fn to_docx(&self) -> io::Result<Vec<u8>> {
        let mut doc = self.doc.clone();
        doc.resolve_sections();
        crate::docx::to_docx(&doc)
    }

    /// Export the document as a Word `.docx` file and write it to `path`.
    ///
    /// Requires the `docx` cargo feature.
    #[cfg(feature = "docx")]
    pub fn write_docx_to_file(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let docx = self.to_docx()?;
        std::fs::write(path, docx)
    }

    /// Export the document as a GitHub-flavored Markdown string.
    ///
    /// This is a structural export (headings, paragraphs, tables, lists and
    /// boxes) rather than a faithful reproduction of the PDF's styling; see the
    /// [`markdown`](crate::markdown) module for the encoding choices taken.
    /// Section references are resolved first, but the headings are left
    /// unnumbered (Markdown renderers commonly number headings themselves), so
    /// this uses
    /// [`resolve_sections_unnumbered`](crate::model::Document::resolve_sections_unnumbered)
    /// rather than the numbering pass [`render`](Self::render) uses.
    ///
    /// Requires the `markdown` cargo feature.
    #[cfg(feature = "markdown")]
    pub fn to_markdown(&self) -> String {
        let mut doc = self.doc.clone();
        doc.resolve_sections_unnumbered();
        crate::markdown::to_markdown(&doc)
    }

    /// Export the document as Markdown and write it to `path`.
    ///
    /// Requires the `markdown` cargo feature.
    #[cfg(feature = "markdown")]
    pub fn write_markdown_to_file(&self, path: impl AsRef<Path>) -> io::Result<()> {
        std::fs::write(path, self.to_markdown())
    }
}

/// A table under construction, handed to the closure of [`Textris::table_with`].
///
/// Defaults to [`TableStyle::data`]; call [`style`](Self::style) to use another.
#[derive(Debug, Default)]
pub struct TableBuilder {
    headers: Vec<Cell>,
    rows: Vec<Vec<Cell>>,
    style: TableStyle,
}

impl TableBuilder {
    /// Set the header row. Cells are any [`IntoCell`].
    pub fn headers<I, C>(&mut self, headers: I) -> &mut Self
    where
        I: IntoIterator<Item = C>,
        C: IntoCell,
    {
        self.headers = cells(headers);
        self
    }

    /// Append a body row. Cells are any [`IntoCell`].
    pub fn row<I, C>(&mut self, row: I) -> &mut Self
    where
        I: IntoIterator<Item = C>,
        C: IntoCell,
    {
        self.rows.push(cells(row));
        self
    }

    /// Set the [`TableStyle`] for this table (defaults to [`TableStyle::data`]).
    pub fn style(&mut self, style: &TableStyle) -> &mut Self {
        self.style = style.clone();
        self
    }

    fn finish(self) -> Table {
        Table {
            style: self.style,
            headers: self.headers,
            rows: self.rows,
        }
    }
}

/// A two-column label table under construction, handed to the closure of
/// [`Textris::label_table_with`]. Each method appends one label row: a label in
/// the left column and a value of the named kind on the right.
#[derive(Debug, Default)]
pub struct LabelTableBuilder {
    rows: Vec<Vec<Cell>>,
}

impl LabelTableBuilder {
    /// Append a labeled value: `label` on the left, `value` on the right. Both
    /// are any [`IntoText`].
    pub fn value(&mut self, label: impl IntoText, value: impl IntoText) -> &mut Self {
        self.rows.push(vec![cell(label), cell(value)]);
        self
    }

    /// Append a label whose value is a blank line to fill in (see [`fill_in`]).
    /// Fill-in rows are given a standard minimum height (the theme's
    /// [`TableMetrics::fill_in_min_height`](crate::theme::TableMetrics)) so
    /// there is room to write above the line.
    pub fn fill_in(&mut self, label: impl IntoText) -> &mut Self {
        self.rows.push(vec![cell(label), Cell::FillIn]);
        self
    }

    /// Append a label whose value is empty but forces the row to be at least
    /// `height` points tall (see [`spacer`]), e.g. a roomy field-notes area.
    pub fn spacer(&mut self, label: impl IntoText, height: f32) -> &mut Self {
        self.rows.push(vec![cell(label), Cell::Spacer(height)]);
        self
    }

    fn finish(self) -> Table {
        let columns = self.rows.iter().map(Vec::len).max().unwrap_or(0);
        Table {
            style: TableStyle::label(),
            headers: vec![Cell::Blank; columns],
            rows: self.rows,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::plain_text;

    #[test]
    fn table_helpers_carry_their_style() {
        let mut doc = Textris::new();
        doc.table(["a", "b"], [["1", "2"]]);
        doc.label_table([["Observer", "Value"]]);
        let custom = TableStyle {
            striped: false,
            ..TableStyle::data()
        };
        doc.table_styled(&custom, ["a"], [["1"]]);

        let styles: Vec<TableStyle> = doc
            .document()
            .blocks
            .iter()
            .map(|b| match b {
                Block::Table(t) => t.style.clone(),
                _ => panic!("expected a table"),
            })
            .collect();
        assert_eq!(styles[0], TableStyle::data());
        assert_eq!(styles[1], TableStyle::label());
        assert_eq!(styles[2], custom);
    }

    #[test]
    fn table_with_builds_rows_dynamically() {
        let mut doc = Textris::new();
        doc.table_with(|t| {
            t.headers(["n", "name"]);
            for i in 0..3 {
                t.row([text(i.to_string()), text("x")]);
            }
        });
        let Block::Table(table) = &doc.document().blocks[0] else {
            panic!()
        };
        assert_eq!(table.style, TableStyle::data());
        assert_eq!(table.rows.len(), 3);
        assert_eq!(plain_text(table.rows[2][0].inlines()), "2");
    }

    #[test]
    fn table_with_honors_an_explicit_style() {
        let mut doc = Textris::new();
        doc.table_with(|t| {
            t.style(&TableStyle::label());
            t.row([text("Observer"), text("Value")]);
        });
        let Block::Table(table) = &doc.document().blocks[0] else {
            panic!()
        };
        assert_eq!(table.style, TableStyle::label());
    }

    #[test]
    fn label_table_with_builds_label_styled_rows() {
        let mut doc = Textris::new();
        doc.label_table_with(|t| {
            t.value("Observer", "Costa, R.");
            t.fill_in("Date");
            t.spacer("Field notes", 60.0);
        });
        let Block::Table(table) = &doc.document().blocks[0] else {
            panic!()
        };
        assert_eq!(table.style, TableStyle::label());
        assert_eq!(table.headers, vec![Cell::Blank, Cell::Blank]);
        assert_eq!(plain_text(table.rows[0][1].inlines()), "Costa, R.");
        assert!(matches!(table.rows[1][1], Cell::FillIn));
        assert!(matches!(table.rows[2][1], Cell::Spacer(60.0)));
    }

    #[test]
    fn task_list_carries_checkboxes() {
        let mut doc = Textris::new();
        doc.task_list([(true, "done"), (false, "todo")]);
        let Block::TaskList(items) = &doc.document().blocks[0] else {
            panic!()
        };
        assert!(items[0].checked);
        assert!(!items[1].checked);
    }

    #[test]
    fn label_table_has_blank_headers() {
        let mut doc = Textris::new();
        doc.label_table([["Observer", "Costa, R."], ["Date", ""]]);
        let Block::Table(table) = &doc.document().blocks[0] else {
            panic!()
        };
        assert_eq!(table.style, TableStyle::label());
        assert_eq!(table.columns(), 2);
    }
}

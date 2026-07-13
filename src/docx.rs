//! Optional Word (`.docx`) export, gated behind the `docx` cargo feature.
//!
//! This translates the layout-agnostic [`Document`] model into a Word document
//! with [`docx-rs`](docx_rs). It is deliberately a *structural* export: the
//! result reproduces the document's content and coarse structure — headings,
//! paragraphs, tables, lists, callout boxes and header/footer chrome — but not
//! the pixel-level styling of the PDF renderer.
//!
//! Several shortcuts are taken where faithful styling is hard to express in the
//! Word model and rarely worth it:
//!
//! - Headings are plain bold paragraphs sized from the theme, tagged with an
//!   outline level so they show up in Word's navigation pane; no named styles.
//! - Lists render their marker (`•`, `1.`, `☐`/`☑`) as literal text rather than
//!   using Word's numbering definitions.
//! - Tables keep their cells, per-column alignment and zebra striping, but drop
//!   the finer metrics (custom column widths, insets, fill-in rules); a
//!   [`Cell::FillIn`](crate::model::Cell) becomes an underscore run.
//! - A callout [`Box`](crate::model::Block::Box) becomes a single-cell,
//!   borderless table shaded with the box's background color.
//! - Header/footer page counters become Word `PAGE`/`NUMPAGES` fields rather
//!   than the model's formatting closure.
//!
//! The entry point is [`to_docx`]; the builder also exposes
//! [`Textris::to_docx`](crate::build::Textris::to_docx) and
//! [`Textris::write_docx_to_file`](crate::build::Textris::write_docx_to_file).

use std::io::{self, Cursor};

use docx_rs::{
    AlignmentType, BreakType, Docx, FieldCharType, Footer, Header, InstrNUMPAGES, InstrPAGE,
    InstrText, LineSpacing, LineSpacingType, PageMargin, Paragraph, Run, RunFonts, Shading, ShdType,
    SpecialIndentType, Table as DocxTable, TableCell, TableRow, WidthType,
};
use krilla::color::rgb;

use crate::{
    model::{Block, Cell, Chrome, Document, Inline, SectionContent},
    theme::{Align, Palette, Theme},
};

/// The monospace family used for `mono` runs.
const MONO_FONT: &str = "Courier New";

/// Translate a [`Document`] into the bytes of a Word `.docx` file.
///
/// The document is translated as-is; section numbering and references are *not*
/// resolved here (use [`Textris::to_docx`](crate::build::Textris::to_docx),
/// which resolves them first, or call
/// [`Document::resolve_sections`](crate::model::Document::resolve_sections)
/// yourself).
pub fn to_docx(document: &Document) -> io::Result<Vec<u8>> {
    let mut buffer = Cursor::new(Vec::new());
    build_docx(document)
        .build()
        .pack(&mut buffer)
        .map_err(io::Error::other)?;
    Ok(buffer.into_inner())
}

/// Assemble the `docx-rs` document from our model.
fn build_docx(document: &Document) -> Docx {
    let theme = &document.theme;
    let page = &theme.page;
    let mut docx = Docx::new()
        // Match the theme's page geometry (A4 by default) so tables and chrome
        // sized to the theme's content width fit the page. Without this, Word
        // falls back to its own default margins, which are wider than the
        // theme's, leaving full-width tables overflowing the text area.
        .page_size(twips(page.width), twips(page.height))
        .page_margin(
            PageMargin::new()
                .top(twips(page.margin_y) as i32)
                .bottom(twips(page.margin_y) as i32)
                .left(twips(page.margin_x) as i32)
                .right(twips(page.margin_x) as i32)
                // The header/footer sit inside the top/bottom margins, offset
                // from the content edge; clamp so they never cross the page edge.
                .header(twips((page.margin_y - page.header_offset).max(0.0)) as i32)
                .footer(twips((page.margin_y - page.footer_offset).max(0.0)) as i32),
        );

    if let Some(table) = chrome_row(&document.header, theme) {
        docx = docx.header(Header::new().add_table(table));
    }
    if let Some(table) = chrome_row(&document.footer, theme) {
        docx = docx.footer(Footer::new().add_table(table));
    }

    for block in &document.blocks {
        docx = append_block(docx, block, theme);
    }
    docx
}

/// A destination that accepts the paragraphs and tables a block translates to.
///
/// Both the top-level [`Docx`] and a [`TableCell`] (used for callout boxes) can
/// receive block content, so [`append_block`] is generic over this trait rather
/// than duplicated for each.
trait BlockSink: Sized {
    fn push_paragraph(self, paragraph: Paragraph) -> Self;
    fn push_table(self, table: DocxTable) -> Self;
}

impl BlockSink for Docx {
    fn push_paragraph(self, paragraph: Paragraph) -> Self {
        self.add_paragraph(paragraph)
    }
    fn push_table(self, table: DocxTable) -> Self {
        self.add_table(table)
    }
}

impl BlockSink for TableCell {
    fn push_paragraph(self, paragraph: Paragraph) -> Self {
        self.add_paragraph(paragraph)
    }
    fn push_table(self, table: DocxTable) -> Self {
        self.add_table(table)
    }
}

/// Translate one block and append it to `sink`.
fn append_block<S: BlockSink>(sink: S, block: &Block, theme: &Theme) -> S {
    let palette = &theme.palette;
    match block {
        Block::Heading { level, content, .. } => {
            sink.push_paragraph(heading_paragraph(content, *level, theme))
        }
        Block::Paragraph(inlines) => sink.push_paragraph(
            text_paragraph(inlines, theme.font_size.body, false, false, palette)
                .line_spacing(block_spacing(theme)),
        ),
        Block::Table(table) => sink.push_table(build_table(table, theme)),
        Block::TaskList(items) => items.iter().fold(sink, |sink, item| {
            let marker = if item.checked {
                "\u{2611} "
            } else {
                "\u{2610} "
            };
            sink.push_paragraph(list_paragraph(
                marker,
                &item.content,
                theme.list.task_gap,
                theme,
            ))
        }),
        Block::BulletList(items) => items.iter().fold(sink, |sink, item| {
            sink.push_paragraph(list_paragraph(
                "\u{2022} ",
                item,
                theme.list.bullet_gap,
                theme,
            ))
        }),
        Block::OrderedList { marker, items } => {
            items.iter().enumerate().fold(sink, |sink, (i, item)| {
                let label = format!("{} ", marker.label(i + 1));
                sink.push_paragraph(list_paragraph(&label, item, theme.list.bullet_gap, theme))
            })
        }
        Block::Box { style, content } => {
            // A callout becomes a single-cell shaded table holding the child
            // blocks. Build the cell by folding the children into it, then wrap.
            let cell = content.iter().fold(
                TableCell::new().shading(fill(style.background)),
                |cell, child| append_block(cell, child, theme),
            );
            let width = content_twips(theme);
            sink.push_table(
                DocxTable::without_borders(vec![TableRow::new(vec![cell])])
                    .width(width, WidthType::Dxa),
            )
        }
        Block::PageBreak => {
            sink.push_paragraph(Paragraph::new().add_run(Run::new().add_break(BreakType::Page)))
        }
        // An empty paragraph of the requested exact height stands in for the spacer.
        Block::Spacer(height) => sink.push_paragraph(
            Paragraph::new().line_spacing(
                LineSpacing::new()
                    .line_rule(LineSpacingType::Exact)
                    .line(twips(*height) as i32),
            ),
        ),
    }
}

/// A heading: a bold paragraph sized from the theme, tagged with an outline
/// level (0-based) so Word lists it in the navigation pane.
fn heading_paragraph(content: &[Inline], level: u8, theme: &Theme) -> Paragraph {
    let sizes = &theme.font_size;
    let size = match level {
        0 | 1 => sizes.h1,
        2 => sizes.h2,
        3 => sizes.h3,
        4 => sizes.h4,
        _ => sizes.h5,
    };
    let outline = usize::from(level.saturating_sub(1)).min(8);
    let level = level.max(1);
    let spacing = &theme.spacing;
    let line_spacing = line_spacing(theme)
        .before(twips(spacing.heading_above.level(level)))
        .after(twips(spacing.heading_below.level(level)));
    content.iter().fold(
        Paragraph::new()
            .outline_lvl(outline)
            .line_spacing(line_spacing),
        |p, inline| p.add_run(build_run(inline, size, true, false, &theme.palette)),
    )
}

/// A plain paragraph of inline runs at the given size, with an optional bold or
/// italic base emphasis applied to every run.
fn text_paragraph(
    inlines: &[Inline],
    size: f32,
    bold: bool,
    italic: bool,
    palette: &Palette,
) -> Paragraph {
    inlines.iter().fold(Paragraph::new(), |p, inline| {
        p.add_run(build_run(inline, size, bold, italic, palette))
    })
}

/// A list item: a literal marker run followed by the item's inline runs, with
/// `gap` of vertical space after it and a hanging indent so wrapped lines align
/// past the marker.
fn list_paragraph(marker: &str, inlines: &[Inline], gap: f32, theme: &Theme) -> Paragraph {
    let size = theme.font_size.body;
    let indent = twips(theme.list.bullet_indent) as i32;
    let start = Paragraph::new()
        .line_spacing(line_spacing(theme).after(twips(gap)))
        .indent(
            Some(indent),
            Some(SpecialIndentType::Hanging(indent)),
            None,
            None,
        )
        .add_run(Run::new().add_text(marker).size(half_points(size)));
    inlines.iter().fold(start, |p, inline| {
        p.add_run(build_run(inline, size, false, false, &theme.palette))
    })
}

/// Build a run from an inline, merging the run's own emphasis with the base
/// emphasis of its context (e.g. a heading makes every run bold).
fn build_run(
    inline: &Inline,
    size: f32,
    base_bold: bool,
    base_italic: bool,
    palette: &Palette,
) -> Run {
    let mut run = Run::new()
        .add_text(inline.text.clone())
        .size(half_points(size));
    if inline.bold || base_bold {
        run = run.bold();
    }
    if inline.italic || base_italic {
        run = run.italic();
    }
    if inline.mono {
        run = run.fonts(RunFonts::new().ascii(MONO_FONT).hi_ansi(MONO_FONT));
    }
    if let Some(color) = inline.color {
        run = run.color(hex(color.resolve(palette)));
    }
    run
}

/// Build a Word table from a model table, keeping per-column alignment and
/// zebra striping. Rows are padded to a rectangle so every row has the same
/// number of cells.
fn build_table(table: &crate::model::Table, theme: &Theme) -> DocxTable {
    let style = &table.style;
    let palette = &theme.palette;
    let columns = table.columns().max(1);
    let size = style.font_size.unwrap_or(theme.font_size.body);

    let mut rows = Vec::new();
    let has_header =
        style.header && !table.headers.is_empty() && !table.headers.iter().all(Cell::is_blank);
    if has_header {
        rows.push(build_row(
            &table.headers,
            columns,
            style,
            size,
            palette,
            true,
            false,
        ));
    }
    for (index, row) in table.rows.iter().enumerate() {
        let striped = style.striped && index % 2 == 1;
        rows.push(build_row(
            row, columns, style, size, palette, false, striped,
        ));
    }

    // No borders: the zebra striping already delineates the rows, matching the
    // PDF renderer's borderless look.
    DocxTable::without_borders(rows).width(content_twips(theme), WidthType::Dxa)
}

/// Build one table row, padding to `columns` cells.
fn build_row(
    cells: &[Cell],
    columns: usize,
    style: &crate::theme::TableStyle,
    size: f32,
    palette: &Palette,
    header: bool,
    striped: bool,
) -> TableRow {
    let base_italic = header && style.header_italic;
    let docx_cells = (0..columns)
        .map(|column| {
            let align = style.align.get(column).copied().unwrap_or(Align::Left);
            let paragraph =
                cell_paragraph(cells.get(column), align, size, header, base_italic, palette);
            let mut cell = TableCell::new().add_paragraph(paragraph);
            if striped {
                cell = cell.shading(fill(palette.highlight));
            }
            cell
        })
        .collect();
    TableRow::new(docx_cells)
}

/// The paragraph for a single cell. Missing/blank cells are empty; a fill-in
/// cell becomes an underscore run to write on.
fn cell_paragraph(
    cell: Option<&Cell>,
    align: Align,
    size: f32,
    base_bold: bool,
    base_italic: bool,
    palette: &Palette,
) -> Paragraph {
    let paragraph = Paragraph::new().align(alignment(align));
    match cell {
        Some(Cell::Text(inlines)) => inlines.iter().fold(paragraph, |p, inline| {
            p.add_run(build_run(inline, size, base_bold, base_italic, palette))
        }),
        Some(Cell::FillIn) => {
            paragraph.add_run(Run::new().add_text("__________").size(half_points(size)))
        }
        _ => paragraph,
    }
}

/// A borderless three-column table (left / center / right) representing one row
/// of page chrome, or `None` when the chrome is empty.
fn chrome_row(chrome: &Chrome, theme: &Theme) -> Option<DocxTable> {
    if chrome.left.is_none() && chrome.center.is_none() && chrome.right.is_none() {
        return None;
    }
    let cell = |content: &Option<SectionContent>, align| {
        let paragraph = Paragraph::new().align(align);
        let paragraph = match content {
            Some(content) => section_runs(content, theme)
                .into_iter()
                .fold(paragraph, Paragraph::add_run),
            None => paragraph,
        };
        TableCell::new().add_paragraph(paragraph)
    };
    let row = TableRow::new(vec![
        cell(&chrome.left, AlignmentType::Left),
        cell(&chrome.center, AlignmentType::Center),
        cell(&chrome.right, AlignmentType::Right),
    ]);
    Some(DocxTable::without_borders(vec![row]).width(content_twips(theme), WidthType::Dxa))
}

/// Translate one chrome section into runs. A page counter becomes a `PAGE` and
/// `NUMPAGES` field pair rather than the model's `(page, total)` closure.
fn section_runs(content: &SectionContent, theme: &Theme) -> Vec<Run> {
    let size = theme.font_size.chrome;
    let palette = &theme.palette;
    match content {
        SectionContent::Text(text) => {
            vec![Run::new().add_text(text.clone()).size(half_points(size))]
        }
        SectionContent::Spans(spans) => spans
            .iter()
            .map(|inline| build_run(inline, size, false, false, palette))
            .collect(),
        SectionContent::PageCounter(_) => vec![
            field_run(InstrText::PAGE(InstrPAGE::new()), size),
            Run::new().add_text(" / ").size(half_points(size)),
            field_run(InstrText::NUMPAGES(InstrNUMPAGES::new()), size),
        ],
    }
}

/// A run holding a single Word field (e.g. `PAGE`). Word computes the value.
fn field_run(instr: InstrText, size: f32) -> Run {
    Run::new()
        .size(half_points(size))
        .add_field_char(FieldCharType::Begin, false)
        .add_instr_text(instr)
        .add_field_char(FieldCharType::Separate, false)
        .add_field_char(FieldCharType::End, false)
}

/// Map our column alignment to Word's.
fn alignment(align: Align) -> AlignmentType {
    match align {
        Align::Left => AlignmentType::Left,
        Align::Center => AlignmentType::Center,
        Align::Right => AlignmentType::Right,
    }
}

/// Solid cell shading in the given color.
fn fill(color: rgb::Color) -> Shading {
    Shading::new().shd_type(ShdType::Clear).fill(hex(color))
}

/// The content width in twips (1/20 pt), used to size full-width tables.
fn content_twips(theme: &Theme) -> usize {
    (theme.page.content_width() * 20.0).round().max(0.0) as usize
}

/// Base line spacing from the theme's leading, with no space before or after.
/// Callers add `.before(..)`/`.after(..)` for inter-block gaps.
fn line_spacing(theme: &Theme) -> LineSpacing {
    LineSpacing::new()
        .line_rule(LineSpacingType::Auto)
        .line((theme.spacing.line_height * 240.0).round() as i32)
}

/// Line spacing for an ordinary block: theme leading plus the inter-block gap
/// below it.
fn block_spacing(theme: &Theme) -> LineSpacing {
    line_spacing(theme).after(twips(theme.spacing.block))
}

/// Convert a length in points to twips (1/20 pt), Word's spacing/indent unit.
fn twips(points: f32) -> u32 {
    (points * 20.0).round().max(0.0) as u32
}

/// Convert a point size to the half-points Word measures font size in.
fn half_points(points: f32) -> usize {
    (points * 2.0).round().max(0.0) as usize
}

/// Format a color as an uppercase `RRGGBB` hex string (no leading `#`).
fn hex(color: rgb::Color) -> String {
    format!(
        "{:02X}{:02X}{:02X}",
        color.red(),
        color.green(),
        color.blue()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        build::{Textris, bold, mono},
        model::ListMarker,
    };

    /// A `.docx` is a ZIP archive; every one starts with the `PK` local-file
    /// signature. A non-trivial byte count means content was actually written.
    fn assert_is_docx(bytes: &[u8]) {
        assert_eq!(&bytes[..2], b"PK", "docx output must be a ZIP archive");
        assert!(bytes.len() > 1000, "docx output looks empty");
    }

    #[test]
    fn exports_a_document_covering_every_block() {
        let mut doc = Textris::new();
        doc.h1("Title");
        doc.h3_numbered("Section").anchor("s");
        doc.paragraph(bold("Bold").normal(" and ").italic("italic"));
        doc.paragraph(mono("code()"));
        doc.bullet_list(["one", "two"]);
        doc.ordered_list(["first", "second"]);
        doc.task_list([(true, "done"), (false, "todo")]);
        doc.table(["a", "b"], [["1", "2"], ["3", "4"]]);
        doc.boxed(|b| {
            b.paragraph("inside a callout");
        });
        doc.page_break();
        doc.spacer(20.0);
        doc.footer_center("Confidential");

        let bytes = doc.to_docx().expect("export succeeds");
        assert_is_docx(&bytes);
    }

    #[test]
    fn ordered_list_markers_use_the_model_labels() {
        let mut doc = Textris::new();
        doc.ordered_list_with(ListMarker::LowerAlpha, ["alpha", "beta"]);
        // Just exercises the LowerAlpha branch end-to-end.
        assert_is_docx(&doc.to_docx().unwrap());
    }

    #[test]
    fn empty_document_still_produces_a_valid_file() {
        assert_is_docx(&Textris::new().to_docx().unwrap());
    }

    #[test]
    fn hex_formats_components() {
        assert_eq!(hex(rgb::Color::new(0, 0, 0)), "000000");
        assert_eq!(hex(rgb::Color::new(0xF6, 0x0A, 0xFF)), "F60AFF");
    }
}

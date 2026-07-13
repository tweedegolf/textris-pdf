//! Optional Markdown export, gated behind the `markdown` cargo feature.
//!
//! This translates the layout-agnostic [`Document`] model into a
//! GitHub-flavored Markdown string. Like the [`docx`](crate::docx) export it is
//! a *structural* translation: it reproduces the document's content and coarse
//! structure (headings, paragraphs, tables, lists and callout boxes) but not
//! the pixel-level styling of the PDF renderer.
//!
//! Markdown cannot express everything the model holds, so a few encoding
//! choices are made:
//!
//! - **Emphasis** maps to `**bold**`, `*italic*`, `***bold italic***` and
//!   `` `mono` `` (an inline code span). Inline [`color`](Inline::color) has no
//!   portable Markdown form and is dropped.
//! - **Text** is backslash-escaped so Markdown metacharacters render literally
//!   rather than as accidental formatting. Content inside a `` `mono` `` run is
//!   emitted verbatim (Markdown does not escape inside code spans).
//! - **Hard line breaks** (a `'\n'` within a run) become Markdown hard breaks
//!   (a line ending in two spaces).
//! - **Tables** become GitHub-flavored tables, keeping per-column alignment in
//!   the delimiter row. A [`Cell::FillIn`] becomes an underscore run; blank and
//!   spacer cells are empty. A table with no header row (a label table) still
//!   gets an empty header, which GitHub-flavored Markdown requires.
//! - **Ordered lists** always number with `1.`, `2.`, … Markdown has no lettered
//!   list, so a [`ListMarker::LowerAlpha`](crate::model::ListMarker) list is
//!   numbered too.
//! - **Callout [`Box`](Block::Box)es** become blockquotes (`>`).
//! - **Headings are not numbered**: even a numbered heading renders as plain
//!   text (Markdown renderers commonly number headings themselves).
//!   [`section_ref`](Inline::section_ref) references resolve to the referenced
//!   heading's title in double quotes (e.g. `"Vision"`) rather than a number,
//!   so [`Textris::to_markdown`](crate::build::Textris::to_markdown) uses
//!   [`Document::resolve_sections_unnumbered`](crate::model::Document::resolve_sections_unnumbered).
//! - The page **footer** is appended at the bottom of the document, below a
//!   `---` rule, one section per line. Page counters have no meaning without
//!   pages and are dropped. The page **header**, page breaks and spacers have
//!   no Markdown equivalent and are dropped.
//!
//! The entry point is [`to_markdown`]; the builder also exposes
//! [`Textris::to_markdown`](crate::build::Textris::to_markdown) and
//! [`Textris::write_markdown_to_file`](crate::build::Textris::write_markdown_to_file).

use crate::{
    model::{Block, Cell, Chrome, Document, Inline, SectionContent, Table},
    theme::Align,
};

/// Translate a [`Document`] into a GitHub-flavored Markdown string.
///
/// The document is translated as-is; references are *not* resolved here (use
/// [`Textris::to_markdown`](crate::build::Textris::to_markdown), which resolves
/// them first, or call
/// [`Document::resolve_sections_unnumbered`](crate::model::Document::resolve_sections_unnumbered)
/// yourself).
pub fn to_markdown(document: &Document) -> String {
    let mut parts = Vec::new();
    let body = render_blocks(&document.blocks);
    if !body.is_empty() {
        parts.push(body);
    }
    if let Some(footer) = render_footer(&document.footer) {
        parts.push(footer);
    }
    let markdown = parts.join("\n\n");
    if markdown.is_empty() {
        markdown
    } else {
        format!("{markdown}\n")
    }
}

/// Render the page footer as a trailing block: a `---` rule followed by each
/// non-empty section on its own line. Returns `None` when the footer holds no
/// static content to show.
fn render_footer(footer: &Chrome) -> Option<String> {
    let sections: Vec<String> = [&footer.left, &footer.center, &footer.right]
        .into_iter()
        .flatten()
        .filter_map(section_text)
        .filter(|section| !section.trim().is_empty())
        .collect();
    if sections.is_empty() {
        return None;
    }
    Some(format!("---\n\n{}", sections.join("  \n")))
}

/// Render one footer/header section to inline Markdown. A page counter has no
/// pages to count without pagination, so it yields `None` and is dropped.
fn section_text(content: &SectionContent) -> Option<String> {
    match content {
        SectionContent::Text(text) => Some(escape(text)),
        SectionContent::Spans(spans) => Some(render_inlines(spans)),
        SectionContent::PageCounter(_) => None,
    }
}

/// Render a sequence of blocks, one blank line between them. Blocks with no
/// Markdown form (page breaks, spacers) drop out.
fn render_blocks(blocks: &[Block]) -> String {
    blocks
        .iter()
        .filter_map(render_block)
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Render one block, or `None` for a block that has no Markdown representation.
fn render_block(block: &Block) -> Option<String> {
    let markdown = match block {
        Block::Heading { level, content, .. } => {
            let hashes = "#".repeat((*level).clamp(1, 6) as usize);
            format!("{hashes} {}", inline_line(content))
        }
        Block::Paragraph(inlines) => hard_breaks(&render_inlines(inlines)),
        Block::Table(table) => render_table(table),
        Block::TaskList(items) => items
            .iter()
            .map(|item| {
                let checkbox = if item.checked { "- [x]" } else { "- [ ]" };
                list_item(checkbox, &item.content)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Block::BulletList(items) => items
            .iter()
            .map(|item| list_item("-", item))
            .collect::<Vec<_>>()
            .join("\n"),
        Block::OrderedList { items, .. } => items
            .iter()
            .enumerate()
            .map(|(i, item)| list_item(&format!("{}.", i + 1), item))
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Box { content, .. } => blockquote(&render_blocks(content)),
        // No Markdown equivalent: a page break is a print-only concept and a
        // spacer's blank line is already implied by the block separation.
        Block::PageBreak | Block::Spacer(_) => return None,
    };
    Some(markdown)
}

/// A list item: `prefix` (the bullet, number or checkbox) then the item's text,
/// with any hard breaks continued at an indent that aligns past the marker.
fn list_item(prefix: &str, inlines: &[Inline]) -> String {
    let indent = " ".repeat(prefix.chars().count() + 1);
    let body = render_inlines(inlines).replace('\n', &format!("  \n{indent}"));
    format!("{prefix} {body}")
}

/// Prefix every line of `content` with a blockquote marker.
fn blockquote(content: &str) -> String {
    content
        .split('\n')
        .map(|line| {
            if line.is_empty() {
                ">".to_string()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render a model table as a GitHub-flavored Markdown table: a header row, a
/// delimiter row carrying per-column alignment, then the body rows. Rows are
/// padded so every row has the same number of cells.
fn render_table(table: &Table) -> String {
    let columns = table.columns().max(1);
    let style = &table.style;

    let header = if header_shown(table) {
        (0..columns)
            .map(|c| cell_text(table.headers.get(c)))
            .collect()
    } else {
        // A GitHub-flavored table always needs a header row, so a headerless
        // (label) table gets an empty one.
        vec![String::new(); columns]
    };

    let delimiter = (0..columns)
        .map(
            |c| match style.align.get(c).copied().unwrap_or(Align::Left) {
                Align::Left => "---".to_string(),
                Align::Center => ":---:".to_string(),
                Align::Right => "---:".to_string(),
            },
        )
        .collect::<Vec<_>>();

    let mut lines = vec![row_line(&header), row_line(&delimiter)];
    for row in &table.rows {
        let cells = (0..columns)
            .map(|c| cell_text(row.get(c)))
            .collect::<Vec<_>>();
        lines.push(row_line(&cells));
    }
    lines.join("\n")
}

/// Whether the table's header row is drawn: a header style with cells that are
/// not all blank, matching the PDF and docx exports.
fn header_shown(table: &Table) -> bool {
    table.style.header && !table.headers.is_empty() && !table.headers.iter().all(Cell::is_blank)
}

/// Wrap a row's cells in the `| a | b |` pipe syntax.
fn row_line(cells: &[String]) -> String {
    format!("| {} |", cells.join(" | "))
}

/// The Markdown for a single table cell. Missing, blank and spacer cells are
/// empty; a fill-in cell becomes an underscore run. Newlines become `<br>` and
/// pipes are escaped, as a cell must stay on one line and inside the table grid.
fn cell_text(cell: Option<&Cell>) -> String {
    match cell {
        Some(Cell::Text(inlines)) => escape_pipes(&render_inlines(inlines).replace('\n', "<br>")),
        Some(Cell::FillIn) => "________".to_string(),
        _ => String::new(),
    }
}

/// Render inlines for a context that must stay on one line (a heading): hard
/// breaks collapse to spaces.
fn inline_line(inlines: &[Inline]) -> String {
    render_inlines(inlines).replace('\n', " ")
}

/// Render a run of inlines to Markdown.
fn render_inlines(inlines: &[Inline]) -> String {
    inlines.iter().map(render_inline).collect()
}

/// Render one inline run: an inline code span for `mono`, otherwise the escaped
/// text wrapped in the run's emphasis markers.
fn render_inline(inline: &Inline) -> String {
    if inline.mono {
        return code_span(&inline.text);
    }
    let escaped = escape(&inline.text);
    let marker = match (inline.bold, inline.italic) {
        (true, true) => "***",
        (true, false) => "**",
        (false, true) => "*",
        (false, false) => return escaped,
    };
    // Emphasis markers must hug non-space text, so keep any surrounding
    // whitespace outside them.
    let start = escaped.len() - escaped.trim_start().len();
    let end = escaped.trim_end().len();
    if start >= end {
        return escaped;
    }
    format!(
        "{}{marker}{}{marker}{}",
        &escaped[..start],
        &escaped[start..end],
        &escaped[end..]
    )
}

/// Wrap `text` in an inline code span, choosing a backtick fence long enough to
/// contain any backticks inside it (and padding with spaces when the text would
/// otherwise touch the fence).
fn code_span(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    if text.starts_with('`') || text.ends_with('`') {
        format!("{fence} {text} {fence}")
    } else {
        format!("{fence}{text}{fence}")
    }
}

/// Backslash-escape the Markdown metacharacters that would otherwise trigger
/// inline formatting, so the text renders literally. Pipes are left alone here
/// and handled per-table-cell by [`escape_pipes`], since `|` is only special
/// inside a table.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(
            ch,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '~'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Escape any unescaped `|` as `\|`, which GitHub-flavored Markdown reads as a
/// literal pipe within a table cell (even inside a code span).
fn escape_pipes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut escaped = false;
    for ch in text.chars() {
        if ch == '|' && !escaped {
            out.push('\\');
        }
        out.push(ch);
        escaped = ch == '\\' && !escaped;
    }
    out
}

/// Turn `'\n'` hard breaks into Markdown hard breaks (a line ending in two
/// spaces).
fn hard_breaks(text: &str) -> String {
    text.replace('\n', "  \n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        build::{Textris, bold, cell, fill_in, mono, muted, text},
        model::{ListMarker, SectionContent},
    };

    #[test]
    fn headings_use_hash_prefixes() {
        let mut doc = Textris::new();
        doc.h1("Title");
        doc.h3("Section");
        let md = doc.to_markdown();
        assert!(md.contains("# Title"), "{md}");
        assert!(md.contains("### Section"), "{md}");
    }

    #[test]
    fn emphasis_maps_to_markers_and_hugs_whitespace() {
        let mut doc = Textris::new();
        doc.paragraph(text("a ").bold("b").normal(" ").italic("c").mono("d"));
        let md = doc.to_markdown();
        // The bold/italic markers sit against the words, not the spaces around
        // them, and the mono run is an inline code span.
        assert!(md.contains("a **b** *c*`d`"), "{md}");
    }

    #[test]
    fn metacharacters_are_escaped() {
        let mut doc = Textris::new();
        doc.paragraph("use *stars* and _underscores_ and a # hash");
        let md = doc.to_markdown();
        assert!(md.contains(r"\*stars\* and \_underscores\_"), "{md}");
        assert!(md.contains(r"\# hash"), "{md}");
    }

    #[test]
    fn mono_run_is_verbatim_inside_a_code_span() {
        let mut doc = Textris::new();
        // Emphasis characters inside a code span must not be escaped.
        doc.paragraph(mono("a*b_c"));
        assert!(doc.to_markdown().contains("`a*b_c`"));
    }

    #[test]
    fn code_span_grows_the_fence_past_inner_backticks() {
        assert_eq!(code_span("plain"), "`plain`");
        assert_eq!(code_span("a`b"), "``a`b``");
        assert_eq!(code_span("`x`"), "`` `x` ``");
    }

    #[test]
    fn hard_line_break_becomes_two_trailing_spaces() {
        let mut doc = Textris::new();
        doc.paragraph(text("one").line_break().normal("two"));
        assert!(doc.to_markdown().contains("one  \ntwo"));
    }

    #[test]
    fn table_carries_a_header_and_alignment_row() {
        let mut doc = Textris::new();
        doc.table(["a", "b"], [[text("1"), text("2")]]);
        let md = doc.to_markdown();
        assert!(md.contains("| a | b |"), "{md}");
        assert!(md.contains("| --- | --- |"), "{md}");
        assert!(md.contains("| 1 | 2 |"), "{md}");
    }

    #[test]
    fn label_table_still_emits_an_empty_header() {
        let mut doc = Textris::new();
        doc.label_table([[cell("Date"), fill_in()]]);
        let md = doc.to_markdown();
        // Header row present but empty, then the fill-in underscores.
        assert!(md.contains("|  |  |\n| --- | --- |"), "{md}");
        assert!(md.contains("| Date | ________ |"), "{md}");
    }

    #[test]
    fn pipes_in_cells_are_escaped() {
        let mut doc = Textris::new();
        doc.table(["a"], [[text("x | y")]]);
        assert!(doc.to_markdown().contains(r"| x \| y |"));
    }

    #[test]
    fn ordered_list_is_numeric_even_when_lettered() {
        let mut doc = Textris::new();
        doc.ordered_list_with(ListMarker::LowerAlpha, ["first", "second"]);
        let md = doc.to_markdown();
        assert!(md.contains("1. first"), "{md}");
        assert!(md.contains("2. second"), "{md}");
    }

    #[test]
    fn task_list_uses_checkboxes() {
        let mut doc = Textris::new();
        doc.task_list([(true, "done"), (false, "todo")]);
        let md = doc.to_markdown();
        assert!(md.contains("- [x] done"), "{md}");
        assert!(md.contains("- [ ] todo"), "{md}");
    }

    #[test]
    fn box_becomes_a_blockquote() {
        let mut doc = Textris::new();
        doc.boxed(|b| {
            b.paragraph(bold("Note."));
            b.paragraph("Body.");
        });
        let md = doc.to_markdown();
        assert!(md.contains("> **Note.**"), "{md}");
        // A blank quoted line keeps the two paragraphs in one blockquote.
        assert!(md.contains("> **Note.**\n>\n> Body."), "{md}");
    }

    #[test]
    fn page_breaks_and_spacers_are_dropped() {
        let mut doc = Textris::new();
        doc.paragraph("before");
        doc.page_break();
        doc.spacer(20.0);
        doc.paragraph("after");
        // Nothing between the two paragraphs but a single blank line.
        assert_eq!(doc.to_markdown(), "before\n\nafter\n");
    }

    #[test]
    fn headings_are_not_numbered_and_references_repeat_the_title() {
        let mut doc = Textris::new();
        doc.h3_numbered("Vision").anchor("vision");
        doc.paragraph(text("see ").section_ref("vision"));
        let md = doc.to_markdown();
        // The heading text stays plain, with no "1." prefix...
        assert!(md.contains("### Vision"), "{md}");
        assert!(!md.contains("### 1."), "{md}");
        // ...and the reference repeats the section title in quotes.
        assert!(md.contains(r#"see "Vision""#), "{md}");
    }

    #[test]
    fn footer_is_appended_at_the_bottom() {
        let mut doc = Textris::new();
        doc.paragraph("Body.");
        doc.footer_left(muted("Revision: ").mono("3"));
        // A page counter has no pages to count in Markdown and is dropped.
        doc.footer_right(SectionContent::page_counter(|page, total| {
            text(format!("Page {page} of {total}"))
        }));
        let md = doc.to_markdown();
        assert!(md.ends_with("---\n\nRevision: `3`\n"), "{md}");
        assert!(!md.contains("Page"), "page counter should be dropped: {md}");
    }

    #[test]
    fn document_without_a_footer_has_no_trailing_rule() {
        let mut doc = Textris::new();
        doc.paragraph("Body.");
        assert_eq!(doc.to_markdown(), "Body.\n");
    }
}

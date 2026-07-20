//! End-to-end checks that the rendered PDF is genuinely tagged and accessible.
//!
//! The renderer targets PDF/A-2A + PDF/UA-1, and krilla fails serialization on
//! any conformance violation — so a successful `render` already proves the
//! document validates. These tests additionally assert that the accessibility
//! scaffolding (structure tree, marked content, title/language metadata,
//! outline) is present in the output bytes, and that the title falls back to the
//! first heading when none is set explicitly.

use std::path::Path;

use textris_pdf::{build::Textris, fonts::Fonts};

fn load_fonts() -> Fonts {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fonts");
    Fonts::from_variable_files(
        dir.join("Newsreader/Newsreader-Variable.ttf"),
        dir.join("Newsreader/Newsreader-Italic-Variable.ttf"),
        dir.join("Fira_Code/FiraCode-Variable.ttf"),
    )
    .expect("test fonts should load")
}

/// Whether `needle` occurs in `haystack` (bytes).
fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

#[test]
fn rendered_pdf_is_tagged_and_carries_accessibility_metadata() {
    let fonts = load_fonts();
    let mut doc = Textris::new();
    doc.title("Accessible Report").language("en-GB");
    doc.h1("Overview");
    doc.paragraph("A short paragraph.");
    doc.bullet_list(["first point", "second point"]);
    doc.table(["name", "value"], [["speed", "23 m/s"]]);

    let pdf = doc
        .render(&fonts)
        .expect("should render a valid tagged PDF");

    // Marked/tagged document with a structure tree and role-mapped elements
    // (these dictionaries are written uncompressed).
    assert!(contains(&pdf, "StructTreeRoot"), "no structure tree");
    assert!(
        contains(&pdf, "/Marked true"),
        "document is not marked tagged"
    );
    assert!(contains(&pdf, "pdfuaid"), "no PDF/UA identifier");
    assert!(
        contains(&pdf, "DisplayDocTitle"),
        "title not shown by viewers"
    );
    assert!(contains(&pdf, "Outlines"), "no outline");
    // Table and list structure elements.
    assert!(
        contains(&pdf, "/TH") && contains(&pdf, "/TD"),
        "no table cells"
    );
    assert!(contains(&pdf, "/LBody"), "no list bodies");
    // The title and language reach the metadata.
    assert!(contains(&pdf, "Accessible Report"), "title missing");
    assert!(contains(&pdf, "en-GB"), "language missing");
}

#[test]
fn the_title_falls_back_to_the_first_heading() {
    let fonts = load_fonts();
    let mut doc = Textris::new();
    // No explicit title set.
    doc.h1("Fallback Heading Title");
    doc.paragraph("Body.");

    let pdf = doc.render(&fonts).expect("should render");
    assert!(
        contains(&pdf, "Fallback Heading Title"),
        "title should fall back to the first heading"
    );
}

#[test]
fn a_table_cell_spanning_multiple_pages_still_renders_accessibly() {
    let fonts = load_fonts();
    let mut doc = Textris::new();
    doc.title("Split row");
    // One table cell that is far taller than a page, so its structure element
    // collects marked content from several pages.
    doc.table(["notes"], [["lorem ipsum ".repeat(1200)]]);

    let pdf = doc
        .render(&fonts)
        .expect("a split table row should still validate as PDF/A-2A + PDF/UA-1");
    assert!(contains(&pdf, "StructTreeRoot"));
}

#[test]
fn chrome_handles_hard_breaks_and_fill_ins() {
    use textris_pdf::build::text;
    let fonts = load_fonts();
    let mut doc = Textris::new();
    doc.h1("Chrome");
    // A hard break folds to a space (chrome is a single line); a fill-in run
    // draws its blank line instead of failing shaping.
    doc.header_left(text("line one").line_break().normal("line two"));
    doc.footer_left(text("Signature: ").fill_in(80.0));
    doc.render(&fonts)
        .expect("chrome with hard breaks and fill-ins should render");
}

#[test]
fn an_empty_heading_is_skipped_rather_than_fatal() {
    let fonts = load_fonts();
    let mut doc = Textris::new();
    doc.title("Empty heading");
    doc.h1("");
    doc.paragraph("Body.");
    doc.render(&fonts)
        .expect("an empty heading must not fail PDF/UA validation");
}

#[test]
fn an_invalid_page_size_is_an_error_not_a_panic() {
    use textris_pdf::render::RenderError;
    let fonts = load_fonts();
    let mut doc = Textris::new();
    doc.paragraph("Body.");
    doc.theme_mut().page.width = 0.0;
    let error = doc.render(&fonts).expect_err("zero page width must fail");
    assert!(matches!(error, RenderError::InvalidPageSize { .. }));
}

#[test]
fn stressed_tables_still_render_accessibly() {
    let fonts = load_fonts();
    let mut doc = Textris::new();
    doc.title("Table stress");
    // A header row taller than a page: split across pages, not repeated.
    doc.table([format!("heading {}", "word ".repeat(1500))], [["body"]]);
    // Many narrow columns on one page.
    doc.table(
        (0..40).map(|c| format!("h{c}")),
        [(0..40).map(|c| format!("v{c}"))],
    );
    let pdf = doc
        .render(&fonts)
        .expect("stressed tables should still validate as PDF/A-2A + PDF/UA-1");
    assert!(contains(&pdf, "StructTreeRoot"));
}

#[test]
fn a_document_without_headings_still_renders_accessibly() {
    let fonts = load_fonts();
    let mut doc = Textris::new();
    // No headings at all: the outline (required by PDF/UA) must still be
    // produced from a synthesized fallback entry, and export must succeed.
    doc.paragraph("Just a paragraph, no headings anywhere.");

    let pdf = doc
        .render(&fonts)
        .expect("a heading-less document should still validate");
    assert!(contains(&pdf, "StructTreeRoot"));
    assert!(
        contains(&pdf, "Outlines"),
        "a fallback outline should exist"
    );
}

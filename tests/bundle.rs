//! End-to-end checks for appending several documents into one PDF with
//! [`Bundle`]: each document keeps its own chrome, page counter, theme and
//! section numbering, and the combined file still validates as PDF/A-2A +
//! PDF/UA-1 (krilla fails serialization otherwise, so a successful render is
//! the conformance check).
//!
//! The first test also writes `tests/bundle-example.pdf` for inspection.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use textris_pdf::{
    build::{Bundle, Textris, text},
    fonts::Fonts,
    model::SectionContent,
    render::RenderError,
};

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

/// How many times `needle` occurs in `haystack` (bytes, non-overlapping).
fn count(haystack: &[u8], needle: &str) -> usize {
    let mut count = 0;
    let mut rest = haystack;
    while let Some(at) = rest
        .windows(needle.len())
        .position(|w| w == needle.as_bytes())
    {
        count += 1;
        rest = &rest[at + needle.len()..];
    }
    count
}

/// A page counter that also records every `(page, total)` it was asked to
/// render, so a test can see how each document's pages were numbered.
fn recording_counter(log: &Arc<Mutex<Vec<(usize, usize)>>>) -> SectionContent {
    let log = Arc::clone(log);
    SectionContent::page_counter(move |page, total| {
        log.lock().expect("no poisoned lock").push((page, total));
        text(format!("Page {page} of {total}"))
    })
}

/// A document of `pages` full pages: the `label` as a level-1 heading and a
/// running header, then page-break-separated filler.
fn document(label: &str, pages: usize) -> Textris {
    let mut doc = Textris::new();
    doc.title(format!("{label} title"));
    doc.header_left(format!("{label} header"));
    doc.h1(label);
    doc.h3_numbered("Intro");
    doc.paragraph("Body.");
    for page in 1..pages {
        doc.page_break();
        doc.h3_numbered(format!("Section on page {}", page + 1));
        doc.paragraph("More body.");
    }
    doc
}

#[test]
fn appended_documents_render_into_one_valid_pdf_written_to_disk() {
    let fonts = load_fonts();
    let mut bundle = Bundle::new();
    bundle.title("Two field guides in one file").language("en");
    bundle
        .push(document("Guide A", 2))
        .push(document("Guide B", 3));

    let pdf = bundle
        .render(&fonts)
        .expect("a bundle should render as valid tagged PDF/A-2A + PDF/UA-1");

    // Written out first, so it can be inspected even when a check below fails.
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/bundle-example.pdf");
    std::fs::write(&out, &pdf).expect("should write PDF to disk");
    assert!(out.exists());

    assert!(pdf.starts_with(b"%PDF-"), "output is not a PDF");
    // All pages of both documents, in one file. Page objects are written as
    // uncompressed dictionaries (`/Type/Pages` is the tree node, not a page).
    assert_eq!(
        count(&pdf, "/Type/Page/"),
        5,
        "expected the pages of both documents"
    );
    // Accessibility scaffolding is present, and the bundle's title and
    // language reached the metadata.
    assert!(contains(&pdf, "StructTreeRoot"), "no structure tree");
    assert!(contains(&pdf, "pdfuaid"), "no PDF/UA identifier");
    assert!(contains(&pdf, "Outlines"), "no outline");
    assert!(
        contains(&pdf, "Two field guides in one file"),
        "bundle title missing"
    );
    // Each appended document is a Part of the structure tree.
    assert_eq!(
        count(&pdf, "/S/Part"),
        2,
        "each document should be tagged as a Part"
    );
}

#[test]
fn page_counters_restart_for_every_document() {
    let fonts = load_fonts();
    let first_log = Arc::new(Mutex::new(Vec::new()));
    let second_log = Arc::new(Mutex::new(Vec::new()));

    let mut first = document("First", 2);
    first.footer_right(recording_counter(&first_log));
    let mut second = document("Second", 3);
    second.footer_right(recording_counter(&second_log));

    let mut bundle = Bundle::new();
    bundle.push(first).push(second);
    bundle.render(&fonts).expect("should render");

    // Each document's counter sees only its own pages: numbered from 1, with
    // its own page count as the total - not 1..=5 of 5.
    assert_eq!(
        first_log.lock().unwrap().as_slice(),
        [(1, 2), (2, 2)],
        "the first document counts its own two pages"
    );
    assert_eq!(
        second_log.lock().unwrap().as_slice(),
        [(1, 3), (2, 3), (3, 3)],
        "the second document restarts at page 1 of 3"
    );
}

#[test]
fn section_numbering_restarts_for_every_document() {
    let fonts = load_fonts();
    let mut bundle = Bundle::new();
    bundle
        .push(document("First", 2))
        .push(document("Second", 1));
    let pdf = bundle.render(&fonts).expect("should render");

    // Headings reach the PDF as outline (bookmark) and structure titles, so
    // the numbering is visible in the bytes: "1. Intro" once per document,
    // and the first document's second section is "2.", not "3.".
    assert_eq!(
        count(&pdf, "1. Intro"),
        2 * count(&pdf, "2. Section on page 2"),
        "both documents should start their numbering at 1"
    );
    assert!(
        !contains(&pdf, "3. "),
        "the second document's sections must not continue the first's count"
    );
}

#[test]
fn documents_may_use_different_themes_and_page_sizes() {
    let fonts = load_fonts();
    let portrait = document("Portrait", 1);
    let mut landscape = document("Landscape", 1);
    {
        let page = &mut landscape.theme_mut().page;
        std::mem::swap(&mut page.width, &mut page.height);
    }
    landscape.theme_mut().spacing.line_height = 1.6;

    let mut bundle = Bundle::new();
    bundle.push(portrait).push(landscape);
    let pdf = bundle
        .render(&fonts)
        .expect("mixed page sizes should render");
    // Both media boxes are present: A4 portrait and A4 landscape.
    assert!(
        contains(&pdf, "595.276 841.89") && contains(&pdf, "841.89 595.276"),
        "expected both a portrait and a landscape page"
    );
}

#[test]
fn metadata_falls_back_to_the_first_document() {
    let fonts = load_fonts();
    let mut first = document("First", 1);
    first.title("First document's own title").language("nl");
    let second = document("Second", 1);

    // Nothing set on the bundle: the first document's title and language win.
    let mut bundle = Bundle::new();
    bundle.push(&first).push(&second);
    let pdf = bundle.render(&fonts).expect("should render");
    assert!(contains(&pdf, "First document's own title"));
    assert!(
        contains(&pdf, "/Lang(nl)"),
        "language should be the first document's"
    );

    // The bundle's own title takes precedence.
    bundle.title("The bundle's title");
    let pdf = bundle.render(&fonts).expect("should render");
    assert!(contains(&pdf, "The bundle's title"));
}

#[test]
fn an_empty_bundle_is_an_error_not_a_panic() {
    let fonts = load_fonts();
    let error = Bundle::new()
        .render(&fonts)
        .expect_err("nothing to render must fail");
    assert_eq!(error, RenderError::NoDocuments);
}

#[test]
fn a_single_document_bundle_matches_rendering_the_document_alone() {
    let fonts = load_fonts();
    let mut doc = document("Alone", 2);
    doc.created_at(1_700_000_000);

    let alone = doc.render(&fonts).expect("should render");
    let mut bundle = Bundle::new();
    bundle.push(&doc);
    let bundled = bundle.render(&fonts).expect("should render");
    assert_eq!(
        alone, bundled,
        "a bundle of one document is the document rendered on its own"
    );
    assert!(
        !contains(&bundled, "/S/Part"),
        "a lone document is not wrapped in a Part"
    );
}

#[test]
fn pinning_the_creation_date_makes_a_bundle_reproducible() {
    let fonts = load_fonts();
    let render = || {
        let mut bundle = Bundle::new();
        bundle.created_at(1_700_000_000);
        bundle.push(document("A", 1)).push(document("B", 1));
        bundle.render(&fonts).expect("should render")
    };
    assert_eq!(render(), render());
    assert!(contains(&render(), "D:20231114221320"));
}

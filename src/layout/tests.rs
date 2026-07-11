use super::*;
use crate::{build::Textris, fonts::test_fonts, theme::Theme};

fn texts(page: &Page) -> Vec<&TextElement> {
    page.elements
        .iter()
        .filter_map(|e| match e {
            Element::Text(t) => Some(t),
            _ => None,
        })
        .collect()
}

#[test]
fn simple_document_lays_out_on_one_page() {
    let fonts = test_fonts();
    let mut doc = Textris::new();
    doc.h1("Title");
    doc.paragraph("A short paragraph.");
    let pages = layout(&doc.build(), &fonts);
    assert_eq!(pages.len(), 1);
    assert!(!pages[0].elements.is_empty());
}

#[test]
fn text_stays_within_the_content_box() {
    let fonts = test_fonts();
    let theme = Theme::default();
    let mut doc = Textris::new();
    doc.h1("Title");
    doc.h3("1. Section");
    doc.paragraph("Some body text here.");
    let pages = layout(&doc.build(), &fonts);
    for page in &pages {
        for text in texts(page) {
            assert!(text.x >= theme.page.content_left() - 0.01, "x={}", text.x);
            assert!(text.baseline >= theme.page.content_top());
            assert!(text.baseline <= theme.page.content_bottom() + 0.01);
        }
    }
}

#[test]
fn long_paragraph_wraps_onto_multiple_lines() {
    let fonts = test_fonts();
    let mut doc = Textris::new();
    doc.paragraph("word ".repeat(200));
    let pages = layout(&doc.build(), &fonts);
    let baselines: std::collections::BTreeSet<_> = pages
        .iter()
        .flat_map(|p| texts(p))
        .map(|t| (t.baseline * 100.0) as i64)
        .collect();
    assert!(baselines.len() > 1, "expected multiple lines");
}

#[test]
fn content_overflowing_a_page_starts_a_new_one() {
    let fonts = test_fonts();
    // Many task-list items force a page break.
    let mut doc = Textris::new();
    doc.h1("T");
    doc.task_list((0..120).map(|i| (true, format!("item number {i}"))));
    let pages = layout(&doc.build(), &fonts);
    assert!(
        pages.len() >= 2,
        "expected pagination, got {} pages",
        pages.len()
    );
}

#[test]
fn table_cell_text_is_vertically_centered_on_its_cap_band() {
    use crate::fonts::Style;
    let fonts = test_fonts();
    let theme = Theme::default();
    let size = theme.font_size.body;

    // A single striped body row whose cell mixes caps, ascenders and descenders.
    let mut doc = Textris::new();
    doc.table(["H"], [["Agpy"]]);
    let pages = layout(&doc.build(), &fonts);

    // The striped body row's rectangle gives the row's vertical extent.
    let (top, height) = pages[0]
        .elements
        .iter()
        .find_map(|e| match e {
            Element::Rect { y, h, .. } => Some((*y, *h)),
            _ => None,
        })
        .expect("striped body row");

    // The body cell's baseline.
    let baseline = texts(&pages[0])
        .into_iter()
        .find(|t| t.text == "Agpy")
        .expect("body cell text")
        .baseline;

    // The cap-height band (cap top -> baseline) is centered in the row, so the
    // text is optically centered rather than riding high in the cell.
    let cap = fonts.cap_height(Style::Regular, size);
    let cap_center = baseline - cap / 2.0;
    let row_center = top + height / 2.0;
    assert!(
        (cap_center - row_center).abs() < 0.01,
        "cap band center ({cap_center}) should sit at the row center ({row_center})"
    );
}

#[test]
fn data_table_stripes_alternate_rows() {
    let fonts = test_fonts();
    let mut doc = Textris::new();
    doc.table(["h"], [["r0"], ["r1"], ["r2"], ["r3"]]);
    let pages = layout(&doc.build(), &fonts);
    let rects = pages[0]
        .elements
        .iter()
        .filter(|e| matches!(e, Element::Rect { .. }))
        .count();
    // Four body rows -> rows 0 and 2 are striped.
    assert_eq!(rects, 2);
}

#[test]
fn column_widths_follow_content_not_an_equal_split() {
    let fonts = test_fonts();
    let theme = Theme::default();
    let engine = Engine::new(&fonts, &theme);
    let mut doc = Textris::new();
    doc.table(
        ["", "common name", "species", "strike", "region"],
        [
            [
                "1",
                "PeacockPeacockPeacockPeacockPeacock",
                "Odontodactylus scyllarus",
                "smasher",
                "Indo-Pacific",
            ],
            [
                "2",
                "Zebra",
                "Lysiosquillina maculata",
                "spearer",
                "Indo-Pacific",
            ],
        ],
    );
    let d = doc.build();
    let crate::model::Block::Table(t) = &d.blocks[0] else {
        panic!("expected a table");
    };
    let widths = engine.column_widths(t, t.columns(), theme.page.content_width());

    // Columns still fill the full content width exactly.
    let sum: f32 = widths.iter().sum();
    assert!(
        (sum - theme.page.content_width()).abs() < 0.5,
        "columns should fill the content width, got {sum}"
    );

    // The content-heavy "common name" column must be clearly wider than the
    // sparse "strike" column.
    assert!(
        widths[1] > widths[3] * 1.5,
        "common name ({}) should be much wider than strike ({})",
        widths[1],
        widths[3]
    );

    // And "common name" must be wide enough to hold its long word without breaking.
    assert!(
        widths[1] >= engine.min_column_width(t, 1) + 2.0 * theme.table.inset_x,
        "common name should fit its widest word on one line"
    );
}

#[test]
fn table_style_overrides_font_size_and_row_height() {
    use crate::theme::TableStyle;

    let fonts = test_fonts();
    let theme = Theme::default();

    // A table whose cells use a larger font and a taller minimum row.
    let big = TableStyle {
        font_size: Some(theme.font_size.body * 2.0),
        row_min_height: Some(theme.table.row_min_height * 3.0),
        ..TableStyle::data()
    };
    let mut doc = Textris::new();
    doc.table_styled(&big, ["h"], [["cell"]]);
    let pages = layout(&doc.build(), &fonts);

    // Cell text is drawn at the overridden size.
    let has_big_text = texts(&pages[0])
        .iter()
        .any(|t| t.text == "cell" && (t.size - theme.font_size.body * 2.0).abs() < 0.01);
    assert!(
        has_big_text,
        "cell text should use the overridden font size"
    );

    // The taller row_min_height produces a taller striped rect than the
    // default style would for the same content.
    let plain = TableStyle::data();
    let mut plain_doc = Textris::new();
    plain_doc.table_styled(&plain, ["h"], [["cell"]]);
    let plain_pages = layout(&plain_doc.build(), &fonts);

    let row_h = |pages: &[Page]| {
        pages[0]
            .elements
            .iter()
            .filter_map(|e| match e {
                Element::Rect { h, .. } => Some(*h),
                _ => None,
            })
            .fold(0.0_f32, f32::max)
    };
    assert!(
        row_h(&pages) > row_h(&plain_pages),
        "overridden row_min_height should make rows taller"
    );
}

#[test]
fn custom_column_widths_honor_absolute_and_split_fractions() {
    use crate::theme::{ColumnWidth, ColumnWidths, TableStyle};

    let fonts = test_fonts();
    let theme = Theme::default();
    let engine = Engine::new(&fonts, &theme);

    // Four columns: content-sized, a 3:7 fractional split, and a fixed 120pt.
    let style = TableStyle {
        columns: ColumnWidths::custom([
            ColumnWidth::Auto,
            ColumnWidth::Fraction(3),
            ColumnWidth::Fraction(7),
            ColumnWidth::Absolute(120.0),
        ]),
        ..TableStyle::data()
    };
    let mut doc = Textris::new();
    doc.table_styled(&style, ["a", "b", "c", "d"], [["1", "22", "333", "4444"]]);
    let d = doc.build();
    let crate::model::Block::Table(t) = &d.blocks[0] else {
        panic!("expected a table");
    };
    let total = theme.page.content_width();
    let widths = engine.column_widths(t, t.columns(), total);

    // The absolute column is exactly its requested width.
    assert!(
        (widths[3] - 120.0).abs() < 0.01,
        "absolute column: {}",
        widths[3]
    );

    // The two fractional columns split the leftover 3:7.
    assert!(
        (widths[2] / widths[1] - 7.0 / 3.0).abs() < 0.01,
        "fraction split 3:7, got {} : {}",
        widths[1],
        widths[2]
    );

    // The auto column sizes to its (small) content, not a fraction share.
    assert!(
        widths[0] < widths[1],
        "auto ({}) < fraction ({})",
        widths[0],
        widths[1]
    );

    // All columns together fill the content width.
    let sum: f32 = widths.iter().sum();
    assert!(
        (sum - total).abs() < 0.5,
        "columns should fill width, got {sum}"
    );
}

#[test]
fn column_alignment_places_cell_text_left_center_and_right() {
    use crate::{
        fonts::Style,
        theme::{Align, ColumnWidth, ColumnWidths, TableStyle},
    };

    let fonts = test_fonts();
    let theme = Theme::default();
    let size = theme.font_size.body;

    // Three equal fractional columns so the column edges are known exactly.
    let style = TableStyle {
        header: false,
        striped: false,
        columns: ColumnWidths::custom([ColumnWidth::Fraction(1); 3]),
        align: vec![Align::Left, Align::Center, Align::Right],
        ..TableStyle::data()
    };
    let mut doc = Textris::new();
    doc.table_styled(&style, ["", "", ""], [["ll", "cc", "rr"]]);
    let pages = layout(&doc.build(), &fonts);

    let w = theme.page.content_width() / 3.0;
    let inset = theme.table.inset_x;
    let x_of = |needle: &str| {
        texts(&pages[0])
            .into_iter()
            .find(|t| t.text == needle)
            .unwrap_or_else(|| panic!("cell {needle:?}"))
            .x
    };

    // Left: flush against the left inset (the default placement).
    let x0 = theme.page.content_left();
    assert!((x_of("ll") - (x0 + inset)).abs() < 0.01, "left column");

    // Center: the text midpoint sits at the cell midpoint.
    let cc_w = fonts.measure(Style::Regular, "cc", size);
    let expected = (x0 + w) + (w - cc_w) / 2.0;
    assert!((x_of("cc") - expected).abs() < 0.01, "center column");

    // Right: flush against the right inset.
    let rr_w = fonts.measure(Style::Regular, "rr", size);
    let expected = (x0 + 3.0 * w) - inset - rr_w;
    assert!((x_of("rr") - expected).abs() < 0.01, "right column");
}

#[test]
fn long_word_in_a_cell_wraps_instead_of_overflowing_the_column() {
    let fonts = test_fonts();
    let theme = Theme::default();
    let mut doc = Textris::new();
    // Two equal flex columns; a single unbreakable word far wider than one
    // column sits in the left cell.
    doc.table(["a", "b"], [["Supercalifragilisticexpialidocious", "b"]]);
    let pages = layout(&doc.build(), &fonts);

    // With two equal flex columns the left column's right edge is the midpoint
    // of the content box.
    let boundary = theme.page.content_left() + theme.page.content_width() / 2.0;

    // Text belonging to the left cell (x left of the boundary) must wrap onto
    // multiple lines and never spill past the boundary into the next column.
    let left_cell: Vec<_> = pages[0]
        .elements
        .iter()
        .filter_map(|e| match e {
            Element::Text(t) if t.x < boundary => Some(t),
            _ => None,
        })
        .collect();

    let baselines: std::collections::BTreeSet<_> = left_cell
        .iter()
        .map(|t| (t.baseline * 100.0) as i64)
        .collect();
    assert!(
        baselines.len() > 1,
        "the long word should wrap onto multiple lines, got {} line(s)",
        baselines.len()
    );

    for t in &left_cell {
        let right = t.x + fonts.measure(t.style, &t.text, t.size);
        assert!(
            right <= boundary + 0.01,
            "text overflows into the next column: right={right} boundary={boundary}"
        );
    }
}

#[test]
fn headings_are_never_orphaned_at_the_bottom_of_a_page() {
    let fonts = test_fonts();
    let theme = Theme::default();
    // Many short sections that together span several pages.
    let mut doc = Textris::new();
    doc.h1("Title");
    for n in 0..40 {
        doc.h3(format!("Section {n}"));
        doc.paragraph(format!("Some body text for section {n}."));
    }
    let pages = layout(&doc.build(), &fonts);
    assert!(pages.len() >= 2);

    for page in &pages {
        let bottom_most = texts(page)
            .into_iter()
            .max_by(|a, b| a.baseline.partial_cmp(&b.baseline).unwrap());
        if let Some(text) = bottom_most {
            // A heading (rendered at the heading size) must always be
            // followed by content on the same page, so it can never be the
            // last thing on a page.
            assert!(
                (text.size - theme.font_size.h3).abs() > 0.01,
                "a heading was left stranded at the bottom of a page"
            );
        }
    }
}

#[test]
fn short_section_is_pushed_to_next_page_instead_of_splitting() {
    let fonts = test_fonts();
    let theme = Theme::default();
    // A first section that nearly fills a page, then a short second section.
    let filler = "line of text ".repeat(6);
    let mut doc = Textris::new();
    doc.h1("Title");
    doc.h3("One");
    for _ in 0..30 {
        doc.paragraph(filler.clone());
    }
    doc.h3("Two");
    doc.paragraph("Short tail paragraph.");
    let pages = layout(&doc.build(), &fonts);

    // The final short section's heading and its paragraph must share a page.
    let heading_page = page_of(&pages, "Two", theme.font_size.h3);
    let body_page = page_of(&pages, "tail", theme.font_size.body);
    assert_eq!(
        heading_page, body_page,
        "the short section was split across pages"
    );
}

/// Index of the first page containing a word matching `needle` at `size`.
fn page_of(pages: &[Page], needle: &str, size: f32) -> Option<usize> {
    pages.iter().position(|page| {
        texts(page)
            .iter()
            .any(|t| (t.size - size).abs() < 0.01 && t.text.split(' ').any(|word| word == needle))
    })
}

#[test]
fn checked_task_item_draws_a_filled_box_and_check() {
    let fonts = test_fonts();
    let mut doc = Textris::new();
    doc.task_list([(true, "done")]);
    let pages = layout(&doc.build(), &fonts);
    let rects = pages[0]
        .elements
        .iter()
        .filter(|e| matches!(e, Element::Rect { .. }))
        .count();
    let strokes = pages[0]
        .elements
        .iter()
        .filter(|e| matches!(e, Element::Stroke { .. }))
        .count();
    assert_eq!(rects, 1, "filled checkbox background");
    assert_eq!(strokes, 1, "white check mark");
}

#[test]
fn task_checkbox_is_centered_on_the_text_cap_band() {
    use crate::fonts::Style;
    let fonts = test_fonts();
    let theme = Theme::default();
    let mut doc = Textris::new();
    doc.task_list([(true, "done")]);
    let pages = layout(&doc.build(), &fonts);

    let box_center = pages[0]
        .elements
        .iter()
        .find_map(|e| match e {
            Element::Rect { y, h, .. } => Some(y + h / 2.0),
            _ => None,
        })
        .expect("checkbox background");
    let label = texts(&pages[0])[0];
    let cap = fonts.cap_height(Style::Regular, theme.font_size.body);
    let band_center = label.baseline - cap / 2.0;
    assert!(
        (box_center - band_center).abs() < 0.01,
        "checkbox center {box_center} should match the text cap-band center {band_center}"
    );
}

#[test]
fn a_custom_theme_flows_through_layout() {
    let fonts = test_fonts();
    let mut theme = Theme::default();
    theme.page.margin_x = 120.0;
    let content_left = theme.page.content_left();

    let mut doc = Textris::with_theme(theme);
    doc.paragraph("Shifted by the custom margin.");
    let pages = layout(&doc.build(), &fonts);

    let first = texts(&pages[0])[0];
    assert!(
        (first.x - content_left).abs() < 0.01,
        "text should start at the custom content edge {content_left}, got {}",
        first.x
    );
}

#[test]
fn an_unstriped_table_style_draws_no_row_fills() {
    use crate::theme::TableStyle;
    let fonts = test_fonts();
    let plain = TableStyle {
        striped: false,
        ..TableStyle::data()
    };
    let mut doc = Textris::new();
    doc.table_styled(&plain, ["h"], [["r0"], ["r1"], ["r2"], ["r3"]]);
    let pages = layout(&doc.build(), &fonts);
    let rects = pages[0]
        .elements
        .iter()
        .filter(|e| matches!(e, Element::Rect { .. }))
        .count();
    assert_eq!(rects, 0, "an unstriped style should draw no row fills");
}

#[test]
fn a_box_draws_a_background_and_insets_its_content() {
    use crate::theme::BoxStyle;
    let fonts = test_fonts();
    let theme = Theme::default();
    let style = BoxStyle::callout();
    let mut doc = Textris::new();
    doc.boxed_styled(&style, |b| {
        b.paragraph(crate::build::bold("Handle with care."));
        b.paragraph("A large smasher can crack aquarium glass.");
    });
    let pages = layout(&doc.build(), &fonts);

    // Exactly one filled rectangle: the box background.
    let rects: Vec<_> = pages[0]
        .elements
        .iter()
        .filter_map(|e| match e {
            Element::Rect { x, y, w, h, .. } => Some((*x, *y, *w, *h)),
            _ => None,
        })
        .collect();
    assert_eq!(
        rects.len(),
        1,
        "the box should draw a single background fill"
    );
    let (bx, _by, bw, _bh) = rects[0];
    assert!(
        (bx - theme.page.content_left()).abs() < 0.01,
        "the box spans from the content edge, got x={bx}"
    );

    // Every glyph sits inside the background, inset by the padding.
    for t in texts(&pages[0]) {
        assert!(
            t.x >= bx + style.padding_x - 0.01,
            "text x={} should be padded from the box edge {bx}",
            t.x
        );
        assert!(
            t.x <= bx + bw - style.padding_x + 0.01,
            "text should stay within the padded box width"
        );
    }
}

#[test]
fn box_spacing_measures_to_the_visible_text_edges() {
    use crate::{fonts::Style, theme::BoxStyle};
    let fonts = test_fonts();
    let theme = Theme::default();
    let style = BoxStyle::callout();
    let mut doc = Textris::new();
    doc.boxed_styled(&style, |b| {
        b.h4("Please note");
        b.paragraph("Body text inside the box.");
    });
    let pages = layout(&doc.build(), &fonts);

    let (box_top, box_h) = pages[0]
        .elements
        .iter()
        .find_map(|e| match e {
            Element::Rect { y, h, .. } => Some((*y, *h)),
            _ => None,
        })
        .expect("the box should draw a background fill");
    let all = texts(&pages[0]);
    let title = all.iter().find(|t| t.text.contains("Please")).unwrap();
    let body = all.iter().find(|t| t.text.contains("Body")).unwrap();

    // The title's cap top sits one padding below the box's top edge.
    let cap_top = title.baseline - fonts.cap_height(Style::Regular, title.size);
    assert!(
        (cap_top - (box_top + style.padding_y)).abs() < 0.01,
        "title cap top {cap_top} should be padding below the box top {box_top}"
    );

    // The gap from the title's descender bottom to the body's cap top is
    // exactly the theme's heading_below spacing, with no stray leading.
    let title_bottom = title.baseline + fonts.descent(Style::Regular, title.size);
    let body_cap_top = body.baseline - fonts.cap_height(Style::Regular, body.size);
    assert!(
        (body_cap_top - title_bottom - theme.spacing.heading_below).abs() < 0.01,
        "visible heading gap {} should equal heading_below {}",
        body_cap_top - title_bottom,
        theme.spacing.heading_below
    );

    // The last line's descender bottom sits one padding above the box's
    // bottom edge.
    let body_bottom = body.baseline + fonts.descent(Style::Regular, body.size);
    assert!(
        (box_top + box_h - style.padding_y - body_bottom).abs() < 0.01,
        "body descender bottom {body_bottom} should be padding above the box bottom"
    );
}

#[test]
fn ordered_list_numbers_its_items() {
    use crate::model::ListMarker;
    let fonts = test_fonts();
    let mut doc = Textris::new();
    doc.ordered_list_with(ListMarker::LowerAlpha, ["first", "second", "third"]);
    let pages = layout(&doc.build(), &fonts);
    let markers: Vec<&str> = texts(&pages[0])
        .iter()
        .map(|t| t.text.as_str())
        .filter(|s| matches!(*s, "a." | "b." | "c."))
        .collect();
    assert_eq!(markers, ["a.", "b.", "c."]);
}

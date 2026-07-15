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
    for page in pages.iter() {
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
    let (min_content_width, _) = engine.column_metrics(t, 1);
    assert!(
        widths[1] >= min_content_width + 2.0 * theme.table.inset_x,
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

    for page in pages.iter() {
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
        (body_cap_top - title_bottom - theme.spacing.heading_below.level(4)).abs() < 0.01,
        "visible heading gap {} should equal heading_below {}",
        body_cap_top - title_bottom,
        theme.spacing.heading_below.level(4)
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
fn an_explicit_page_break_starts_a_new_page() {
    let fonts = test_fonts();
    let mut doc = Textris::new();
    // A leading page break on an empty page must not create a blank page.
    doc.page_break();
    doc.paragraph("First page.");
    doc.page_break();
    doc.paragraph("Second page.");
    let pages = layout(&doc.build(), &fonts);
    assert_eq!(pages.len(), 2);
    assert!(texts(&pages[0]).iter().any(|t| t.text.contains("First")));
    assert!(texts(&pages[1]).iter().any(|t| t.text.contains("Second")));
}

#[test]
fn a_spacer_is_exactly_the_distance_between_its_neighbours() {
    let fonts = test_fonts();
    let theme = Theme::default();
    let size = theme.font_size.body;
    let line_h = size * theme.spacing.line_height;
    let gap = 50.0;

    let mut doc = Textris::new();
    doc.paragraph("above");
    doc.spacer(gap);
    doc.paragraph("below");
    let pages = layout(&doc.build(), &fonts);

    let baseline_of = |needle: &str| {
        texts(&pages[0])
            .into_iter()
            .find(|t| t.text == needle)
            .unwrap_or_else(|| panic!("paragraph {needle:?}"))
            .baseline
    };
    // No block gap is added around the spacer: the second paragraph's line box
    // starts exactly `gap` below the first one's.
    assert!(
        (baseline_of("below") - baseline_of("above") - (line_h + gap)).abs() < 0.01,
        "spacer should replace the inter-block gap"
    );
}

#[test]
fn spacer_blocks_suppress_the_gaps_around_them() {
    let fonts = test_fonts();
    let theme = Theme::default();
    let engine = Engine::new(&fonts, &theme);
    assert_eq!(engine.gap_before(Some(Kind::Spacer), Kind::Other), 0.0);
    assert_eq!(engine.gap_before(Some(Kind::Other), Kind::Spacer), 0.0);
    assert_eq!(engine.gap_before(Some(Kind::Spacer), Kind::Heading(3)), 0.0);
}

#[test]
fn heading_spacing_is_resolved_per_level() {
    let fonts = test_fonts();
    let theme = Theme::default();
    let engine = Engine::new(&fonts, &theme);
    let above = |level| engine.gap_before(Some(Kind::Other), Kind::Heading(level));
    let below = |level| engine.gap_before(Some(Kind::Heading(level)), Kind::Other);
    assert_eq!(above(1), theme.spacing.heading_above.h1);
    assert_eq!(above(3), theme.spacing.heading_above.h3);
    assert!(
        above(1) > above(4),
        "level-1 headings should get more air than subsections"
    );
    assert_eq!(below(2), theme.spacing.heading_below.h2);
}

#[test]
fn fill_in_cells_draw_a_line_and_blank_cells_draw_nothing() {
    use crate::build::{blank, cell, fill_in};
    let fonts = test_fonts();
    let mut doc = Textris::new();
    doc.label_table([
        [cell("Observer"), cell("Costa, R.")],
        [cell("Date"), fill_in()],
        [cell("Remarks"), blank()],
    ]);
    let pages = layout(&doc.build(), &fonts);

    // Exactly one stroke: the Date row's fill-in line. The blank cell and the
    // filled text cells draw none.
    let strokes = pages[0]
        .elements
        .iter()
        .filter(|e| matches!(e, Element::Stroke { .. }))
        .count();
    assert_eq!(strokes, 1, "one fill-in line expected");
}

#[test]
fn inline_fill_in_draws_a_baseline_stroke_between_the_words() {
    use crate::build::text;
    let fonts = test_fonts();
    let len = 80.0;
    let mut doc = Textris::new();
    doc.paragraph(
        text("My name is ")
            .fill_in(len)
            .normal(" and I am ")
            .fill_in(40.0)
            .normal(" years old."),
    );
    let pages = layout(&doc.build(), &fonts);

    // Two fill-in lines flow inline with the text.
    let strokes: Vec<_> = pages[0]
        .elements
        .iter()
        .filter_map(|e| match e {
            Element::Stroke { points, .. } => Some(points.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(strokes.len(), 2, "one stroke per fill-in");

    // The first blank is horizontal, exactly `len` points wide, and sits on the
    // same baseline as its surrounding text.
    let [(x0, y0), (x1, y1)] = strokes[0][..] else {
        panic!("a fill-in is a two-point line");
    };
    assert!((y0 - y1).abs() < 0.001, "the line is horizontal");
    assert!((x1 - x0 - len).abs() < 0.01, "the line is `len` wide");
    let baseline = texts(&pages[0]).first().expect("leading text run").baseline;
    assert!(
        (y0 - baseline).abs() < 0.001,
        "the line sits on the baseline"
    );
    // The blank follows the leading "My name is " text, so it starts to its right.
    assert!(
        x0 > pages[0]
            .elements
            .iter()
            .find_map(|e| match e {
                Element::Text(t) => Some(t.x),
                _ => None,
            })
            .unwrap()
    );
}

#[test]
fn a_table_header_is_never_stranded_at_the_bottom_of_a_page() {
    let fonts = test_fonts();
    // Fill the page so only a sliver remains, then start a table: the header
    // must move to the next page together with the first body row, not be
    // drawn alone at (or beyond) the bottom margin.
    let mut doc = Textris::new();
    doc.paragraph("intro");
    doc.spacer(680.0);
    doc.table(["name", "value"], [["a", "b"]]);
    let pages = layout(&doc.build(), &fonts);

    assert_eq!(pages.len(), 2, "the table should break to a fresh page");
    let page_texts =
        |i: usize| -> Vec<String> { texts(&pages[i]).iter().map(|t| t.text.clone()).collect() };
    assert!(
        !page_texts(0).iter().any(|t| t == "name" || t == "a"),
        "no table content on the crowded page: {:?}",
        page_texts(0)
    );
    assert!(
        page_texts(1).iter().any(|t| t == "name") && page_texts(1).iter().any(|t| t == "a"),
        "header and first row share the fresh page: {:?}",
        page_texts(1)
    );
}

#[test]
fn a_cell_taller_than_a_page_splits_across_pages() {
    let fonts = test_fonts();
    let theme = Theme::default();
    // A single-column table whose one body cell wraps to far more lines than
    // one page can hold.
    let mut doc = Textris::new();
    doc.table(["v"], [["word ".repeat(2000)]]);
    let laid_out = layout(&doc.build(), &fonts);
    let pages = &laid_out.pages;
    assert!(pages.len() >= 2, "the cell should continue on a next page");

    for (index, page) in pages.iter().enumerate() {
        // Every fragment keeps its text inside the content box.
        for t in texts(page) {
            assert!(
                t.baseline <= theme.page.content_bottom() + 0.01,
                "page {index}: baseline {} beyond the bottom margin",
                t.baseline
            );
            assert!(t.baseline >= theme.page.content_top() - 0.01);
        }
        // The stripe fill is drawn per fragment and stays on its page.
        for e in &page.elements {
            if let Element::Rect { y, h, .. } = e {
                assert!(y + h <= theme.page.content_bottom() + 0.01);
            }
        }
        // The cell's content flows onto every page, under a repeated header.
        assert!(
            texts(page).iter().any(|t| t.text.contains("word")),
            "page {index} should carry a fragment of the cell"
        );
        assert!(
            texts(page).iter().any(|t| t.text == "v"),
            "page {index} should carry the (repeated) header"
        );
    }

    // Continuation pages redraw the header as an artifact; the real header is
    // tagged once, and the split row stays a single structure row.
    assert!(
        texts(&pages[1])
            .iter()
            .any(|t| t.text == "v" && t.tag == Tagging::Artifact),
        "the repeated header is an artifact"
    );
    let table = &laid_out.structure[0];
    assert_eq!(
        names(kids(table)),
        ["TR", "TR"],
        "one header row and one body row, however many fragments were drawn"
    );
}

#[test]
fn a_spacer_cell_taller_than_a_page_flows_over_multiple_pages() {
    let fonts = test_fonts();
    // A label-table field asking for far more writing room than one page.
    let mut doc = Textris::new();
    doc.label_table_with(|t| {
        t.spacer("Notes", 2000.0);
    });
    let pages = layout(&doc.build(), &fonts);

    // ~700pt of content fits per page, so 2000pt spans three pages.
    assert_eq!(pages.len(), 3, "the spacer height should consume pages");
    // The label sits on the first fragment.
    assert!(texts(&pages[0]).iter().any(|t| t.text == "Notes"));
}

#[test]
fn an_overlong_word_in_a_paragraph_wraps_within_the_content_width() {
    let fonts = test_fonts();
    let theme = Theme::default();
    let mut doc = Textris::new();
    // A single unbreakable "word" wider than several lines.
    doc.paragraph("W".repeat(300));
    let pages = layout(&doc.build(), &fonts);

    let runs = texts(&pages[0]);
    assert!(
        runs.len() > 1,
        "the word should break into character fragments across lines"
    );
    for t in &runs {
        let right = t.x + fonts.measure(t.style, &t.text, t.size);
        assert!(
            right <= theme.page.content_right() + 0.01,
            "a fragment overflows the right margin: {right}"
        );
    }
}

#[test]
fn a_spacer_cell_stretches_its_row() {
    use crate::build::spacer;
    let fonts = test_fonts();
    let theme = Theme::default();
    let tall = 100.0;
    let mut doc = Textris::new();
    doc.table(["h", ""], [[crate::build::cell("a"), spacer(tall)]]);
    let pages = layout(&doc.build(), &fonts);

    // The striped body row's rect reflects the row height.
    let row_h = pages[0]
        .elements
        .iter()
        .find_map(|e| match e {
            Element::Rect { h, .. } => Some(*h),
            _ => None,
        })
        .expect("striped body row");
    assert!(
        (row_h - (tall + 2.0 * theme.table.inset_y)).abs() < 0.01,
        "row should be the spacer height plus insets, got {row_h}"
    );
}

#[test]
fn numbered_headings_and_references_resolve_through_the_builder() {
    let fonts = test_fonts();
    let mut doc = Textris::new();
    doc.h3_numbered("Introduction").anchor("intro");
    doc.h4_numbered("Scope");
    doc.h3_numbered("Methods");
    doc.paragraph(
        crate::build::text("See section ")
            .section_ref("intro")
            .normal("."),
    );
    let pages = layout(&doc.build(), &fonts);

    let all: Vec<String> = texts(&pages[0]).iter().map(|t| t.text.clone()).collect();
    assert!(
        all.iter().any(|t| t.starts_with("1.")),
        "numbered h3: {all:?}"
    );
    assert!(
        all.iter().any(|t| t.starts_with("1.1.")),
        "nested h4: {all:?}"
    );
    assert!(all.iter().any(|t| t.starts_with("2.")), "second h3");
    // The reference resolves to the plain section number.
    assert!(
        all.iter().any(|t| t.split(' ').any(|w| w == "1")),
        "section reference should resolve to \"1\": {all:?}"
    );
}

#[test]
fn hard_line_breaks_split_lines_and_make_empty_lines() {
    let fonts = test_fonts();
    let theme = Theme::default();
    let line_h = theme.font_size.body * theme.spacing.line_height;
    let mut doc = Textris::new();
    doc.paragraph("one\ntwo\n\nthree");
    let pages = layout(&doc.build(), &fonts);

    let mut baselines: Vec<f32> = texts(&pages[0]).iter().map(|t| t.baseline).collect();
    baselines.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(baselines.len(), 3, "three visible lines");
    assert!(
        (baselines[1] - baselines[0] - line_h).abs() < 0.01,
        "single break advances one line"
    );
    assert!(
        (baselines[2] - baselines[1] - 2.0 * line_h).abs() < 0.01,
        "a double break leaves an empty line"
    );
}

#[test]
fn muted_text_follows_the_theme_palette() {
    use crate::build::muted;
    use krilla::color::rgb;
    let fonts = test_fonts();
    let mut theme = Theme::default();
    theme.palette.muted = rgb::Color::new(200, 10, 10);

    let mut doc = Textris::with_theme(theme.clone());
    doc.paragraph(muted("secondary"));
    let pages = layout(&doc.build(), &fonts);

    let run = texts(&pages[0])
        .into_iter()
        .find(|t| t.text == "secondary")
        .expect("muted run");
    assert_eq!(
        run.color, theme.palette.muted,
        "muted text should resolve the theme's muted color at layout time"
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

// --- Tagged-PDF logical structure ---------------------------------------

/// A short tag name for a structure node, for readable assertions.
fn tag_name(node: &StructNode) -> &'static str {
    let tag = match node {
        StructNode::Group { tag, .. } | StructNode::Leaf { tag, .. } => tag,
    };
    match tag {
        StructTag::Heading { .. } => "H",
        StructTag::Paragraph => "P",
        StructTag::List(_) => "L",
        StructTag::ListItem => "LI",
        StructTag::Label => "Lbl",
        StructTag::Body => "LBody",
        StructTag::Div => "Div",
        StructTag::Table => "Table",
        StructTag::TableRow => "TR",
        StructTag::TableHeaderCell(_) => "TH",
        StructTag::TableCell => "TD",
    }
}

fn kids(node: &StructNode) -> &[StructNode] {
    match node {
        StructNode::Group { children, .. } => children,
        StructNode::Leaf { .. } => &[],
    }
}

fn names(nodes: &[StructNode]) -> Vec<&'static str> {
    nodes.iter().map(tag_name).collect()
}

#[test]
fn structure_tree_tags_headings_and_paragraphs() {
    let fonts = test_fonts();
    let mut doc = Textris::new();
    doc.h1("Title");
    doc.paragraph("Body text.");
    let laid_out = layout(&doc.build(), &fonts);

    assert_eq!(names(&laid_out.structure), ["H", "P"]);
    // The heading keeps its level and its text as the required tag title.
    let StructNode::Leaf {
        tag: StructTag::Heading { level, title },
        ..
    } = &laid_out.structure[0]
    else {
        panic!("first node should be a heading leaf");
    };
    assert_eq!(*level, 1);
    assert_eq!(title, "Title");
}

#[test]
fn bullet_list_is_tagged_as_list_items_with_label_and_body() {
    use krilla::tagging::ListNumbering;
    let fonts = test_fonts();
    let mut doc = Textris::new();
    doc.bullet_list(["one", "two"]);
    let laid_out = layout(&doc.build(), &fonts);

    assert_eq!(laid_out.structure.len(), 1, "one list");
    let list = &laid_out.structure[0];
    assert!(matches!(
        list,
        StructNode::Group {
            tag: StructTag::List(ListNumbering::Disc),
            ..
        }
    ));
    let items = kids(list);
    assert_eq!(names(items), ["LI", "LI"]);
    for item in items {
        // Each item is a label (its bullet) followed by a body (its text).
        assert_eq!(names(kids(item)), ["Lbl", "LBody"]);
    }
}

#[test]
fn ordered_list_records_its_numbering_style() {
    use crate::model::ListMarker;
    use krilla::tagging::ListNumbering;
    let fonts = test_fonts();
    let mut doc = Textris::new();
    doc.ordered_list_with(ListMarker::LowerAlpha, ["a", "b"]);
    let laid_out = layout(&doc.build(), &fonts);
    assert!(matches!(
        &laid_out.structure[0],
        StructNode::Group {
            tag: StructTag::List(ListNumbering::LowerAlpha),
            ..
        }
    ));
}

#[test]
fn data_table_tags_header_and_body_cells() {
    use krilla::tagging::TableHeaderScope;
    let fonts = test_fonts();
    let mut doc = Textris::new();
    doc.table(["a", "b"], [["1", "2"], ["3", "4"]]);
    let laid_out = layout(&doc.build(), &fonts);

    let table = &laid_out.structure[0];
    assert!(matches!(
        table,
        StructNode::Group {
            tag: StructTag::Table,
            ..
        }
    ));
    let rows = kids(table);
    assert_eq!(names(rows), ["TR", "TR", "TR"], "header + two body rows");

    // The header row's cells are column-scoped header cells.
    let header = kids(&rows[0]);
    assert_eq!(header.len(), 2);
    assert!(header.iter().all(|c| matches!(
        c,
        StructNode::Leaf {
            tag: StructTag::TableHeaderCell(TableHeaderScope::Column),
            ..
        }
    )));
    // Body rows hold data cells.
    assert_eq!(names(kids(&rows[1])), ["TD", "TD"]);
}

#[test]
fn a_repeated_table_header_is_tagged_only_once() {
    let fonts = test_fonts();
    let mut doc = Textris::new();
    // Enough rows to break across pages, so the header repeats.
    doc.table(
        ["n", "value"],
        (0..120).map(|i| [i.to_string(), format!("row {i}")]),
    );
    let laid_out = layout(&doc.build(), &fonts);
    assert!(laid_out.pages.len() >= 2, "table should span pages");

    // Exactly one header row of header cells exists in the structure tree,
    // even though the header is drawn again atop each continuation page.
    let mut header_cells = 0;
    fn count_headers(node: &StructNode, n: &mut usize) {
        if let StructNode::Leaf {
            tag: StructTag::TableHeaderCell(_),
            ..
        } = node
        {
            *n += 1;
        }
        for child in kids(node) {
            count_headers(child, n);
        }
    }
    for node in &laid_out.structure {
        count_headers(node, &mut header_cells);
    }
    assert_eq!(header_cells, 2, "two header cells, tagged once");

    // The redrawn headers on later pages are emitted as artifacts, not content.
    let artifact_runs = laid_out
        .pages
        .iter()
        .skip(1)
        .flat_map(|p| &p.elements)
        .filter(|e| matches!(e, Element::Text(t) if t.tag == Tagging::Artifact))
        .count();
    assert!(
        artifact_runs > 0,
        "repeated header text should be tagged as artifacts on later pages"
    );
}

#[test]
fn a_boxed_callout_groups_its_content() {
    let fonts = test_fonts();
    let mut doc = Textris::new();
    doc.boxed(|b| {
        b.paragraph("Inside the box.");
    });
    let laid_out = layout(&doc.build(), &fonts);
    // The box is a generic grouping element wrapping its child blocks.
    assert_eq!(names(&laid_out.structure), ["Div"]);
    assert_eq!(names(kids(&laid_out.structure[0])), ["P"]);
}

#[test]
fn the_outline_lists_headings_in_order_with_their_levels() {
    let fonts = test_fonts();
    let mut doc = Textris::new();
    doc.h1("Top");
    doc.h3("Sub");
    doc.paragraph("body");
    doc.h1("Next");
    let laid_out = layout(&doc.build(), &fonts);

    let entries: Vec<(u8, &str)> = laid_out
        .outline
        .iter()
        .map(|e| (e.level, e.title.as_str()))
        .collect();
    assert_eq!(entries, [(1, "Top"), (3, "Sub"), (1, "Next")]);
    // Every bookmark points at a real page.
    assert!(
        laid_out
            .outline
            .iter()
            .all(|e| e.page_index < laid_out.pages.len())
    );
}

#[test]
fn every_text_run_belongs_to_a_structure_node() {
    let fonts = test_fonts();
    let mut doc = Textris::new();
    doc.h2("Sub");
    doc.h1("Title");
    doc.paragraph("A paragraph.");
    doc.bullet_list(["first", "second"]);
    doc.table(["h"], [["cell"]]);
    doc.task_list([(true, "done"), (false, "todo")]);
    let laid_out = layout(&doc.build(), &fonts);

    // On a single page (no repeated headers), every drawn text run is content
    // linked to an allocated structure node.
    assert_eq!(laid_out.pages.len(), 1);
    for element in &laid_out.pages[0].elements {
        if let Element::Text(text) = element {
            match text.tag {
                Tagging::Content(id) => {
                    assert!(id < laid_out.nodes, "node id {id} out of range")
                }
                Tagging::Artifact => panic!("unexpected artifact text {:?}", text.text),
            }
        }
    }
}

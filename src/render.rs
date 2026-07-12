//! Paint a laid-out document into a PDF using krilla.
//!
//! This module is the only place that talks to krilla. It walks the display list
//! produced by the [`crate::layout`] engine and, for every page, also draws the
//! running header and footer (which depend on the total page count and therefore
//! cannot be produced during layout).
//!
//! The output conforms to PDF/A-2b, the archival profile of PDF 1.7: krilla
//! validates the document while serializing, embeds an sRGB output intent and
//! writes XMP metadata. Content that cannot conform (for example a character
//! that maps to a font's `.notdef` glyph) surfaces as a [`RenderError`].

use std::fmt;

use krilla::{
    Document as KrillaDocument, SerializeSettings,
    color::rgb,
    configure::{Archival, ConfigurationBuilder},
    error::KrillaError,
    geom::{PathBuilder, Point, Rect},
    num::NormalizedF32,
    page::PageSettings,
    paint::{Fill, Stroke},
    surface::Surface,
};

use crate::{
    fonts::Fonts,
    layout::{Element, Page},
    model::{Chrome, Document, Inline},
    theme::Theme,
};

/// An error produced while serializing the PDF, most commonly a PDF/A-2b
/// validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderError(KrillaError);

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to serialize PDF: {}", self.0)
    }
}

impl std::error::Error for RenderError {}

/// Render laid-out pages plus document chrome into PDF/A-2b bytes.
pub fn render(pages: &[Page], document: &Document, fonts: &Fonts) -> Result<Vec<u8>, RenderError> {
    let configuration = ConfigurationBuilder::new()
        .with_archival_validator(Archival::A2_B)
        .finish()
        .expect("PDF/A-2b on its own is a valid configuration");
    let mut pdf = KrillaDocument::new_with(SerializeSettings {
        configuration,
        ..Default::default()
    });
    let total = pages.len();
    let theme = &document.theme;

    let header_baseline = theme.page.content_top() - theme.page.header_offset;
    let footer_baseline = theme.page.content_bottom() + theme.page.footer_offset;

    for (index, page) in pages.iter().enumerate() {
        let settings = PageSettings::from_wh(theme.page.width, theme.page.height)
            .expect("valid page dimensions");
        let mut krilla_page = pdf.start_page_with(settings);
        let mut surface = krilla_page.surface();

        let page_number = index + 1;
        draw_chrome(
            &mut surface,
            fonts,
            theme,
            &document.header,
            header_baseline,
            page_number,
            total,
        );
        for element in &page.elements {
            draw_element(&mut surface, fonts, element);
        }
        draw_chrome(
            &mut surface,
            fonts,
            theme,
            &document.footer,
            footer_baseline,
            page_number,
            total,
        );

        surface.finish();
        krilla_page.finish();
    }

    pdf.finish().map_err(RenderError)
}

fn draw_element(surface: &mut Surface, fonts: &Fonts, element: &Element) {
    match element {
        Element::Text(text) => {
            // Already shaped by layout; draw the glyphs directly.
            surface.set_stroke(None);
            surface.set_fill(Some(solid_fill(text.color)));
            surface.draw_glyphs(
                Point::from_xy(text.x, text.baseline),
                &text.glyphs,
                fonts.krilla_font(text.style),
                &text.text,
                text.size,
                false,
            );
        }
        Element::Rect { x, y, w, h, fill } => {
            let mut builder = PathBuilder::new();
            if let Some(rect) = Rect::from_xywh(*x, *y, *w, *h) {
                builder.push_rect(rect);
            }
            if let Some(path) = builder.finish() {
                surface.set_stroke(None);
                surface.set_fill(Some(solid_fill(*fill)));
                surface.draw_path(&path);
            }
        }
        Element::Stroke {
            points,
            width,
            color,
            closed,
        } => {
            if points.len() < 2 {
                return;
            }
            let mut builder = PathBuilder::new();
            builder.move_to(points[0].0, points[0].1);
            for point in &points[1..] {
                builder.line_to(point.0, point.1);
            }
            if *closed {
                builder.close();
            }
            if let Some(path) = builder.finish() {
                surface.set_fill(None);
                surface.set_stroke(Some(solid_stroke(*color, *width)));
                surface.draw_path(&path);
            }
        }
    }
}

/// Draw one row of page chrome (the running header or footer): its left section
/// starts at the content edge, its center section is centered in the content
/// width, and its right section ends at the right content edge.
fn draw_chrome(
    surface: &mut Surface,
    fonts: &Fonts,
    theme: &Theme,
    chrome: &Chrome,
    baseline: f32,
    page: usize,
    total: usize,
) {
    let size = theme.font_size.chrome;
    let page_theme = &theme.page;

    if let Some(content) = &chrome.left {
        let spans = content.resolve(page, total);
        draw_spans(
            surface,
            fonts,
            theme,
            &spans,
            page_theme.content_left(),
            baseline,
            size,
        );
    }
    if let Some(content) = &chrome.center {
        let spans = content.resolve(page, total);
        let width = spans_width(&spans, fonts, size);
        let x = page_theme.content_left() + (page_theme.content_width() - width) / 2.0;
        draw_spans(surface, fonts, theme, &spans, x, baseline, size);
    }
    if let Some(content) = &chrome.right {
        let spans = content.resolve(page, total);
        let width = spans_width(&spans, fonts, size);
        draw_spans(
            surface,
            fonts,
            theme,
            &spans,
            page_theme.content_right() - width,
            baseline,
            size,
        );
    }
}

/// The rendered width of a sequence of styled runs, for alignment purposes.
fn spans_width(spans: &[Inline], fonts: &Fonts, size: f32) -> f32 {
    spans
        .iter()
        .map(|sp| fonts.measure(sp.resolve_style(false, false), &sp.text, size))
        .sum()
}

/// Draw a sequence of styled runs left-to-right starting at `x`.
fn draw_spans(
    surface: &mut Surface,
    fonts: &Fonts,
    theme: &Theme,
    spans: &[Inline],
    mut x: f32,
    baseline: f32,
    size: f32,
) {
    let text_color = theme.palette.text;
    for span in spans {
        let style = span.resolve_style(false, false);
        let color = span
            .color
            .map(|c| c.resolve(&theme.palette))
            .unwrap_or(text_color);
        let shaped = fonts.shape(style, &span.text);
        surface.set_stroke(None);
        surface.set_fill(Some(solid_fill(color)));
        surface.draw_glyphs(
            Point::from_xy(x, baseline),
            &shaped.glyphs,
            fonts.krilla_font(style),
            &span.text,
            size,
            false,
        );
        x += shaped.width(size);
    }
}

fn solid_fill(color: rgb::Color) -> Fill {
    Fill {
        paint: color.into(),
        opacity: NormalizedF32::ONE,
        rule: Default::default(),
    }
}

fn solid_stroke(color: rgb::Color, width: f32) -> Stroke {
    Stroke {
        paint: color.into(),
        width,
        ..Default::default()
    }
}

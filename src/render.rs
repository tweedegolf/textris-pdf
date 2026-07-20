//! Paint a laid-out document into a tagged, accessible PDF using krilla.
//!
//! This module is the only place that talks to krilla. It walks the display
//! list produced by the [`crate::layout`] engine and, for every page, also
//! draws the running header and footer (which depend on the total page count
//! and therefore cannot be produced during layout).
//!
//! ## Accessibility
//!
//! The output conforms to **PDF/A-2A** (the accessible archival profile of PDF
//! 1.7) *and* **PDF/UA-1** (the universal-accessibility standard). To satisfy
//! these, the renderer produces a fully tagged PDF:
//!
//! * Every drawn piece of content is wrapped in a marked-content sequence.
//!   Real content (text) is linked to a node of the logical structure tree the
//!   layout engine built ([`crate::layout::StructNode`]); page furniture and
//!   decoration (header/footer, backgrounds, rules, checkboxes) is marked as an
//!   *artifact* and kept out of the structure tree.
//! * The structure tree — headings, paragraphs, lists (label + body), tables
//!   (rows and header/data cells) — is emitted in reading order.
//! * Document metadata carries a title, language and creation date; a bookmark
//!   outline is built from the headings.
//!
//! krilla validates the document while serializing; content that cannot conform
//! (for example a character that maps to a font's `.notdef` glyph, or a missing
//! required attribute) surfaces as a [`RenderError`].

use std::{fmt, num::NonZeroU16};

use krilla::{
    Document as KrillaDocument, SerializeSettings,
    color::rgb,
    configure::{Accessibility, Archival, ConfigurationBuilder},
    destination::XyzDestination,
    error::KrillaError,
    geom::{PathBuilder, Point, Rect},
    metadata::{DateTime, Metadata},
    num::NormalizedF32,
    outline::{Outline, OutlineNode},
    page::PageSettings,
    paint::{Fill, Stroke},
    surface::Surface,
    tagging::{
        Artifact, ArtifactType, ContentTag, Identifier, Node, SpanTag, Tag, TagGroup, TagKind,
        TagTree,
    },
};

use crate::{
    fonts::Fonts,
    layout::{Element, Layout, OutlineEntry, StructNode, StructTag, Tagging, TextElement},
    model::{Chrome, Document, Inline},
    theme::Theme,
};

/// An error produced while rendering the PDF.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderError {
    /// The theme's page width or height is not a positive, finite size.
    InvalidPageSize { width: f32, height: f32 },
    /// krilla failed to serialize the document, most commonly a validation
    /// failure against the PDF/A-2A or PDF/UA-1 profile.
    Serialize(KrillaError),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPageSize { width, height } => {
                write!(f, "invalid page size: {width} x {height} pt")
            }
            Self::Serialize(error) => write!(f, "failed to serialize PDF: {error}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// Render a laid-out document plus its chrome into tagged PDF/A-2A + PDF/UA-1
/// bytes.
///
/// `layout` must be the result of [`layout`](crate::layout::layout) for this
/// same `document` (and the same `fonts`): the structure tree indexes into
/// node ids allocated during layout, and a mismatched pair panics.
pub fn render(layout: &Layout, document: &Document, fonts: &Fonts) -> Result<Vec<u8>, RenderError> {
    let configuration = ConfigurationBuilder::new()
        .with_archival_validator(Archival::A2_A)
        .with_accessibility_validator(Accessibility::UA1)
        .finish()
        .expect("PDF/A-2A + PDF/UA-1 is a valid configuration");
    let mut pdf = KrillaDocument::new_with(SerializeSettings {
        configuration,
        ..Default::default()
    });
    let total = layout.pages.len();
    let theme = &document.theme;

    let header_baseline = theme.page.content_top() - theme.page.header_offset;
    let footer_baseline = theme.page.content_bottom() + theme.page.footer_offset;

    // Marked-content identifiers collected per structure-tree leaf, indexed by
    // node id. Filled as content is drawn, then woven into the tag tree below.
    let mut idents: Vec<Vec<Identifier>> = vec![Vec::new(); layout.nodes];

    for (index, page) in layout.pages.iter().enumerate() {
        let settings = PageSettings::from_wh(theme.page.width, theme.page.height).ok_or(
            RenderError::InvalidPageSize {
                width: theme.page.width,
                height: theme.page.height,
            },
        )?;
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
            ArtifactType::Header,
        );
        for element in &page.elements {
            draw_element(&mut surface, fonts, element, &mut idents);
        }
        draw_chrome(
            &mut surface,
            fonts,
            theme,
            &document.footer,
            footer_baseline,
            page_number,
            total,
            ArtifactType::Footer,
        );

        surface.finish();
        krilla_page.finish();
    }

    // Logical structure tree, in reading order. krilla wraps these top-level
    // nodes in an implicit Document root.
    let mut tree = TagTree::new();
    for node in &layout.structure {
        tree.push(build_node(node, &idents));
    }
    pdf.set_tag_tree(tree);

    // Metadata: a title and language are required by PDF/UA, a creation date by
    // PDF/A. Fall back to the first heading for the title and to English for the
    // language when the document leaves them unset.
    let title = document
        .title
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            layout
                .outline
                .iter()
                .map(|e| e.title.clone())
                .find(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "Untitled document".to_string());
    let language = document
        .language
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "en".to_string());
    pdf.set_metadata(
        Metadata::new()
            .title(title.clone())
            .language(language)
            .creation_date(now_datetime()),
    );

    // A bookmark outline (required by PDF/UA), nested by heading level.
    pdf.set_outline(build_outline(&layout.outline, &title));

    pdf.finish().map_err(RenderError::Serialize)
}

/// Draw one display-list element, wrapped in the marked-content sequence its
/// tagging calls for: real text is linked to its structure node (its identifier
/// recorded in `idents`), everything else is an artifact excluded from the
/// structure tree.
fn draw_element(
    surface: &mut Surface,
    fonts: &Fonts,
    element: &Element,
    idents: &mut [Vec<Identifier>],
) {
    match element {
        Element::Text(text) => match text.tag {
            Tagging::Content(node) => {
                let id = surface.start_tagged(ContentTag::Span(SpanTag::empty()));
                draw_text(surface, fonts, text);
                surface.end_tagged();
                idents[node].push(id);
            }
            Tagging::Artifact => {
                surface.start_tagged(ContentTag::Artifact(Artifact::with_kind(
                    ArtifactType::Other,
                )));
                draw_text(surface, fonts, text);
                surface.end_tagged();
            }
        },
        Element::Rect { x, y, w, h, fill } => {
            surface.start_tagged(ContentTag::Artifact(Artifact::with_kind(
                ArtifactType::Layout,
            )));
            draw_rect(surface, *x, *y, *w, *h, *fill);
            surface.end_tagged();
        }
        Element::Stroke {
            points,
            width,
            color,
            closed,
        } => {
            surface.start_tagged(ContentTag::Artifact(Artifact::with_kind(
                ArtifactType::Layout,
            )));
            draw_stroke(surface, points, *width, *color, *closed);
            surface.end_tagged();
        }
    }
}

/// Draw a run of already-shaped glyphs (no tagging; the caller wraps it).
fn draw_text(surface: &mut Surface, fonts: &Fonts, text: &TextElement) {
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

fn draw_rect(surface: &mut Surface, x: f32, y: f32, w: f32, h: f32, fill: rgb::Color) {
    let mut builder = PathBuilder::new();
    if let Some(rect) = Rect::from_xywh(x, y, w, h) {
        builder.push_rect(rect);
    }
    if let Some(path) = builder.finish() {
        surface.set_stroke(None);
        surface.set_fill(Some(solid_fill(fill)));
        surface.draw_path(&path);
    }
}

fn draw_stroke(
    surface: &mut Surface,
    points: &[(f32, f32)],
    width: f32,
    color: rgb::Color,
    closed: bool,
) {
    if points.len() < 2 {
        return;
    }
    let mut builder = PathBuilder::new();
    builder.move_to(points[0].0, points[0].1);
    for point in &points[1..] {
        builder.line_to(point.0, point.1);
    }
    if closed {
        builder.close();
    }
    if let Some(path) = builder.finish() {
        surface.set_fill(None);
        surface.set_stroke(Some(solid_stroke(color, width)));
        surface.draw_path(&path);
    }
}

/// Draw one row of page chrome (the running header or footer): its left section
/// starts at the content edge, its center section is centered in the content
/// width, and its right section ends at the right content edge. The whole row
/// is a pagination artifact, kept out of the logical structure tree.
#[allow(clippy::too_many_arguments)]
fn draw_chrome(
    surface: &mut Surface,
    fonts: &Fonts,
    theme: &Theme,
    chrome: &Chrome,
    baseline: f32,
    page: usize,
    total: usize,
    artifact: ArtifactType,
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
            artifact,
        );
    }
    if let Some(content) = &chrome.center {
        let spans = content.resolve(page, total);
        let width = spans_width(&spans, fonts, size);
        let x = page_theme.content_left() + (page_theme.content_width() - width) / 2.0;
        draw_spans(surface, fonts, theme, &spans, x, baseline, size, artifact);
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
            artifact,
        );
    }
}

/// Chrome is a single line: fold every whitespace character (including the
/// hard breaks body text honors) to a space and drop other control
/// characters, which have no glyph and would fail shaping.
fn chrome_text(text: &str) -> String {
    text.chars()
        .filter_map(|c| {
            if c.is_whitespace() {
                Some(' ')
            } else if c.is_control() {
                None
            } else {
                Some(c)
            }
        })
        .collect()
}

/// The rendered width of a sequence of styled runs, for alignment purposes.
/// Mirrors how [`draw_spans`] advances, including fill-in blanks.
fn spans_width(spans: &[Inline], fonts: &Fonts, size: f32) -> f32 {
    spans
        .iter()
        .map(|sp| match sp.fill_in {
            Some(width) => width,
            None => fonts.measure(sp.resolve_style(false, false), &chrome_text(&sp.text), size),
        })
        .sum()
}

/// Draw a sequence of styled runs left-to-right starting at `x`, each run its
/// own artifact marked-content sequence. A fill-in run draws its blank line
/// along the baseline, as in body text.
#[allow(clippy::too_many_arguments)]
fn draw_spans(
    surface: &mut Surface,
    fonts: &Fonts,
    theme: &Theme,
    spans: &[Inline],
    mut x: f32,
    baseline: f32,
    size: f32,
    artifact: ArtifactType,
) {
    let text_color = theme.palette.text;
    for span in spans {
        let style = span.resolve_style(false, false);
        let color = span
            .color
            .map(|c| c.resolve(&theme.palette))
            .unwrap_or(text_color);
        surface.start_tagged(ContentTag::Artifact(Artifact::with_kind(artifact)));
        if let Some(width) = span.fill_in {
            draw_stroke(
                surface,
                &[(x, baseline), (x + width, baseline)],
                0.7,
                color,
                false,
            );
            x += width;
        } else {
            let text = chrome_text(&span.text);
            let shaped = fonts.shape(style, &text);
            surface.set_stroke(None);
            surface.set_fill(Some(solid_fill(color)));
            surface.draw_glyphs(
                Point::from_xy(x, baseline),
                &shaped.glyphs,
                fonts.krilla_font(style),
                &text,
                size,
                false,
            );
            x += shaped.width(size);
        }
        surface.end_tagged();
    }
}

/// Recursively turn a layout structure node into a krilla tag-tree node. A leaf
/// becomes a tag group whose children are the marked-content identifiers drawn
/// for it; a group recurses over its structural children.
fn build_node(node: &StructNode, idents: &[Vec<Identifier>]) -> Node {
    match node {
        StructNode::Leaf { tag, id } => {
            let children: Vec<Node> = idents[*id].iter().copied().map(Node::Leaf).collect();
            Node::Group(TagGroup::with_children(to_tag_kind(tag), children))
        }
        StructNode::Group { tag, children } => {
            let kids: Vec<Node> = children.iter().map(|c| build_node(c, idents)).collect();
            Node::Group(TagGroup::with_children(to_tag_kind(tag), kids))
        }
    }
}

/// Map a layout structure role to a concrete krilla structure tag.
fn to_tag_kind(tag: &StructTag) -> TagKind {
    match tag {
        StructTag::Heading { level, title } => {
            let level = NonZeroU16::new((*level).max(1) as u16).expect("level is at least 1");
            Tag::Hn(level, Some(title.clone())).into()
        }
        StructTag::Paragraph => Tag::P.into(),
        StructTag::List(numbering) => Tag::L(*numbering).into(),
        StructTag::ListItem => Tag::LI.into(),
        StructTag::Label => Tag::Lbl.into(),
        StructTag::Body => Tag::LBody.into(),
        StructTag::Div => Tag::Div.into(),
        StructTag::Table => Tag::Table.into(),
        StructTag::TableRow => Tag::TR.into(),
        StructTag::TableHeaderCell(scope) => Tag::TH(*scope).into(),
        StructTag::TableCell => Tag::TD.into(),
    }
}

/// Build the bookmark outline from the flat list of headings, nesting each
/// entry under the most recent shallower one. Levels need not be contiguous.
/// A destination points at the heading's top on its page.
fn build_outline(entries: &[OutlineEntry], fallback_title: &str) -> Outline {
    let mut outline = Outline::new();

    // PDF/UA requires an outline; if the document has no headings, point a
    // single entry at the start of the document so one always exists.
    if entries.is_empty() {
        outline.push_child(OutlineNode::new(
            fallback_title.to_string(),
            XyzDestination::new(0, Point::from_xy(0.0, 0.0)),
        ));
        return outline;
    }

    // A stack of open ancestors (by level). A new entry closes every open node
    // at its level or deeper, attaching each to its parent, then becomes the
    // new deepest open node.
    let mut stack: Vec<(u8, OutlineNode)> = Vec::new();
    for entry in entries {
        let node = OutlineNode::new(
            entry.title.clone(),
            XyzDestination::new(entry.page_index, Point::from_xy(0.0, entry.y)),
        );
        while stack.last().is_some_and(|(level, _)| *level >= entry.level) {
            let (_, done) = stack.pop().expect("checked non-empty");
            attach_outline(&mut stack, &mut outline, done);
        }
        stack.push((entry.level, node));
    }
    while let Some((_, done)) = stack.pop() {
        attach_outline(&mut stack, &mut outline, done);
    }
    outline
}

/// Attach a completed outline node to its parent: the innermost still-open
/// ancestor, or the outline root when there is none.
fn attach_outline(stack: &mut [(u8, OutlineNode)], outline: &mut Outline, node: OutlineNode) {
    match stack.last_mut() {
        Some((_, parent)) => parent.push_child(node),
        None => outline.push_child(node),
    }
}

/// The current UTC time as a krilla [`DateTime`], for the document's creation
/// date (required by PDF/A).
fn now_datetime() -> DateTime {
    let now = time::OffsetDateTime::now_utc();
    DateTime::new(now.year().clamp(0, 9999) as u16)
        .month(now.month() as u8)
        .day(now.day())
        .hour(now.hour())
        .minute(now.minute())
        .second(now.second())
        .utc_offset_hour(0)
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

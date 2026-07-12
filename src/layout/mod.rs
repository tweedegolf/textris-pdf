//! The layout engine: it turns the abstract [`Document`] model into a sequence of
//! [`Page`]s filled with absolutely-positioned drawing [`Element`]s.
//!
//! Layout is deliberately decoupled from PDF emission: the engine produces a
//! display list that the `render` module later paints with krilla, which keeps
//! the layout logic unit-testable without a PDF backend.
//!
//! The coordinate system matches krilla's page space: the origin is the top-left
//! corner and y grows downward. All values are in points.

mod display;
mod list;
mod table;
#[cfg(test)]
mod tests;
mod text;

pub use display::{Element, Page, TextElement};

use crate::{
    fonts::{Fonts, Style},
    model::{Block, Document},
    theme::{BoxStyle, Theme},
};

/// Lay out a whole document into pages, using the document's own [`Theme`].
pub fn layout(document: &Document, fonts: &Fonts) -> Vec<Page> {
    let mut engine = Engine::new(fonts, &document.theme);
    engine.layout_document(document);
    engine.pages
}

/// Which spacing rule applies before a block.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// A heading at the given level.
    Heading(u8),
    /// A fixed spacer, which suppresses the gaps around it.
    Spacer,
    Other,
}

fn kind_of(block: &Block) -> Kind {
    match block {
        Block::Heading { level, .. } => Kind::Heading(*level),
        Block::Spacer(_) => Kind::Spacer,
        _ => Kind::Other,
    }
}

/// The inner content width of a box laid out within `outer` width.
fn box_inner_width(style: &BoxStyle, outer: f32) -> f32 {
    (outer - 2.0 * style.margin_x - 2.0 * style.padding_x).max(1.0)
}

struct Engine<'a> {
    fonts: &'a Fonts,
    theme: &'a Theme,
    pages: Vec<Page>,
    /// Current vertical pen position (distance from the top of the page).
    y: f32,
    /// Left edge of the current content region. Normally the page's content
    /// margin, but narrowed while laying out the inside of a box.
    left: f32,
    /// Right edge of the current content region.
    right: f32,
}

impl<'a> Engine<'a> {
    fn new(fonts: &'a Fonts, theme: &'a Theme) -> Self {
        Self {
            fonts,
            theme,
            pages: vec![Page::default()],
            y: theme.page.content_top(),
            left: theme.page.content_left(),
            right: theme.page.content_right(),
        }
    }

    /// Width of the current content region.
    fn width(&self) -> f32 {
        self.right - self.left
    }

    fn page(&mut self) -> &mut Page {
        self.pages.last_mut().expect("always at least one page")
    }

    fn push(&mut self, element: Element) {
        self.page().elements.push(element);
    }

    fn new_page(&mut self) {
        self.pages.push(Page::default());
        self.y = self.theme.page.content_top();
    }

    /// Break to a new page if `height` would not fit and we are not already at
    /// the top of a page (in which case breaking cannot help).
    fn ensure(&mut self, height: f32) {
        let page = &self.theme.page;
        if self.y + height > page.content_bottom() && self.y > page.content_top() {
            self.new_page();
        }
    }

    /// Vertical gap to insert before a block of the given kind.
    fn gap_before(&self, prev: Option<Kind>, cur: Kind) -> f32 {
        let spacing = &self.theme.spacing;
        match (prev, cur) {
            (None, _) => 0.0,
            // A spacer *is* the gap between its neighbours.
            (Some(Kind::Spacer), _) | (_, Kind::Spacer) => 0.0,
            (Some(Kind::Heading(_)), Kind::Heading(_)) => 0.0,
            (_, Kind::Heading(level)) => spacing.heading_above.level(level),
            (Some(Kind::Heading(level)), _) => spacing.heading_below.level(level),
            _ => spacing.block,
        }
    }

    /// Start a new page, unless the current one is still empty (in which case
    /// breaking would leave a blank page behind).
    fn force_page_break(&mut self) {
        if self.y > self.theme.page.content_top() || !self.page().elements.is_empty() {
            self.new_page();
        }
    }

    fn layout_document(&mut self, document: &Document) {
        let blocks = &document.blocks;
        let mut prev: Option<Kind> = None;
        let mut i = 0;
        while i < blocks.len() {
            if matches!(blocks[i], Block::PageBreak) {
                self.force_page_break();
                prev = None;
                i += 1;
            } else if matches!(blocks[i], Block::Heading { .. }) {
                // Group the heading with everything up to the next heading or
                // page break, so the section can be kept together across page
                // breaks.
                let start = i;
                i += 1;
                while i < blocks.len()
                    && !matches!(blocks[i], Block::Heading { .. } | Block::PageBreak)
                {
                    i += 1;
                }
                prev = Some(self.layout_section(&blocks[start..i], prev));
            } else {
                let cur = kind_of(&blocks[i]);
                self.y += self.gap_before(prev, cur);
                self.layout_block(&blocks[i]);
                prev = Some(cur);
                i += 1;
            }
        }
    }

    fn layout_block(&mut self, block: &Block) {
        let text_color = self.theme.palette.text;
        match block {
            Block::Heading { level, content, .. } => {
                let size = self.theme.font_size.heading(*level);
                self.layout_paragraph(content, size, true, false, text_color)
            }
            Block::Paragraph(inlines) => {
                let size = self.theme.font_size.body;
                self.layout_paragraph(inlines, size, false, false, text_color)
            }
            Block::Table(table) => self.layout_table(table),
            Block::TaskList(items) => self.layout_task_list(items),
            Block::BulletList(items) => self.layout_bullet_list(items),
            Block::OrderedList { marker, items } => self.layout_ordered_list(*marker, items),
            Block::Box { style, content } => self.layout_box(style, content),
            Block::PageBreak => self.force_page_break(),
            Block::Spacer(height) => self.y += height,
        }
    }

    /// The uniform font size of a block's text lines, when the block is flowing
    /// text (a heading or a paragraph). Other blocks return `None`.
    fn text_size(&self, block: &Block) -> Option<f32> {
        match block {
            Block::Heading { level, .. } => Some(self.theme.font_size.heading(*level)),
            Block::Paragraph(_) => Some(self.theme.font_size.body),
            _ => None,
        }
    }

    /// Dead space between a text block's first line-box top and the cap top of
    /// its text. Zero for blocks that are not flowing text.
    fn leading_above(&self, block: &Block) -> f32 {
        self.text_size(block).map_or(0.0, |size| {
            self.fonts.ascent(Style::Regular, size) - self.fonts.cap_height(Style::Regular, size)
        })
    }

    /// Dead space between a text block's last descender bottom and its line-box
    /// bottom (a line's extra line-height leading all sits below the text).
    /// Zero for blocks that are not flowing text.
    fn leading_below(&self, block: &Block) -> f32 {
        self.text_size(block).map_or(0.0, |size| {
            size * self.theme.spacing.line_height
                - self.fonts.ascent(Style::Regular, size)
                - self.fonts.descent(Style::Regular, size)
        })
    }

    /// Lay out the contents of a box, inserting the usual inter-block gaps.
    /// Padding and gaps are measured against the visible edges of text blocks
    /// (cap top / descender bottom) rather than their line boxes: a box's
    /// background makes the leading below a line visible, so a heading would
    /// otherwise hug the top edge and push its content away. Unlike
    /// [`layout_section`](Self::layout_section) this applies no keep-together
    /// logic. Must advance `y` by exactly the height that
    /// [`measure_box_blocks`](Self::measure_box_blocks) reports.
    fn layout_box_blocks(&mut self, blocks: &[Block]) {
        let mut prev: Option<&Block> = None;
        for block in blocks {
            // A box is kept together on one page; a page break inside it has
            // no meaning.
            if matches!(block, Block::PageBreak) {
                continue;
            }
            match prev {
                None => self.y -= self.leading_above(block),
                Some(prev) => {
                    self.y += self.gap_before(Some(kind_of(prev)), kind_of(block))
                        - self.leading_below(prev)
                        - self.leading_above(block);
                }
            }
            self.layout_block(block);
            prev = Some(block);
        }
    }

    /// Lay out a boxed callout: a filled background with padding and margin,
    /// wrapping its child blocks. The box is kept together on one page.
    fn layout_box(&mut self, style: &BoxStyle, content: &[Block]) {
        let box_left = self.left + style.margin_x;
        let box_right = self.right - style.margin_x;
        let box_width = box_right - box_left;
        let inner_width = box_inner_width(style, self.width());

        let inner_height = self.measure_box_blocks(content, inner_width);
        let box_height = inner_height + 2.0 * style.padding_y;

        // Break before the box if it (plus its vertical margin) would not fit.
        self.ensure(box_height + 2.0 * style.margin_y);

        self.y += style.margin_y;
        let box_top = self.y;
        self.push(Element::Rect {
            x: box_left,
            y: box_top,
            w: box_width,
            h: box_height,
            fill: style.background,
        });

        // Lay out the children within the padded inner region.
        let (saved_left, saved_right) = (self.left, self.right);
        self.left = box_left + style.padding_x;
        self.right = box_right - style.padding_x;
        self.y = box_top + style.padding_y;
        self.layout_box_blocks(content);
        self.left = saved_left;
        self.right = saved_right;

        self.y = box_top + box_height + style.margin_y;
    }

    /// Lay out a section (a heading plus its following blocks). A section below
    /// the keep-together threshold starts on a fresh page rather than breaking;
    /// a taller one flows, but the heading plus at least one line of content
    /// must fit or we break first (orphan control).
    ///
    /// Returns the kind of the section's last block, for the caller's spacing.
    fn layout_section(&mut self, section: &[Block], prev: Option<Kind>) -> Kind {
        let Block::Heading { level, .. } = &section[0] else {
            unreachable!("a section starts with its heading");
        };
        let lead_gap = self.gap_before(prev, Kind::Heading(*level));
        let content_top = self.theme.page.content_top();
        let content_bottom = self.theme.page.content_bottom();
        let page_height = content_bottom - content_top;
        let section_height = self.measure_blocks(section, self.width());
        let keep_together =
            section_height <= self.theme.spacing.keep_together_max_fraction * page_height;

        let need = if keep_together {
            lead_gap + section_height
        } else {
            let heading_height = self.measure_block(&section[0], self.width());
            let min_follow = self.theme.font_size.body * self.theme.spacing.line_height;
            lead_gap + heading_height + min_follow
        };

        if self.y + need > content_bottom && self.y > content_top {
            self.new_page();
        } else {
            self.y += lead_gap;
        }

        let mut prev = None;
        for (index, block) in section.iter().enumerate() {
            if index > 0 {
                self.y += self.gap_before(prev, kind_of(block));
            }
            self.layout_block(block);
            prev = Some(kind_of(block));
        }
        prev.unwrap_or(Kind::Heading(*level))
    }

    // --- Measurement (dry-run height, no drawing) ------------------------

    /// The total height a sequence of blocks would occupy at the given content
    /// width, including the inter-block spacing.
    fn measure_blocks(&self, blocks: &[Block], width: f32) -> f32 {
        let mut height = 0.0;
        let mut prev = None;
        for (index, block) in blocks.iter().enumerate() {
            let cur = kind_of(block);
            if index > 0 {
                height += self.gap_before(prev, cur);
            }
            height += self.measure_block(block, width);
            prev = Some(cur);
        }
        height
    }

    /// The height the contents of a box occupy: like
    /// [`measure_blocks`](Self::measure_blocks) but with the visible-edge gap
    /// accounting of [`layout_box_blocks`](Self::layout_box_blocks), and with
    /// the dead leading above the first and below the last text block trimmed
    /// so the box's padding measures to the visible text.
    fn measure_box_blocks(&self, blocks: &[Block], width: f32) -> f32 {
        let mut height = 0.0;
        let mut prev: Option<&Block> = None;
        for block in blocks {
            if matches!(block, Block::PageBreak) {
                continue;
            }
            match prev {
                None => height -= self.leading_above(block),
                Some(prev) => {
                    height += self.gap_before(Some(kind_of(prev)), kind_of(block))
                        - self.leading_below(prev)
                        - self.leading_above(block);
                }
            }
            height += self.measure_block(block, width);
            prev = Some(block);
        }
        if let Some(last) = prev {
            height -= self.leading_below(last);
        }
        height.max(0.0)
    }

    /// The height a single block would occupy at the given content width.
    fn measure_block(&self, block: &Block, width: f32) -> f32 {
        match block {
            Block::Heading { level, content, .. } => {
                let size = self.theme.font_size.heading(*level);
                self.paragraph_height(content, size, true, false, width)
            }
            Block::Paragraph(inlines) => {
                self.paragraph_height(inlines, self.theme.font_size.body, false, false, width)
            }
            Block::Table(table) => self.table_height(table, width),
            Block::TaskList(items) => self.task_list_height(items, width),
            Block::BulletList(items) => self.list_height(items, width),
            Block::OrderedList { items, .. } => self.list_height(items, width),
            Block::Box { style, content } => {
                let inner = self.measure_box_blocks(content, box_inner_width(style, width));
                inner + 2.0 * style.padding_y + 2.0 * style.margin_y
            }
            Block::PageBreak => 0.0,
            Block::Spacer(height) => *height,
        }
    }
}

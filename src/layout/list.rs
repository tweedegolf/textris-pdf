//! List layout: task lists (with drawn checkboxes) and marker lists (bullets
//! and ordered lists, which share their geometry and differ only in the marker
//! text drawn in the indent).

use krilla::{color::rgb, tagging::ListNumbering};

use crate::{
    fonts::Style,
    layout::{Element, Engine, StructTag, Tagging, TextElement},
    model::{Inline, ListMarker, TaskItem},
};

impl Engine<'_> {
    // --- Task lists --------------------------------------------------------

    pub(super) fn task_list_height(&self, items: &[TaskItem], width: f32) -> f32 {
        let mut height = 0.0;
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                height += self.theme.list.task_gap;
            }
            height += self.task_item_height(item, width);
        }
        height
    }

    fn task_item_height(&self, item: &TaskItem, width: f32) -> f32 {
        let size = self.theme.font_size.body;
        let line_h = size * self.theme.spacing.line_height;
        let box_size = self.theme.checkbox.size;
        let text_width = width - box_size - self.theme.checkbox.gap;
        let words = self.tokenize(&item.content, false, false, size);
        let lines = self.wrap(words, text_width, size);
        (lines.len() as f32 * line_h).max(box_size)
    }

    pub(super) fn layout_task_list(&mut self, items: &[TaskItem]) {
        let gap = self.theme.list.task_gap;
        // A checklist is a list with no visible numbering; each item's drawn
        // checkbox is decorative (an artifact) and its text is the item body.
        self.structure.open(StructTag::List(ListNumbering::None));
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                self.y += gap;
            }
            self.structure.open(StructTag::ListItem);
            let body = self.structure.leaf(StructTag::Body);
            self.layout_task_item(item, Tagging::Content(body));
            self.structure.close();
        }
        self.structure.close();
    }

    fn layout_task_item(&mut self, item: &TaskItem, body_tag: Tagging) {
        let size = self.theme.font_size.body;
        let line_h = size * self.theme.spacing.line_height;
        let box_size = self.theme.checkbox.size;
        let content_left = self.left;
        let text_x = content_left + box_size + self.theme.checkbox.gap;
        let text_width = self.right - text_x;
        let text_color = self.theme.palette.text;

        let words = self.tokenize(&item.content, false, false, size);
        let lines = self.wrap(words, text_width, size);
        let content_h = lines.len() as f32 * line_h;
        let height = content_h.max(box_size);

        self.ensure(height);
        let top = self.y;

        // Center the box on the first line's visible text — the cap-height
        // band (cap top to baseline) — not the line box, whose extra leading
        // all sits below the baseline and would push the box too low.
        let cap = self.fonts.cap_height(Style::Regular, size);
        let band_center = top + self.fonts.ascent(Style::Regular, size) - cap / 2.0;
        let box_y = band_center - box_size / 2.0;
        self.draw_checkbox(content_left, box_y, box_size, item.checked);

        self.current_tag = body_tag;
        let mut line_top = top;
        for line in &lines {
            self.draw_line(line, text_x, line_top, size, text_color);
            line_top += line_h;
        }
        self.y = top + height;
    }

    fn draw_checkbox(&mut self, x: f32, y: f32, size: f32, checked: bool) {
        let color = self.theme.palette.text;
        if checked {
            self.push(Element::Rect {
                x,
                y,
                w: size,
                h: size,
                fill: color,
            });
            let width = self.theme.checkbox.check_width;
            self.push(Element::Stroke {
                points: vec![
                    (x + 0.22 * size, y + 0.52 * size),
                    (x + 0.42 * size, y + 0.72 * size),
                    (x + 0.78 * size, y + 0.28 * size),
                ],
                width,
                color: rgb::Color::new(255, 255, 255),
                closed: false,
            });
        } else {
            self.push(Element::Stroke {
                points: vec![(x, y), (x + size, y), (x + size, y + size), (x, y + size)],
                width: 0.5,
                color,
                closed: true,
            });
        }
    }

    // --- Marker lists (bullets and ordered lists) ---------------------------

    /// The height a bullet or ordered list would occupy at the given width.
    /// Both list forms share their geometry, so one measurement serves both.
    pub(super) fn list_height(&self, items: &[Vec<Inline>], width: f32) -> f32 {
        let size = self.theme.font_size.body;
        let line_h = size * self.theme.spacing.line_height;
        let text_width = width - self.theme.list.bullet_indent;
        let mut height = 0.0;
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                height += self.theme.list.bullet_gap;
            }
            let words = self.tokenize(item, false, false, size);
            let lines = self.wrap(words, text_width, size);
            height += (lines.len() as f32 * line_h).max(line_h);
        }
        height
    }

    pub(super) fn layout_bullet_list(&mut self, items: &[Vec<Inline>]) {
        self.layout_marker_list(items, ListNumbering::Disc, |_| "•".to_string());
    }

    pub(super) fn layout_ordered_list(&mut self, marker: ListMarker, items: &[Vec<Inline>]) {
        let numbering = match marker {
            ListMarker::Decimal => ListNumbering::Decimal,
            ListMarker::LowerAlpha => ListNumbering::LowerAlpha,
        };
        self.layout_marker_list(items, numbering, |index| marker.label(index + 1));
    }

    /// Lay out a list whose items are indented text preceded by a marker
    /// ("•", "1.", "a.", …) drawn on the first line. `marker` maps the 0-based
    /// item index to its marker text; `numbering` records the list style in the
    /// structure tree. Each item becomes a list item whose label holds the
    /// marker and whose body holds the wrapped text.
    fn layout_marker_list(
        &mut self,
        items: &[Vec<Inline>],
        numbering: ListNumbering,
        marker: impl Fn(usize) -> String,
    ) {
        let size = self.theme.font_size.body;
        let line_h = size * self.theme.spacing.line_height;
        let content_left = self.left;
        let text_x = content_left + self.theme.list.bullet_indent;
        let text_width = self.right - text_x;
        let text_color = self.theme.palette.text;
        let gap = self.theme.list.bullet_gap;

        self.structure.open(StructTag::List(numbering));
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                self.y += gap;
            }
            let words = self.tokenize(item, false, false, size);
            let lines = self.wrap(words, text_width, size);
            let height = (lines.len() as f32 * line_h).max(line_h);
            self.ensure(height);
            let top = self.y;

            self.structure.open(StructTag::ListItem);

            // Marker left-aligned in the indent, on the first line: the label.
            let label = marker(index);
            let label_id = self.structure.leaf(StructTag::Label);
            let shaped = self.fonts.shape(Style::Regular, &label);
            let baseline = top + self.fonts.ascent(Style::Regular, size);
            self.push(Element::Text(TextElement {
                x: content_left,
                baseline,
                size,
                color: text_color,
                style: Style::Regular,
                glyphs: shaped.glyphs,
                text: label,
                tag: Tagging::Content(label_id),
            }));

            // The wrapped item text: the body.
            let body_id = self.structure.leaf(StructTag::Body);
            self.current_tag = Tagging::Content(body_id);
            let mut line_top = top;
            for line in &lines {
                self.draw_line(line, text_x, line_top, size, text_color);
                line_top += line_h;
            }

            self.structure.close();
            self.y = top + height;
        }
        self.structure.close();
    }
}

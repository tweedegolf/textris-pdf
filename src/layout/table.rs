//! Table layout: column sizing (auto, label and custom models), row heights,
//! page-breaking with repeated headers, and row emission.

use krilla::color::rgb;

use crate::{
    fonts::Style,
    layout::{Element, Engine},
    model::{Inline, Table, plain_text},
    theme::{Align, ColumnWidth, ColumnWidths, TableStyle},
};

/// Styling for one table row, resolved from the table's [`TableStyle`] and the
/// theme.
pub(super) struct RowStyle<'a> {
    italic: bool,
    fill: Option<rgb::Color>,
    flush_first_column: bool,
    fill_in_blanks: bool,
    align: &'a [Align],
    size: f32,
    row_min_height: f32,
}

impl RowStyle<'_> {
    /// The alignment of a column's content; columns past the end of the style's
    /// list are left-aligned.
    fn align(&self, column: usize) -> Align {
        self.align.get(column).copied().unwrap_or_default()
    }
}

impl Engine<'_> {
    /// The font size for a table's cells: its style override, or the theme body size.
    fn table_font_size(&self, style: &TableStyle) -> f32 {
        style.font_size.unwrap_or(self.theme.font_size.body)
    }

    /// The row style for the table's header row.
    fn header_row_style<'a>(&self, style: &'a TableStyle) -> RowStyle<'a> {
        RowStyle {
            italic: style.header_italic,
            fill: None,
            flush_first_column: style.flush_first_column,
            fill_in_blanks: style.fill_in_blanks,
            align: &style.align,
            size: self.table_font_size(style),
            row_min_height: style
                .row_min_height
                .unwrap_or(self.theme.table.row_min_height),
        }
    }

    /// The row style for a body row, optionally filled (zebra striping).
    fn body_row_style<'a>(&self, style: &'a TableStyle, fill: Option<rgb::Color>) -> RowStyle<'a> {
        RowStyle {
            italic: false,
            fill,
            ..self.header_row_style(style)
        }
    }

    /// Shared table geometry: column widths, column count and whether a header
    /// row should be drawn. Used by both layout and measurement.
    fn table_setup(&self, table: &Table, width: f32) -> (Vec<f32>, usize, bool) {
        let columns = table.columns();
        let widths = self.column_widths(table, columns, width);
        let has_header = table.style.header
            && !table
                .headers
                .iter()
                .all(|c| plain_text(c).trim().is_empty());
        (widths, columns, has_header)
    }

    /// The height the whole table would occupy at the given content width.
    pub(super) fn table_height(&self, table: &Table, width: f32) -> f32 {
        if table.columns() == 0 {
            return 0.0;
        }
        let (widths, columns, has_header) = self.table_setup(table, width);
        let style = &table.style;
        let mut height = 0.0;
        if has_header {
            let header_style = self.header_row_style(style);
            height += self.row_height(&table.headers, &widths, columns, &header_style);
        }
        let body_style = self.body_row_style(style, None);
        for row in &table.rows {
            height += self.row_height(row, &widths, columns, &body_style);
        }
        height
    }

    pub(super) fn layout_table(&mut self, table: &Table) {
        if table.columns() == 0 {
            return;
        }
        let (widths, columns, has_header) = self.table_setup(table, self.width());
        let xs = self.column_offsets(&widths);
        let style = &table.style;

        let header_style = self.header_row_style(style);

        // Reused every time the header row repeats after a page break.
        let header_height =
            has_header.then(|| self.row_height(&table.headers, &widths, columns, &header_style));

        if let Some(header_height) = header_height {
            self.emit_row(
                &table.headers,
                &xs,
                &widths,
                columns,
                &header_style,
                header_height,
            );
        }

        for (index, row) in table.rows.iter().enumerate() {
            let fill = if style.striped && index % 2 == 0 {
                Some(self.theme.palette.highlight)
            } else {
                None
            };
            let row_style = self.body_row_style(style, fill);

            let height = self.row_height(row, &widths, columns, &row_style);
            let content_top = self.theme.page.content_top();
            let content_bottom = self.theme.page.content_bottom();
            if self.y + height > content_bottom && self.y > content_top {
                self.new_page();
                if let Some(header_height) = header_height {
                    self.emit_row(
                        &table.headers,
                        &xs,
                        &widths,
                        columns,
                        &header_style,
                        header_height,
                    );
                }
            }
            self.emit_row(row, &xs, &widths, columns, &row_style, height);
        }
    }

    /// Compute the width of every column within the given total content width.
    pub(super) fn column_widths(&self, table: &Table, columns: usize, total: f32) -> Vec<f32> {
        if columns == 0 {
            return Vec::new();
        }

        match &table.style.columns {
            ColumnWidths::Labels => {
                // Two columns default to a 1:2 label/value split; otherwise equal.
                if columns == 2 {
                    vec![total / 3.0, total * 2.0 / 3.0]
                } else {
                    vec![total / columns as f32; columns]
                }
            }
            ColumnWidths::Auto => {
                // CSS auto-table model: each column has a min width (its widest
                // unbreakable word) and a max width (its content unwrapped);
                // the page width is distributed between those bounds.
                let pad = 2.0 * self.theme.table.inset_x;
                let metrics: Vec<(f32, f32)> = (0..columns)
                    .map(|c| self.column_metrics(table, c))
                    .collect();
                let max: Vec<f32> = metrics.iter().map(|&(_, natural)| natural + pad).collect();
                let min: Vec<f32> = metrics.iter().map(|&(min, _)| min + pad).collect();
                let max_total: f32 = max.iter().sum();
                let min_total: f32 = min.iter().sum();

                if max_total <= 0.0 {
                    // Empty table: fall back to an equal split.
                    return vec![total / columns as f32; columns];
                }

                if max_total <= total {
                    // Everything fits unwrapped; grow columns proportionally to
                    // fill the width.
                    let slack = total - max_total;
                    return (0..columns)
                        .map(|c| max[c] + slack * max[c] / max_total)
                        .collect();
                }

                if min_total <= total {
                    // Give each column its minimum, then share the rest in
                    // proportion to how much more each column could use.
                    let extra = total - min_total;
                    let flex_total: f32 = (0..columns).map(|c| max[c] - min[c]).sum();
                    return (0..columns)
                        .map(|c| {
                            let share = if flex_total > 0.0 {
                                extra * (max[c] - min[c]) / flex_total
                            } else {
                                extra / columns as f32
                            };
                            min[c] + share
                        })
                        .collect();
                }

                // Even the minimum widths overflow the page; scale them down
                // proportionally and let content wrap or hard-break.
                (0..columns).map(|c| min[c] * total / min_total).collect()
            }
            ColumnWidths::Custom(specs) => {
                let pad = 2.0 * self.theme.table.inset_x;
                let metrics: Vec<(f32, f32)> = (0..columns)
                    .map(|c| self.column_metrics(table, c))
                    .collect();
                let min: Vec<f32> = metrics.iter().map(|&(min, _)| min + pad).collect();
                // Columns past the end of the spec list fall back to Auto.
                let spec = |c: usize| specs.get(c).copied().unwrap_or(ColumnWidth::Auto);

                // Absolute and Auto columns claim their width first; fractional
                // columns then split the leftover by weight.
                let mut widths = vec![0.0_f32; columns];
                let mut used = 0.0;
                let mut frac_total = 0_u32;
                for c in 0..columns {
                    match spec(c) {
                        ColumnWidth::Absolute(w) => {
                            widths[c] = w;
                            used += w;
                        }
                        ColumnWidth::Auto => {
                            let w = (metrics[c].1 + pad).max(min[c]);
                            widths[c] = w;
                            used += w;
                        }
                        ColumnWidth::Fraction(n) => frac_total += n,
                    }
                }

                let leftover = (total - used).max(0.0);
                for c in 0..columns {
                    if let ColumnWidth::Fraction(n) = spec(c) {
                        let share = if frac_total > 0 {
                            leftover * n as f32 / frac_total as f32
                        } else {
                            0.0
                        };
                        widths[c] = share.max(min[c]);
                    }
                }

                widths
            }
        }
    }

    /// The `(min, natural)` content widths of a column across header and body:
    /// the widest single unbreakable word, and the widest cell with everything
    /// on one line.
    fn column_metrics(&self, table: &Table, column: usize) -> (f32, f32) {
        let size = self.table_font_size(&table.style);
        let mut min = 0.0_f32;
        let mut natural = 0.0_f32;
        let mut consider = |cell: &[Inline], italic: bool| {
            let words = self.tokenize(cell, false, italic, size);
            let mut cell_width = 0.0;
            for (i, word) in words.iter().enumerate() {
                if i > 0 {
                    cell_width += self.fonts.space_width(word.style, size);
                }
                cell_width += word.width;
                min = min.max(word.width);
            }
            natural = natural.max(cell_width);
        };
        if let Some(cell) = table.headers.get(column) {
            consider(cell, table.style.header_italic);
        }
        for row in &table.rows {
            if let Some(cell) = row.get(column) {
                consider(cell, false);
            }
        }
        (min, natural)
    }

    /// The min-content width of a column: the widest single unbreakable word
    /// across header and body.
    #[cfg(test)]
    pub(super) fn min_column_width(&self, table: &Table, column: usize) -> f32 {
        self.column_metrics(table, column).0
    }

    /// Height of a row: the tallest wrapped cell, clamped to a minimum, plus
    /// vertical insets.
    fn row_height(
        &self,
        cells: &[Vec<Inline>],
        widths: &[f32],
        columns: usize,
        style: &RowStyle,
    ) -> f32 {
        let body = style.size;
        let line_h = body * self.theme.spacing.line_height;
        let mut content = style.row_min_height;
        #[allow(clippy::needless_range_loop)] // reads parallel per-column arrays
        for c in 0..columns {
            let avail = self.cell_available_width(widths[c], c, style);
            let words = self.tokenize(
                cells.get(c).map(Vec::as_slice).unwrap_or(&[]),
                false,
                style.italic,
                body,
            );
            let lines = self.wrap(words, avail, body);
            content = content.max(lines.len() as f32 * line_h);
        }
        content + 2.0 * self.theme.table.inset_y
    }

    /// Draw one table row at the current pen position and advance past it.
    /// `height` is the row's height, as computed by [`row_height`](Self::row_height).
    #[allow(clippy::too_many_arguments)]
    fn emit_row(
        &mut self,
        cells: &[Vec<Inline>],
        xs: &[f32],
        widths: &[f32],
        columns: usize,
        style: &RowStyle,
        height: f32,
    ) {
        let top = self.y;

        if let Some(fill) = style.fill {
            let start = xs[0];
            let end = xs[columns - 1] + widths[columns - 1];
            self.push(Element::Rect {
                x: start,
                y: top,
                w: end - start,
                h: height,
                fill,
            });
        }

        let body = style.size;
        let line_h = body * self.theme.spacing.line_height;
        let inset_x = self.theme.table.inset_x;
        let inset_y = self.theme.table.inset_y;
        let text_color = self.theme.palette.text;
        for c in 0..columns {
            let cell = cells.get(c).map(Vec::as_slice).unwrap_or(&[]);
            if cell.is_empty() {
                if style.fill_in_blanks && c > 0 {
                    let y = top + height - inset_y;
                    let x0 = xs[c] + inset_x;
                    let x1 = xs[c] + widths[c] - inset_x;
                    self.push(Element::Stroke {
                        points: vec![(x0, y), (x1, y)],
                        width: 0.7,
                        color: text_color,
                        closed: false,
                    });
                }
                continue;
            }
            let inset_left = self.cell_inset_left(c, style);
            let avail = self.cell_available_width(widths[c], c, style);
            let words = self.tokenize(cell, false, style.italic, body);
            let lines = self.wrap(words, avail, body);
            // Vertically center the visible text body within the row. The
            // reference is the cap-height→baseline band (as Typst does), not the
            // full em box with its leading, so short cells sit optically centered
            // instead of riding high. For multi-line cells the band spans the
            // first line's cap top to the last line's baseline.
            let cap = self.fonts.cap_height(Style::Regular, body);
            let block_h = cap + (lines.len() as f32 - 1.0) * line_h;
            let first_baseline = top + (height - block_h) / 2.0 + cap;
            let mut top_of_text = first_baseline - self.fonts.ascent(Style::Regular, body);
            let align = style.align(c);
            for line in &lines {
                let x = match align {
                    Align::Left => xs[c] + inset_left,
                    Align::Center => {
                        xs[c] + inset_left + (avail - self.line_width(line, body)) / 2.0
                    }
                    Align::Right => xs[c] + widths[c] - inset_x - self.line_width(line, body),
                };
                self.draw_line(line, x, top_of_text, body, text_color);
                top_of_text += line_h;
            }
        }

        self.y = top + height;
    }

    fn cell_inset_left(&self, column: usize, style: &RowStyle) -> f32 {
        if style.flush_first_column && column == 0 {
            0.0
        } else {
            self.theme.table.inset_x
        }
    }

    fn cell_available_width(&self, width: f32, column: usize, style: &RowStyle) -> f32 {
        (width - self.cell_inset_left(column, style) - self.theme.table.inset_x).max(1.0)
    }

    /// Cumulative left edges of columns given their widths, starting at the left
    /// content margin.
    fn column_offsets(&self, widths: &[f32]) -> Vec<f32> {
        let mut xs = Vec::with_capacity(widths.len());
        let mut x = self.left;
        for w in widths {
            xs.push(x);
            x += w;
        }
        xs
    }
}

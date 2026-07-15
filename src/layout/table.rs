//! Table layout: column sizing (auto, label and custom models), row heights,
//! page-breaking with repeated headers, and row emission. A row that fits a
//! page but not the space left breaks to the next page as a whole; a row (or
//! cell) taller than a page splits and continues across pages.

use krilla::{color::rgb, tagging::TableHeaderScope};

use crate::{
    fonts::Style,
    layout::{
        Element, Engine, StructTag, Tagging,
        text::{Word, WordKind},
    },
    model::{Cell, Table},
    theme::{Align, ColumnWidth, ColumnWidths, TableStyle},
};

/// Styling for one table row, resolved from the table's [`TableStyle`] and the
/// theme.
pub(super) struct RowStyle<'a> {
    italic: bool,
    fill: Option<rgb::Color>,
    flush_first_column: bool,
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

/// Everything needed to redraw a table's header row (as an artifact) at the top
/// of a continuation page.
struct HeaderRepeat<'a> {
    cells: &'a [Cell],
    style: &'a RowStyle<'a>,
    height: f32,
    tags: &'a [Tagging],
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

    /// The height the whole table would occupy at the given content width.
    pub(super) fn table_height(&self, table: &Table, width: f32) -> f32 {
        let columns = table.columns();
        if columns == 0 {
            return 0.0;
        }
        let widths = self.column_widths(table, columns, width);
        let style = &table.style;
        let mut height = 0.0;
        if table.has_header() {
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
        let columns = table.columns();
        if columns == 0 {
            return;
        }
        let widths = self.column_widths(table, columns, self.width());
        let xs = self.column_offsets(&widths);
        let style = &table.style;

        let header_style = self.header_row_style(style);

        // Reused every time the header row repeats after a page break.
        let header_height = table
            .has_header()
            .then(|| self.row_height(&table.headers, &widths, columns, &header_style));
        let repeat_header_tags = vec![Tagging::Artifact; columns];
        let repeat_header = header_height.map(|height| HeaderRepeat {
            cells: &table.headers,
            style: &header_style,
            height,
            tags: &repeat_header_tags,
        });

        // A row taller than this cannot fit any page (a fresh page still
        // carries the repeated header) and is split across pages instead of
        // broken to the next one.
        let page_capacity = self.theme.page.content_bottom()
            - self.theme.page.content_top()
            - header_height.unwrap_or(0.0);
        // The smallest useful fragment of a split row: one line plus insets.
        let min_fragment = self.table_font_size(style) * self.theme.spacing.line_height
            + 2.0 * self.theme.table.inset_y;

        self.structure.open(StructTag::Table);

        // The header row is tagged once, on its first appearance, as header
        // cells scoped to their column. When it repeats after a page break it
        // is redrawn as an artifact so assistive tech reads it only once.
        if let Some(header_height) = header_height {
            // Orphan control: keep the header together with the first body row
            // — or, when that row will split anyway, with its first fragment —
            // so a table starting at the bottom of a page breaks *before* its
            // header instead of stranding it (or overflowing the margin).
            let first_row_height = table.rows.first().map_or(0.0, |row| {
                self.row_height(row, &widths, columns, &self.body_row_style(style, None))
            });
            let first_keep = if first_row_height > page_capacity {
                min_fragment
            } else {
                first_row_height
            };
            self.ensure(header_height + first_keep);
            let tags = self.push_row_structure(Some(TableHeaderScope::Column), columns);
            self.emit_row(
                &table.headers,
                &xs,
                &widths,
                columns,
                &header_style,
                header_height,
                &tags,
            );
        }

        for (index, row) in table.rows.iter().enumerate() {
            let fill = if style.striped && index % 2 == 0 {
                Some(self.theme.palette.highlight)
            } else {
                None
            };
            let row_style = self.body_row_style(style, fill);

            // A row that fits a page breaks to the next page as a whole; a
            // taller one splits across pages, so it only needs room for its
            // first fragment here.
            let height = self.row_height(row, &widths, columns, &row_style);
            let splits = height > page_capacity;
            if self.needs_break(if splits { min_fragment } else { height }) {
                self.new_page();
                if let Some(header) = &repeat_header {
                    self.emit_repeated_header(header, &xs, &widths, columns);
                }
            }
            let tags = self.push_row_structure(None, columns);
            if splits {
                self.emit_split_row(
                    row,
                    &xs,
                    &widths,
                    columns,
                    &row_style,
                    &tags,
                    repeat_header.as_ref(),
                );
            } else {
                self.emit_row(row, &xs, &widths, columns, &row_style, height, &tags);
            }
        }

        self.structure.close();
    }

    /// Redraw the header row (as an artifact) at the top of a continuation page.
    fn emit_repeated_header(
        &mut self,
        header: &HeaderRepeat,
        xs: &[f32],
        widths: &[f32],
        columns: usize,
    ) {
        self.emit_row(
            header.cells,
            xs,
            widths,
            columns,
            header.style,
            header.height,
            header.tags,
        );
    }

    /// Add a table row to the structure tree with one cell per column, and
    /// return the [`Tagging`] each column's content should carry. `header` gives
    /// the header-cell scope when this is a header row, or `None` for a body row
    /// of data cells.
    fn push_row_structure(
        &mut self,
        header: Option<TableHeaderScope>,
        columns: usize,
    ) -> Vec<Tagging> {
        self.structure.open(StructTag::TableRow);
        let tags = (0..columns)
            .map(|_| {
                let tag = match header {
                    Some(scope) => StructTag::TableHeaderCell(scope),
                    None => StructTag::TableCell,
                };
                Tagging::Content(self.structure.leaf(tag))
            })
            .collect();
        self.structure.close();
        tags
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
    /// the widest single unbreakable word, and the widest cell with each of
    /// its (hard-break-separated) lines unwrapped.
    pub(super) fn column_metrics(&self, table: &Table, column: usize) -> (f32, f32) {
        let size = self.table_font_size(&table.style);
        let mut min = 0.0_f32;
        let mut natural = 0.0_f32;
        let mut consider = |cell: &Cell, italic: bool| {
            let words = self.tokenize(cell.inlines(), false, italic, size);
            let mut line_width = 0.0;
            let mut first = true;
            for word in &words {
                if word.kind == WordKind::HardBreak {
                    natural = natural.max(line_width);
                    line_width = 0.0;
                    first = true;
                    continue;
                }
                if !first {
                    line_width += self.fonts.space_width(word.style, size);
                }
                line_width += word.width;
                min = min.max(word.width);
                first = false;
            }
            natural = natural.max(line_width);
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

    /// The minimum content height a row's *non-text* cells ask for: the style's
    /// floor, any spacer cell's height, and the fill-in minimum (room to write
    /// above a fill-in line).
    fn row_min_content(&self, cells: &[Cell], columns: usize, style: &RowStyle) -> f32 {
        let mut content = style.row_min_height;
        for c in 0..columns {
            match cells.get(c) {
                Some(Cell::Spacer(height)) => content = content.max(*height),
                Some(Cell::FillIn) => content = content.max(self.theme.table.fill_in_min_height),
                _ => {}
            }
        }
        content
    }

    /// Height of a row: the tallest wrapped cell, clamped to the row's minimum
    /// content height, plus vertical insets.
    fn row_height(&self, cells: &[Cell], widths: &[f32], columns: usize, style: &RowStyle) -> f32 {
        let body = style.size;
        let line_h = body * self.theme.spacing.line_height;
        let mut content = self.row_min_content(cells, columns, style);
        #[allow(clippy::needless_range_loop)] // reads parallel per-column arrays
        for c in 0..columns {
            let cell = cells.get(c);
            // Spacer and fill-in cells contribute via `row_min_content`; every
            // other cell wraps to at least one (possibly empty) line.
            if matches!(cell, Some(Cell::Spacer(_) | Cell::FillIn)) {
                continue;
            }
            let avail = self.cell_available_width(widths[c], c, style);
            let words = self.tokenize(
                cell.map(Cell::inlines).unwrap_or(&[]),
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
    /// `tags` gives the structure tagging for each column's text (one entry per
    /// column), so cells land in their header/data cell in the structure tree.
    #[allow(clippy::too_many_arguments)]
    fn emit_row(
        &mut self,
        cells: &[Cell],
        xs: &[f32],
        widths: &[f32],
        columns: usize,
        style: &RowStyle,
        height: f32,
        tags: &[Tagging],
    ) {
        let top = self.y;
        self.push_row_fill(xs, widths, columns, top, height, style);

        let body = style.size;
        let line_h = body * self.theme.spacing.line_height;
        let text_color = self.theme.palette.text;
        for c in 0..columns {
            // Text drawn below lands in this column's structure cell; the fill
            // and fill-in line drawn as strokes/rects stay artifacts regardless.
            self.current_tag = tags[c];
            let cell = match cells.get(c) {
                Some(Cell::FillIn) => {
                    // A fill-in line along the cell's bottom inset.
                    self.push_fill_in_line(c, xs, widths, style, top + height);
                    continue;
                }
                Some(Cell::Text(inlines)) => inlines.as_slice(),
                Some(Cell::Blank | Cell::Spacer(_)) | None => continue,
            };
            if cell.is_empty() {
                continue;
            }
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
            for line in &lines {
                let x = self.line_x(line, c, xs, widths, style);
                self.draw_line(line, x, top_of_text, body, text_color);
                top_of_text += line_h;
            }
        }

        self.y = top + height;
    }

    /// Draw a row (or row fragment) that is too tall for any single page by
    /// splitting it into page-sized pieces: every cell's wrapped lines continue
    /// top-aligned across pages (repeating the table header, when there is
    /// one), each fragment carries the row's fill and insets, and a fill-in
    /// line lands on the final fragment. Spacer heights beyond the last text
    /// line keep consuming pages until they are used up.
    #[allow(clippy::too_many_arguments)]
    fn emit_split_row(
        &mut self,
        cells: &[Cell],
        xs: &[f32],
        widths: &[f32],
        columns: usize,
        style: &RowStyle,
        tags: &[Tagging],
        header: Option<&HeaderRepeat>,
    ) {
        let body = style.size;
        let line_h = body * self.theme.spacing.line_height;
        let inset_y = self.theme.table.inset_y;
        let text_color = self.theme.palette.text;

        // Wrap every text cell once, up front; the shared cursor below walks
        // all columns in lockstep, one page-sized batch of lines at a time.
        let cell_lines: Vec<Vec<Vec<Word>>> = (0..columns)
            .map(|c| {
                let inlines = cells.get(c).map(Cell::inlines).unwrap_or(&[]);
                if inlines.is_empty() {
                    return Vec::new();
                }
                let avail = self.cell_available_width(widths[c], c, style);
                self.wrap(
                    self.tokenize(inlines, false, style.italic, body),
                    avail,
                    body,
                )
            })
            .collect();
        let total_lines = cell_lines.iter().map(Vec::len).max().unwrap_or(0);
        let min_content = self.row_min_content(cells, columns, style);

        let mut cursor = 0; // lines drawn so far
        let mut consumed = 0.0; // content height drawn so far
        loop {
            // The content this fragment can hold. At least one line, so a
            // caller starting the row too low overflows a little instead of
            // never making progress.
            let avail = (self.theme.page.content_bottom() - self.y - 2.0 * inset_y).max(line_h);
            let take = ((avail / line_h) as usize).min(total_lines - cursor);
            // Beyond its lines, a fragment grows toward any outstanding
            // minimum content (a spacer cell, the style's row minimum),
            // clamped to the page.
            let content_h = (take as f32 * line_h).max((min_content - consumed).clamp(0.0, avail));
            let last = cursor + take == total_lines && consumed + content_h >= min_content - 0.01;

            let top = self.y;
            let height = content_h + 2.0 * inset_y;
            self.push_row_fill(xs, widths, columns, top, height, style);

            for c in 0..columns {
                self.current_tag = tags[c];
                if matches!(cells.get(c), Some(Cell::FillIn)) {
                    // The fill-in line lands on the final fragment's bottom.
                    if last {
                        self.push_fill_in_line(c, xs, widths, style, top + height);
                    }
                    continue;
                }
                let lines = &cell_lines[c];
                let end = (cursor + take).min(lines.len());
                let mut top_of_text = top + inset_y;
                for line in &lines[cursor.min(end)..end] {
                    let x = self.line_x(line, c, xs, widths, style);
                    self.draw_line(line, x, top_of_text, body, text_color);
                    top_of_text += line_h;
                }
            }

            self.y = top + height;
            cursor += take;
            consumed += content_h;
            if last {
                break;
            }
            self.new_page();
            if let Some(header) = header {
                self.emit_repeated_header(header, xs, widths, columns);
            }
        }
    }

    /// Push the row's background fill (zebra striping), when it has one.
    fn push_row_fill(
        &mut self,
        xs: &[f32],
        widths: &[f32],
        columns: usize,
        top: f32,
        height: f32,
        style: &RowStyle,
    ) {
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
    }

    /// Push a fill-in cell's line, spanning column `c` along its bottom inset;
    /// `bottom` is the y of the row's bottom edge.
    fn push_fill_in_line(
        &mut self,
        c: usize,
        xs: &[f32],
        widths: &[f32],
        style: &RowStyle,
        bottom: f32,
    ) {
        let y = bottom - self.theme.table.inset_y;
        let x0 = xs[c] + self.cell_inset_left(c, style);
        let x1 = xs[c] + widths[c] - self.theme.table.inset_x;
        self.push(Element::Stroke {
            points: vec![(x0, y), (x1, y)],
            width: 0.7,
            color: self.theme.palette.text,
            closed: false,
        });
    }

    /// The x where one line of column `c` starts, honoring the column's
    /// alignment.
    fn line_x(&self, line: &[Word], c: usize, xs: &[f32], widths: &[f32], style: &RowStyle) -> f32 {
        let inset_left = self.cell_inset_left(c, style);
        match style.align(c) {
            Align::Left => xs[c] + inset_left,
            Align::Center => {
                let avail = self.cell_available_width(widths[c], c, style);
                xs[c] + inset_left + (avail - self.line_width(line, style.size)) / 2.0
            }
            Align::Right => {
                xs[c] + widths[c] - self.theme.table.inset_x - self.line_width(line, style.size)
            }
        }
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

//! Per-element presentation styles: how one table or one callout box looks.
//!
//! Unlike the document-wide [`Theme`](super::Theme) tokens, these are chosen per
//! element when it is added to the document. Define a set of styles up front and
//! pass one when adding a table or box.

use krilla::color::rgb;

use super::em;

/// Horizontal alignment of a column's cell content, including its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    /// Align lines to the left cell edge.
    #[default]
    Left,
    /// Center lines within the cell.
    Center,
    /// Align lines to the right cell edge.
    Right,
}

/// How a single column is sized within a [`ColumnWidths::Custom`] layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColumnWidth {
    /// Size the column to its content (its max-content width, clamped to fit).
    Auto,
    /// Take a share of the leftover space (after `Auto` and `Absolute` columns),
    /// proportional to this weight relative to the other fractional columns.
    Fraction(u32),
    /// A fixed width in points.
    Absolute(f32),
}

/// How a table's columns are sized.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnWidths {
    /// Size every column to its content (min/max-content, distributing slack).
    Auto,
    /// Label layout: a two-column table gets a 1:2 label/value split; any other
    /// column count is divided equally.
    Labels,
    /// Size each column explicitly. The `n`th entry sizes the `n`th column;
    /// columns past the end of the list fall back to [`ColumnWidth::Auto`].
    ///
    /// ```
    /// use textris_pdf::theme::{ColumnWidth, ColumnWidths};
    ///
    /// // Four columns: content-sized, a 3:7 fractional split, then a fixed 300pt.
    /// let widths = ColumnWidths::custom([
    ///     ColumnWidth::Auto,
    ///     ColumnWidth::Fraction(3),
    ///     ColumnWidth::Fraction(7),
    ///     ColumnWidth::Absolute(300.0),
    /// ]);
    /// ```
    Custom(Vec<ColumnWidth>),
}

impl ColumnWidths {
    /// Build a [`ColumnWidths::Custom`] from any iterator of [`ColumnWidth`].
    pub fn custom(widths: impl IntoIterator<Item = ColumnWidth>) -> Self {
        Self::Custom(widths.into_iter().collect())
    }
}

/// The presentation of a single table.
///
/// Build the common cases with [`TableStyle::data`] / [`TableStyle::label`],
/// then tweak individual fields:
///
/// ```
/// use textris_pdf::theme::TableStyle;
///
/// // A data table with the zebra striping turned off.
/// let plain = TableStyle { striped: false, ..TableStyle::data() };
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct TableStyle {
    /// Treat the first row as a header, rendered in the header style. A header
    /// row is still omitted when its cells are all blank.
    pub header: bool,
    /// Render the header row in italics.
    pub header_italic: bool,
    /// Zebra-stripe alternating body rows with [`Palette::highlight`](super::Palette::highlight).
    pub striped: bool,
    /// Drop the left inset on the first column (a flush key column).
    pub flush_first_column: bool,
    /// Turn empty cells (after the first column) into fill-in lines.
    pub fill_in_blanks: bool,
    /// How column widths are computed.
    pub columns: ColumnWidths,
    /// Per-column horizontal alignment. The `n`th entry aligns the `n`th
    /// column; columns past the end of the list fall back to [`Align::Left`].
    ///
    /// ```
    /// use textris_pdf::theme::{Align, TableStyle};
    ///
    /// // A data table with right-aligned amounts in the last of three columns.
    /// let amounts = TableStyle {
    ///     align: vec![Align::Left, Align::Left, Align::Right],
    ///     ..TableStyle::data()
    /// };
    /// ```
    pub align: Vec<Align>,
    /// Font size for the table's cells, in points. `None` uses the theme's
    /// [body size](super::FontSizes::body).
    pub font_size: Option<f32>,
    /// Minimum height of a row's content area, in points. Rows still grow to fit
    /// taller content. `None` uses the theme's
    /// [`TableMetrics::row_min_height`](super::TableMetrics::row_min_height).
    pub row_min_height: Option<f32>,
}

impl TableStyle {
    /// A data table: italic header row, zebra-striped body rows, content-sized
    /// columns.
    pub fn data() -> Self {
        Self {
            header: true,
            header_italic: true,
            striped: true,
            flush_first_column: false,
            fill_in_blanks: false,
            columns: ColumnWidths::Auto,
            align: Vec::new(),
            font_size: None,
            row_min_height: None,
        }
    }

    /// A label table: no header, a flush left label column, empty value cells
    /// become fill-in lines, and a 1:2 label/value column split.
    pub fn label() -> Self {
        Self {
            header: false,
            header_italic: false,
            striped: false,
            flush_first_column: true,
            fill_in_blanks: true,
            columns: ColumnWidths::Labels,
            align: Vec::new(),
            font_size: None,
            row_min_height: None,
        }
    }
}

impl Default for TableStyle {
    fn default() -> Self {
        Self::data()
    }
}

/// The presentation of a boxed callout.
///
/// A box draws its child blocks on a filled background, with `padding` between the
/// background edge and the content and `margin` between the background edge and
/// the surrounding blocks. Build the common case with [`BoxStyle::callout`], then
/// tweak individual fields:
///
/// ```
/// use textris_pdf::theme::BoxStyle;
/// use krilla::color::rgb;
///
/// // A callout tinted blue instead of grey.
/// let note = BoxStyle {
///     background: rgb::Color::new(0xEE, 0xF4, 0xFF),
///     ..BoxStyle::callout()
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxStyle {
    /// Background fill drawn behind the box's content.
    pub background: rgb::Color,
    /// Horizontal space between the background edge and the content.
    pub padding_x: f32,
    /// Vertical space between the background edge and the content.
    pub padding_y: f32,
    /// Horizontal space between the surrounding content edge and the background,
    /// inset on both sides.
    pub margin_x: f32,
    /// Vertical space reserved above and below the background.
    pub margin_y: f32,
}

impl BoxStyle {
    /// A grey callout box, suited to highlighted notes and warnings.
    pub fn callout() -> Self {
        Self {
            background: rgb::Color::new(0xEC, 0xEC, 0xEC),
            padding_x: em(1.0),
            padding_y: em(1.0),
            margin_x: 0.0,
            margin_y: 0.0,
        }
    }
}

impl Default for BoxStyle {
    fn default() -> Self {
        Self::callout()
    }
}

//! Visual design tokens for the document, gathered into a configurable [`Theme`].
//!
//! All measurements are in PDF points (1pt = 1/72 inch). [`Theme::default`]
//! reproduces the reference design; construct a `Theme` and override individual
//! fields to re-skin the renderer without touching layout logic, then hand it to
//! the builder with [`Textris::with_theme`](crate::build::Textris::with_theme).
//!
//! The theme holds *document-wide* tokens. Presentation choices that vary per
//! element (how one table or one callout box looks) live in the per-element
//! styles ([`TableStyle`], [`BoxStyle`]) and are passed alongside the element
//! when it is added to the document.

mod style;

pub use style::{Align, BoxStyle, ColumnWidth, ColumnWidths, TableStyle};

/// The RGB color type used throughout the theme and text APIs, re-exported
/// from krilla (also available as [`crate::Color`]).
pub use krilla::color::rgb::Color;

use krilla::color::rgb;

/// One centimeter expressed in points.
const CM: f32 = 72.0 / 2.54;

/// The default base font size; the default token sizes are expressed relative to
/// it via [`em`].
pub const BASE_FONT_SIZE: f32 = 9.0;

/// Convert a length given in em (relative to [`BASE_FONT_SIZE`]) into points.
/// Handy for expressing overrides in the same relative units the defaults use.
pub const fn em(value: f32) -> f32 {
    value * BASE_FONT_SIZE
}

/// The complete set of design tokens used to lay out and render a document.
///
/// Every stage of the pipeline reads its measurements from here, so overriding a
/// field re-skins the output. Get the reference design with [`Theme::default`]
/// and change what you need:
///
/// ```
/// use textris_pdf::{Color, theme::Theme};
///
/// let mut theme = Theme::default();
/// theme.palette.highlight = Color::new(0xEE, 0xF4, 0xFF); // bluer stripes
/// theme.spacing.line_height = 1.5;                        // looser leading
/// theme.spacing.heading_above.h3 = 12.0;                  // tighter sections
/// ```
#[derive(Debug, Clone, Default)]
pub struct Theme {
    /// Page geometry and margins.
    pub page: PageTheme,
    /// Font sizes for the various text roles.
    pub font_size: FontSizes,
    /// Vertical spacing between blocks.
    pub spacing: Spacing,
    /// Metrics shared by every table (insets, minimum row height).
    pub table: TableMetrics,
    /// Checkbox metrics for task lists.
    pub checkbox: Checkbox,
    /// Bullet- and task-list spacing.
    pub list: ListTheme,
    /// The document color palette.
    pub palette: Palette,
}

/// A4 page geometry and margins, plus the derived content box.
#[derive(Debug, Clone)]
pub struct PageTheme {
    pub width: f32,
    pub height: f32,
    pub margin_x: f32,
    pub margin_y: f32,
    /// Distance the header baseline sits above the top content edge.
    pub header_offset: f32,
    /// Distance the footer baseline sits below the bottom content edge.
    pub footer_offset: f32,
}

impl Default for PageTheme {
    fn default() -> Self {
        Self {
            width: 595.276,
            height: 841.89,
            margin_x: 1.5 * CM,
            margin_y: 2.5 * CM,
            header_offset: 30.0,
            footer_offset: 30.0,
        }
    }
}

impl PageTheme {
    /// Width available for content between the left and right margins.
    pub fn content_width(&self) -> f32 {
        self.width - 2.0 * self.margin_x
    }

    /// The x coordinate of the left content edge.
    pub fn content_left(&self) -> f32 {
        self.margin_x
    }

    /// The x coordinate of the right content edge.
    pub fn content_right(&self) -> f32 {
        self.width - self.margin_x
    }

    /// The y coordinate where body content starts (top margin).
    pub fn content_top(&self) -> f32 {
        self.margin_y
    }

    /// The y coordinate where body content must stop (bottom margin).
    pub fn content_bottom(&self) -> f32 {
        self.height - self.margin_y
    }
}

/// Font sizes for the various text roles.
#[derive(Debug, Clone)]
pub struct FontSizes {
    /// Body text.
    pub body: f32,
    /// Level-1 heading (document title).
    pub h1: f32,
    /// Level-2 heading (subtitle / label).
    pub h2: f32,
    /// Level-3 heading (section heading).
    pub h3: f32,
    /// Level-4 heading (subsection heading).
    pub h4: f32,
    /// Level-5 and deeper headings.
    pub h5: f32,
    /// Header and footer text.
    pub chrome: f32,
}

impl Default for FontSizes {
    fn default() -> Self {
        Self {
            body: BASE_FONT_SIZE,
            h1: em(2.0),
            h2: em(1.5),
            h3: em(1.4),
            h4: em(1.2),
            h5: em(1.1),
            chrome: BASE_FONT_SIZE,
        }
    }
}

impl FontSizes {
    /// The size for a heading at `level` (1 = largest; levels beyond 5 share the
    /// level-5 size).
    pub fn heading(&self, level: u8) -> f32 {
        match level {
            1 => self.h1,
            2 => self.h2,
            3 => self.h3,
            4 => self.h4,
            _ => self.h5,
        }
    }
}

/// Vertical spacing between blocks, tuned for a consistent vertical rhythm.
#[derive(Debug, Clone)]
pub struct Spacing {
    /// Space above a heading, per heading level.
    pub heading_above: HeadingSpacing,
    /// Space below a heading (before its content), per heading level.
    pub heading_below: HeadingSpacing,
    /// Space between ordinary blocks (paragraphs, tables, lists).
    pub block: f32,
    /// Leading multiplier: baseline-to-baseline distance as a factor of font size.
    pub line_height: f32,
    /// Keep-together threshold: a section (heading plus its content) is pushed to
    /// the next page rather than broken, as long as its height does not exceed
    /// this fraction of the page's content height. Taller sections are allowed to
    /// break across pages so they never get stuck.
    pub keep_together_max_fraction: f32,
}

impl Default for Spacing {
    fn default() -> Self {
        Self {
            heading_above: HeadingSpacing {
                h1: em(3.0),
                h2: em(2.5),
                h3: em(2.0),
                h4: em(1.75),
                h5: em(1.5),
            },
            heading_below: HeadingSpacing {
                h1: em(1.25),
                h2: em(1.0),
                h3: em(0.75),
                h4: em(0.75),
                h5: em(0.75),
            },
            block: em(0.75),
            line_height: 1.35,
            keep_together_max_fraction: 0.5,
        }
    }
}

/// A vertical distance defined per heading level, so top-level headings can get
/// more air than subsections. Levels beyond 5 use the level-5 value.
#[derive(Debug, Clone)]
pub struct HeadingSpacing {
    pub h1: f32,
    pub h2: f32,
    pub h3: f32,
    pub h4: f32,
    pub h5: f32,
}

impl HeadingSpacing {
    /// The same distance for every heading level.
    pub const fn uniform(value: f32) -> Self {
        Self {
            h1: value,
            h2: value,
            h3: value,
            h4: value,
            h5: value,
        }
    }

    /// The distance for a heading at `level` (levels beyond 5 share the
    /// level-5 value).
    pub fn level(&self, level: u8) -> f32 {
        match level {
            1 => self.h1,
            2 => self.h2,
            3 => self.h3,
            4 => self.h4,
            _ => self.h5,
        }
    }
}

/// Metrics shared by every table, regardless of its [`TableStyle`].
#[derive(Debug, Clone)]
pub struct TableMetrics {
    /// Horizontal padding inside a cell.
    pub inset_x: f32,
    /// Vertical padding inside a cell.
    pub inset_y: f32,
    /// Minimum height of a row's content area.
    pub row_min_height: f32,
    /// Minimum content height of a row holding a fill-in cell, so there is room
    /// to write above the line.
    pub fill_in_min_height: f32,
}

impl Default for TableMetrics {
    fn default() -> Self {
        Self {
            inset_x: em(0.75),
            inset_y: em(0.42),
            row_min_height: em(1.0),
            fill_in_min_height: em(3.0),
        }
    }
}

/// Checkbox styling for task-list items.
#[derive(Debug, Clone)]
pub struct Checkbox {
    /// Side length of the square.
    pub size: f32,
    /// Gap between the box and its label.
    pub gap: f32,
    /// Stroke width of the checkmark.
    pub check_width: f32,
}

impl Default for Checkbox {
    fn default() -> Self {
        Self {
            size: em(1.0),
            gap: em(0.67),
            check_width: 1.2,
        }
    }
}

/// Spacing for bullet and task lists.
#[derive(Debug, Clone)]
pub struct ListTheme {
    /// Vertical gap between task-list items.
    pub task_gap: f32,
    /// Vertical gap between bullet-list items.
    pub bullet_gap: f32,
    /// Indent from the content edge to a bullet item's text.
    pub bullet_indent: f32,
}

impl Default for ListTheme {
    fn default() -> Self {
        Self {
            task_gap: em(0.6),
            bullet_gap: em(0.3),
            bullet_indent: em(1.2),
        }
    }
}

/// The document color palette.
#[derive(Debug, Clone)]
pub struct Palette {
    /// Default text color.
    pub text: rgb::Color,
    /// Muted (secondary) text.
    pub muted: rgb::Color,
    /// Background fill for zebra-striped rows.
    pub highlight: rgb::Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            text: rgb::Color::new(0, 0, 0),
            muted: rgb::Color::new(0x88, 0x88, 0x88),
            highlight: rgb::Color::new(0xF6, 0xF6, 0xF6),
        }
    }
}

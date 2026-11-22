//! Font loading, text shaping and measurement.
//!
//! krilla can *draw* glyphs but keeps its font metrics private, so shaping is
//! done here with `harfrust` (HarfBuzz's official Rust port), built on the
//! same fontations font-parsing stack krilla uses, so the widths the layout
//! engine measures match what krilla draws.
//!
//! No typeface is baked into the library. Supply your own with one of the
//! constructors: [`Fonts::from_variable_files`], [`Fonts::from_variable`], or
//! [`Fonts::from_faces`] (fully general: one [`FaceSource`] per style). Font
//! bytes are `&'static [u8]`: embed them with `include_bytes!`, or let
//! [`FaceSource::from_owned`] / [`Fonts::from_variable_files`] leak
//! runtime-loaded bytes once. Construct [`Fonts`] once per process.
//!
//! Variation coordinates (axis tag + value) are applied both when embedding a
//! font and before every shaping pass, so measurement and drawing agree on the
//! exact instance.
//!
//! A character the face of its style has no glyph for is drawn from another
//! face instead: first the fallback faces added with
//! [`Fonts::with_fallback`], in order, then the monospace face (the regular
//! one for monospace text). Only a character none of them has is drawn as the
//! font's "missing glyph" box.
//!
//! Shaping is left-to-right only: right-to-left scripts (Hebrew, Arabic) are
//! not supported and would render in the wrong visual order.

mod face;

pub use face::{FaceSource, Shaped};

use std::{ops::Range, path::Path};

use face::Face;
use harfrust::Language;
use krilla::text::Font;

/// A font variation axis tag, e.g. `*b"wght"` for weight.
pub type AxisTag = [u8; 4];

/// The weight axis (`wght`), the most common variation axis.
pub const WEIGHT: AxisTag = *b"wght";
/// The optical-size axis (`opsz`).
pub const OPTICAL_SIZE: AxisTag = *b"opsz";

const REGULAR_WEIGHT: f32 = 400.0;
const BOLD_WEIGHT: f32 = 700.0;

/// The typographic styles a text run can be rendered in.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum Style {
    Regular,
    Bold,
    Italic,
    BoldItalic,
    /// Monospace (rendered like inline code).
    Mono,
    /// The fallback face at this index, in the order they were added with
    /// [`Fonts::with_fallback`]. Not a style to ask for: layout picks it for
    /// characters the requested style's face lacks.
    Fallback(u8),
}

/// The set of fonts used to render a document.
pub struct Fonts {
    regular: Face,
    bold: Face,
    italic: Face,
    bold_italic: Face,
    mono: Face,
    fallbacks: Vec<Face>,
}

impl Fonts {
    /// Read a roman, an italic and a monospace variable font from files and build
    /// the font set as [`from_variable`](Self::from_variable) does.
    ///
    /// The bytes read are leaked into `'static` slices (see
    /// [`FaceSource::from_owned`]): load fonts once per process, not per document.
    pub fn from_variable_files(
        roman: impl AsRef<Path>,
        italic: impl AsRef<Path>,
        mono: impl AsRef<Path>,
    ) -> std::io::Result<Self> {
        fn leak(data: Vec<u8>) -> &'static [u8] {
            Box::leak(data.into_boxed_slice())
        }
        Self::from_variable(
            leak(std::fs::read(roman)?),
            leak(std::fs::read(italic)?),
            leak(std::fs::read(mono)?),
        )
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "failed to parse a font file",
            )
        })
    }

    /// Build the font set from a roman and an italic variable font (sharing the
    /// `wght` axis) plus a monospace font. Regular/bold are weight 400/700.
    ///
    /// The `'static` bytes are typically embedded with `include_bytes!`; for
    /// bytes read at runtime, go through [`FaceSource::from_owned`] or
    /// [`from_variable_files`](Self::from_variable_files).
    pub fn from_variable(
        roman: &'static [u8],
        italic: &'static [u8],
        mono: &'static [u8],
    ) -> Option<Self> {
        Self::from_faces(
            FaceSource::new(roman).with_variation(WEIGHT, REGULAR_WEIGHT),
            FaceSource::new(roman).with_variation(WEIGHT, BOLD_WEIGHT),
            FaceSource::new(italic).with_variation(WEIGHT, REGULAR_WEIGHT),
            FaceSource::new(italic).with_variation(WEIGHT, BOLD_WEIGHT),
            FaceSource::new(mono).with_variation(WEIGHT, REGULAR_WEIGHT),
        )
    }

    /// Fully general constructor: supply an explicit [`FaceSource`] for each
    /// style. This is the low-level entry point; the sources may be static fonts
    /// (no variations) or variable fonts pinned to any instance.
    pub fn from_faces(
        regular: FaceSource,
        bold: FaceSource,
        italic: FaceSource,
        bold_italic: FaceSource,
        mono: FaceSource,
    ) -> Option<Self> {
        Some(Self {
            regular: Face::new(regular)?,
            bold: Face::new(bold)?,
            italic: Face::new(italic)?,
            bold_italic: Face::new(bold_italic)?,
            mono: Face::new(mono)?,
            fallbacks: Vec::new(),
        })
    }

    /// Add a fallback face: one that characters missing from the face of
    /// their style are drawn from, such as a symbol or CJK font. Fallbacks
    /// are tried in the order they were added; at most 256 are used. `None`
    /// if the font cannot be parsed.
    ///
    /// ```
    /// use textris_pdf::fonts::{FaceSource, Fonts};
    /// static ROMAN: &[u8] =
    ///     include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/Newsreader/Newsreader-Variable.ttf"));
    /// static ITALIC: &[u8] =
    ///     include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/Newsreader/Newsreader-Italic-Variable.ttf"));
    /// static MONO: &[u8] =
    ///     include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/Fira_Code/FiraCode-Variable.ttf"));
    /// let fonts = Fonts::from_variable(ROMAN, ITALIC, MONO)
    ///     .unwrap()
    ///     .with_fallback(FaceSource::new(MONO))
    ///     .unwrap();
    /// ```
    pub fn with_fallback(mut self, source: FaceSource) -> Option<Self> {
        self.fallbacks.push(Face::new(source)?);
        Some(self)
    }

    fn face(&self, style: Style) -> &Face {
        match style {
            Style::Regular => &self.regular,
            Style::Bold => &self.bold,
            Style::Italic => &self.italic,
            Style::BoldItalic => &self.bold_italic,
            Style::Mono => &self.mono,
            // An index no fallback was added for can only be made up.
            Style::Fallback(index) => self.fallbacks.get(index as usize).unwrap_or(&self.regular),
        }
    }

    /// The faces to try for text in `style`, in order: its own, the added
    /// fallbacks, then the one built-in face likely to cover other characters.
    fn fallback_chain(&self, style: Style) -> Vec<Style> {
        let fallbacks = (0..=u8::MAX)
            .take(self.fallbacks.len())
            .map(Style::Fallback);
        let other = match style {
            Style::Mono => Style::Regular,
            _ => Style::Mono,
        };
        std::iter::once(style)
            .chain(fallbacks)
            .chain([other])
            .collect()
    }

    /// The krilla font for a style, needed when emitting glyphs.
    pub fn krilla_font(&self, style: Style) -> Font {
        self.face(style).krilla.clone()
    }

    /// Shape a text run in the given style.
    pub fn shape(&self, style: Style, text: &str) -> Shaped {
        self.face(style).shape(text, &None)
    }

    /// Measure the width of a text run at a font size, in points.
    pub fn measure(&self, style: Style, text: &str, size: f32) -> f32 {
        self.shape(style, text).width(size)
    }

    /// Width of a single space at a font size, in points.
    pub fn space_width(&self, style: Style, size: f32) -> f32 {
        self.face(style).space_advance * size
    }

    /// Distance from the baseline to the top of the text, in points.
    pub fn ascent(&self, style: Style, size: f32) -> f32 {
        self.face(style).ascender * size
    }

    /// Distance from the baseline to the bottom of the text, in points.
    pub fn descent(&self, style: Style, size: f32) -> f32 {
        self.face(style).descender * size
    }

    /// Distance from the baseline to the top of the capitals, in points. Used
    /// to vertically center a line on its visible body rather than its em box.
    /// Falls back to the ascent for fonts that do not report a cap height.
    pub fn cap_height(&self, style: Style, size: f32) -> f32 {
        self.face(style).cap_height * size
    }
}

/// A [`Fonts`] set shaping text in one language: what a document is laid out
/// and drawn with, so that fonts with language-specific forms (a Dutch `ĳ`, a
/// Turkish dotless `ı`, Serbian italics) show the right ones.
#[derive(Clone)]
pub(crate) struct Shaper<'a> {
    fonts: &'a Fonts,
    language: Option<Language>,
}

impl<'a> Shaper<'a> {
    /// Shape with `fonts`, reading text as being in `language` (a BCP 47 tag
    /// such as `nl`), if any.
    pub(crate) fn new(fonts: &'a Fonts, language: Option<&str>) -> Self {
        Self {
            fonts,
            language: language.and_then(Language::new),
        }
    }

    /// See [`Fonts::krilla_font`].
    pub(crate) fn krilla_font(&self, style: Style) -> Font {
        self.fonts.krilla_font(style)
    }

    /// Shape a text run in the given style.
    pub(crate) fn shape(&self, style: Style, text: &str) -> Shaped {
        self.fonts.face(style).shape(text, &self.language)
    }

    /// See [`Fonts::measure`].
    pub(crate) fn measure(&self, style: Style, text: &str, size: f32) -> f32 {
        self.shape(style, text).width(size)
    }

    /// See [`Fonts::space_width`].
    pub(crate) fn space_width(&self, style: Style, size: f32) -> f32 {
        self.fonts.space_width(style, size)
    }

    /// See [`Fonts::ascent`].
    pub(crate) fn ascent(&self, style: Style, size: f32) -> f32 {
        self.fonts.ascent(style, size)
    }

    /// See [`Fonts::descent`].
    pub(crate) fn descent(&self, style: Style, size: f32) -> f32 {
        self.fonts.descent(style, size)
    }

    /// See [`Fonts::cap_height`].
    pub(crate) fn cap_height(&self, style: Style, size: f32) -> f32 {
        self.fonts.cap_height(style, size)
    }

    /// Split `text` in `style` into the pieces to shape with one face each:
    /// all of it in `style` when its face has every glyph, otherwise the
    /// uncovered grapheme clusters in the first face of the
    /// [fallback chain](Fonts::fallback_chain) that has them. What no face
    /// has stays in `style`. The pieces are in order and cover `text`.
    pub(crate) fn split(&self, style: Style, text: &str) -> Vec<(Style, Range<usize>)> {
        let whole = 0..text.len();
        if self.shape(style, text).missing(text).is_empty() {
            return vec![(style, whole)];
        }
        let mut pieces = Vec::new();
        self.cover(
            &self.fonts.fallback_chain(style),
            style,
            text,
            whole,
            &mut pieces,
        );
        pieces.dedup_by(|next, piece| {
            let same = next.0 == piece.0;
            if same {
                piece.1.end = next.1.end;
            }
            same
        });
        pieces
    }

    /// Append the pieces of `text[range]` to `pieces`, trying the faces of
    /// `chain` in order and leaving what none of them has to `style`.
    fn cover(
        &self,
        chain: &[Style],
        style: Style,
        text: &str,
        range: Range<usize>,
        pieces: &mut Vec<(Style, Range<usize>)>,
    ) {
        let Some((&face, rest)) = chain.split_first() else {
            pieces.push((style, range));
            return;
        };
        let piece = &text[range.clone()];
        let mut covered = range.start;
        for missing in self.shape(face, piece).missing(piece) {
            let missing = range.start + missing.start..range.start + missing.end;
            if covered < missing.start {
                pieces.push((face, covered..missing.start));
            }
            covered = missing.end;
            self.cover(rest, style, text, missing, pieces);
        }
        if covered < range.end {
            pieces.push((face, covered..range.end));
        }
    }
}

/// Load the bundled test fonts (Newsreader + Fira Code from `tests/fonts`).
/// Test scaffolding only; the library itself names no typeface.
#[cfg(test)]
pub(crate) fn test_fonts() -> Fonts {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts");
    Fonts::from_variable_files(
        format!("{dir}/Newsreader/Newsreader-Variable.ttf"),
        format!("{dir}/Newsreader/Newsreader-Italic-Variable.ttf"),
        format!("{dir}/Fira_Code/FiraCode-Variable.ttf"),
    )
    .expect("test fonts should load")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaping_produces_glyphs_and_positive_width() {
        let fonts = test_fonts();
        let shaped = fonts.shape(Style::Regular, "Stomatopoda");
        assert_eq!(shaped.glyphs.len(), "Stomatopoda".chars().count());
        assert!(shaped.width(9.0) > 0.0);
    }

    #[test]
    fn the_glyphs_of_a_cluster_share_its_text_range() {
        let fonts = test_fonts();
        // No precomposed form exists: a base glyph plus two mark glyphs.
        let shaped = fonts.shape(Style::Regular, "x\u{301}\u{302}y");
        let ranges: Vec<_> = shaped.glyphs.iter().map(|g| g.text_range.clone()).collect();
        assert_eq!(ranges, [0..5, 0..5, 0..5, 5..6]);
    }

    #[test]
    fn text_splits_over_the_faces_that_have_its_glyphs() {
        let fonts = test_fonts();
        let shaper = Shaper::new(&fonts, None);
        assert_eq!(
            shaper.split(Style::Regular, "plain"),
            [(Style::Regular, 0..5)]
        );
        // "→" and "✓" are three bytes each; only the monospace face has them.
        assert_eq!(
            shaper.split(Style::Bold, "a→✓b日"),
            [
                (Style::Bold, 0..1),
                (Style::Mono, 1..7),
                (Style::Bold, 7..11),
            ]
        );
        assert_eq!(shaper.split(Style::Mono, "→"), [(Style::Mono, 0..3)]);
    }

    #[test]
    fn width_scales_linearly_with_size() {
        let fonts = test_fonts();
        let w9 = fonts.measure(Style::Regular, "abc", 9.0);
        let w18 = fonts.measure(Style::Regular, "abc", 18.0);
        assert!((w18 - 2.0 * w9).abs() < 0.01);
    }

    #[test]
    fn wider_text_measures_wider() {
        let fonts = test_fonts();
        let short = fonts.measure(Style::Regular, "i", 9.0);
        let long = fonts.measure(Style::Regular, "immmm", 9.0);
        assert!(long > short);
    }

    /// The error kind `from_variable_files` fails with, panicking on success.
    fn load_error_kind(path: &str) -> std::io::ErrorKind {
        match Fonts::from_variable_files(path, path, path) {
            Ok(_) => panic!("loading {path} should fail"),
            Err(err) => err.kind(),
        }
    }

    #[test]
    fn from_variable_files_reports_unparsable_font_data() {
        // Any existing non-font file: this very source file.
        let src = concat!(env!("CARGO_MANIFEST_DIR"), "/src/fonts/mod.rs");
        assert_eq!(load_error_kind(src), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn from_variable_files_reports_a_missing_file() {
        assert_eq!(
            load_error_kind("/nonexistent/font.ttf"),
            std::io::ErrorKind::NotFound
        );
    }

    #[test]
    fn vertical_metrics_are_sane() {
        let fonts = test_fonts();
        assert!(fonts.ascent(Style::Regular, 9.0) > 0.0);
        assert!(fonts.descent(Style::Regular, 9.0) > 0.0);
        assert!(fonts.ascent(Style::Regular, 9.0) > fonts.descent(Style::Regular, 9.0));
    }

    #[test]
    fn heavier_weight_is_wider_from_the_same_variable_font() {
        // Regular and Bold come from the *same* Newsreader variable file, differing
        // only on the `wght` axis, so bold must render wider than regular.
        let fonts = test_fonts();
        let regular = fonts.measure(Style::Regular, "Stomatopoda", 9.0);
        let bold = fonts.measure(Style::Bold, "Stomatopoda", 9.0);
        assert!(
            bold > regular,
            "bold ({bold}) should be wider than regular ({regular})"
        );
    }

    #[test]
    fn explicit_weights_bracket_the_default() {
        // Build the same face at three weights and confirm advances are ordered.
        static ROMAN: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fonts/Newsreader/Newsreader-Variable.ttf"
        ));
        let make = |w: f32| {
            Fonts::from_faces(
                FaceSource::new(ROMAN).with_variation(WEIGHT, w),
                FaceSource::new(ROMAN).with_variation(WEIGHT, BOLD_WEIGHT),
                FaceSource::new(ROMAN).with_variation(WEIGHT, w),
                FaceSource::new(ROMAN).with_variation(WEIGHT, w),
                FaceSource::new(ROMAN).with_variation(WEIGHT, w),
            )
            .unwrap()
            .measure(Style::Regular, "Stomatopoda", 12.0)
        };
        let (thin, mid, heavy) = (make(100.0), make(400.0), make(900.0));
        assert!(
            thin < mid && mid < heavy,
            "widths not ordered: {thin} {mid} {heavy}"
        );
    }
}

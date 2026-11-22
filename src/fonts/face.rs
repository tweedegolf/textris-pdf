//! A single loaded font face: parsing, variation pinning, shaping and metrics.

use std::{
    collections::HashMap,
    ops::Range,
    sync::{Arc, Mutex},
};

use harfrust::{Buffer, Direction, Language, ShapeOptions, ShaperFont, Tag as HrTag};
use krilla::text::{Font, Glyph, GlyphId, KrillaGlyph, Tag as KrillaTag};
use read_fonts::{
    TableProvider,
    model::{self, NormalizedCoord, Tables, Variation},
    tables::os2::SelectionFlags,
};
use unicode_segmentation::UnicodeSegmentation;

use super::AxisTag;

/// A description of a font face to load: its bytes, collection index and the
/// variation coordinates that pin it to a specific instance.
///
/// The bytes are `&'static`: either embedded in the binary or leaked once at
/// load time (see [`FaceSource::from_owned`]).
///
/// ```
/// use textris_pdf::fonts::{FaceSource, WEIGHT};
/// static REGULAR: &[u8] =
///     include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/Newsreader/Newsreader-Variable.ttf"));
/// let bold = FaceSource::new(REGULAR).with_variation(WEIGHT, 700.0);
/// ```
#[derive(Clone, Debug)]
pub struct FaceSource {
    pub data: &'static [u8],
    pub index: u32,
    pub variations: Vec<(AxisTag, f32)>,
}

impl FaceSource {
    /// A face from raw font bytes, at collection index 0, with no variations.
    pub fn new(data: &'static [u8]) -> Self {
        Self {
            data,
            index: 0,
            variations: Vec::new(),
        }
    }

    /// A face from owned bytes (e.g. read from a file at runtime), leaked into
    /// a `'static` slice.
    ///
    /// The leak is deliberate: fonts are expected to be loaded once per process
    /// and used for its lifetime. Avoid calling this repeatedly (say, per
    /// rendered document); each call leaks its bytes.
    pub fn from_owned(data: Vec<u8>) -> Self {
        Self::new(Box::leak(data.into_boxed_slice()))
    }

    /// Set the collection index (for TrueType collections).
    pub fn with_index(mut self, index: u32) -> Self {
        self.index = index;
        self
    }

    /// Pin a variation axis to a value. Chainable for multiple axes.
    pub fn with_variation(mut self, axis: AxisTag, value: f32) -> Self {
        self.variations.push((axis, value));
        self
    }
}

/// The result of shaping a text run: the glyphs to draw plus the total advance
/// width, expressed in font-size-relative (em) units.
///
/// The glyphs are behind an [`Arc`], so a `Shaped` clones cheaply: runs are
/// shaped once and the glyph list is shared from then on.
#[derive(Clone, Debug)]
pub struct Shaped {
    pub glyphs: Arc<[KrillaGlyph]>,
    /// Total advance width, normalized by units-per-em. Multiply by the font
    /// size to get the width in points.
    pub advance: f32,
}

impl Shaped {
    /// Width of the shaped run at the given font size, in points.
    pub fn width(&self, size: f32) -> f32 {
        self.advance * size
    }

    /// The parts of `text`, the text this was shaped from, that the face has
    /// no glyphs for: whole grapheme clusters, in order, neighbours joined.
    pub(super) fn missing(&self, text: &str) -> Vec<Range<usize>> {
        let notdef = |glyph: &&KrillaGlyph| glyph.glyph_id().to_u32() == 0;
        let mut missing: Vec<Range<usize>> = Vec::new();
        if !self.glyphs.iter().any(|glyph| notdef(&glyph)) {
            return missing;
        }
        for (start, grapheme) in text.grapheme_indices(true) {
            let end = start + grapheme.len();
            let lacks =
                |glyph: &KrillaGlyph| glyph.text_range.start < end && start < glyph.text_range.end;
            if !self.glyphs.iter().filter(notdef).any(lacks) {
                continue;
            }
            match missing.last_mut() {
                Some(last) if last.end == start => last.end = end,
                _ => missing.push(start..end),
            }
        }
        missing
    }
}

/// The longest text, in bytes, whose shaping is memoized: words recur, whole
/// lines hardly do.
const CACHE_MAX_TEXT: usize = 32;
/// The number of memoized shapings at which the cache starts over.
const CACHE_CAPACITY: usize = 16_384;

/// A single loaded font face: the krilla font used for drawing plus a parsed
/// `read-fonts` instance (with variations applied) used for shaping.
pub(super) struct Face {
    pub(super) krilla: Font,
    /// The pinned variation instance. It caches the shaping tables, so the
    /// per-call [`ShaperFont`] is cheap to build.
    font: model::Font,
    /// Memoized shaping results; documents shape the same short runs many
    /// times. Bounded by [`CACHE_MAX_TEXT`] and [`CACHE_CAPACITY`]: a face
    /// lives as long as the process does. Keyed by language first: the same
    /// text can shape differently in another one.
    cache: Mutex<HashMap<Option<Language>, HashMap<String, Shaped>>>,
    units_per_em: f32,
    /// Ascender, normalized by units-per-em (positive, above baseline).
    pub(super) ascender: f32,
    /// Descender, normalized by units-per-em (positive, below baseline).
    pub(super) descender: f32,
    /// Cap height, normalized by units-per-em (positive, above baseline).
    /// Falls back to the ascender for fonts that do not report one.
    pub(super) cap_height: f32,
    /// Advance of a single space, normalized by units-per-em; precomputed for
    /// the line breaker.
    pub(super) space_advance: f32,
}

impl Face {
    pub(super) fn new(source: FaceSource) -> Option<Self> {
        let FaceSource {
            data,
            index,
            variations,
        } = source;

        let krilla_coords: Vec<(KrillaTag, f32)> = variations
            .iter()
            .map(|(tag, value)| (KrillaTag::new(tag), *value))
            .collect();
        let krilla = Font::new_variable(data.into(), index, &krilla_coords)?;

        let font = model::Font::new(data, index)?
            .instance_builder()
            .variations(
                variations
                    .iter()
                    .map(|(tag, value)| Variation::new(HrTag::new(tag), *value)),
            )
            .build();

        let tables = font.tables();
        let coords = font.normalized_coords();
        let units_per_em = tables.head().ok()?.units_per_em() as f32;
        let ascender = ascender(tables, coords) as f32 / units_per_em;
        let descender = descender(tables, coords).unsigned_abs() as f32 / units_per_em;
        // Prefer the font's reported cap height; fall back to the ascender for
        // faces that omit it (or report a non-positive value).
        let cap_height = capital_height(tables, coords)
            .filter(|&c| c > 0)
            .map_or(ascender, |c| c as f32 / units_per_em);

        let mut face = Self {
            krilla,
            font,
            cache: Mutex::new(HashMap::new()),
            units_per_em,
            ascender,
            descender,
            cap_height,
            space_advance: 0.0,
        };
        face.space_advance = face.shape(" ", &None).advance;
        Some(face)
    }

    /// Shape `text`, read as being in `language` (which selects the font's
    /// language-specific forms, where it has any).
    pub(super) fn shape(&self, text: &str, language: &Option<Language>) -> Shaped {
        let cached = text.len() <= CACHE_MAX_TEXT;
        if cached
            && let Some(hit) = self
                .cache
                .lock()
                .unwrap()
                .get(language)
                .and_then(|shaped| shaped.get(text))
        {
            return hit.clone();
        }
        let mut buffer = Buffer::new();
        buffer.push_str(text);
        buffer.set_language(language.clone());
        buffer.guess_segment_properties();
        // Shaping is left-to-right only (RTL scripts are unsupported); the
        // forced direction also keeps the cluster arithmetic below valid.
        buffer.set_direction(Direction::LeftToRight);

        // Only fails on API misuse: a reshaped buffer, an unset direction or a
        // mismatched plan, none of which can happen here.
        harfrust::shape(
            &ShaperFont::new(&self.font),
            &mut buffer,
            ShapeOptions::new(),
        )
        .expect("fresh left-to-right buffer always shapes");
        let positions = buffer.glyph_positions();
        let infos = buffer.glyph_infos();

        let mut glyphs = Vec::with_capacity(buffer.len());
        let mut advance = 0.0;
        for i in 0..buffer.len() {
            let pos = positions[i];
            let info = infos[i];
            let start = info.cluster as usize;
            // The cluster runs up to the next cluster's start (LTR). All the
            // glyphs of a cluster share that range: krilla maps each to the
            // text in its range, and an empty one would map to nothing.
            let end = infos[i + 1..]
                .iter()
                .find(|next| next.cluster != info.cluster)
                .map_or(text.len(), |next| next.cluster as usize);

            let x_advance = pos.x_advance as f32 / self.units_per_em;
            advance += x_advance;
            glyphs.push(KrillaGlyph::new(
                GlyphId::new(info.glyph_id),
                x_advance,
                pos.x_offset as f32 / self.units_per_em,
                pos.y_offset as f32 / self.units_per_em,
                pos.y_advance as f32 / self.units_per_em,
                start..end,
                None,
            ));
        }

        let shaped = Shaped {
            glyphs: glyphs.into(),
            advance,
        };
        if cached {
            let mut cache = self.cache.lock().unwrap();
            if cache.values().map(HashMap::len).sum::<usize>() >= CACHE_CAPACITY {
                cache.clear();
            }
            cache
                .entry(language.clone())
                .or_default()
                .insert(text.to_owned(), shaped.clone());
        }
        shaped
    }
}

// Vertical metrics: prefer the OS/2 typographic metrics when the
// USE_TYPO_METRICS flag is set, otherwise hhea, with the OS/2 typographic and
// Windows metrics as fallbacks for zero hhea values. Every path applies the
// matching MVAR delta so pinned variable-font instances get instance metrics.

fn ascender(font: &Tables, coords: &[NormalizedCoord]) -> i16 {
    let os2 = font.os2().ok();
    if let Some(os2) = &os2
        && os2.version() >= 4
        && os2
            .fs_selection()
            .contains(SelectionFlags::USE_TYPO_METRICS)
    {
        return apply_metric_delta(font, coords, b"hasc", os2.s_typo_ascender());
    }
    let mut value = font.hhea().map_or(0, |hhea| hhea.ascender().to_i16());
    if value == 0
        && let Some(os2) = &os2
    {
        value = os2.s_typo_ascender();
        if value == 0 {
            return apply_metric_delta(font, coords, b"hcla", saturate(os2.us_win_ascent()));
        }
    }
    apply_metric_delta(font, coords, b"hasc", value)
}

fn descender(font: &Tables, coords: &[NormalizedCoord]) -> i16 {
    let os2 = font.os2().ok();
    if let Some(os2) = &os2
        && os2.version() >= 4
        && os2
            .fs_selection()
            .contains(SelectionFlags::USE_TYPO_METRICS)
    {
        return apply_metric_delta(font, coords, b"hdsc", os2.s_typo_descender());
    }
    let mut value = font.hhea().map_or(0, |hhea| hhea.descender().to_i16());
    if value == 0
        && let Some(os2) = &os2
    {
        value = os2.s_typo_descender();
        if value == 0 {
            // usWinDescent is positive-below-baseline; negate to match the
            // hhea sign convention.
            return apply_metric_delta(font, coords, b"hcld", -saturate(os2.us_win_descent()));
        }
    }
    apply_metric_delta(font, coords, b"hdsc", value)
}

/// Clamp a u16 Windows metric into i16 range instead of wrapping.
fn saturate(value: u16) -> i16 {
    i16::try_from(value).unwrap_or(i16::MAX)
}

fn capital_height(font: &Tables, coords: &[NormalizedCoord]) -> Option<i16> {
    font.os2()
        .ok()
        .and_then(|os2| os2.s_cap_height())
        .map(|value| apply_metric_delta(font, coords, b"cpht", value))
}

/// Add the MVAR delta for `tag` at `coords` to a metric, keeping the
/// unadjusted value when the result would overflow.
fn apply_metric_delta(font: &Tables, coords: &[NormalizedCoord], tag: &[u8; 4], value: i16) -> i16 {
    if coords.is_empty() {
        return value;
    }
    let delta = font
        .mvar()
        .ok()
        .and_then(|mvar| mvar.metric_delta(HrTag::new(tag), coords))
        .map_or(0.0, |delta| delta.to_f64() as f32);
    let adjusted = value as f32 + delta;
    if adjusted >= i16::MIN as f32 && adjusted <= i16::MAX as f32 {
        adjusted as i16
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::{CACHE_CAPACITY, CACHE_MAX_TEXT};
    use crate::fonts::test_fonts;

    #[test]
    fn the_shape_cache_is_bounded() {
        let fonts = test_fonts();
        let face = &fonts.regular;
        let entries = || -> usize { face.cache.lock().unwrap().values().map(|m| m.len()).sum() };
        let before = entries();
        let long = "a".repeat(CACHE_MAX_TEXT + 1);
        assert!(face.shape(&long, &None).advance > 0.0);
        assert_eq!(entries(), before);

        for i in 0..CACHE_CAPACITY + 10 {
            face.shape(&i.to_string(), &None);
        }
        let len = entries();
        assert!(0 < len && len <= CACHE_CAPACITY, "{len} entries");
    }
}

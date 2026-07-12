//! A single loaded font face: parsing, variation pinning, shaping and metrics.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use harfrust::{
    Direction, FontRef, NormalizedCoord, ShapeOptions, ShaperData, ShaperInstance, Tag as HrTag,
    UnicodeBuffer, Variation,
};
use krilla::text::{Font, GlyphId, KrillaGlyph, Tag as KrillaTag};
use read_fonts::{TableProvider, tables::os2::SelectionFlags};

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
}

/// A single loaded font face: the krilla font used for drawing plus a parsed
/// `harfrust` font (with variations applied) used for shaping.
pub(super) struct Face {
    pub(super) krilla: Font,
    font: FontRef<'static>,
    /// Shaping caches shared by every [`shape`](Self::shape) call; building the
    /// per-call `Shaper` from these is cheap.
    shaper_data: ShaperData,
    /// The pinned variation instance (normalized coordinates).
    instance: ShaperInstance,
    /// Memoized shaping results; documents shape the same short runs many times.
    cache: Mutex<HashMap<String, Shaped>>,
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

        let font = FontRef::from_index(data, index).ok()?;
        let instance = ShaperInstance::from_variations(
            &font,
            variations.iter().map(|(tag, value)| Variation {
                tag: HrTag::new(tag),
                value: *value,
            }),
        );
        let shaper_data = ShaperData::new(&font);

        let coords = instance.coords();
        let units_per_em = font.head().ok()?.units_per_em() as f32;
        let ascender = ascender(&font, coords) as f32 / units_per_em;
        let descender = descender(&font, coords).unsigned_abs() as f32 / units_per_em;
        // Prefer the font's reported cap height; fall back to the ascender for
        // faces that omit it (or report a non-positive value).
        let cap_height = capital_height(&font, coords)
            .filter(|&c| c > 0)
            .map_or(ascender, |c| c as f32 / units_per_em);

        let mut face = Self {
            krilla,
            font,
            shaper_data,
            instance,
            cache: Mutex::new(HashMap::new()),
            units_per_em,
            ascender,
            descender,
            cap_height,
            space_advance: 0.0,
        };
        face.space_advance = face.shape(" ").advance;
        Some(face)
    }

    pub(super) fn shape(&self, text: &str) -> Shaped {
        if let Some(hit) = self.cache.lock().unwrap().get(text) {
            return hit.clone();
        }
        let shaper = self
            .shaper_data
            .shaper(&self.font)
            .instance(Some(&self.instance))
            .build();

        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        buffer.set_direction(Direction::LeftToRight);

        let output = shaper.shape(buffer, ShapeOptions::new());
        let positions = output.glyph_positions();
        let infos = output.glyph_infos();

        let mut glyphs = Vec::with_capacity(output.len());
        let mut advance = 0.0;
        for i in 0..output.len() {
            let pos = positions[i];
            let info = infos[i];
            let start = info.cluster as usize;
            // Extend the cluster to the next glyph's cluster start (LTR).
            let end = infos
                .get(i + 1)
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
        self.cache
            .lock()
            .unwrap()
            .insert(text.to_owned(), shaped.clone());
        shaped
    }
}

// Vertical metrics: prefer the OS/2 typographic metrics when the
// USE_TYPO_METRICS flag is set, otherwise hhea, with the OS/2 typographic and
// Windows metrics as fallbacks for zero hhea values. MVAR deltas apply at
// non-default variation coordinates.

fn ascender(font: &FontRef, coords: &[NormalizedCoord]) -> i16 {
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
            value = apply_metric_delta(font, coords, b"hcla", os2.us_win_ascent() as i16);
        } else {
            value = apply_metric_delta(font, coords, b"hasc", value);
        }
    }
    value
}

fn descender(font: &FontRef, coords: &[NormalizedCoord]) -> i16 {
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
            value = apply_metric_delta(font, coords, b"hcld", -(os2.us_win_descent() as i16));
        } else {
            value = apply_metric_delta(font, coords, b"hdsc", value);
        }
    }
    value
}

fn capital_height(font: &FontRef, coords: &[NormalizedCoord]) -> Option<i16> {
    font.os2()
        .ok()
        .and_then(|os2| os2.s_cap_height())
        .map(|value| apply_metric_delta(font, coords, b"cpht", value))
}

/// Add the MVAR delta for `tag` at `coords` to a metric, keeping the
/// unadjusted value when the result would overflow.
fn apply_metric_delta(
    font: &FontRef,
    coords: &[NormalizedCoord],
    tag: &[u8; 4],
    value: i16,
) -> i16 {
    if coords.is_empty() {
        return value;
    }
    let delta = font
        .mvar()
        .ok()
        .and_then(|mvar| mvar.metric_delta(HrTag::new(tag), coords).ok())
        .map_or(0.0, |delta| delta.to_f64() as f32);
    let adjusted = value as f32 + delta;
    if adjusted >= i16::MIN as f32 && adjusted <= i16::MAX as f32 {
        adjusted as i16
    } else {
        value
    }
}

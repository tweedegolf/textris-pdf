//! A single loaded font face: parsing, variation pinning, shaping and metrics.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use krilla::text::{Font, GlyphId, KrillaGlyph, Tag as KrillaTag};
use rustybuzz::{Direction, Face as RbFace, UnicodeBuffer, Variation, ttf_parser::Tag as RbTag};

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
/// `rustybuzz` face (with variations applied) used for shaping.
pub(super) struct Face {
    pub(super) krilla: Font,
    rb: RbFace<'static>,
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

        let mut rb = RbFace::from_slice(data, index)?;
        rb.set_variations(&rb_variations(&variations));

        let units_per_em = rb.units_per_em() as f32;
        let ascender = rb.ascender() as f32 / units_per_em;
        let descender = rb.descender().unsigned_abs() as f32 / units_per_em;
        // Prefer the font's reported cap height; fall back to the ascender for
        // faces that omit it (or report a non-positive value).
        let cap_height = rb
            .capital_height()
            .filter(|&c| c > 0)
            .map_or(ascender, |c| c as f32 / units_per_em);

        let mut face = Self {
            krilla,
            rb,
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
        let face = &self.rb;

        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        buffer.set_direction(Direction::LeftToRight);

        let output = rustybuzz::shape(face, &[], buffer);
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

fn rb_variations(variations: &[(AxisTag, f32)]) -> Vec<Variation> {
    variations
        .iter()
        .map(|(tag, value)| Variation {
            tag: RbTag::from_bytes(tag),
            value: *value,
        })
        .collect()
}

//! `wasm-bindgen` bindings for `textris-pdf`, backing the live editor whose
//! static files sit next to this crate in `web/`.
//!
//! Glue only: this crate hands JS-supplied font bytes to [`Fonts`], parses a
//! [Markdown dialect](textris_pdf::markdown) source and returns PDF bytes. All
//! rendering logic stays in the library.
//!
//! `wasm32-unknown-unknown` has no clock, so the caller passes the creation
//! date in explicitly (see [`Renderer::render`]). That is the only thing a
//! browser build must supply that a native one does not.

use textris_pdf::{
    build::Textris,
    fonts::{FaceSource, Fonts},
    layout::layout,
    markdown::ParseOptions,
    render::render,
};
use wasm_bindgen::prelude::*;

/// Install a panic hook so a Rust panic reaches the browser console as a
/// readable message and stack trace rather than an opaque `unreachable`.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// A failure to load fonts, parse the source or render the PDF.
#[wasm_bindgen(getter_with_clone)]
pub struct TextrisError {
    /// What went wrong, e.g. ``unknown attribute key `stripd` for a table``.
    pub message: String,
    /// The 1-based source line the message points at, or 0 when the failure
    /// carries no source position (font and render errors).
    pub line: u32,
}

impl TextrisError {
    /// An error with no source position.
    fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            line: 0,
        }
    }
}

/// A rendered document, plus the map from source lines to the pages they
/// landed on.
///
/// `lines` and `pages` are parallel and ascending by line: `lines[i]` is the
/// 1-based source line a block starts at, and `pages[i]` the 1-based page it
/// renders on. To find the page for a cursor line, take the last entry whose
/// line is at or before it.
#[wasm_bindgen(getter_with_clone)]
pub struct Rendered {
    pub pdf: Vec<u8>,
    pub lines: Vec<u32>,
    pub pages: Vec<u32>,
    pub page_count: u32,
}

/// A loaded font set, ready to render documents.
///
/// Build one per page load and reuse it: the font bytes are leaked into
/// `'static` (see [`FaceSource::from_owned`]), so every construction leaks
/// another copy.
#[wasm_bindgen]
pub struct Renderer {
    fonts: Fonts,
}

#[wasm_bindgen]
impl Renderer {
    /// Build the font set from three variable fonts: a roman, its italic
    /// companion — both sharing the `wght` axis, from which regular and bold are
    /// derived — and a monospace face.
    #[wasm_bindgen(constructor)]
    pub fn new(roman: Vec<u8>, italic: Vec<u8>, mono: Vec<u8>) -> Result<Renderer, TextrisError> {
        // `from_owned` is the library's leak-once helper; take the `'static`
        // slice back out of it so the roman face can serve both regular and bold.
        let leak = |bytes: Vec<u8>| FaceSource::from_owned(bytes).data;
        Fonts::from_variable(leak(roman), leak(italic), leak(mono))
            .map(|fonts| Renderer { fonts })
            .ok_or_else(|| TextrisError::plain("failed to parse a font file"))
    }

    /// Parse `source` as dialect Markdown and render it to tagged PDF bytes.
    ///
    /// `numbered_heading_levels` are the heading levels that get automatic
    /// section numbers (the bundled example uses `[3, 4]`). `now_unix` is the
    /// creation date in Unix seconds, which the browser supplies from
    /// `Date.now()` because wasm has no clock of its own.
    ///
    /// This walks the pipeline stage by stage rather than calling
    /// `Textris::render`, because the source map needs the [`Layout`] that the
    /// convenience method discards.
    pub fn render(
        &self,
        source: &str,
        numbered_heading_levels: Vec<u8>,
        now_unix: f64,
    ) -> Result<Rendered, TextrisError> {
        let options = ParseOptions {
            numbered_heading_levels,
            ..ParseOptions::default()
        };
        let mut builder = Textris::new();
        builder.created_at(now_unix as i64);
        builder
            .push_markdown(source, &options)
            .map_err(|e| TextrisError {
                message: e.message,
                line: e.line as u32,
            })?;

        let source_map = builder.source_map().to_vec();
        let document = builder.build();
        let laid_out = layout(&document, &self.fonts);
        let pdf = render(&laid_out, &document, &self.fonts)
            .map_err(|e| TextrisError::plain(e.to_string()))?;

        // `get` rather than indexing: an out-of-range block index would panic,
        // and a panic traps the wasm module for the rest of the page's life.
        let (lines, pages) = source_map
            .iter()
            .filter_map(|&(block, line)| {
                let page = laid_out.block_pages.get(block)?;
                Some((line as u32, *page as u32 + 1))
            })
            .unzip();

        Ok(Rendered {
            pdf,
            lines,
            pages,
            page_count: laid_out.pages.len() as u32,
        })
    }
}

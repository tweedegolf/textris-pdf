//! The display list: absolutely-positioned drawing primitives produced by
//! layout and consumed by the renderer.

use std::sync::Arc;

use krilla::{color::rgb, text::KrillaGlyph};

use crate::fonts::Style;

/// A single laid-out page: a flat list of drawing primitives.
#[derive(Debug, Default)]
pub struct Page {
    pub elements: Vec<Element>,
}

/// A drawing primitive positioned in page space.
#[derive(Debug, Clone)]
pub enum Element {
    /// A run of shaped glyphs drawn from `(x, baseline)`.
    Text(TextElement),
    /// A filled rectangle.
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: rgb::Color,
    },
    /// A stroked polyline; `closed` connects the last point back to the first.
    Stroke {
        points: Vec<(f32, f32)>,
        width: f32,
        color: rgb::Color,
        closed: bool,
    },
}

/// A positioned run of shaped text.
#[derive(Debug, Clone)]
pub struct TextElement {
    pub x: f32,
    pub baseline: f32,
    pub size: f32,
    pub color: rgb::Color,
    pub style: Style,
    /// Shaped glyphs, shared with the shaping cache (see [`crate::fonts::Shaped`]).
    pub glyphs: Arc<[KrillaGlyph]>,
    pub text: String,
}

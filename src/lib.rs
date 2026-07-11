//! # textris-pdf
//!
//! A lightweight document renderer: it builds a PDF with a clean, form-like
//! design from Rust code, using [`krilla`] as the PDF backend.
//!
//! Documents are assembled with the imperative [`build`] API and then run through
//! three decoupled stages, each in its own module:
//!
//! 1. [`build`] assembles a [`model::Document`] block by block.
//! 2. [`layout`] turns that model into positioned pages of drawing primitives,
//!    using [`fonts`] for text measurement.
//! 3. [`render`] paints those primitives into a PDF with krilla.
//!
//! [`theme`] holds the visual design tokens shared across stages, plus the
//! per-element styles ([`theme::TableStyle`], [`theme::BoxStyle`]).
//!
//! ## Example
//!
//! ```no_run
//! use textris_pdf::build::Textris;
//! use textris_pdf::fonts::Fonts;
//!
//! let fonts = Fonts::from_variable_files("regular.ttf", "italic.ttf", "mono.ttf").unwrap();
//!
//! let mut doc = Textris::new();
//! doc.h2("Marine species profile");
//! doc.h1("The Mantis Shrimp");
//! doc.paragraph("Hello.");
//!
//! doc.render_to_file("out.pdf", &fonts).unwrap();
//! ```

pub mod build;
pub mod fonts;
pub mod layout;
pub mod model;
pub mod render;
pub mod theme;

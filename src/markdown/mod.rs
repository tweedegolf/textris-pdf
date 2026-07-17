//! Markdown support, gated behind the `markdown` cargo feature.
//!
//! Two directions, sharing one escaping convention:
//!
//! - **Export** ([`to_markdown`], [`Textris::to_markdown`](crate::build::Textris::to_markdown)):
//!   translate a built [`Document`](crate::model::Document) into a
//!   GitHub-flavored Markdown string, e.g. for display on a forge or in chat.
//! - **Parse** (`parse_markdown`, `Textris::push_markdown`, behind the
//!   additional `markdown-parser` cargo feature): the inverse, turning a
//!   string in a strict Markdown dialect into document blocks, so a host can author
//!   documents as (templated) Markdown text instead of builder calls. The
//!   dialect is documented in the `parse` module.
//!
//! The [`escape`], [`escape_cell`], [`mono`] and [`mono_cell`] functions define
//! the escaping the exporter emits and the parser understands. Host template
//! engines interpolating untrusted data into dialect templates must run every
//! interpolated value through them; see the `parse` module's *Escaping* section.

mod escape;
mod export;
#[cfg(feature = "markdown-parser")]
pub mod parse;

pub use escape::{escape, escape_cell, mono, mono_cell};
pub use export::to_markdown;
#[cfg(feature = "markdown-parser")]
pub use parse::{MarkdownParseError, ParseOptions, parse_markdown};

# Markdown dialect parser

Status: implemented, behind the `markdown-parser` cargo feature.

The design that used to live in this file is now implemented and documented
where it is maintained:

- **Dialect reference**: the module rustdoc of
  [`src/markdown/parse.rs`](../src/markdown/parse.rs) (front matter, blocks,
  attribute lines, directives, inlines, strictness).
- **Escaping design and guarantees**:
  [`src/markdown/escape.rs`](../src/markdown/escape.rs): `escape`,
  `escape_cell`, `mono`, `mono_cell`, shared by the exporter, the parser and
  host template engines. The round-trip guarantees are enforced by property
  tests in `parse.rs`.
- **Entry points**: `markdown::parse_markdown` (free function, default
  palette, body blocks only, rejects front matter) and `Textris::push_markdown`
  (applies front-matter chrome to the document, appends the body, and resolves
  box `background` palette roles against the document theme).
- **README**: the *Markdown, docx* section gives the overview and a usage
  example; [`tests/mantis-shrimp-example-input.md`](../tests/mantis-shrimp-example-input.md)
  re-authors the whole bundled field guide (chrome and body) as dialect text,
  and `tests/render_example.rs` asserts it parses back to the same document the
  builder produces.

The theme and section-number resolution stay on the builder API. Document
chrome (title, language, headers, footers) can be set on the builder or, for a
self-contained file, in a `+++` front-matter block that `push_markdown`
applies; a chrome value carrying `{page}` / `{total}` placeholders becomes a
page counter. Inline colors (muted text) have no dialect syntax, matching the
exporter, so a front-matter footer is plain where a builder one may be muted.

## Host-side template wiring

The parser is template-engine agnostic: it consumes a plain string. What the
host must guarantee is that **every interpolated value passes through the
crate's escape functions**, so untrusted data (names, addresses) can never
change document structure:

- **askama**: register a custom escaper for the `.md` extension in
  `askama.toml` whose `Escaper` impl delegates to `textris_pdf::markdown::escape`.
  Auto-escaping then covers every `{{ }}` by default, so template authors
  cannot forget it; expose `escape_cell` / `mono` / `mono_cell` as filters
  (e.g. `{{ value | cell }}`). Only static template text, which is trusted,
  bypasses escaping.
- **minijinja**: the same via `set_auto_escape_callback` plus a custom
  formatter.

One sharp edge: askama's **default (HTML) escaper must never be used** for
these templates; HTML escaping corrupts the text and leaves Markdown syntax
live. The `.md` extension mapping makes the safe escaper the default.

## Deliberately lossy round-trip

`to_markdown` stays GFM-for-display. The parser accepts its output, and a test
asserts the block structure and text survive; heading numbering, section-ref
resolution, fill-in widths, ordered-list marker style, table styles beyond
alignment, box styles, colors, page breaks and spacers are known-lossy. An
opt-in `to_dialect_markdown` emitting attribute lines would close that gap;
no consumer needs it today.

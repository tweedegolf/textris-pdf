# textris-web-editor

A live editor for the [`textris-pdf`](..) Markdown dialect: type on the left,
see the rendered PDF on the right. The whole renderer - parsing, shaping,
layout and PDF serialization - runs in the browser as WebAssembly; nothing is
uploaded anywhere.

A build of `main` is published at
**<https://tweedegolf.github.io/textris-pdf/>** by
[`pages.yml`](../.github/workflows/pages.yml).

![The editor: highlighted dialect source on the left, the rendered PDF on the right](../docs/web-editor.png)

## Running it

```sh
cargo install trunk                    # once
rustup target add wasm32-unknown-unknown

trunk serve                            # http://localhost:8080, rebuilds on save
trunk build --release                  # or: a static site in dist/
```

Trunk fetches a matching `wasm-bindgen` and `wasm-opt` itself, so there is no
CLI version to keep in step with the `wasm-bindgen` crate. `dist/` is
self-contained and can be served by any static file host.

CodeMirror 6 loads as ESM from `esm.sh` through the import map in
[`index.html`](index.html), so there is no npm dependency and no bundler. Each
package pins its version and lists its CodeMirror peers as `external` so they
resolve back through that map - two copies of `@codemirror/state` in one page
break CodeMirror outright. To run fully offline, vendor the graph with
`npx esbuild main.js --bundle --format=esm --outfile=bundle.js` and point the
module script at the result.

## How it fits together

| File | Role |
| --- | --- |
| [`src/lib.rs`](src/lib.rs) | `wasm-bindgen` glue: font bytes in, PDF bytes out. No rendering logic. |
| [`main.js`](main.js) | Editor setup, debounced rendering, preview and error reporting. |
| [`dialect.js`](dialect.js) | CodeMirror syntax highlighting for the dialect. |
| [`index.html`](index.html) | Trunk asset directives and the import map. |

The editor calls `Renderer::render` on a 300 ms debounce after the last
keystroke. A render of the bundled sample takes about 6 ms once the font cache
is warm, so the delay is the debounce, not the renderer.

On failure the preview keeps showing the last good PDF, and the parse error -
which carries a 1-based line number but no column, matching
`MarkdownParseError` - is shown in the error bar and marked in the gutter.

## The preview

The preview is [pdf.js](https://mozilla.github.io/pdf.js/) (pinned from
jsdelivr in `main.js`, parsing in a real worker), drawing one canvas per page
into a scrollable pane. Not the browser's own `<iframe>` viewer, for one
reason: an iframe viewer can only be re-pointed by navigating to a fresh blob
URL, which reloads the whole viewer and blinks on every re-render. With
canvases, a re-render draws each page off-DOM and swaps it in only when
finished, so the last good rendering stays up throughout and an edit appears
as a seamless in-place update.

Pages are drawn lazily - an `IntersectionObserver` renders them half a
viewport before they scroll in - so a re-render costs only the pages you are
looking at. The `fit` selector and pane resizes just pick a new render scale;
they re-draw the canvases without re-rendering the PDF.

## Following the edit - in both directions

The preview follows the editor, and a click in the preview jumps the editor
back. That needs a map between source lines and positions on the rendered
pages, which the library provides in two halves:

- [`Textris::source_map`](../src/build/mod.rs) - the source line each parsed
  block came from, recorded by `push_markdown`;
- [`Layout::block_pages`](../src/layout/display.rs) and
  [`Layout::block_tops`](../src/layout/display.rs) - the page each top-level
  block started on and its top edge on that page, recorded by the layout
  engine.

Joined, they give one anchor per block: *(source line, fractional page
position)*. Both sides are ascending, so the editor binary-searches for the
last anchor at or before its input and interpolates toward the next - the
same lookup in either direction. The map drives three behaviours:

- **Scrolling the editor scrolls the preview** to match the line at the top of
  the view.
- **Clicking a page** moves the cursor to the corresponding source line,
  centred in the editor. (Sync back to the preview is suppressed for a beat,
  or the click's own jump would scroll the preview away from the spot that
  was just clicked.)
- **After an edit re-renders**, the preview scrolls to where the edited line
  landed - but only if it is not already in view.

Positions are exact at every block start and interpolated inside a block, so
a click deep inside a long table lands on or near the table's first source
line. A block that fits nothing on its starting page is recorded on the page
it actually lands on (the layout patches the record after the block's own
page break).

## One thing worth knowing

**The creation date is fixed per session.** `wasm32-unknown-unknown` has no
clock, so the page passes one in (`Document::created`, via
`Textris::created_at`). Pinning it for the session rather than reading
`Date.now()` per render also means an edit that does not change the layout
produces byte-identical output, which the preview uses to skip the pdf.js
reload entirely.

## Fonts

The bundled Newsreader and Fira Code variable fonts come from
[`tests/fonts/`](../tests/fonts), copied into `dist/` at build time. Both are
OFL-1.1 and their licences ship alongside them. `textris-pdf` embeds no
typeface of its own, so swapping them is a matter of pointing the `copy-file`
directives and the `FONTS` list in [`main.js`](main.js) somewhere else.

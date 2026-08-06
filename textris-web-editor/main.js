// Wires the CodeMirror editor to the wasm renderer and the PDF preview.

import { EditorState } from "@codemirror/state";
import {
  EditorView,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
  rectangularSelection,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { bracketMatching } from "@codemirror/language";
import { lintGutter, lintKeymap, setDiagnostics } from "@codemirror/lint";

import { textrisDialect } from "./dialect.js";

const DEBOUNCE_MS = 300;

const SEED = "mantis-shrimp-example-input.md";

const FONTS = [
  "fonts/Newsreader-Variable.ttf",
  "fonts/Newsreader-Italic-Variable.ttf",
  "fonts/FiraCode-Variable.ttf",
];

// Acrobat-style open parameters for the browser's PDF viewer, which Chrome
// honours on the initial navigation to a blob URL. `toolbar=0` is what does
// the work — it takes the sidebar with it — and `navpanes=0` is harmless
// insurance for viewers that separate the two. Verified in Chrome; other
// browsers may ignore these and simply show their own chrome.
const VIEWER_CHROME = "toolbar=0&navpanes=0";

const els = {
  editor: document.getElementById("editor"),
  preview: document.getElementById("preview"),
  status: document.getElementById("status"),
  error: document.getElementById("error"),
  levels: document.getElementById("levels"),
  fit: document.getElementById("fit"),
  download: document.getElementById("download"),
};

// The creation date is fixed for the session rather than read per render: wasm
// has no clock of its own, and pinning it means an edit that does not change
// the layout produces byte-identical output, which `showPdf` uses to leave the
// preview (and its scroll position) alone.
const CREATED_AT = Date.now() / 1000;

let renderer = null;
let view = null;
let timer = null;
let lastBytes = null;
let lastUrl = null;
let shownPage = 0;
let shownFit = null;

// The line of the most recent edit, which is what the preview follows. Raw
// cursor movement deliberately does not: re-pointing the viewer costs a
// reload (see `showPdf`), and paying that for every arrow key would flicker.
let editedLine = null;

boot().catch((err) => {
  fail(`could not start: ${err && err.message ? err.message : err}`);
});

async function boot() {
  const [bindings, fonts, seed] = await Promise.all([
    wasmBindings(),
    Promise.all(FONTS.map(fetchBytes)),
    fetchText(SEED),
  ]);

  // One `Renderer` per page load: it leaks the font bytes into `'static`.
  renderer = new bindings.Renderer(fonts[0], fonts[1], fonts[2]);

  view = new EditorView({
    parent: els.editor,
    state: EditorState.create({
      doc: seed,
      extensions: [
        lineNumbers(),
        highlightActiveLineGutter(),
        highlightActiveLine(),
        highlightSpecialChars(),
        drawSelection(),
        rectangularSelection(),
        bracketMatching(),
        history(),
        lintGutter(),
        keymap.of([...defaultKeymap, ...historyKeymap, ...lintKeymap]),
        EditorView.lineWrapping,
        textrisDialect(),
        EditorView.updateListener.of((update) => {
          if (!update.docChanged) return;
          // The end of the last changed range, in the new document.
          let edited = null;
          update.changes.iterChangedRanges((_fromA, _toA, _fromB, toB) => {
            edited = toB;
          });
          if (edited !== null) editedLine = update.state.doc.lineAt(edited).number;
          schedule();
        }),
      ],
    }),
  });

  els.levels.addEventListener("input", schedule);
  // Re-rendering to change the fit is wasteful in principle but costs a few
  // milliseconds, and it keeps one path to the preview instead of two.
  els.fit.addEventListener("change", render);
  els.download.addEventListener("click", download);
  render();
}

function schedule() {
  clearTimeout(timer);
  timer = setTimeout(render, DEBOUNCE_MS);
}

function render() {
  const source = view.state.doc.toString();
  const started = performance.now();

  let result;
  try {
    const rendered = renderer.render(source, parseLevels(els.levels.value), CREATED_AT);
    // Copy the fields out and hand the Rust allocation back: this runs on
    // every keystroke, and wasm-bindgen objects are not garbage collected.
    result = {
      pdf: rendered.pdf,
      lines: rendered.lines,
      pages: rendered.pages,
      pageCount: rendered.page_count,
    };
    rendered.free();
  } catch (err) {
    reportFailure(err);
    return;
  }

  clearFailure();
  showPdf(result, performance.now() - started);
}

/**
 * Show a freshly rendered document, scrolled to the page holding the last
 * edit.
 *
 * Chrome's PDF viewer only honours a `#page=` fragment on the *initial*
 * navigation to a blob URL — changing the fragment afterwards, by `src` or by
 * `location.hash`, is silently ignored. So targeting a page means minting a
 * fresh blob URL, which reloads the viewer. That is free on a re-render (the
 * bytes changed, so the URL had to change anyway) and is why the preview is
 * left strictly alone when the bytes, the target page and the fit all held
 * still. The same fragment carries the parameters that strip the viewer's own
 * toolbar and sidebar, so those are reapplied on every navigation.
 */
function showPdf(result, elapsedMs) {
  const bytes = result.pdf;
  const page = pageForLine(result, editedLine);
  const fit = els.fit.value;
  const timing = `${elapsedMs.toFixed(0)} ms`;
  const where = `p${page}/${result.pageCount}`;

  if (lastBytes && equalBytes(lastBytes, bytes) && page === shownPage && fit === shownFit) {
    setStatus(`unchanged · ${where} · ${timing}`);
    return;
  }

  // `view=Fit` scales the whole page into view; omitting it leaves the
  // viewer's default, which is fit-to-width.
  const view = fit === "page" ? "&view=Fit" : "";
  const url = URL.createObjectURL(new Blob([bytes], { type: "application/pdf" }));
  els.preview.src = `${url}#page=${page}&${VIEWER_CHROME}${view}`;
  if (lastUrl) URL.revokeObjectURL(lastUrl);
  lastUrl = url;
  lastBytes = bytes;
  shownPage = page;
  shownFit = fit;
  els.download.disabled = false;
  setStatus(`${(bytes.length / 1024).toFixed(0)} kB · ${where} · ${timing}`);
}

/**
 * The 1-based page a source line renders on: the last mapped block starting at
 * or before it. `result.lines` is ascending, so this is a binary search.
 */
function pageForLine(result, line) {
  const { lines, pages } = result;
  if (line === null || lines.length === 0) return 1;

  let lo = 0;
  let hi = lines.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (lines[mid] <= line) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  // A line above the first block (front matter, or a leading blank) maps to
  // the first page.
  return found === -1 ? 1 : pages[found];
}

function download() {
  if (!lastUrl) return;
  const link = document.createElement("a");
  link.href = lastUrl;
  link.download = "document.pdf";
  link.click();
}

/**
 * Surface a render failure. A `TextrisError` from the Rust side carries a
 * 1-based source line (0 when the failure has no position); anything else is a
 * panic, which traps the wasm module for good and needs a reload.
 */
function reportFailure(err) {
  if (err instanceof WebAssembly.RuntimeError) {
    fail("the renderer panicked — reload the page (details in the console)");
    console.error(err);
    return;
  }

  let line = 0;
  let message = String(err);
  if (err && typeof err.line === "number" && typeof err.message === "string") {
    line = err.line;
    message = err.message;
    if (typeof err.free === "function") err.free();
  } else if (err && typeof err.message === "string") {
    message = err.message;
  }

  els.error.hidden = false;
  els.error.textContent = line > 0 ? `line ${line}: ${message}` : message;
  setStatus("not rendered", "error");
  setDiagnosticFor(line, message);
}

function clearFailure() {
  els.error.hidden = true;
  els.error.textContent = "";
  view.dispatch(setDiagnostics(view.state, []));
}

/**
 * Mark the offending line. The parser reports a line and no column
 * (`MarkdownParseError` in src/markdown/parse.rs), so the whole line is marked
 * rather than inventing a span.
 */
function setDiagnosticFor(lineNumber, message) {
  if (lineNumber < 1) {
    view.dispatch(setDiagnostics(view.state, []));
    return;
  }
  const doc = view.state.doc;
  const line = doc.line(Math.min(lineNumber, doc.lines));
  view.dispatch(
    setDiagnostics(view.state, [
      { from: line.from, to: line.to, severity: "error", message },
    ]),
  );
}

/** `"3,4"` → `[3, 4]`, ignoring anything that is not a heading level. */
function parseLevels(value) {
  return value
    .split(/[,\s]+/)
    .map((part) => Number.parseInt(part, 10))
    .filter((level) => Number.isInteger(level) && level >= 1 && level <= 6);
}

function equalBytes(a, b) {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function setStatus(text, state) {
  els.status.textContent = text;
  if (state) els.status.dataset.state = state;
  else delete els.status.dataset.state;
}

function fail(message) {
  setStatus("failed", "error");
  els.error.hidden = false;
  els.error.textContent = message;
}

/**
 * The wasm exports, which Trunk's own loader initializes and publishes on
 * `window.wasmBindings`. It may already have landed by the time this module
 * runs, so check before waiting for the event.
 */
function wasmBindings() {
  if (window.wasmBindings) return Promise.resolve(window.wasmBindings);
  return new Promise((resolve) => {
    addEventListener("TrunkApplicationStarted", () => resolve(window.wasmBindings), {
      once: true,
    });
  });
}

async function fetchBytes(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: ${response.status} ${response.statusText}`);
  return new Uint8Array(await response.arrayBuffer());
}

async function fetchText(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: ${response.status} ${response.statusText}`);
  return response.text();
}

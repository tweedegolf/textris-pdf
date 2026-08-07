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

// pdf.js draws the preview onto per-page canvases, so a re-render can swap
// each page in only once it is fully drawn - an iframe viewer reloads whole
// and blinks. The library and its worker must come from the same release.
// jsdelivr sends CORS headers, which the worker fetch in `boot` relies on.
const PDFJS_BASE = "https://cdn.jsdelivr.net/npm/pdfjs-dist@5.7.284";

// The space around and between pages, which `fitScale` reserves and
// `scrollPreviewTo` skips over. Must match the padding/margin in style.css.
const PAGE_GUTTER = 16;

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
// preview alone.
const CREATED_AT = Date.now() / 1000;

let pdfjs = null;
let renderer = null;
let view = null;
let timer = null;
let lastBytes = null;
let renderSeq = 0;

// The line of the most recent edit. After a re-render the preview scrolls to
// where that line landed, if it is not already in view; between renders the
// preview follows the editor's scroll position instead (see `onEditorScroll`).
let editedLine = null;

/**
 * The pdf.js side of the preview: one `.page` wrapper per page inside
 * `els.preview`, each holding a canvas. Pages are drawn lazily as they scroll
 * into view, and a new document draws into fresh canvases off-DOM, swapping
 * them in only when finished - the old page stays visible in the meantime.
 */
const preview = {
  doc: null, // the current pdf.js PDFDocumentProxy
  map: null, // { lines, pages, pageCount, docLines } of the shown document
  entries: [], // one per page: { wrapper, task, visible, stale }
  baseSize: null, // page 1 viewport at scale 1; pages are assumed uniform
  scale: 1,
  seq: 0, // bumped when the document or scale changes; async work re-checks it
  observer: null,
};

boot().catch((err) => {
  fail(`could not start: ${err && err.message ? err.message : err}`);
});

async function boot() {
  const [bindings, fonts, seed, pdfjsModule, workerScript] = await Promise.all([
    wasmBindings(),
    Promise.all(FONTS.map(fetchBytes)),
    fetchText(SEED),
    import(`${PDFJS_BASE}/build/pdf.min.mjs`),
    fetchText(`${PDFJS_BASE}/build/pdf.worker.min.mjs`),
  ]);

  pdfjs = pdfjsModule;
  // A `Worker` cannot be constructed from a cross-origin URL, so the (self-
  // contained) worker bundle is fetched and inlined through a blob URL.
  // Skipping this would work but drops pdf.js to parsing on the main thread.
  pdfjs.GlobalWorkerOptions.workerPort = new Worker(
    URL.createObjectURL(new Blob([workerScript], { type: "text/javascript" })),
    { type: "module" },
  );

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

  preview.observer = new IntersectionObserver(onPageVisibility, {
    root: els.preview,
    // Half a viewport of lookahead, so pages are usually drawn by the time
    // they scroll in.
    rootMargin: "50% 0px",
  });

  let resizeTimer = null;
  new ResizeObserver(() => {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(rescale, 100);
  }).observe(els.preview);

  view.scrollDOM.addEventListener("scroll", onEditorScroll, { passive: true });
  els.preview.addEventListener("click", onPreviewClick);
  els.levels.addEventListener("input", schedule);
  els.fit.addEventListener("change", rescale);
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
      tops: rendered.tops,
      pageCount: rendered.page_count,
    };
    rendered.free();
  } catch (err) {
    reportFailure(err);
    return;
  }

  clearFailure();
  showPdf(result, view.state.doc.lines, performance.now() - started).catch((err) => {
    console.error(err);
    fail(`preview failed: ${err && err.message ? err.message : err}`);
  });
}

/**
 * Show a freshly rendered document and scroll the preview to the last edit,
 * unless it is already in view. When the bytes did not change the shown pages
 * are left strictly alone.
 */
async function showPdf(result, docLines, elapsedMs) {
  const bytes = result.pdf;
  const timing = `${elapsedMs.toFixed(0)} ms`;

  if (lastBytes && equalBytes(lastBytes, bytes)) {
    // Identical bytes still refresh the map: an edit that moves lines without
    // changing the layout (say, an extra blank line) shifts it.
    preview.map = {
      lines: result.lines,
      pages: result.pages,
      tops: result.tops,
      pageCount: result.pageCount,
      docLines,
    };
    setStatus(`unchanged · ${whereIs(editedLine)} · ${timing}`);
    revealLine(editedLine);
    return;
  }

  const seq = ++renderSeq;
  // pdf.js transfers the buffer to its worker, detaching it, so it gets a
  // copy: `bytes` stays behind for the equality check and the download.
  const doc = await pdfjs.getDocument({
    data: bytes.slice(),
    standardFontDataUrl: `${PDFJS_BASE}/standard_fonts/`,
  }).promise;
  if (seq !== renderSeq) {
    doc.destroy();
    return;
  }

  await setDocument(doc, {
    lines: result.lines,
    pages: result.pages,
    tops: result.tops,
    pageCount: result.pageCount,
    docLines,
  });
  lastBytes = bytes;
  els.download.disabled = false;
  setStatus(`${(bytes.length / 1024).toFixed(0)} kB · ${whereIs(editedLine)} · ${timing}`);
  revealLine(editedLine);
}

/** Swap the preview over to a freshly parsed document. */
async function setDocument(doc, map) {
  const seq = ++preview.seq;
  const old = preview.doc;
  preview.doc = doc;
  preview.map = map;
  // Destroying the old document cancels its in-flight page renders; their
  // canvases were never swapped in, so nothing visible changes.
  if (old) old.destroy();

  const first = await doc.getPage(1);
  if (seq !== preview.seq) return;
  preview.baseSize = first.getViewport({ scale: 1 });
  preview.scale = fitScale();
  syncPageList(doc.numPages);
  refreshPages();
}

/** Give the preview `count` page wrappers, keeping the ones it has. */
function syncPageList(count) {
  while (preview.entries.length > count) {
    const entry = preview.entries.pop();
    if (entry.task) entry.task.cancel();
    preview.observer.unobserve(entry.wrapper);
    entry.wrapper.remove();
  }
  while (preview.entries.length < count) {
    const wrapper = document.createElement("div");
    wrapper.className = "page";
    wrapper.dataset.index = preview.entries.length;
    els.preview.appendChild(wrapper);
    preview.observer.observe(wrapper);
    preview.entries.push({ wrapper, task: null, visible: false, stale: true });
  }
}

/** Mark every page out of date and redraw the ones that are on screen. */
function refreshPages() {
  for (let i = 0; i < preview.entries.length; i++) {
    const entry = preview.entries[i];
    entry.stale = true;
    sizeWrapper(entry.wrapper);
    if (entry.visible) renderPage(i);
  }
}

function onPageVisibility(observed) {
  for (const seen of observed) {
    const index = Number(seen.target.dataset.index);
    const entry = preview.entries[index];
    if (!entry || entry.wrapper !== seen.target) continue;
    entry.visible = seen.isIntersecting;
    if (entry.visible && entry.stale) renderPage(index);
  }
}

/**
 * Draw one page into a fresh canvas and swap it in. The swap happens only
 * after the draw completes, so the previous rendering stays up throughout -
 * this is what keeps the preview from blinking. A page drawn at a stale
 * sequence number is simply dropped; whoever bumped the sequence has already
 * queued a replacement.
 */
async function renderPage(index) {
  const entry = preview.entries[index];
  const doc = preview.doc;
  if (!entry || !doc || !entry.stale) return;
  entry.stale = false;
  const seq = preview.seq;
  if (entry.task) entry.task.cancel();

  try {
    const page = await doc.getPage(index + 1);
    if (seq !== preview.seq) return;
    const viewport = page.getViewport({ scale: preview.scale });
    const dpr = window.devicePixelRatio || 1;
    const canvas = document.createElement("canvas");
    canvas.width = Math.round(viewport.width * dpr);
    canvas.height = Math.round(viewport.height * dpr);
    canvas.style.width = `${Math.round(viewport.width)}px`;
    canvas.style.height = `${Math.round(viewport.height)}px`;
    entry.task = page.render({
      canvas,
      viewport,
      transform: dpr === 1 ? null : [dpr, 0, 0, dpr, 0, 0],
    });
    await entry.task.promise;
    entry.task = null;
    if (seq !== preview.seq) return;
    entry.wrapper.replaceChildren(canvas);
  } catch (err) {
    if (err && err.name === "RenderingCancelledException") return;
    console.error(`page ${index + 1} failed to draw`, err);
  }
}

function sizeWrapper(wrapper) {
  const width = `${Math.round(preview.baseSize.width * preview.scale)}px`;
  const height = `${Math.round(preview.baseSize.height * preview.scale)}px`;
  wrapper.style.width = width;
  wrapper.style.height = height;
  // Stretch a previously drawn canvas along: it shows scaled (soft) until its
  // redraw lands, rather than spilling out of the resized wrapper.
  const canvas = wrapper.firstElementChild;
  if (canvas) {
    canvas.style.width = width;
    canvas.style.height = height;
  }
}

/** The scale that fits a page to the pane, per the toolbar's fit mode. */
function fitScale() {
  const box = els.preview;
  const availableWidth = Math.max(box.clientWidth - 2 * PAGE_GUTTER, 50);
  const availableHeight = Math.max(box.clientHeight - 2 * PAGE_GUTTER, 50);
  const toWidth = availableWidth / preview.baseSize.width;
  if (els.fit.value !== "page") return toWidth;
  return Math.min(toWidth, availableHeight / preview.baseSize.height);
}

/** Re-fit after a fit-mode change or a pane resize. Cheap: no PDF re-render. */
function rescale() {
  if (!preview.doc || !preview.baseSize) return;
  const scale = fitScale();
  // Ignore sub-half-percent changes, e.g. a scrollbar appearing.
  if (Math.abs(scale - preview.scale) / preview.scale < 0.005) return;
  preview.scale = scale;
  preview.seq++;
  refreshPages();
}

// --- sync: the preview follows the editor, a click jumps back ---

let syncScheduled = false;

// Clicking the preview moves the cursor and scrolls the editor along; until
// this deadline, editor scrolls do not sync back, or the preview would be
// yanked out from under the click it is answering.
let suppressSyncUntil = 0;

function onEditorScroll() {
  if (syncScheduled) return;
  syncScheduled = true;
  requestAnimationFrame(() => {
    syncScheduled = false;
    if (performance.now() < suppressSyncUntil) return;
    if (!preview.map || preview.entries.length === 0) return;
    const top = view.scrollDOM.scrollTop;
    const block = view.lineBlockAtHeight(top);
    const within = block.height > 0 ? Math.min(Math.max((top - block.top) / block.height, 0), 1) : 0;
    const line = view.state.doc.lineAt(block.from).number + within;
    scrollPreviewTo(pagePosForLine(line), false);
  });
}

/** Jump the editor cursor to the source of the clicked spot in the preview. */
function onPreviewClick(event) {
  if (!preview.map || !preview.baseSize) return;
  const wrapper = event.target.closest(".page");
  if (!wrapper) return;

  const rect = wrapper.getBoundingClientRect();
  const within = Math.min(Math.max((event.clientY - rect.top) / rect.height, 0), 0.999);
  const line = lineForPagePos(Number(wrapper.dataset.index) + within);

  const doc = view.state.doc;
  const anchor = doc.line(Math.min(Math.max(line, 1), doc.lines)).from;
  suppressSyncUntil = performance.now() + 300;
  view.dispatch({
    selection: { anchor },
    effects: EditorView.scrollIntoView(anchor, { y: "center" }),
  });
  view.focus();
}

/*
 * Both directions of the sync share one anchor map: block i anchors source
 * line `lines[i]` to the fractional 0-based page position `anchorPos(i)`
 * (2.5 is halfway down the third page - the block's real top edge, since the
 * layout reports it in points). Both arrays are ascending, so either side
 * binary-searches for the last anchor at or before its input and linearly
 * interpolates toward the next one; the tail runs out at (docLines + 1,
 * pageCount). Between anchors the interpolation assumes lines are spread
 * evenly, which is approximate inside a tall block but exact at every block
 * start.
 */

function anchorPos(i) {
  const { pages, tops } = preview.map;
  // A spacer's recorded top can exceed the page height; keep the anchor on
  // its own page.
  return pages[i] - 1 + Math.min(tops[i] / preview.baseSize.height, 0.999);
}

/** Where a source line lands in the document, as a page position. */
function pagePosForLine(line) {
  const { lines, pageCount, docLines } = preview.map;
  if (line === null || lines.length === 0 || !preview.baseSize) return 0;

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
  // the top of the first page.
  if (found === -1) return 0;

  const fromLine = lines[found];
  const fromPos = anchorPos(found);
  const last = found + 1 >= lines.length;
  const toLine = last ? docLines + 1 : lines[found + 1];
  const toPos = last ? pageCount : anchorPos(found + 1);
  const frac = toLine > fromLine ? (line - fromLine) / (toLine - fromLine) : 0;
  return Math.min(fromPos + frac * (toPos - fromPos), pageCount - 0.001);
}

/** The mirror image: the source line that lands at a page position. */
function lineForPagePos(pos) {
  const { lines, pageCount, docLines } = preview.map;
  if (lines.length === 0 || !preview.baseSize) return 1;

  let lo = 0;
  let hi = lines.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (anchorPos(mid) <= pos) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  // A click above the first block maps to the first line.
  if (found === -1) return 1;

  const fromPos = anchorPos(found);
  const fromLine = lines[found];
  const last = found + 1 >= lines.length;
  const toPos = last ? pageCount : anchorPos(found + 1);
  const toLine = last ? docLines + 1 : lines[found + 1];
  const frac = toPos > fromPos ? Math.min((pos - fromPos) / (toPos - fromPos), 1) : 0;
  return Math.min(Math.round(fromLine + frac * (toLine - fromLine)), docLines);
}

/** `p3/7`-style status text for the page holding `line`. */
function whereIs(line) {
  return `p${Math.floor(pagePosForLine(line)) + 1}/${preview.map.pageCount}`;
}

/** Scroll the page position holding `line` into view, unless it already is. */
function revealLine(line) {
  if (line === null || !preview.map) return;
  scrollPreviewTo(pagePosForLine(line), true);
}

function scrollPreviewTo(pos, onlyIfHidden) {
  const count = preview.entries.length;
  if (count === 0) return;
  const clamped = Math.min(Math.max(pos, 0), count - 0.001);
  const index = Math.floor(clamped);
  const wrapper = preview.entries[index].wrapper;
  const y = wrapper.offsetTop + (clamped - index) * wrapper.offsetHeight;
  const box = els.preview;
  if (onlyIfHidden && y >= box.scrollTop && y <= box.scrollTop + box.clientHeight - 40) return;
  box.scrollTop = Math.max(0, y - PAGE_GUTTER);
}

// --- everything below is unrelated to the preview ---

function download() {
  if (!lastBytes) return;
  const url = URL.createObjectURL(new Blob([lastBytes], { type: "application/pdf" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = "document.pdf";
  link.click();
  // Not revoked synchronously: the click's navigation may still be reading it.
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

/**
 * Surface a render failure. A `TextrisError` from the Rust side carries a
 * 1-based source line (0 when the failure has no position); anything else is a
 * panic, which traps the wasm module for good and needs a reload.
 */
function reportFailure(err) {
  if (err instanceof WebAssembly.RuntimeError) {
    fail("the renderer panicked - reload the page (details in the console)");
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

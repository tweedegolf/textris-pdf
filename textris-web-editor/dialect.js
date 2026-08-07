// Syntax highlighting for the textris-pdf Markdown dialect.
//
// The dialect is line-oriented - front matter, attribute lines, directives,
// headings, table rows - so a `StreamLanguage` tokenizer covers it without a
// grammar. The authoritative dialect reference is the module rustdoc at
// src/markdown/parse.rs; this file only has to *look* right, and deliberately
// stays lenient: the Rust parser is what decides whether a document is valid.

import { StreamLanguage, HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags } from "@lezer/highlight";

/** Our token names, mapped onto the standard highlight tags. */
const tokenTable = {
  frontmatter: tags.meta,
  key: tags.propertyName,
  string: tags.string,
  directive: tags.keyword,
  heading: tags.heading,
  strong: tags.strong,
  emphasis: tags.emphasis,
  mono: tags.monospace,
  ref: tags.link,
  blank: tags.contentSeparator,
  quote: tags.quote,
  listmark: tags.list,
  pipe: tags.punctuation,
  delimrow: tags.contentSeparator,
  brace: tags.attributeName,
  atom: tags.atom,
  number: tags.number,
  br: tags.tagName,
};

const parser = {
  name: "textris",

  startState() {
    // `frontMatter` is a three-state latch: a `+++` fence only opens front
    // matter while nothing but blank lines has been seen, so a stray `+++`
    // further down does not silently reinterpret the rest of the document.
    return { frontMatter: "before", inAttrs: false };
  },

  token(stream, state) {
    if (stream.sol()) {
      state.inAttrs = false;
      if (state.frontMatter === "before" && stream.string.trim() && !isFence(stream.string)) {
        state.frontMatter = "done";
      }
    }

    if (stream.sol() && isFence(stream.string) && stream.match(/^\+\+\+[ \t]*$/)) {
      state.frontMatter = state.frontMatter === "before" ? "in" : "done";
      return "frontmatter";
    }

    if (state.frontMatter === "in") return frontMatterToken(stream);
    if (state.inAttrs) return attributeToken(stream, state);
    if (stream.sol()) {
      const lineToken = lineStartToken(stream, state);
      if (lineToken !== undefined) return lineToken;
    }
    return inlineToken(stream);
  },

  languageData: {
    commentTokens: {},
    // Blank lines separate blocks in this dialect, so the whole document is
    // one indentation level; suppress CodeMirror's auto-indent guessing.
    indentOnInput: /^$/,
  },
};

function isFence(line) {
  return /^\+\+\+[ \t]*$/.test(line);
}

/** `key = "value"` pairs between the `+++` fences. */
function frontMatterToken(stream) {
  if (stream.sol() && stream.match(/^[A-Za-z_][\w-]*/)) return "key";
  if (stream.match(/^"(?:[^"\\]|\\.)*"?/)) return "string";
  if (stream.match(/^\{(?:page|total)\}/)) return "ref";
  stream.next();
  return null;
}

/** The inside of a `{ … }` attribute line. */
function attributeToken(stream, state) {
  if (stream.match(/^\}/)) {
    state.inAttrs = false;
    return "brace";
  }
  if (stream.match(/^(?:true|false)\b/)) return "atom";
  if (stream.match(/^"(?:[^"\\]|\\.)*"?/)) return "string";
  if (stream.match(/^-?\d+(?:\.\d+)?(?:em|pt)?/)) return "number";
  if (stream.match(/^[A-Za-z_][\w-]*/)) return "key";
  stream.next();
  return null;
}

/**
 * Constructs that are only meaningful at the start of a line. Returns
 * `undefined` when the line starts with ordinary inline content.
 */
function lineStartToken(stream, state) {
  if (stream.match(/^\{/)) {
    state.inAttrs = true;
    return "brace";
  }
  if (stream.match(/^@[a-z]+/)) return "directive";
  if (stream.match(/^#{1,6}(?=[ \t])/)) {
    // Headings carry no inline markup worth distinguishing; take the line.
    stream.skipToEnd();
    return "heading";
  }
  if (stream.match(/^>[ \t]?/)) return "quote";
  // A table's delimiter row: `|---|:---:|`
  if (stream.match(/^\|[-:|\t ]+\|?[ \t]*$/)) return "delimrow";
  if (stream.match(/^[ \t]*(?:[-*+]|\d+\.|[a-zA-Z]\.)[ \t]+/)) return "listmark";
  return undefined;
}

/** Inline markup, valid anywhere in body text and in table cells. */
function inlineToken(stream) {
  if (stream.match(/^\|/)) return "pipe";
  if (stream.match(/^`[^`]*`?/)) return "mono";
  if (stream.match(/^\*\*(?:[^*]|\*(?!\*))+\*\*/)) return "strong";
  if (stream.match(/^\*[^*]+\*/)) return "emphasis";
  if (stream.match(/^_{3,}(?:\(\d+\))?/)) return "blank";
  if (stream.match(/^\[#[\w-]+\]/)) return "ref";
  if (stream.match(/^\[[ xX]\]/)) return "atom";
  if (stream.match(/^<br[ \t]*\/?>/i)) return "br";
  if (stream.match(/^\\$/)) return "br";

  // Ordinary text: consume the current character, then run to the next one
  // that could start any of the constructs above.
  stream.next();
  stream.eatWhile(/[^*`_[<|\\]/);
  return null;
}

const highlightStyle = HighlightStyle.define([
  { tag: tags.meta, color: "#6b6560", fontWeight: "600" },
  { tag: tags.propertyName, color: "#8a4b2a" },
  { tag: tags.string, color: "#2c6e49" },
  { tag: tags.keyword, color: "#7048a8", fontWeight: "600" },
  { tag: tags.heading, color: "#1b1a17", fontWeight: "700" },
  { tag: tags.strong, fontWeight: "700" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: tags.monospace, color: "#2c6e49", background: "#eef3ef" },
  { tag: tags.link, color: "#1d5c8f", textDecoration: "underline" },
  { tag: tags.contentSeparator, color: "#9c958e" },
  { tag: tags.quote, color: "#8a4b2a", fontWeight: "600" },
  { tag: tags.list, color: "#8a4b2a" },
  { tag: tags.punctuation, color: "#b0a89f" },
  { tag: tags.attributeName, color: "#7048a8" },
  { tag: tags.atom, color: "#1d5c8f" },
  { tag: tags.number, color: "#1d5c8f" },
  { tag: tags.tagName, color: "#9c958e" },
]);

/** Editor extensions: the dialect tokenizer plus its colours. */
export function textrisDialect() {
  return [StreamLanguage.define({ ...parser, tokenTable }), syntaxHighlighting(highlightStyle)];
}

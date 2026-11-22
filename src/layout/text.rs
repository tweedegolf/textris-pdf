//! Flowing text: tokenizing inlines into shaped words, greedy line breaking,
//! and emitting lines of merged same-style runs.
//!
//! Where a line may break follows the Unicode line breaking algorithm
//! (UAX #14), and a word too wide for a line splits between grapheme clusters
//! (UAX #29).

use std::sync::Arc;

use krilla::{color::rgb, text::KrillaGlyph};
use unicode_linebreak::{BreakOpportunity, linebreaks};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    fonts::Style,
    layout::{Element, Engine, FIT_EPSILON, Tagging, TextElement},
    model::Inline,
};

/// What a [`Word`] stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WordKind {
    /// A shaped run of glyphs.
    Text,
    /// A zero-width marker for a hard line break (`'\n'`): [`Engine::wrap`]
    /// ends the current line here and the marker itself is never drawn.
    HardBreak,
    /// A glyph-less inline fill-in line of `width` points, participating in
    /// line breaking as an atomic word but drawn as a baseline stroke by
    /// [`Engine::draw_line`].
    FillIn,
}

/// A shaped word plus its measured width, the atomic unit of line breaking.
#[derive(Debug, Clone)]
pub(super) struct Word {
    pub(super) style: Style,
    pub(super) color: Option<rgb::Color>,
    pub(super) text: String,
    pub(super) glyphs: Arc<[KrillaGlyph]>,
    pub(super) width: f32,
    pub(super) kind: WordKind,
    /// The style of the whitespace separating this word from the one before
    /// it, if any: drawn as a single space of that style unless the word
    /// starts a line. The space belongs to the run it was typed in, so the
    /// one before `code` in "a `code`" is as wide as the one after it.
    pub(super) space_before: Option<Style>,
    /// Whether a line may break before this word. A word without it continues
    /// the previous one (the `,` after a `**bold**` run, or the `!` in
    /// `Bonjour !`): the two only break apart when they cannot share a line.
    /// A break needs no whitespace: `well-` and `known` are two words.
    pub(super) break_before: bool,
}

/// Stands in for a fill-in line when finding break opportunities. A letter, so
/// the line stays glued to adjoining text and punctuation like a word would.
const FILL_IN_STAND_IN: char = 'x';

impl Word {
    /// A glyph-less marker word (a hard break or a fill-in line).
    fn marker(kind: WordKind, color: Option<rgb::Color>, width: f32) -> Self {
        Self {
            style: Style::Regular,
            color,
            text: String::new(),
            glyphs: Vec::new().into(),
            width,
            kind,
            space_before: None,
            break_before: false,
        }
    }
}

/// A soft hyphen: an invisible break opportunity inside a word, drawn as a
/// hyphen only where a line breaks at it.
const SOFT_HYPHEN: char = '\u{AD}';

/// Whether `word` ends in a soft hyphen, to be drawn if a line ends there.
fn ends_in_soft_hyphen(word: &Word) -> bool {
    word.kind == WordKind::Text && word.text.ends_with(SOFT_HYPHEN)
}

/// Whitespace that separates words and collapses into a single space: the
/// ASCII space, tabs and line separators. Every other Unicode space (em, thin,
/// no-break, …) is typographic and keeps its own width inside a word.
pub(crate) fn collapses(c: char) -> bool {
    matches!(c, ' ' | '\t'..='\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// How many lines of a paragraph of `count` lines must share a page with
/// line `index`, counting itself, for a page break before it to leave no
/// orphan (a first line alone at the bottom of a page) or widow (a last line
/// alone at the top of one): at least two lines stay together on either side
/// of a break, which keeps a paragraph of up to three lines whole.
pub(super) fn kept_lines(index: usize, count: usize) -> usize {
    match index {
        0 if count <= 3 => count,
        0 => 2,
        _ if index + 2 == count => 2,
        _ => 1,
    }
}

/// Split a line into the pieces drawn as one: maximal runs of text in one
/// style and color (spaces included), and each fill-in line by itself.
fn runs(line: &[Word]) -> impl Iterator<Item = &[Word]> {
    line.chunk_by(|a, b| {
        a.kind == WordKind::Text
            && b.kind == WordKind::Text
            && a.style == b.style
            && a.color == b.color
            && b.space_before.is_none_or(|space| space == b.style)
    })
}

/// Append `glyphs` to `out`, shifting their text ranges so they point into a
/// merged string whose relevant piece starts at byte `offset`.
fn append_glyphs(out: &mut Vec<KrillaGlyph>, glyphs: &[KrillaGlyph], offset: usize) {
    for glyph in glyphs {
        let mut glyph = glyph.clone();
        glyph.text_range = glyph.text_range.start + offset..glyph.text_range.end + offset;
        out.push(glyph);
    }
}

impl Engine<'_> {
    /// The height a paragraph of inlines would occupy at the given width.
    pub(super) fn paragraph_height(
        &self,
        inlines: &[Inline],
        size: f32,
        bold: bool,
        italic: bool,
        width: f32,
    ) -> f32 {
        let lines = self.paragraph_lines(inlines, size, bold, italic, width);
        lines as f32 * size * self.theme.spacing.line_height
    }

    /// The number of lines a paragraph of inlines wraps to at the given width.
    pub(super) fn paragraph_lines(
        &self,
        inlines: &[Inline],
        size: f32,
        bold: bool,
        italic: bool,
        width: f32,
    ) -> usize {
        let words = self.tokenize(inlines, bold, italic, size);
        self.wrap(words, width, size).len()
    }

    /// Lay out flowing text across the full content width, breaking pages as
    /// needed, without leaving a widow or orphan (see [`kept_lines`]).
    pub(super) fn layout_paragraph(
        &mut self,
        inlines: &[Inline],
        size: f32,
        bold: bool,
        italic: bool,
        color: rgb::Color,
    ) {
        let words = self.tokenize(inlines, bold, italic, size);
        let lines = self.wrap(words, self.width(), size);
        let line_h = size * self.theme.spacing.line_height;
        let content_left = self.left;
        for (index, line) in lines.iter().enumerate() {
            self.ensure(kept_lines(index, lines.len()) as f32 * line_h);
            let top = self.y;
            self.draw_line(line, content_left, top, size, color);
            self.y += line_h;
        }
    }

    /// Draw `text` on one line from `x`, in `style` where its face has the
    /// glyphs and a fallback face where it does not.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_text(
        &mut self,
        style: Style,
        text: &str,
        mut x: f32,
        baseline: f32,
        size: f32,
        color: rgb::Color,
        tag: Tagging,
    ) {
        for (style, range) in self.fonts.split(style, text) {
            let text = &text[range];
            let shaped = self.fonts.shape(style, text);
            let width = shaped.width(size);
            self.push(Element::Text(TextElement {
                x,
                baseline,
                size,
                color,
                style,
                glyphs: shaped.glyphs,
                text: text.to_string(),
                tag,
            }));
            x += width;
        }
    }

    /// The width of the space drawn before `word` when it does not start a
    /// line.
    pub(super) fn space_width(&self, word: &Word, size: f32) -> f32 {
        word.space_before
            .map_or(0.0, |style| self.fonts.space_width(style, size))
    }

    /// The width of `words` on one line with every word shaped by itself: what
    /// line breaking budgets for, and never less than the
    /// [`line_width`](Self::line_width) they are drawn at.
    pub(super) fn words_width(&self, words: &[Word], size: f32) -> f32 {
        let mut width = 0.0;
        for (i, word) in words.iter().enumerate() {
            if i > 0 {
                width += self.space_width(word, size);
            }
            width += word.width;
        }
        width
    }

    /// The width one line of words is drawn at by
    /// [`draw_line`](Self::draw_line).
    pub(super) fn line_width(&self, line: &[Word], size: f32) -> f32 {
        let mut width = 0.0;
        for (i, run) in runs(line).enumerate() {
            if i > 0 {
                width += self.space_width(&run[0], size);
            }
            width += match run {
                [word] => word.width,
                _ => self.merge_run(run, size).2,
            };
        }
        width
    }

    /// Emit one line of words starting at `(x_left, top)`, merging consecutive
    /// same-style, same-color words into a single text element.
    pub(super) fn draw_line(
        &mut self,
        line: &[Word],
        x_left: f32,
        top: f32,
        size: f32,
        color: rgb::Color,
    ) {
        let baseline = top + self.fonts.ascent(Style::Regular, size);
        let mut x = x_left;
        for (i, run) in runs(line).enumerate() {
            if i > 0 {
                x += self.space_width(&run[0], size);
            }
            let run_color = run[0].color.unwrap_or(color);
            // A fill-in line draws a baseline stroke rather than glyphs.
            if run[0].kind == WordKind::FillIn {
                let width = run[0].width;
                self.push(Element::Stroke {
                    points: vec![(x, baseline), (x + width, baseline)],
                    width: 0.7,
                    color: run_color,
                    closed: false,
                });
                x += width;
                continue;
            }
            let (text, glyphs, width) = self.merge_run(run, size);
            self.push(Element::Text(TextElement {
                x,
                baseline,
                size,
                color: run_color,
                style: run[0].style,
                glyphs,
                text,
                tag: self.current_tag,
            }));
            x += width;
        }
    }

    /// Join same-style words into one text + glyph sequence, with a space
    /// between each pair that whitespace separates. The text is shaped as a
    /// whole, so kerning and ligatures apply across its words. Should that
    /// come out wider than the words shaped apart, which is what line
    /// breaking budgeted for, the words' own glyphs are joined instead.
    fn merge_run(&self, words: &[Word], size: f32) -> (String, Arc<[KrillaGlyph]>, f32) {
        if let [word] = words {
            return (word.text.clone(), word.glyphs.clone(), word.width);
        }
        let style = words[0].style;
        let mut text = String::new();
        for (i, word) in words.iter().enumerate() {
            if i > 0 && word.space_before.is_some() {
                text.push(' ');
            }
            text.push_str(&word.text);
        }
        let apart = self.words_width(words, size);
        let whole = self.fonts.shape(style, &text);
        if whole.width(size) <= apart + FIT_EPSILON {
            return (text, whole.glyphs.clone(), whole.width(size));
        }

        let space = self.fonts.shape(style, " ");
        let mut glyphs = Vec::new();
        let mut offset = 0;
        for (i, word) in words.iter().enumerate() {
            if i > 0 && word.space_before.is_some() {
                append_glyphs(&mut glyphs, &space.glyphs, offset);
                offset += 1;
            }
            append_glyphs(&mut glyphs, &word.glyphs, offset);
            offset += word.text.len();
        }
        (text, glyphs.into(), apart)
    }

    /// Shape the inlines into [`Word`]s: the pieces between whitespace, line
    /// break opportunities and run boundaries. Newlines become hard-break
    /// markers, honored by [`wrap`](Self::wrap); all other collapsible
    /// whitespace (tabs, `\r`, …) separates words like a space. Typographic
    /// spaces (no-break, em, thin, …) stay inside their word at their own
    /// width. A word is split further only where a fallback face takes over
    /// for characters the face of its style lacks. Break opportunities are found across the whole
    /// text, so a run boundary by itself neither spaces nor breaks:
    /// `**bold**,` stays together.
    pub(super) fn tokenize(
        &self,
        inlines: &[Inline],
        base_bold: bool,
        base_italic: bool,
        size: f32,
    ) -> Vec<Word> {
        // The text of all runs, with the byte offset each run ends at.
        let mut text = String::new();
        let mut run_ends = Vec::with_capacity(inlines.len());
        for inline in inlines {
            match inline.fill_in {
                Some(_) => text.push(FILL_IN_STAND_IN),
                None => text.push_str(&inline.text),
            }
            run_ends.push(text.len());
        }

        let mut words = Vec::new();
        // The style of the first whitespace since the last word, if any.
        let mut space = None;
        let mut run = 0;
        let mut start = 0;
        for (end, opportunity) in linebreaks(&text) {
            // Only the first word of each unbreakable segment may start a line.
            let mut first = true;
            let mut add = |mut word: Word, space: &mut Option<Style>| {
                word.space_before = space.take();
                word.break_before = std::mem::take(&mut first);
                words.push(word);
            };
            while start < end {
                while run_ends[run] <= start {
                    run += 1;
                }
                let inline = &inlines[run];
                let piece_end = end.min(run_ends[run]);
                let color = inline.color.map(|c| c.resolve(&self.theme.palette));
                if let Some(length) = inline.fill_in {
                    // An atomic blank, not text to be tokenized.
                    add(Word::marker(WordKind::FillIn, color, length), &mut space);
                } else {
                    let style = inline.resolve_style(base_bold, base_italic);
                    for (i, token) in text[start..piece_end].split(collapses).enumerate() {
                        if i > 0 {
                            space.get_or_insert(style);
                        }
                        // One word per face: a character the style's face
                        // lacks is drawn from a fallback face.
                        for (face, piece) in self.fonts.split(style, token) {
                            if !piece.is_empty() {
                                add(
                                    self.shape_word(face, color, &token[piece], size),
                                    &mut space,
                                );
                            }
                        }
                    }
                }
                start = piece_end;
            }
            // The end of the text is a mandatory break too, but not a line.
            if opportunity == BreakOpportunity::Mandatory && text[..end].ends_with('\n') {
                words.push(Word::marker(WordKind::HardBreak, None, 0.0));
                space = None;
            }
        }
        words
    }

    /// Greedy line breaking: pack words until the next one would overflow,
    /// only breaking before a word that allows it. An unbreakable group wider
    /// than `max_width` breaks wherever it overflows: between its words, and
    /// within a word that is too wide by itself, between grapheme clusters.
    /// Hard-break markers end the current line unconditionally (two in a row
    /// yield an empty line). A line that ends at a soft hyphen draws it as a
    /// hyphen, which the line has room for.
    pub(super) fn wrap(&self, words: Vec<Word>, max_width: f32, size: f32) -> Vec<Vec<Word>> {
        // The units a line may not break inside.
        let mut units: Vec<Vec<Word>> = Vec::new();
        let mut words = words.into_iter().peekable();
        while let Some(word) = words.next() {
            let mut group = vec![word];
            if group[0].kind != WordKind::HardBreak {
                while let Some(next) =
                    words.next_if(|w| w.kind != WordKind::HardBreak && !w.break_before)
                {
                    group.push(next);
                }
            }
            if self.words_width(&group, size) <= max_width + FIT_EPSILON {
                units.push(group);
                continue;
            }
            for word in group {
                // Only text can be split; a fill-in line is atomic.
                if word.width > max_width + FIT_EPSILON && word.kind == WordKind::Text {
                    units.extend(
                        self.break_word(&word, max_width, size)
                            .into_iter()
                            .map(|fragment| vec![fragment]),
                    );
                } else {
                    units.push(vec![word]);
                }
            }
        }

        let mut lines: Vec<Vec<Word>> = Vec::new();
        let mut line: Vec<Word> = Vec::new();
        let mut width = 0.0;

        for unit in units {
            if unit[0].kind == WordKind::HardBreak {
                lines.push(std::mem::take(&mut line));
                width = 0.0;
                continue;
            }
            let unit_width = self.words_width(&unit, size);
            let space = if line.is_empty() {
                0.0
            } else {
                self.space_width(&unit[0], size)
            };
            // A unit ending in a soft hyphen only fits with room for the
            // hyphen it gets should the line end there.
            let hyphen = match unit.last() {
                Some(last) if ends_in_soft_hyphen(last) => {
                    self.fonts.measure(last.style, "-", size)
                }
                _ => 0.0,
            };
            if !line.is_empty() && width + space + unit_width + hyphen > max_width + FIT_EPSILON {
                if let Some(last) = line.last_mut().filter(|last| ends_in_soft_hyphen(last)) {
                    let text = last.text.trim_end_matches(SOFT_HYPHEN).to_owned() + "-";
                    *last = Word {
                        space_before: last.space_before,
                        break_before: last.break_before,
                        ..self.shape_word(last.style, last.color, &text, size)
                    };
                }
                lines.push(std::mem::take(&mut line));
                width = unit_width;
            } else {
                width += space + unit_width;
            }
            line.extend(unit);
        }
        if !line.is_empty() {
            lines.push(line);
        }
        if lines.is_empty() {
            lines.push(Vec::new());
        }
        lines
    }

    /// Break an over-wide word into re-shaped fragments of whole grapheme
    /// clusters; a single cluster wider than `max_width` is emitted on its
    /// own. Only the first fragment keeps the word's space before it.
    fn break_word(&self, word: &Word, max_width: f32, size: f32) -> Vec<Word> {
        // The byte offset each grapheme cluster ends at, and the width of the
        // word up to there. A fragment shaped by itself may differ a little
        // (a split ligature, lost kerning), so these only say where to try.
        let mut glyphs = word.glyphs.iter().peekable();
        let mut width = 0.0;
        let ends: Vec<(usize, f32)> = word
            .text
            .grapheme_indices(true)
            .map(|(index, grapheme)| {
                let end = index + grapheme.len();
                while let Some(glyph) = glyphs.next_if(|glyph| glyph.text_range.start < end) {
                    width += glyph.x_advance * size;
                }
                (end, width)
            })
            .collect();

        let mut fragments: Vec<Word> = Vec::new();
        let (mut first, mut start, mut start_width) = (0, 0, 0.0);
        while first < ends.len() {
            // The clusters that should fit; then fewer until they really do,
            // or more while they still do.
            let budget = start_width + max_width + FIT_EPSILON;
            let estimate = ends[first..]
                .partition_point(|&(_, width)| width <= budget)
                .max(1);
            let fitting = |count: usize| {
                let text = &word.text[start..ends[first + count - 1].0];
                let fragment = self.shape_word(word.style, word.color, text, size);
                (count == 1 || fragment.width <= max_width + FIT_EPSILON).then_some(fragment)
            };
            let mut count = estimate;
            let mut fragment = loop {
                match fitting(count) {
                    Some(fragment) => break fragment,
                    None => count -= 1,
                }
            };
            if count == estimate {
                while first + count < ends.len()
                    && let Some(longer) = fitting(count + 1)
                {
                    fragment = longer;
                    count += 1;
                }
            }
            fragments.push(Word {
                space_before: word.space_before.filter(|_| fragments.is_empty()),
                break_before: true,
                ..fragment
            });
            first += count;
            (start, start_width) = ends[first - 1];
        }
        fragments
    }

    /// Shape a piece of text into a [`Word`] at the given size, to be placed
    /// by its caller: it neither follows a space nor allows a break.
    fn shape_word(&self, style: Style, color: Option<rgb::Color>, text: &str, size: f32) -> Word {
        let shaped = self.fonts.shape(style, text);
        Word {
            style,
            color,
            text: text.to_string(),
            width: shaped.width(size),
            glyphs: shaped.glyphs,
            kind: WordKind::Text,
            space_before: None,
            break_before: false,
        }
    }
}

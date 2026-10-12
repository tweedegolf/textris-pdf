//! Flowing text: tokenizing inlines into shaped words, greedy line breaking,
//! and emitting lines of merged same-style runs.

use std::sync::Arc;

use krilla::{color::rgb, text::KrillaGlyph};

use crate::{
    fonts::Style,
    layout::{Element, Engine, FIT_EPSILON, TextElement},
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
    /// Whether whitespace separates this word from the one before it. A word
    /// without it continues the previous word in another run (the `,` after a
    /// `**bold**` run): it is drawn without a space and the two only break
    /// apart when they cannot share a line.
    pub(super) space_before: bool,
}

impl Word {
    /// A glyph-less marker word (a hard break or a fill-in line).
    fn marker(kind: WordKind, color: Option<rgb::Color>, width: f32, space_before: bool) -> Self {
        Self {
            style: Style::Regular,
            color,
            text: String::new(),
            glyphs: Vec::new().into(),
            width,
            kind,
            space_before,
        }
    }
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
        let words = self.tokenize(inlines, bold, italic, size);
        let lines = self.wrap(words, width, size);
        lines.len() as f32 * size * self.theme.spacing.line_height
    }

    /// Lay out flowing text across the full content width, breaking pages as
    /// needed.
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
        for line in lines {
            self.ensure(line_h);
            let top = self.y;
            self.draw_line(&line, content_left, top, size, color);
            self.y += line_h;
        }
    }

    /// The rendered width of one line of words: word widths plus a space before
    /// every word after the first that follows whitespace, mirroring how
    /// [`draw_line`](Self::draw_line) advances.
    pub(super) fn line_width(&self, line: &[Word], size: f32) -> f32 {
        let mut width = 0.0;
        for (i, word) in line.iter().enumerate() {
            if i > 0 && word.space_before {
                width += self.fonts.space_width(word.style, size);
            }
            width += word.width;
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
        let mut index = 0;
        while index < line.len() {
            let style = line[index].style;
            let run_color = line[index].color.unwrap_or(color);
            if index > 0 && line[index].space_before {
                x += self.fonts.space_width(style, size);
            }
            // A fill-in line draws a baseline stroke rather than glyphs, and is
            // never merged into a neighbouring text run.
            if line[index].kind == WordKind::FillIn {
                let width = line[index].width;
                self.push(Element::Stroke {
                    points: vec![(x, baseline), (x + width, baseline)],
                    width: 0.7,
                    color: run_color,
                    closed: false,
                });
                x += width;
                index += 1;
                continue;
            }
            let mut run_end = index + 1;
            while run_end < line.len()
                && line[run_end].kind == WordKind::Text
                && line[run_end].style == style
                && line[run_end].color.unwrap_or(color) == run_color
            {
                run_end += 1;
            }
            let (text, glyphs, width) = self.merge_run(&line[index..run_end], style, size);
            self.push(Element::Text(TextElement {
                x,
                baseline,
                size,
                color: run_color,
                style,
                glyphs,
                text,
                tag: self.current_tag,
            }));
            x += width;
            index = run_end;
        }
    }

    /// Concatenate same-style words into one text + glyph sequence, with a
    /// shaped space between each pair that whitespace separates.
    fn merge_run(
        &self,
        words: &[Word],
        style: Style,
        size: f32,
    ) -> (String, Arc<[KrillaGlyph]>, f32) {
        if let [word] = words {
            return (word.text.clone(), word.glyphs.clone(), word.width);
        }
        let space = self.fonts.shape(style, " ");
        let mut text = String::new();
        let mut glyphs = Vec::new();
        let mut width = 0.0;
        for (i, word) in words.iter().enumerate() {
            if i > 0 && word.space_before {
                append_glyphs(&mut glyphs, &space.glyphs, text.len());
                text.push(' ');
                width += space.width(size);
            }
            append_glyphs(&mut glyphs, &word.glyphs, text.len());
            text.push_str(&word.text);
            width += word.width;
        }
        (text, glyphs.into(), width)
    }

    /// Shape each whitespace-separated word of the inlines into a [`Word`].
    /// Newlines become hard-break markers, honored by [`wrap`](Self::wrap);
    /// all other breaking whitespace (tabs, `\r`, …) separates words like a
    /// space. Non-breaking spaces stay inside their word. Runs only separate
    /// words where their text has whitespace, so `**bold**,` stays one word.
    pub(super) fn tokenize(
        &self,
        inlines: &[Inline],
        base_bold: bool,
        base_italic: bool,
        size: f32,
    ) -> Vec<Word> {
        fn breaks_words(c: char) -> bool {
            c.is_whitespace() && !matches!(c, '\u{A0}' | '\u{202F}' | '\u{2007}')
        }
        let mut words = Vec::new();
        // Whether whitespace has passed since the last word, carried across
        // runs.
        let mut space = false;
        for inline in inlines {
            let style = inline.resolve_style(base_bold, base_italic);
            let color = inline.color.map(|c| c.resolve(&self.theme.palette));
            // A fill-in run is an atomic blank, not text to be tokenized.
            if let Some(length) = inline.fill_in {
                words.push(Word::marker(WordKind::FillIn, color, length, space));
                space = false;
                continue;
            }
            for (index, segment) in inline.text.split('\n').enumerate() {
                if index > 0 {
                    words.push(Word::marker(WordKind::HardBreak, None, 0.0, false));
                }
                for (i, token) in segment.split(breaks_words).enumerate() {
                    space |= i > 0;
                    if !token.is_empty() {
                        words.push(self.shape_word(style, color, token, size, space));
                        space = false;
                    }
                }
            }
        }
        words
    }

    /// Greedy line breaking: pack words until the next one would overflow,
    /// keeping words glued without whitespace together. A glued group wider
    /// than `max_width` breaks wherever it overflows: between its words, and
    /// within a word that is too wide by itself, by character-level fragments.
    /// Hard-break markers end the current line unconditionally (two in a row
    /// yield an empty line).
    pub(super) fn wrap(&self, words: Vec<Word>, max_width: f32, size: f32) -> Vec<Vec<Word>> {
        // The units a line may not break inside.
        let mut units: Vec<Vec<Word>> = Vec::new();
        let mut words = words.into_iter().peekable();
        while let Some(word) = words.next() {
            let mut group = vec![word];
            if group[0].kind != WordKind::HardBreak {
                while let Some(next) =
                    words.next_if(|w| w.kind != WordKind::HardBreak && !w.space_before)
                {
                    group.push(next);
                }
            }
            if group.iter().map(|w| w.width).sum::<f32>() <= max_width + FIT_EPSILON {
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
            let unit_width: f32 = unit.iter().map(|w| w.width).sum();
            let space = if line.is_empty() || !unit[0].space_before {
                0.0
            } else {
                self.fonts.space_width(unit[0].style, size)
            };
            if !line.is_empty() && width + space + unit_width > max_width + FIT_EPSILON {
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

    /// Break an over-wide word into re-shaped character-level fragments; a
    /// single character wider than `max_width` is emitted on its own. Only the
    /// first fragment keeps the word's space before it.
    fn break_word(&self, word: &Word, max_width: f32, size: f32) -> Vec<Word> {
        let mut fragments = Vec::new();
        let mut current = String::new();
        for ch in word.text.chars() {
            let mut candidate = current.clone();
            candidate.push(ch);
            let candidate_width = self.fonts.measure(word.style, &candidate, size);
            if !current.is_empty() && candidate_width > max_width + FIT_EPSILON {
                let space_before = fragments.is_empty() && word.space_before;
                fragments.push(self.shape_word(
                    word.style,
                    word.color,
                    &current,
                    size,
                    space_before,
                ));
                current = ch.to_string();
            } else {
                current = candidate;
            }
        }
        if !current.is_empty() {
            let space_before = fragments.is_empty() && word.space_before;
            fragments.push(self.shape_word(word.style, word.color, &current, size, space_before));
        }
        fragments
    }

    /// Shape a piece of text into a [`Word`] at the given size.
    fn shape_word(
        &self,
        style: Style,
        color: Option<rgb::Color>,
        text: &str,
        size: f32,
        space_before: bool,
    ) -> Word {
        let shaped = self.fonts.shape(style, text);
        Word {
            style,
            color,
            text: text.to_string(),
            width: shaped.width(size),
            glyphs: shaped.glyphs,
            kind: WordKind::Text,
            space_before,
        }
    }
}

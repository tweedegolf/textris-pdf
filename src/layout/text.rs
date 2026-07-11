//! Flowing text: tokenizing inlines into shaped words, greedy line breaking,
//! and emitting lines of merged same-style runs.

use std::sync::Arc;

use krilla::{color::rgb, text::KrillaGlyph};

use crate::{
    fonts::Style,
    layout::{Element, Engine, TextElement},
    model::Inline,
};

/// A shaped word plus its measured width, the atomic unit of line breaking.
#[derive(Debug, Clone)]
pub(super) struct Word {
    pub(super) style: Style,
    pub(super) color: Option<rgb::Color>,
    pub(super) text: String,
    pub(super) glyphs: Arc<[KrillaGlyph]>,
    pub(super) width: f32,
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
    /// every word after the first, mirroring how [`draw_line`](Self::draw_line)
    /// advances.
    pub(super) fn line_width(&self, line: &[Word], size: f32) -> f32 {
        let mut width = 0.0;
        for (i, word) in line.iter().enumerate() {
            if i > 0 {
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
            if index > 0 {
                x += self.fonts.space_width(style, size);
            }
            let mut run_end = index + 1;
            while run_end < line.len()
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
            }));
            x += width;
            index = run_end;
        }
    }

    /// Concatenate same-style words into one text + glyph sequence, with a
    /// shaped space between each pair.
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
            if i > 0 {
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
    pub(super) fn tokenize(
        &self,
        inlines: &[Inline],
        base_bold: bool,
        base_italic: bool,
        size: f32,
    ) -> Vec<Word> {
        let mut words = Vec::new();
        for inline in inlines {
            let style = inline.resolve_style(base_bold, base_italic);
            for token in inline.text.split(' ') {
                if token.is_empty() {
                    continue;
                }
                words.push(self.shape_word(style, inline.color, token, size));
            }
        }
        words
    }

    /// Greedy line breaking: pack words until the next one would overflow. A
    /// word wider than `max_width` is broken into character-level fragments so
    /// it wraps instead of overflowing.
    pub(super) fn wrap(&self, words: Vec<Word>, max_width: f32, size: f32) -> Vec<Vec<Word>> {
        // `space_before` is false for the continuation fragments of a broken
        // word so no space is inserted mid-word.
        let mut pieces: Vec<(Word, bool)> = Vec::new();
        for word in words {
            if word.width > max_width {
                for (i, frag) in self
                    .break_word(&word, max_width, size)
                    .into_iter()
                    .enumerate()
                {
                    pieces.push((frag, i == 0));
                }
            } else {
                pieces.push((word, true));
            }
        }

        let mut lines: Vec<Vec<Word>> = Vec::new();
        let mut line: Vec<Word> = Vec::new();
        let mut width = 0.0;

        for (word, space_before) in pieces {
            let space = if line.is_empty() || !space_before {
                0.0
            } else {
                self.fonts.space_width(word.style, size)
            };
            if !line.is_empty() && width + space + word.width > max_width {
                lines.push(std::mem::take(&mut line));
                width = word.width;
            } else {
                width += space + word.width;
            }
            line.push(word);
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
    /// single character wider than `max_width` is emitted on its own.
    fn break_word(&self, word: &Word, max_width: f32, size: f32) -> Vec<Word> {
        let mut fragments = Vec::new();
        let mut current = String::new();
        for ch in word.text.chars() {
            let mut candidate = current.clone();
            candidate.push(ch);
            let candidate_width = self.fonts.measure(word.style, &candidate, size);
            if !current.is_empty() && candidate_width > max_width {
                fragments.push(self.shape_word(word.style, word.color, &current, size));
                current = ch.to_string();
            } else {
                current = candidate;
            }
        }
        if !current.is_empty() {
            fragments.push(self.shape_word(word.style, word.color, &current, size));
        }
        fragments
    }

    /// Shape a piece of text into a [`Word`] at the given size.
    fn shape_word(&self, style: Style, color: Option<rgb::Color>, text: &str, size: f32) -> Word {
        let shaped = self.fonts.shape(style, text);
        Word {
            style,
            color,
            text: text.to_string(),
            width: shaped.width(size),
            glyphs: shaped.glyphs,
        }
    }
}

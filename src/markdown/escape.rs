//! The escaping shared by the Markdown exporter, the dialect parser, and host
//! template engines.
//!
//! The dialect string a host feeds to the parser is typically assembled by a
//! template engine from trusted template text plus *untrusted interpolated
//! data* (names, addresses, …). Interpolated data must never be able to change
//! document structure, so every interpolated value has to pass through one of
//! the functions here (wire them up as the template engine's auto-escaper and
//! filters). Two rules make this sound:
//!
//! 1. The parser reads a backslash before **any** ASCII punctuation character
//!    as that literal character (CommonMark's rule).
//! 2. [`escape`] and [`escape_cell`] backslash-escape **every** ASCII
//!    punctuation character, so any character that is (or could ever become)
//!    dialect syntax is inert. The intermediate string is machine-consumed, so
//!    its readability does not matter.
//!
//! Newlines are the one non-punctuation hazard (a blank line inside data would
//! split the enclosing block): each function folds them to the hard-break form
//! of its context. Other control characters are dropped.
//!
//! The exporter's output is read by people instead, so it escapes only the
//! characters that would otherwise be syntax ([`escape_line`]); the parser
//! reads that back just the same.

/// Escape `text` for a flow context: a paragraph, heading, list item or quote.
///
/// Every ASCII punctuation character is backslash-escaped. Whitespace runs
/// that contain a newline collapse to a single hard break (a `\` at the end of
/// the line); other control characters are dropped. The parser yields the text
/// back as a single plain run, up to that newline normalization and the
/// trimming of outer whitespace that flow contexts perform.
pub fn escape(text: &str) -> String {
    fold_and_escape(text, "\\\n", true)
}

/// Escape `text` for a table cell: [`escape`], but newlines become `<br>`,
/// since a cell must stay on its source line.
pub fn escape_cell(text: &str) -> String {
    fold_and_escape(text, "<br>", true)
}

/// Wrap `text` in a verbatim inline code span (a `mono` run), growing the
/// backtick fence past any backticks inside it. Backslash escapes do not work
/// inside a code span, hence the fence growing; for the same reason newline
/// runs fold to a single space and other control characters are dropped (a
/// code span cannot hold a line break). An empty `text` yields an empty
/// string, as there is no Markdown for an empty code span.
///
/// Not for table cells: use [`mono_cell`] there.
pub fn mono(text: &str) -> String {
    code_span(&fold_and_escape(text, " ", false))
}

/// [`mono`] for a table cell: additionally encodes `|` (as `\|`, which GitHub
/// honors even inside a code span within a table) and doubles backslashes so
/// the two stay distinguishable. The parser decodes both inside a cell's code
/// span; GitHub renders the doubled backslash literally, a small display
/// artifact for backslash-carrying cell content.
pub fn mono_cell(text: &str) -> String {
    let folded = fold_and_escape(text, " ", false);
    let mut encoded = String::with_capacity(folded.len());
    for ch in folded.chars() {
        match ch {
            '\\' => encoded.push_str("\\\\"),
            '|' => encoded.push_str("\\|"),
            c => encoded.push(c),
        }
    }
    code_span(&encoded)
}

/// The shared body of the escape functions: drop control characters, collapse
/// every whitespace run that contains a newline to `line_break`, and (when
/// `escape_punctuation` is set) backslash-escape all ASCII punctuation.
fn fold_and_escape(text: &str, line_break: &str, escape_punctuation: bool) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    // Whitespace is held back so a newline can swallow the spaces around it.
    let mut whitespace = String::new();
    let mut break_pending = false;
    for ch in text.chars() {
        if ch == '\n' {
            break_pending = true;
            whitespace.clear();
        } else if ch.is_control() {
            // Dropped: control characters have no dialect representation.
        } else if ch.is_whitespace() {
            if !break_pending {
                whitespace.push(ch);
            }
        } else {
            if break_pending {
                out.push_str(line_break);
                break_pending = false;
            } else {
                out.push_str(&whitespace);
            }
            whitespace.clear();
            if escape_punctuation && ch.is_ascii_punctuation() {
                out.push('\\');
            }
            out.push(ch);
        }
    }
    // Trailing whitespace would read as a hard break (two trailing spaces) or
    // dangle a break marker; both fold away, matching the trimming the parser
    // applies at block edges.
    out
}

/// Escape one line of exported text (no newlines) for people to read: only a
/// character that would otherwise be Markdown or dialect syntax gets a
/// backslash, so `Costa, R. (observer)` stays as written. `line_start` says
/// whether the text opens a line, where block markers (`# `, `- `, `1. `, …)
/// count. A trailing `\` or `<` is escaped too, as the next run continues the
/// line.
pub(crate) fn escape_line(text: &str, line_start: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    let marker = if line_start {
        block_marker(&chars)
    } else {
        None
    };
    let mut out = String::with_capacity(text.len());
    for (i, &ch) in chars.iter().enumerate() {
        let next = chars.get(i + 1);
        let syntax = match ch {
            '\\' => next.is_none_or(char::is_ascii_punctuation),
            '*' | '_' | '`' | '[' | '|' | '~' => true,
            // An HTML tag or autolink.
            '<' => next.is_none_or(|&c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?')),
            '&' => is_entity(&chars[i + 1..]),
            _ => marker == Some(i),
        };
        if syntax {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// The index of the character that makes a line open a block, if any: a
/// heading, quote, list item, rule or setext underline, or a dialect attribute
/// line, directive, lettered item or front-matter fence.
fn block_marker(chars: &[char]) -> Option<usize> {
    let start = chars.iter().position(|c| !c.is_whitespace())?;
    let rest = &chars[start..];
    let spaced = |i: usize| rest.get(i).is_none_or(|c| c.is_whitespace());
    let only = |marker: char| rest.iter().all(|&c| c == marker || c.is_whitespace());
    match rest[0] {
        '#' | '>' | '{' | '@' => Some(start),
        '-' | '+' if spaced(1) || only(rest[0]) => Some(start),
        '=' if only('=') => Some(start),
        c if c.is_ascii_digit() => {
            let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
            (matches!(rest.get(digits), Some('.' | ')')) && spaced(digits + 1))
                .then_some(start + digits)
        }
        c if c.is_ascii_lowercase() => {
            (rest.get(1) == Some(&'.') && spaced(2)).then_some(start + 1)
        }
        _ => None,
    }
}

/// Whether the text after an `&` makes it a character reference (`&amp;`,
/// `&#38;`), which Markdown would decode.
fn is_entity(after: &[char]) -> bool {
    let after = after.strip_prefix(&['#']).unwrap_or(after);
    let name = after
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric())
        .count();
    name > 0 && after.get(name) == Some(&';')
}

/// Wrap `text` in an inline code span, choosing a backtick fence long enough
/// to contain any backticks inside it. The content is padded with one space on
/// each side when it would otherwise touch the fence, or when it both starts
/// and ends with a space (the padding survives the one-space strip readers
/// apply, preserving the spaces).
pub(crate) fn code_span(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    let padded_spaces = text.starts_with(' ') && text.ends_with(' ') && !text.trim().is_empty();
    if text.starts_with('`') || text.ends_with('`') || padded_spaces {
        format!("{fence} {text} {fence}")
    } else {
        format!("{fence}{text}{fence}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_covers_all_ascii_punctuation() {
        let punctuation = r##"!"#$%&'()*+,-./:;<=>?@[\]^_`{|}~"##;
        let escaped = escape(punctuation);
        let mut expected = String::new();
        for ch in punctuation.chars() {
            expected.push('\\');
            expected.push(ch);
        }
        assert_eq!(escaped, expected);
    }

    #[test]
    fn escape_folds_newline_runs_to_one_hard_break() {
        assert_eq!(escape("a\nb"), "a\\\nb");
        assert_eq!(escape("a\n\n\nb"), "a\\\nb");
        // Spaces around the newline fold into the break.
        assert_eq!(escape("a \n b"), "a\\\nb");
        // Interior whitespace without a newline is preserved.
        assert_eq!(escape("a  b"), "a  b");
    }

    #[test]
    fn escape_drops_other_control_characters() {
        assert_eq!(escape("a\tb\u{1}c\rd"), "abcd");
        // CRLF folds like a bare newline.
        assert_eq!(escape("a\r\nb"), "a\\\nb");
    }

    #[test]
    fn escape_drops_trailing_whitespace_and_breaks() {
        // Trailing whitespace would read as a hard break; leading whitespace
        // is harmless (flow contexts trim it) and passes through.
        assert_eq!(escape("  a  "), "  a");
        assert_eq!(escape("a\n"), "a");
        // A leading newline still breaks at the interpolation point.
        assert_eq!(escape("\na"), "\\\na");
    }

    #[test]
    fn escape_cell_uses_br_for_newlines() {
        assert_eq!(escape_cell("a\n\nb"), "a<br>b");
        assert_eq!(escape_cell("x|y"), "x\\|y");
    }

    #[test]
    fn escape_line_leaves_plain_punctuation_alone() {
        let text = "Kandidaat nr. 18: Holthuizen, A.H. (Adrianus) 's-Gravenhage 15-02-1904 R&D!";
        assert_eq!(escape_line(text, true), text);
    }

    #[test]
    fn escape_line_escapes_inline_syntax() {
        assert_eq!(
            escape_line("*a* _b_ `c` [d] e|f ~g~", false),
            r"\*a\* \_b\_ \`c\` \[d] e\|f \~g\~"
        );
        assert_eq!(escape_line("a <b> < c", false), r"a \<b> < c");
        assert_eq!(
            escape_line("&amp; &#38; & amp;", false),
            r"\&amp; \&#38; & amp;"
        );
        // A backslash is literal unless punctuation (or the next run) follows.
        assert_eq!(escape_line(r"C:\Users \* \", false), r"C:\Users \\\* \\");
    }

    #[test]
    fn escape_line_escapes_block_markers_at_line_start_only() {
        for (line, escaped) in [
            ("# a", r"\# a"),
            ("> a", r"\> a"),
            ("{ a }", r"\{ a }"),
            ("@a", r"\@a"),
            ("- a", r"\- a"),
            ("+ a", r"\+ a"),
            ("---", r"\---"),
            ("+++", r"\+++"),
            ("==", r"\=="),
            ("18. a", r"18\. a"),
            ("1) a", r"1\) a"),
            ("a. b", r"a\. b"),
            ("  - a", r"  \- a"),
        ] {
            assert_eq!(escape_line(line, true), escaped);
            assert_eq!(escape_line(line, false), line);
        }
        for line in ["-5 graden", "2026 begint", "ab. c", "A. b", "=a"] {
            assert_eq!(escape_line(line, true), line);
        }
    }

    #[test]
    fn mono_wraps_verbatim_and_folds_newlines_to_spaces() {
        assert_eq!(mono("a*b_c"), "`a*b_c`");
        assert_eq!(mono("a\nb"), "`a b`");
        assert_eq!(mono(""), "");
    }

    #[test]
    fn mono_cell_encodes_pipes_and_backslashes() {
        assert_eq!(mono_cell("a|b"), r"`a\|b`");
        assert_eq!(mono_cell(r"a\b"), r"`a\\b`");
        assert_eq!(mono_cell(r"a\|b"), r"`a\\\|b`");
    }

    #[test]
    fn code_span_grows_the_fence_past_inner_backticks() {
        assert_eq!(code_span("plain"), "`plain`");
        assert_eq!(code_span("a`b"), "``a`b``");
        assert_eq!(code_span("`x`"), "`` `x` ``");
    }

    #[test]
    fn code_span_pads_to_preserve_outer_spaces() {
        assert_eq!(code_span(" x "), "`  x  `");
        // One-sided spaces survive without padding.
        assert_eq!(code_span(" x"), "` x`");
        // All-space content is not stripped by readers, so no padding.
        assert_eq!(code_span("  "), "`  `");
    }
}

//! Byte offset → LSP `Position` (zero-based line, **UTF-16 code unit**
//! character).
//!
//! This is the one piece of the server that is easy to get subtly, silently
//! wrong. Three separate column conventions are in play:
//!
//! | who | line | character |
//! |---|---|---|
//! | [`rustyfi_syntax::Loc`] | 1-based | 0-based **`char`s** (`Loc::col`) |
//! | LSP `Position` | 0-based | 0-based **UTF-16 code units** |
//! | a naive implementation | — | bytes |
//!
//! For ASCII all three agree, which is exactly why a byte- or char-based
//! implementation passes every ASCII test and then puts every squiggle in the
//! wrong place on the Japanese documents this port exists to typeset. `あ` is
//! 3 bytes, 1 `char` and 1 UTF-16 unit; `🎉` is 4 bytes, 1 `char` and **2**
//! UTF-16 units (a surrogate pair). So `Loc::col` is not usable either — it
//! is right for `あ` and wrong for `🎉`.
//!
//! Hence this module ignores [`rustyfi_syntax::Loc`]'s `line`/`col` entirely
//! and re-derives both coordinates from `Loc::byte`, which is a plain UTF-8
//! offset the lexer maintains with `char::len_utf8` and is therefore exact.

/// Line-start byte offsets for one source text, for repeated byte → position
/// queries.
///
/// Built once per analysis pass: the conversion is O(log lines) for the line
/// and O(line length) for the character, rather than O(file) per diagnostic.
pub struct LineIndex<'s> {
    src: &'s str,
    /// Byte offset of the first byte of each line. Always starts with `0`, so
    /// it is never empty and `line_starts[0] == 0` even for an empty file.
    line_starts: Vec<usize>,
}

/// A zero-based LSP position: `line` in lines, `character` in UTF-16 code
/// units from the start of that line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// Zero-based line number.
    pub line: u32,
    /// Zero-based offset within the line, in UTF-16 code units.
    pub character: u32,
}

impl<'s> LineIndex<'s> {
    /// Index `src`.
    ///
    /// Line terminators recognized are `\n` and a lone `\r`, matching the
    /// lexer's own `bump` (`\r\n` counts once, as the `\n`). Getting this to
    /// agree with the lexer matters only for CRLF files, where disagreeing
    /// would offset every diagnostic after the first line by a whole line.
    pub fn new(src: &'s str) -> Self {
        let mut line_starts = vec![0usize];
        let bytes = src.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => line_starts.push(i + 1),
                b'\r' => {
                    // `\r\n` is one terminator; the `\n` arm above will not
                    // fire for it because we skip past both here.
                    if bytes.get(i + 1) == Some(&b'\n') {
                        line_starts.push(i + 2);
                        i += 1;
                    } else {
                        line_starts.push(i + 1);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        LineIndex { src, line_starts }
    }

    /// Total number of lines (always ≥ 1).
    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }

    /// Convert a UTF-8 byte offset into a zero-based UTF-16 [`Position`].
    ///
    /// Out-of-range offsets clamp to the end of the file, and an offset that
    /// lands inside a multi-byte character rounds **down** to that
    /// character's start. Both are deliberate: a diagnostic with a slightly
    /// generous range is useful, whereas a panic in a language server takes
    /// the editor's whole diagnostics pane down with it.
    pub fn position(&self, byte: usize) -> Position {
        let byte = byte.min(self.src.len());
        // `partition_point` gives the count of line starts at or before
        // `byte`; minus one is that line's index. Never underflows —
        // `line_starts[0] == 0 <= byte`.
        let line = self.line_starts.partition_point(|&s| s <= byte) - 1;
        let line_start = self.line_starts[line];
        let character = utf16_len(floor_char_boundary(self.src, line_start, byte));
        Position {
            line: line as u32,
            character,
        }
    }

    /// The position one past the last character of the file — the end of a
    /// range for an error the parser could only report as "ran out of input".
    pub fn eof(&self) -> Position {
        self.position(self.src.len())
    }
}

/// `src[start..end]`, with `end` rounded down to the nearest character
/// boundary at or after `start`. Never panics for `start <= end <=
/// src.len()` with `start` on a boundary.
fn floor_char_boundary(src: &str, start: usize, end: usize) -> &str {
    let mut end = end.max(start);
    while end > start && !src.is_char_boundary(end) {
        end -= 1;
    }
    &src[start..end]
}

/// Length of `s` in UTF-16 code units. Astral-plane characters (emoji, rarer
/// CJK extensions) count as 2.
fn utf16_len(s: &str) -> u32 {
    s.chars().map(|c| c.len_utf16() as u32).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_positions_are_the_obvious_ones() {
        let src = "let x = 1\nlet y = 2\n";
        let idx = LineIndex::new(src);
        assert_eq!(idx.position(0), Position { line: 0, character: 0 });
        assert_eq!(idx.position(4), Position { line: 0, character: 4 });
        // The `\n` itself is still on line 0, one past `1`.
        assert_eq!(idx.position(9), Position { line: 0, character: 9 });
        assert_eq!(idx.position(10), Position { line: 1, character: 0 });
        assert_eq!(idx.position(14), Position { line: 1, character: 4 });
    }

    #[test]
    fn japanese_columns_are_utf16_units_not_bytes() {
        // `こんにちは` is 5 chars / 15 bytes / 5 UTF-16 units.
        let src = "こんにちは x";
        let idx = LineIndex::new(src);
        assert_eq!(src.find('x'), Some(16));
        assert_eq!(
            idx.position(16),
            Position {
                line: 0,
                character: 6
            },
            "5 kana + 1 space = 6 UTF-16 units, NOT 16 bytes"
        );
    }

    #[test]
    fn astral_characters_count_as_two_units() {
        // `🎉` is 1 char, 4 bytes, 2 UTF-16 units — the case a `char`-based
        // column (which `rustyfi_syntax::Loc::col` is) gets wrong.
        let src = "🎉x";
        let idx = LineIndex::new(src);
        assert_eq!(
            idx.position(4),
            Position {
                line: 0,
                character: 2
            }
        );
    }

    #[test]
    fn crlf_terminators_count_once() {
        let src = "a\r\nb\r\nc";
        let idx = LineIndex::new(src);
        assert_eq!(idx.line_count(), 3);
        assert_eq!(idx.position(3), Position { line: 1, character: 0 });
        assert_eq!(idx.position(6), Position { line: 2, character: 0 });
    }

    #[test]
    fn lone_cr_terminates_a_line_like_the_lexer_says() {
        let src = "a\rb";
        let idx = LineIndex::new(src);
        assert_eq!(idx.line_count(), 2);
        assert_eq!(idx.position(2), Position { line: 1, character: 0 });
    }

    #[test]
    fn out_of_range_and_mid_character_offsets_clamp_rather_than_panic() {
        let src = "あい";
        let idx = LineIndex::new(src);
        // Past the end.
        assert_eq!(idx.position(999), Position { line: 0, character: 2 });
        // Inside `い`'s three bytes: rounds down to its start.
        assert_eq!(idx.position(4), Position { line: 0, character: 1 });
        assert_eq!(idx.position(5), Position { line: 0, character: 1 });
    }

    #[test]
    fn empty_file_has_one_line_and_a_zero_eof() {
        let idx = LineIndex::new("");
        assert_eq!(idx.line_count(), 1);
        assert_eq!(idx.eof(), Position { line: 0, character: 0 });
    }

    #[test]
    fn trailing_newline_opens_a_final_empty_line() {
        let idx = LineIndex::new("a\n");
        assert_eq!(idx.line_count(), 2);
        assert_eq!(idx.eof(), Position { line: 1, character: 0 });
    }
}

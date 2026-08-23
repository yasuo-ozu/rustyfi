//! Byte offset → LSP `Position` (zero-based line, **UTF-16 code unit**
//! character).
//!
//! Three column conventions are in play, and they agree only on ASCII — which
//! is why a byte- or char-based implementation passes every ASCII test and
//! then misplaces every squiggle in the Japanese documents this port exists to
//! typeset:
//!
//! | who | line | character |
//! |---|---|---|
//! | [`rustyfi_syntax::Loc`] | 1-based | 0-based **`char`s** (`Loc::col`) |
//! | LSP `Position` | 0-based | 0-based **UTF-16 code units** |
//! | a naive implementation | — | bytes |
//!
//! `あ` is 3 bytes, 1 `char` and 1 UTF-16 unit; `🎉` is 4 bytes, 1 `char` and
//! **2** UTF-16 units. So `Loc::col` is not usable either. This module ignores
//! `Loc`'s `line`/`col` entirely and re-derives both coordinates from
//! `Loc::byte`, a plain UTF-8 offset that is exact.

/// Line-start byte offsets for one source text, for repeated byte → position
/// queries.
///
/// Built once per analysis pass: a conversion is then O(log lines) for the
/// line and O(line length) for the character, rather than O(file).
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
    /// Line terminators are `\n` and a lone `\r`, matching the lexer's own
    /// `bump` (`\r\n` counts once). Disagreeing with the lexer would offset
    /// every diagnostic in a CRLF file by a whole line.
    pub fn new(src: &'s str) -> Self {
        let mut line_starts = vec![0usize];
        let bytes = src.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => line_starts.push(i + 1),
                b'\r' => {
                    // `\r\n` is one terminator; skipping past both here keeps
                    // the `\n` arm above from firing for it.
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

    /// The text this index was built over.
    ///
    /// Exists so a caller needing both the index and the source cannot be
    /// handed two that disagree, which would silently produce nonsense.
    pub fn source(&self) -> &'s str {
        self.src
    }

    /// Convert a UTF-8 byte offset into a zero-based UTF-16 [`Position`].
    ///
    /// Out-of-range offsets clamp to the end of the file, and an offset inside
    /// a multi-byte character rounds **down** to that character's start. A
    /// slightly generous range is useful; a panic in a language server takes
    /// the editor's whole diagnostics pane down with it.
    pub fn position(&self, byte: usize) -> Position {
        let byte = byte.min(self.src.len());
        // Never underflows: `line_starts[0] == 0 <= byte`.
        let line = self.line_starts.partition_point(|&s| s <= byte) - 1;
        let line_start = self.line_starts[line];
        // `line_start` is a boundary by construction and `floor_boundary`
        // never goes below its argument's floor, so this slice is always safe.
        let character = utf16_len(&self.src[line_start..floor_boundary(self.src, byte)]);
        Position {
            line: line as u32,
            character,
        }
    }
}

/// `byte`, rounded **down** to the nearest UTF-8 character boundary.
///
/// The single place this crate copes with an offset pointing into the middle
/// of a character; rounding down rather than panicking keeps a bad offset from
/// costing the editor its whole diagnostics pane.
pub(crate) fn floor_boundary(src: &str, mut byte: usize) -> usize {
    byte = byte.min(src.len());
    while byte > 0 && !src.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
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
        // 1 char, 4 bytes, 2 UTF-16 units — the case `Loc::col` gets wrong.
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
        assert_eq!(idx.position(3), Position { line: 1, character: 0 });
        assert_eq!(idx.position(6), Position { line: 2, character: 0 });
    }

    #[test]
    fn lone_cr_terminates_a_line_like_the_lexer_says() {
        let src = "a\rb";
        let idx = LineIndex::new(src);
        assert_eq!(idx.position(2), Position { line: 1, character: 0 });
    }

    #[test]
    fn out_of_range_and_mid_character_offsets_clamp_rather_than_panic() {
        let src = "あい";
        let idx = LineIndex::new(src);
        assert_eq!(idx.position(999), Position { line: 0, character: 2 });
        // Inside `い`'s three bytes: rounds down to its start.
        assert_eq!(idx.position(4), Position { line: 0, character: 1 });
        assert_eq!(idx.position(5), Position { line: 0, character: 1 });
    }

    #[test]
    fn an_empty_file_maps_offset_zero_to_the_origin() {
        let idx = LineIndex::new("");
        assert_eq!(idx.position(0), Position { line: 0, character: 0 });
    }

    #[test]
    fn a_trailing_newline_opens_a_final_empty_line() {
        let idx = LineIndex::new("a\n");
        assert_eq!(idx.position(2), Position { line: 1, character: 0 });
    }
}

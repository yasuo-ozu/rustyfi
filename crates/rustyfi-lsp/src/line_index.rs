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

    /// The text this index was built over.
    ///
    /// Exists so a caller that needs both the index and the source cannot be
    /// handed two that disagree — see [`crate::analyze`]'s `span_to_range`,
    /// which would otherwise take them as two parameters and silently produce
    /// nonsense if they were ever mismatched.
    pub fn source(&self) -> &'s str {
        self.src
    }

    /// Convert a zero-based UTF-16 [`Position`] into a UTF-8 byte offset —
    /// the inverse of [`Self::position`], and the direction every *request*
    /// needs: an editor asks about a cursor, and everything below the
    /// protocol works in bytes.
    ///
    /// Total, by the same reasoning [`Self::position`] gives for clamping: a
    /// client that is one keystroke ahead of the server sends a position past
    /// the end of the text the server holds, and the useful answer is "the end
    /// of the file", not a panic that takes the session down. So a line past
    /// the last one clamps to the end of the text, a character past the end of
    /// its line clamps to that line's terminator, and a character landing on
    /// the second half of a surrogate pair rounds **down** to the start of
    /// that character (the mirror of [`floor_boundary`]'s rule).
    ///
    /// Round-trips with [`Self::position`] for every position that names a
    /// real character boundary, which is the property the tests pin.
    pub fn offset(&self, pos: Position) -> usize {
        let Some(&line_start) = self.line_starts.get(pos.line as usize) else {
            return self.src.len();
        };
        // Stop at this line's terminator rather than running into the next
        // line: an over-long `character` is a clamp, not a reason to return an
        // offset the client would see as a different line.
        let line_end = self
            .line_starts
            .get(pos.line as usize + 1)
            .map(|&next| line_terminator_start(self.src, line_start, next))
            .unwrap_or(self.src.len());

        let mut units = 0u32;
        for (rel, c) in self.src[line_start..line_end].char_indices() {
            // `>` rather than `>=`, in one test: it fires both when the target
            // IS this character's start (`units == pos.character`) and when the
            // target is the second unit of this character's surrogate pair
            // (`units < pos.character < units + 2`). The latter is the
            // round-down case.
            if units + c.len_utf16() as u32 > pos.character {
                return line_start + rel;
            }
            units += c.len_utf16() as u32;
        }
        line_end
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
        // `line_start` is a boundary by construction, and `floor_boundary`
        // never goes below its argument's own floor, so this slice is safe
        // for any `byte` at all.
        let character = utf16_len(&self.src[line_start..floor_boundary(self.src, byte)]);
        Position {
            line: line as u32,
            character,
        }
    }
}

/// `byte`, rounded **down** to the nearest UTF-8 character boundary.
///
/// The single place this crate copes with an offset that points into the
/// middle of a character. Rounding down rather than panicking is deliberate:
/// a diagnostic range that is a character too generous is useful, whereas a
/// panic in a language server takes the editor's whole diagnostics pane down
/// with it.
pub(crate) fn floor_boundary(src: &str, mut byte: usize) -> usize {
    byte = byte.min(src.len());
    while byte > 0 && !src.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

/// Where the terminator of the line starting at `start` and ending (exclusive
/// of the next line's first byte) at `next` begins.
///
/// `next` is the *following* line's start, so it sits one past a `\n`, a lone
/// `\r`, or a `\r\n` pair; this backs over whichever of the three is there.
/// Written against the bytes rather than by remembering the terminator at
/// index-building time so that [`LineIndex::new`] stays a plain scan.
fn line_terminator_start(src: &str, start: usize, next: usize) -> usize {
    let bytes = src.as_bytes();
    let mut end = next;
    if end > start && bytes.get(end - 1) == Some(&b'\n') {
        end -= 1;
    }
    if end > start && bytes.get(end - 1) == Some(&b'\r') {
        end -= 1;
    }
    end
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
        // Past the end.
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

    // ---- the request direction: UTF-16 position -> byte offset -------------

    /// The property that matters: for every character boundary in the text,
    /// `offset(position(b)) == b`. Swept over a buffer mixing ASCII, kana and
    /// an astral character, on several lines, so a units-vs-bytes slip cannot
    /// pass.
    #[test]
    fn offset_inverts_position_at_every_character_boundary() {
        let src = "let あ = 1\n  \\emph{🎉 ok}\nlet 漢字 = 2\n";
        let idx = LineIndex::new(src);
        for (b, _) in src.char_indices().chain(std::iter::once((src.len(), ' '))) {
            assert_eq!(idx.offset(idx.position(b)), b, "byte {b} of {src:?}");
        }
    }

    #[test]
    fn offset_counts_utf16_units_not_bytes_or_chars() {
        // `こんにちは x`: the `x` is 6 UTF-16 units in and 16 bytes in.
        let idx = LineIndex::new("こんにちは x");
        assert_eq!(
            idx.offset(Position {
                line: 0,
                character: 6
            }),
            16
        );
        // `🎉` is two units wide; the `x` after it is at unit 2, byte 4.
        let idx = LineIndex::new("🎉x");
        assert_eq!(
            idx.offset(Position {
                line: 0,
                character: 2
            }),
            4
        );
    }

    /// A position pointing at the *second* unit of a surrogate pair is not a
    /// character boundary. Rounding down matches `floor_boundary`'s rule for
    /// the other direction, and is what keeps a cursor inside an emoji from
    /// slicing a `str`.
    #[test]
    fn a_position_inside_a_surrogate_pair_rounds_down() {
        let idx = LineIndex::new("🎉x");
        assert_eq!(
            idx.offset(Position {
                line: 0,
                character: 1
            }),
            0
        );
    }

    #[test]
    fn out_of_range_positions_clamp_rather_than_panic() {
        let src = "ab\ncd\n";
        let idx = LineIndex::new(src);
        // Past the last line entirely.
        assert_eq!(
            idx.offset(Position {
                line: 99,
                character: 0
            }),
            src.len()
        );
        // Past the end of a line: clamps to that line's terminator, NOT into
        // the next line.
        assert_eq!(
            idx.offset(Position {
                line: 0,
                character: 99
            }),
            2
        );
    }

    #[test]
    fn offset_stops_before_a_crlf_terminator() {
        let src = "ab\r\ncd";
        let idx = LineIndex::new(src);
        assert_eq!(
            idx.offset(Position {
                line: 0,
                character: 5
            }),
            2
        );
        assert_eq!(
            idx.offset(Position {
                line: 1,
                character: 0
            }),
            4
        );
    }
}

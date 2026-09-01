//! What sits in the gap between two tokens.
//!
//! A gap is everything strictly between one token's span end and the next
//! token's span start. Because the lexer's spans tile the source
//! (`lexer.rs:281-286`), a gap can hold only what the lexer *skipped*: spaces,
//! tabs, line breaks, and `%` comments run to end of line (`lexer.rs:308-333`).
//!
//! [`classify`] is the decline-rather-than-guess backstop: anything else in a
//! gap means this code has misread the stream, and the honest answer is to stop.
//! `format.rs:336-343` carries the same reflex, and `area.rs:47-65` records why
//! it matters — the `<[`/`]>` bug walked the area replay out of a math area
//! early, and it was *latent rather than corrupting* only because the gap
//! arithmetic independently held. A formatter that moves bytes loses that second
//! line of defence, so the first one has to be real.
//!
//! # The two traps a gap-only reading does not see
//!
//! Recorded here because both are invisible until they corrupt something, and
//! neither is slice 0's problem only because slice 0 changes nothing.
//!
//! 1. **A comment inside inline text is not in a gap.** In horizontal mode, when
//!    whitespace precedes a `%`, the emitted `Space`/`Break` token's span
//!    swallows the whitespace run, the comment, *and* the whitespace after it
//!    (`lexer.rs:1149-1155`, which calls `skip_spaces()` after `bump_n(ws)`). So
//!    such a comment lives inside a token span, and a formatter must never
//!    rewrite an inline `Space`/`Break` — only copy it. This also means an inline
//!    comment can **delete a space**: `{Alpha% c⏎beta}` sets `Alphabeta`.
//! 2. **A header token owns its own line terminator** (`lexer.rs:915-933`), so
//!    the gap after a header begins at column 0 and its first terminator ends a
//!    *blank* line rather than the header's. `format.rs:225-243` and `:536-556`
//!    both carry workarounds for this, the second recording that judging by
//!    emitted text alone appended a second newline and made `format`
//!    non-idempotent on format-on-save.

/// One piece of a gap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Piece<'s> {
    /// Horizontal whitespace: spaces and tabs.
    Space(&'s str),
    /// A `%` comment, *not* including its terminator.
    Comment(&'s str),
    /// One line terminator: `\n`, `\r\n` or a bare `\r`.
    Newline(&'s str),
}

/// Split a gap into its pieces, or `None` if it holds anything the lexer would
/// not have skipped.
pub(crate) fn classify(gap: &str) -> Option<Vec<Piece<'_>>> {
    let mut out = Vec::new();
    let bytes = gap.as_bytes();
    let mut i = 0usize;
    while i < gap.len() {
        match bytes[i] {
            b' ' | b'\t' => {
                let start = i;
                while i < gap.len() && matches!(bytes[i], b' ' | b'\t') {
                    i += 1;
                }
                out.push(Piece::Space(&gap[start..i]));
            }
            // A bare `\r` is a terminator too. Treating it as one is what keeps
            // a CRLF file from being split across a token boundary — the root
            // cause of four separate bugs in the lex-based formatter, all of
            // them from `lex_header` consuming one break character and leaving
            // the other behind.
            b'\r' => {
                let start = i;
                i += 1;
                if i < gap.len() && bytes[i] == b'\n' {
                    i += 1;
                }
                out.push(Piece::Newline(&gap[start..i]));
            }
            b'\n' => {
                out.push(Piece::Newline(&gap[i..i + 1]));
                i += 1;
            }
            b'%' => {
                let start = i;
                while i < gap.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                    i += 1;
                }
                out.push(Piece::Comment(&gap[start..i]));
            }
            // Anything else cannot be in a gap. Decline.
            _ => return None,
        }
    }
    Some(out)
}

/// Reassemble the pieces' bytes, for the round-trip property the tests assert.
#[cfg(test)]
fn rejoin(pieces: &[Piece<'_>]) -> String {
    pieces
        .iter()
        .map(|p| match p {
            Piece::Space(s) | Piece::Comment(s) | Piece::Newline(s) => *s,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_round_trips_every_gap_it_accepts() {
        for gap in [
            "",
            " ",
            "   ",
            "\t",
            " \t ",
            "\n",
            "\r\n",
            "\r",
            "\n\n\n",
            "\r\n\r\n",
            "  % a comment\n  ",
            "% one\n% two\n",
            " \t% trailing\r\n\t",
            "\n% c",
        ] {
            let pieces = classify(gap).unwrap_or_else(|| panic!("declined {gap:?}"));
            assert_eq!(rejoin(&pieces), gap, "round-trip failed for {gap:?}");
        }
    }

    #[test]
    fn a_gap_holding_anything_else_is_declined() {
        // Each of these would mean the caller handed over a range that is not a
        // gap — a token's own bytes, or a misread span.
        for bad in ["x", " let ", "%c\nx", "\u{3000}"] {
            assert!(classify(bad).is_none(), "{bad:?} should be declined");
        }
    }

    #[test]
    fn crlf_is_one_terminator_and_a_bare_cr_is_one_too() {
        assert_eq!(classify("\r\n").unwrap(), vec![Piece::Newline("\r\n")]);
        assert_eq!(classify("\r").unwrap(), vec![Piece::Newline("\r")]);
        // The failure this guards: `\r` and a following `\n` from *separate*
        // pieces fusing into one terminator, or a CRLF being split so that the
        // `\n` reads as a second blank line.
        assert_eq!(
            classify("\r\n\r\n").unwrap(),
            vec![Piece::Newline("\r\n"), Piece::Newline("\r\n")]
        );
    }

    #[test]
    fn a_comment_does_not_swallow_its_terminator() {
        // It must not: the terminator is what ends the line the comment is on,
        // and a caller counting blank lines needs to see it.
        assert_eq!(
            classify("% hi\n").unwrap(),
            vec![Piece::Comment("% hi"), Piece::Newline("\n")]
        );
    }
}

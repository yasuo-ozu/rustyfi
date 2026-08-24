//! `--unicode-math`: a math run as its characters, in reading order.
//!
//! ## Where this sits among the three
//!
//! Math is laid out during compilation, so no backend sees a `\frac` node —
//! [`crate::mathrec`]'s module comment has the detail, and it is the reason
//! all three of this crate's math renderings are RECOVERIES rather than
//! serializations. The three, and what each is for:
//!
//! 1. **An outlined inline `<svg>`** ([`crate::mathsvg`]) — the DEFAULT.
//!    Faithful and self-contained. Markdown permits raw HTML, so this is
//!    possible, and it draws exactly what the PDF draws.
//! 2. **This module.** The characters, in reading order.
//! 3. **LaTeX in `$…$`** ([`crate::latex`]), for a KaTeX/MathJax reader.
//!
//! **What keeps this one reachable** is the case the default cannot serve. A
//! renderer that sanitizes HTML — GitHub's comment fields, most static-site
//! pipelines — drops the `<svg>` and leaves nothing whatever in its place; a
//! reader looking at the raw file sees a wall of path data; and neither
//! `grep` nor a terminal pager can read a `<path d="…">`. This is the only
//! form that is TEXT, and for a `.md` file — which is mostly read as text —
//! that is worth having a flag for. It is also the only one with no
//! dependencies at all: option 3 needs the reader to be running a math
//! typesetter, and option 1 needs them to be rendering HTML.
//!
//! A fourth option, a bare `[math]` placeholder, was considered and rejected:
//! `latexcmds`' manual is math in nearly every paragraph, and a document whose
//! formulae are all spelled `[math]` has lost more than the ones it keeps.
//!
//! ## What "reading order" means here, precisely
//!
//! The glyphs carry real Unicode text — that is what makes the PDF's
//! `ToUnicode` table work — so `∑`, `α` and `≤` come out as themselves.
//! [`crate::mathrec`] sorts them and recovers the structure; this module
//! decides how to SAY each recovered piece in a vocabulary that has no
//! two-dimensional notation:
//!
//! - **Scripts** are written with Unicode's own superscript/subscript
//!   character where one exists (`x²`, `a₁`) — which needs no escaping,
//!   renders in a terminal, and survives copy-paste. Where Unicode has no
//!   form (`x^{q}`), it falls back to `^q` / `_q`.
//! - **Fractions** are written `(a+b)/(c+d)`, parenthesized unless the half is
//!   a single token.
//!
//! ## What is still lost, and is stated as lost in the README
//!
//! Radicals, matrices, aligned environments and nested fractions — all of it
//! for reasons that belong to the recovery rather than to this writer, and all
//! of it listed in [`crate::mathrec`]'s doc comment. Unicode's own gaps add
//! one more: it has no superscript `q`, and no script forms at all for the
//! Mathematical Alphanumeric Symbols a math letter actually is.

use rustyfi_backend::{GraphicsElem, MathGlyph};

use crate::mathrec::{self, Atom};

/// One `PureHorzBox::Math` as reading-order text. See this module's doc
/// comment for what that means and what it costs. Empty when the box draws
/// nothing this layer can name.
pub(super) fn math_text(glyphs: &[MathGlyph], rules: &[GraphicsElem]) -> String {
    write_atoms(&mathrec::recover(glyphs, rules), true)
}

/// Fold recovered atoms into text.
///
/// `wrap_fracs` is false inside a fraction's own half, where a nested bar has
/// already been dissolved away by the recovery and there is nothing left to
/// parenthesize — see [`crate::mathrec`]'s "one level deep".
fn write_atoms(atoms: &[Atom<'_>], wrap_fracs: bool) -> String {
    let mut out = String::new();
    for atom in atoms {
        match atom {
            // The guard is on the OUTPUT, not on the atom's position: a run
            // whose leading glyph records carry no characters would otherwise
            // let a space through as the first thing written.
            Atom::Space => {
                if !out.is_empty() {
                    out.push(' ');
                }
            }
            Atom::Glyph { g, script } => match script {
                Some(s) => out.push_str(&script_text(&g.text, s.up)),
                None => out.push_str(&g.text),
            },
            Atom::Frac { above, below } => {
                let (a, b) = (write_atoms(above, false), write_atoms(below, false));
                if wrap_fracs {
                    out.push_str(&wrap(&a));
                    out.push('/');
                    out.push_str(&wrap(&b));
                } else {
                    out.push_str(&a);
                    out.push('/');
                    out.push_str(&b);
                }
            }
        }
    }
    out
}

/// Parenthesize a fraction half unless it is a single token, so `1/2` stays
/// `1/2` while `a+b` over `c` becomes `(a+b)/c` rather than the ambiguous
/// `a+b/c`.
fn wrap(part: &str) -> String {
    let atomic = part.chars().count() <= 1
        || part
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | ',' | '\'' | '′'));
    if atomic {
        part.to_string()
    } else {
        format!("({part})")
    }
}

/// `text` as Unicode superscript (`up`) or subscript characters, falling back
/// to a `^`/`_` prefix for anything Unicode has no form for.
///
/// The Unicode form is preferred wherever it exists because it needs no
/// escaping, renders in a terminal and in a plain-text search, and survives
/// being copied out of the rendered page — none of which is true of `x^2`.
///
/// A math letter is a Mathematical Alphanumeric Symbol, not an ASCII one:
/// `${\sum_a^b}`'s limits are `𝑎` U+1D44E and `𝑏` U+1D44F, for which Unicode
/// has no script forms at all. `math_alphanumeric_base` — the same inverse
/// mapping the math layout uses as a last resort for an uncovered codepoint —
/// takes them back to `a`/`b`, which DO have script forms, so `∑ₐᵇ` comes out
/// instead of `∑_𝑎^𝑏`. The style (italic, bold, fraktur) is lost in the
/// process, which is the correct trade: a script's letter matters, its
/// weight does not. (`--katex` keeps the style, because LaTeX has `\mathbb`
/// and this vocabulary does not — see [`crate::latex::style_wrapper`].)
fn script_text(text: &str, up: bool) -> String {
    let mapped: Option<String> = text
        .chars()
        .map(|c| {
            let plain = rustyfi_backend::math_alphanumeric_base(c).unwrap_or(c);
            if up {
                superscript(plain)
            } else {
                subscript(plain)
            }
        })
        .collect();
    match mapped {
        Some(s) => s,
        // No Unicode form for at least one character, so the whole token
        // takes the notational fallback — mixing `x²ᐟ` and `x^{2/n}` inside
        // one expression would be worse than either.
        None => format!("{}{}", if up { '^' } else { '_' }, text),
    }
}

fn superscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '\u{2070}',
        '1' => '\u{00B9}',
        '2' => '\u{00B2}',
        '3' => '\u{00B3}',
        '4'..='9' => char::from_u32(0x2074 + (c as u32 - '4' as u32))?,
        '+' => '\u{207A}',
        '-' | '\u{2212}' => '\u{207B}',
        '=' => '\u{207C}',
        '(' => '\u{207D}',
        ')' => '\u{207E}',
        'n' => '\u{207F}',
        'i' => '\u{2071}',
        'a' => '\u{1D43}',
        'b' => '\u{1D47}',
        'c' => '\u{1D9C}',
        'd' => '\u{1D48}',
        'e' => '\u{1D49}',
        'f' => '\u{1DA0}',
        'g' => '\u{1D4D}',
        'h' => '\u{02B0}',
        'j' => '\u{02B2}',
        'k' => '\u{1D4F}',
        'l' => '\u{02E1}',
        'm' => '\u{1D50}',
        'o' => '\u{1D52}',
        'p' => '\u{1D56}',
        'r' => '\u{02B3}',
        's' => '\u{02E2}',
        't' => '\u{1D57}',
        'u' => '\u{1D58}',
        'v' => '\u{1D5B}',
        'w' => '\u{02B7}',
        'x' => '\u{02E3}',
        'y' => '\u{02B8}',
        'z' => '\u{1DBB}',
        _ => return None,
    })
}

fn subscript(c: char) -> Option<char> {
    Some(match c {
        '0'..='9' => char::from_u32(0x2080 + (c as u32 - '0' as u32))?,
        '+' => '\u{208A}',
        '-' | '\u{2212}' => '\u{208B}',
        '=' => '\u{208C}',
        '(' => '\u{208D}',
        ')' => '\u{208E}',
        'a' => '\u{2090}',
        'e' => '\u{2091}',
        'h' => '\u{2095}',
        'i' => '\u{1D62}',
        'j' => '\u{2C7C}',
        'k' => '\u{2096}',
        'l' => '\u{2097}',
        'm' => '\u{2098}',
        'n' => '\u{2099}',
        'o' => '\u{2092}',
        'p' => '\u{209A}',
        'r' => '\u{1D63}',
        's' => '\u{209B}',
        't' => '\u{209C}',
        'u' => '\u{1D64}',
        'v' => '\u{1D65}',
        'x' => '\u{2093}',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mathrec::tests::glyph;

    #[test]
    fn glyphs_come_out_in_reading_order_whatever_order_they_are_stored_in() {
        let gs = vec![glyph("c", 10.0, 0.0, 10.0), glyph("a", 0.0, 0.0, 10.0)];
        assert_eq!(math_text(&gs, &[]), "a c");
    }

    /// A superscript uses Unicode's own character, so nothing has to be
    /// escaped and the result is still searchable text.
    #[test]
    fn a_script_becomes_a_unicode_script_character() {
        let gs = vec![glyph("x", 0.0, 0.0, 10.0), glyph("2", 5.0, 5.0, 7.0)];
        assert_eq!(math_text(&gs, &[]), "x²");
        let gs = vec![glyph("a", 0.0, 0.0, 10.0), glyph("1", 5.0, -3.0, 7.0)];
        assert_eq!(math_text(&gs, &[]), "a₁");
    }

    /// Unicode has no superscript `q`, so the whole token takes the
    /// notational fallback rather than coming out silently unmarked.
    #[test]
    fn a_script_with_no_unicode_form_falls_back_to_the_caret() {
        let gs = vec![glyph("x", 0.0, 0.0, 10.0), glyph("q", 5.0, 5.0, 7.0)];
        assert_eq!(math_text(&gs, &[]), "x^q");
    }

    /// A base-size glyph nudged vertically is not a script — otherwise every
    /// manual `rising` in a formula would grow a caret.
    #[test]
    fn a_full_size_raised_glyph_is_not_a_script() {
        let gs = vec![glyph("x", 0.0, 0.0, 10.0), glyph("y", 5.0, 5.0, 10.0)];
        assert_eq!(math_text(&gs, &[]), "xy");
    }

    /// A fraction is split at its bar and each half parenthesized unless it is
    /// a single token — the one construct this vocabulary can still say.
    #[test]
    fn a_fraction_is_split_at_its_bar() {
        use rustyfi_backend::{Closing, Color, Length, Path, PathSeg, Subpath};
        let bar = GraphicsElem::Fill(
            Color::Gray(0.0),
            Path {
                subpaths: vec![Subpath {
                    start: (Length(0.0), Length(4.0)),
                    segs: vec![
                        PathSeg::Line((Length(20.0), Length(4.0))),
                        PathSeg::Line((Length(20.0), Length(4.5))),
                        PathSeg::Line((Length(0.0), Length(4.5))),
                    ],
                    closing: Closing::Line,
                }],
            },
        );
        let gs = vec![
            glyph("a", 2.0, 8.0, 10.0),
            glyph("+", 7.0, 8.0, 10.0),
            glyph("b", 12.0, 8.0, 10.0),
            glyph("c", 8.0, -4.0, 10.0),
        ];
        assert_eq!(math_text(&gs, &[bar]), "(a+b)/c");
    }
}

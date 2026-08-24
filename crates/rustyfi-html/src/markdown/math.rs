//! Math, which is the one thing in the box stream that is genuinely gone.
//!
//! ## What was lost, and where
//!
//! `${\frac{a}{b}}` is parsed, elaborated, evaluated and LAID OUT during
//! compilation: `read_math`/`layout_math_value` turn the expression tree into
//! a `PureHorzBox::Math` carrying a flat `Vec<MathGlyph>`, each glyph a
//! character with an `(dx, dy)` offset from the run's baseline, plus a
//! `Vec<GraphicsElem>` of the FILLED PATHS a font cannot draw — the fraction
//! bar and the radical sign. By the time any backend sees it there is no
//! `\frac` node, no numerator, no argument list. This is the same reason the
//! HTML backend cannot emit MathML and draws an `<svg>` instead.
//!
//! So the choice is not "how do I serialize the math" but "what do I write
//! where the math was". Three options were on the table:
//!
//! 1. **An inline `<svg>`**, as the HTML backend does. Markdown permits raw
//!    HTML, so this is possible, and it would be pixel-faithful. It is
//!    rejected because it stops being Markdown: GitHub and every other
//!    sanitizing renderer strip `<svg>` outright, leaving nothing at all; a
//!    reader looking at the raw file sees a wall of `<text x= y=>`; and it is
//!    unsearchable, uncopyable and unreadable in a terminal, which is where
//!    most `.md` files are actually read.
//! 2. **A placeholder** (`[math]`). Honest, and useless: `latexcmds`' manual
//!    is math in nearly every paragraph, and a document whose formulae are
//!    all spelled `[math]` has lost more than the ones it keeps.
//! 3. **The characters, in reading order.** What this module does.
//!
//! ## What "reading order" means here, precisely
//!
//! The glyphs carry real Unicode text — that is what makes the PDF's
//! `ToUnicode` table work — so `∑`, `α` and `≤` come out as themselves.
//! [`math_text`] sorts them by `dx` and writes them, and then recovers the
//! two pieces of two-dimensional structure that the flat order would
//! otherwise mangle beyond reading:
//!
//! - **Scripts.** A glyph set smaller than the run's base size and offset
//!   vertically is a superscript or a subscript, and is written with
//!   Unicode's own superscript/subscript character where one exists (`x²`,
//!   `a₁`) — which needs no escaping, renders in a terminal, and survives
//!   copy-paste. Where Unicode has no form (`x^{q}`), it falls back to `^q`
//!   / `_q`.
//! - **Fractions.** A numerator and a denominator have overlapping `dx`, so
//!   plain `dx` order interleaves them: `(a+b)/(c+d)` would come out as
//!   `ac++bd`. The fraction BAR survives as a wide, flat `Fill` in `rules`,
//!   and [`fraction_bars`] uses each bar's own extent to split the glyphs it
//!   spans into the part above it and the part below, written `(a+b)/(c+d)`.
//!
//! ## What is still lost, and is stated as lost in the README
//!
//! Radicals (the sign is a path, not a glyph — `√` is not written), matrices
//! and aligned environments (their row/column arrangement flattens), and
//! anything whose meaning is carried by position rather than by its
//! characters. Fraction recovery is one level deep: a fraction inside a
//! fraction contributes its glyphs to the outer one. None of this can be
//! recovered by trying harder at this layer — it would have to be captured
//! before layout, the way `ListMark`/`InlineMark` capture list and emphasis
//! structure.

use rustyfi_backend::{graphics_bbox, GraphicsElem, Length, MathGlyph};

/// A `rules` fill this flat (its width at least this many times its height)
/// is a horizontal bar rather than a drawn shape.
const BAR_ASPECT: f64 = 3.0;

/// …and no taller than this many points. A fraction bar is a hairline; a
/// filled box that happens to be wide is not one.
const BAR_MAX_HEIGHT_PT: f64 = 2.5;

/// A gap between two glyphs wider than this fraction of the base font size
/// becomes a space, reproducing the inter-atom spacing the math layout
/// inserted around a relation or a binary operator.
const SPACE_GAP_RATIO: f64 = 0.22;

/// A glyph offset vertically by more than this fraction of the base size,
/// AND set smaller than the base, is a script rather than part of the base
/// line.
const SCRIPT_SHIFT_RATIO: f64 = 0.12;

/// A glyph set at this fraction of the base size or smaller is a candidate
/// script. Scripts are conventionally 0.7 of their base; a base-size glyph
/// nudged vertically (a `\mathstrut`, a manual `rising`) is not one.
const SCRIPT_SIZE_RATIO: f64 = 0.9;

/// One `PureHorzBox::Math` as reading-order text. See this module's doc
/// comment for what that means and what it costs. Empty when the box draws
/// nothing this layer can name.
pub(super) fn math_text(glyphs: &[MathGlyph], rules: &[GraphicsElem]) -> String {
    if glyphs.is_empty() {
        return String::new();
    }
    let base = base_size(glyphs);
    let bars = fraction_bars(rules);

    // Each glyph belongs either to a bar (as its numerator or denominator) or
    // to the top-level run. A bar claims a glyph when the glyph's centre lies
    // within the bar's horizontal extent; ties between two bars go to the
    // narrower one, which is the inner fraction of a nested pair.
    let mut loose: Vec<&MathGlyph> = Vec::new();
    let mut parts: Vec<(Vec<&MathGlyph>, Vec<&MathGlyph>)> =
        bars.iter().map(|_| (Vec::new(), Vec::new())).collect();
    for g in glyphs {
        match claim(g, &bars) {
            Some(i) => {
                let (above, below) = &mut parts[i];
                if g.dy.0 >= bars[i].y {
                    above.push(g);
                } else {
                    below.push(g);
                }
            }
            None => loose.push(g),
        }
    }

    // Written in one left-to-right pass over both populations at once: a
    // fraction occupies its bar's own x span, so it sorts among the loose
    // glyphs exactly where the reader's eye reaches it.
    enum Item<'a> {
        Glyph(&'a MathGlyph),
        Frac(&'a Vec<&'a MathGlyph>, &'a Vec<&'a MathGlyph>),
    }
    let mut items: Vec<(f64, f64, Item)> = Vec::new();
    for g in &loose {
        items.push((g.dx.0, g.dx.0 + g.width.0, Item::Glyph(g)));
    }
    for (i, bar) in bars.iter().enumerate() {
        // A bar that claimed nothing on one side is not a fraction after all
        // (a radical's overbar, an underline rule): its glyphs go back to the
        // flat run rather than becoming `(x)/()`.
        let (above, below) = &parts[i];
        if above.is_empty() || below.is_empty() {
            for g in above.iter().chain(below.iter()) {
                items.push((g.dx.0, g.dx.0 + g.width.0, Item::Glyph(g)));
            }
        } else {
            items.push((bar.x0, bar.x1, Item::Frac(above, below)));
        }
    }
    items.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut out = String::new();
    let mut prev_right: Option<f64> = None;
    for (left, right, item) in &items {
        if let Some(pr) = prev_right {
            if left - pr > base * SPACE_GAP_RATIO && !out.is_empty() {
                out.push(' ');
            }
        }
        match item {
            Item::Glyph(g) => out.push_str(&glyph_text(g, base)),
            Item::Frac(above, below) => {
                out.push_str(&wrap(&flat(above, base)));
                out.push('/');
                out.push_str(&wrap(&flat(below, base)));
            }
        }
        prev_right = Some(*right);
    }
    out
}

/// A fraction's own half, written flat — scripts and spacing still apply, but
/// a bar nested inside this one has already been dissolved into it (see the
/// module doc comment's "one level deep").
fn flat(glyphs: &[&MathGlyph], base: f64) -> String {
    let mut sorted: Vec<&&MathGlyph> = glyphs.iter().collect();
    sorted.sort_by(|a, b| a.dx.0.total_cmp(&b.dx.0));
    let mut out = String::new();
    let mut prev_right: Option<f64> = None;
    for g in sorted {
        if let Some(pr) = prev_right {
            if g.dx.0 - pr > base * SPACE_GAP_RATIO && !out.is_empty() {
                out.push(' ');
            }
        }
        out.push_str(&glyph_text(g, base));
        prev_right = Some(g.dx.0 + g.width.0);
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

/// One glyph's characters, as a script when it is one.
fn glyph_text(g: &MathGlyph, base: f64) -> String {
    let small = g.info.size.0 <= base * SCRIPT_SIZE_RATIO;
    let shift = g.dy.0;
    if small && shift > base * SCRIPT_SHIFT_RATIO {
        script(&g.text, true)
    } else if small && shift < -base * SCRIPT_SHIFT_RATIO {
        script(&g.text, false)
    } else {
        g.text.clone()
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
/// weight does not.
fn script(text: &str, up: bool) -> String {
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

/// One recovered fraction bar: its horizontal extent and the height it sits
/// at, in the math box's own baseline-relative, y-up coordinates.
struct Bar {
    x0: f64,
    x1: f64,
    y: f64,
}

/// Every wide, flat `Fill` in a math box's `rules` — the shape a fraction bar
/// has. `Stroke`s are excluded deliberately: `layout_math_atom` draws the bar
/// as a filled rectangle, and a stroked line in a math run is a user's own
/// `\overline`-style drawing, whose two sides are not a numerator and a
/// denominator.
fn fraction_bars(rules: &[GraphicsElem]) -> Vec<Bar> {
    let mut bars = Vec::new();
    collect_bars(rules, &mut bars);
    bars
}

fn collect_bars(rules: &[GraphicsElem], out: &mut Vec<Bar>) {
    for elem in rules {
        match elem {
            GraphicsElem::Fill(_, _) => {
                let Some(((Length(x0), Length(y0)), (Length(x1), Length(y1)))) =
                    graphics_bbox(elem)
                else {
                    continue;
                };
                let (w, h) = (x1 - x0, y1 - y0);
                if h <= BAR_MAX_HEIGHT_PT && w >= h * BAR_ASPECT && w > 0.0 {
                    out.push(Bar {
                        x0,
                        x1,
                        y: (y0 + y1) / 2.0,
                    });
                }
            }
            GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => collect_bars(inner, out),
            _ => {}
        }
    }
}

/// Which bar, if any, spans `g` — the narrowest one that does, so a fraction
/// nested inside another is claimed by its own bar rather than by the outer
/// one.
fn claim(g: &MathGlyph, bars: &[Bar]) -> Option<usize> {
    let centre = g.dx.0 + g.width.0 / 2.0;
    bars.iter()
        .enumerate()
        .filter(|(_, b)| centre >= b.x0 && centre <= b.x1)
        .min_by(|(_, a), (_, b)| (a.x1 - a.x0).total_cmp(&(b.x1 - b.x0)))
        .map(|(i, _)| i)
}

/// The run's base font size: the largest any glyph is set in, which is the
/// size scripts are measured against. Never zero, so the ratios above are
/// always safe to multiply by.
fn base_size(glyphs: &[MathGlyph]) -> f64 {
    glyphs
        .iter()
        .map(|g| g.info.size.0)
        .fold(0.0f64, f64::max)
        .max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyfi_backend::{Color, HorzStringInfo, FontKey};

    fn glyph(text: &str, dx: f64, dy: f64, size: f64) -> MathGlyph {
        MathGlyph {
            info: HorzStringInfo {
                font: FontKey(0),
                size: Length(size),
                color: Color::Gray(0.0),
                rising: Length(0.0),
            },
            text: text.to_string(),
            gid: None,
            dx: Length(dx),
            dy: Length(dy),
            width: Length(size * 0.5),
            height: Length(size * 0.7),
            depth: Length(0.0),
        }
    }

    #[test]
    fn glyphs_come_out_in_reading_order_whatever_order_they_are_stored_in() {
        let gs = vec![glyph("c", 10.0, 0.0, 10.0), glyph("a", 0.0, 0.0, 10.0)];
        assert_eq!(math_text(&gs, &[]), "a c");
    }

    /// A superscript uses Unicode's own character, so nothing has to be
    /// escaped and the result is still searchable text.
    #[test]
    fn a_script_becomes_a_unicode_script_character() {
        let gs = vec![
            glyph("x", 0.0, 0.0, 10.0),
            glyph("2", 5.0, 5.0, 7.0),
        ];
        assert_eq!(math_text(&gs, &[]), "x²");
        let gs = vec![
            glyph("a", 0.0, 0.0, 10.0),
            glyph("1", 5.0, -3.0, 7.0),
        ];
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
}

//! What can be read back out of a laid-out math run, once.
//!
//! ## Why this module exists apart from its writers
//!
//! `${\frac{a}{b}}` is parsed, elaborated, evaluated and LAID OUT during
//! compilation: `read_math`/`layout_math_value` turn the expression tree into
//! a `PureHorzBox::Math` carrying a flat `Vec<MathGlyph>`, each glyph a
//! character with an `(dx, dy)` offset from the run's baseline, plus a
//! `Vec<GraphicsElem>` of the FILLED PATHS a font cannot draw — the fraction
//! bar and the radical sign. By the time any backend sees it there is no
//! `\frac` node, no numerator, no argument list.
//!
//! So every non-pictorial rendering of math in this crate has to RECOVER the
//! two-dimensional structure from coordinates: which glyph is a script, which
//! glyphs are a fraction's numerator and which its denominator, and where the
//! layout put a space that carries no character. Two writers need exactly
//! that recovery and would otherwise each carry their own copy of these
//! heuristics — `markdown::math`, which writes Unicode characters in reading
//! order, and [`crate::latex`], which writes LaTeX. Getting the thresholds to
//! agree by coincidence is not a thing that stays true, so the recovery is
//! here and the writers fold over [`Atom`].
//!
//! It is the same bargain the crate's root module comment describes for
//! `recover`: one implementation of the part that was got wrong before it was
//! got right, and as many callers as there are output vocabularies.
//!
//! ## What is recovered, and how
//!
//! The glyphs carry real Unicode text — that is what makes the PDF's
//! `ToUnicode` table work — so `∑`, `α` and `≤` arrive as themselves.
//! [`recover`] sorts them by `dx` and then puts back what a flat left-to-right
//! order would mangle:
//!
//! - **Scripts.** A glyph set smaller than the run's base size and offset
//!   vertically is a superscript or a subscript ([`Atom::Glyph`]'s `script`).
//! - **Limits vs. scripts.** A script whose horizontal centre lies INSIDE its
//!   base's own advance is a limit set above or below it (`\sum` with its
//!   bounds), not a script set beside it (`x²`). `layout_math_list`'s
//!   `UpperLimit`/`LowerLimit` arms centre a limit on the base's width
//!   (`center_offsets`) while `Sup`/`Sub` set the script AT that width, so the
//!   two populations are cleanly separated by the test and no threshold is
//!   involved. Only [`crate::latex`] uses it — Unicode has no notation for the
//!   difference — but it is recovered here because it is geometry, not
//!   vocabulary.
//! - **Fractions.** A numerator and a denominator have overlapping `dx`, so
//!   plain `dx` order interleaves them: `(a+b)/(c+d)` would come out as
//!   `ac++bd`. The fraction BAR survives as a wide, flat `Fill` in `rules`,
//!   and [`fraction_bars`] uses each bar's own extent to split the glyphs it
//!   spans into the part above it and the part below.
//! - **Spaces.** A gap wider than a fraction of the base size is the
//!   inter-atom spacing the math layout inserted around a relation or a binary
//!   operator, and is the only surviving evidence that a space was set at all.
//!
//! ## What is NOT recovered, at this layer or any other
//!
//! Radicals (the sign is a path, not a glyph — `√` is not in `glyphs`),
//! matrices and aligned environments (their row/column arrangement flattens),
//! and anything whose meaning is carried by position rather than by its
//! characters. Fraction recovery is one level deep: a fraction inside a
//! fraction contributes its glyphs to the outer one. None of this can be had
//! by trying harder here — it would have to be captured before layout, the
//! way `ListMark`/`InlineMark` capture list and emphasis structure. Each
//! writer states the consequence in its own vocabulary; this module states the
//! cause once.

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

/// Which of the two vertical positions a script sits in, and whether it is set
/// BESIDE its base or centred OVER/UNDER it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) struct Script {
    /// A superscript / upper limit, as opposed to a subscript / lower limit.
    pub(crate) up: bool,
    /// Centred on the base's own advance rather than set after it — a big
    /// operator's limit. See this module's doc comment for why the test is
    /// exact rather than a threshold.
    pub(crate) limit: bool,
}

/// One recovered piece of a math run, in reading order.
///
/// Deliberately shallow: the only nesting is a fraction's two halves, because
/// a fraction bar is the only structure the box stream still carries. A
/// writer folds over a `&[Atom]` and never has to know a threshold.
pub(crate) enum Atom<'a> {
    /// One glyph record's characters, and whether the layout set them as a
    /// script. A record may hold MANY characters: `math_boxes_of_inline_boxes`
    /// folds a whole `text-in-math` run into one `MathGlyph`.
    Glyph {
        g: &'a MathGlyph,
        script: Option<Script>,
    },
    /// A fraction bar's two halves, each recovered one level deep.
    Frac {
        above: Vec<Atom<'a>>,
        below: Vec<Atom<'a>>,
    },
    /// A measured gap wide enough to be a word space. Emitted where the gap
    /// is, never at the very start of a run; a writer still drops it when its
    /// own output is so far empty (see [`recover`]'s note on why the two
    /// guards are not the same one).
    Space,
}

/// Recover `glyphs`/`rules` into reading-order [`Atom`]s.
///
/// **A writer must still suppress a leading [`Atom::Space`] itself.** This
/// function suppresses one only when the space would be the FIRST atom;
/// a run whose first glyph carries no characters at all (an advance-only
/// record) leaves the writer's own buffer empty while an atom has already been
/// consumed, and the space after it is then leading too. Keeping that second
/// guard in the writer, where the buffer is, is what makes the Unicode
/// writer's output identical to the hand-rolled version this replaced.
pub(crate) fn recover<'a>(glyphs: &'a [MathGlyph], rules: &[GraphicsElem]) -> Vec<Atom<'a>> {
    if glyphs.is_empty() {
        return Vec::new();
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

    // Read in one left-to-right pass over both populations at once: a
    // fraction occupies its bar's own x span, so it sorts among the loose
    // glyphs exactly where the reader's eye reaches it.
    enum Item<'a> {
        Glyph(&'a MathGlyph),
        Frac(Vec<&'a MathGlyph>, Vec<&'a MathGlyph>),
    }
    let mut items: Vec<(f64, f64, Item<'a>)> = Vec::new();
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
            items.push((
                bar.x0,
                bar.x1,
                Item::Frac(above.clone(), below.clone()),
            ));
        }
    }
    items.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut out: Vec<Atom<'a>> = Vec::new();
    let mut prev_right: Option<f64> = None;
    // The most recent atom that a script could be a script OF, as its own
    // `(dx, dx + width)` span — see [`Script::limit`].
    let mut base_span: Option<(f64, f64)> = None;
    for (left, right, item) in items {
        if let Some(pr) = prev_right {
            if left - pr > base * SPACE_GAP_RATIO && !out.is_empty() {
                out.push(Atom::Space);
            }
        }
        match item {
            Item::Glyph(g) => {
                let script = classify_script(g, base, base_span);
                if script.is_none() {
                    base_span = Some((g.dx.0, g.dx.0 + g.width.0));
                }
                out.push(Atom::Glyph { g, script });
            }
            Item::Frac(above, below) => {
                out.push(Atom::Frac {
                    above: flat(&above, base),
                    below: flat(&below, base),
                });
                base_span = Some((left, right));
            }
        }
        prev_right = Some(right);
    }
    out
}

/// A fraction's own half, recovered flat — scripts and spacing still apply,
/// but a bar nested inside this one has already been dissolved into it (see
/// this module's "one level deep").
///
/// `base` is the WHOLE RUN's base size, not the half's own: a fraction's
/// numerator is typeset smaller than the surrounding text, and measuring its
/// scripts against its own largest glyph would find scripts in ordinary
/// characters.
fn flat<'a>(glyphs: &[&'a MathGlyph], base: f64) -> Vec<Atom<'a>> {
    let mut sorted: Vec<&&'a MathGlyph> = glyphs.iter().collect();
    sorted.sort_by(|a, b| a.dx.0.total_cmp(&b.dx.0));
    let mut out = Vec::new();
    let mut prev_right: Option<f64> = None;
    let mut base_span: Option<(f64, f64)> = None;
    for g in sorted {
        if let Some(pr) = prev_right {
            if g.dx.0 - pr > base * SPACE_GAP_RATIO && !out.is_empty() {
                out.push(Atom::Space);
            }
        }
        let script = classify_script(g, base, base_span);
        if script.is_none() {
            base_span = Some((g.dx.0, g.dx.0 + g.width.0));
        }
        out.push(Atom::Glyph { g, script });
        prev_right = Some(g.dx.0 + g.width.0);
    }
    out
}

/// Is `g` a script, and if so which kind — measured against `base` (the run's
/// own font size) and `base_span` (the advance of the atom it would be a
/// script of, when there is one).
fn classify_script(g: &MathGlyph, base: f64, base_span: Option<(f64, f64)>) -> Option<Script> {
    if g.info.size.0 > base * SCRIPT_SIZE_RATIO {
        return None;
    }
    let shift = g.dy.0;
    let up = if shift > base * SCRIPT_SHIFT_RATIO {
        true
    } else if shift < -base * SCRIPT_SHIFT_RATIO {
        false
    } else {
        return None;
    };
    // Centred on the base's advance -> a limit; set after it -> a script.
    // With no base to measure against (a run that opens with a small raised
    // glyph) it can only be a script.
    let centre = g.dx.0 + g.width.0 / 2.0;
    let limit = base_span.is_some_and(|(lo, hi)| centre > lo && centre < hi);
    Some(Script { up, limit })
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

/// `pub(crate)` so the three WRITERS' own test modules can build a `MathGlyph`
/// through [`tests::glyph`] instead of each repeating the ten-field literal.
/// The recovery is shared, so the fixtures for it should be too.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rustyfi_backend::{Color, FontKey, HorzStringInfo};

    /// One glyph record, with the metrics a real one would have: `width` is
    /// half the size and `height` seven tenths, which is roughly true of a
    /// Latin letter and is what the script/limit geometry is measured against.
    pub(crate) fn glyph(text: &str, dx: f64, dy: f64, size: f64) -> MathGlyph {
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

    /// A script set AFTER its base's advance is a script; one centred ON it is
    /// a limit. Nothing but geometry separates them, and the distinction is
    /// what tells `\sum\limits_{k}` from `x^{2}`.
    #[test]
    fn a_centred_script_is_a_limit_and_a_trailing_one_is_not() {
        // Base 10pt wide 5.0 at dx 0; the script's centre at 2.75 is inside.
        let limits = vec![glyph("S", 0.0, 0.0, 10.0), glyph("k", 1.0, -4.0, 7.0)];
        let atoms = recover(&limits, &[]);
        let Atom::Glyph { script, .. } = &atoms[1] else {
            panic!("expected a glyph")
        };
        assert_eq!(
            *script,
            Some(Script {
                up: false,
                limit: true
            })
        );
        // The same script set past the base's right edge is an ordinary one.
        let beside = vec![glyph("x", 0.0, 0.0, 10.0), glyph("2", 5.0, 4.0, 7.0)];
        let atoms = recover(&beside, &[]);
        let Atom::Glyph { script, .. } = &atoms[1] else {
            panic!("expected a glyph")
        };
        assert_eq!(
            *script,
            Some(Script {
                up: true,
                limit: false
            })
        );
    }

    /// A bar with glyphs on only one side is not a fraction — a radical's
    /// overbar is the shape that reaches this — and its glyphs stay in the
    /// flat run rather than becoming an empty denominator.
    #[test]
    fn a_one_sided_bar_is_not_a_fraction() {
        use rustyfi_backend::{Closing, Path, PathSeg, Subpath};
        let bar = GraphicsElem::Fill(
            Color::Gray(0.0),
            Path {
                subpaths: vec![Subpath {
                    start: (Length(0.0), Length(5.0)),
                    segs: vec![
                        PathSeg::Line((Length(20.0), Length(5.0))),
                        PathSeg::Line((Length(20.0), Length(5.5))),
                        PathSeg::Line((Length(0.0), Length(5.5))),
                    ],
                    closing: Closing::Line,
                }],
            },
        );
        // Both glyphs sit ABOVE the bar, so there is no denominator.
        let gs = vec![glyph("a", 2.0, 8.0, 10.0), glyph("b", 8.0, 8.0, 10.0)];
        let atoms = recover(&gs, std::slice::from_ref(&bar));
        assert_eq!(atoms.len(), 2, "no fraction should have been formed");
        assert!(atoms.iter().all(|a| matches!(a, Atom::Glyph { .. })));
    }
}

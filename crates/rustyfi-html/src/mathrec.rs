//! What can be read back out of a laid-out math run, once.
//!
//! ## Why this module exists apart from its writers
//!
//! `${\frac{a}{b}}` is parsed, elaborated, evaluated and LAID OUT during
//! compilation: `read_math`/`layout_math_value` turn the expression tree into
//! a `PureHorzBox::Math` carrying a flat `Vec<MathGlyph>`, each glyph a
//! character with an `(dx, dy)` offset from the run's baseline, plus a
//! `Vec<GraphicsElem>` of the PATHS a font cannot draw — the fraction bar, the
//! radical sign, and every delimiter `math-paren` draws. By the time any
//! backend sees it there is no `\frac` node, no numerator, no argument list.
//!
//! So every non-pictorial rendering of math in this crate has to RECOVER the
//! two-dimensional structure from coordinates: which glyph is a script, which
//! glyphs are a fraction's numerator and which its denominator, what a filled
//! path is a picture OF, and where the layout put a space that carries no
//! character. Two writers need exactly that recovery and would otherwise each
//! carry their own copy of these heuristics — `markdown::math`, which writes
//! Unicode characters in reading order, and [`crate::latex`], which writes
//! LaTeX. Getting the thresholds to agree by coincidence is not a thing that
//! stays true, so the recovery is here and the writers fold over [`Atom`].
//!
//! It is the same bargain the crate's root module comment describes for
//! `recover`: one implementation of the part that was got wrong before it was
//! got right, and as many callers as there are output vocabularies.
//!
//! ## Why the paths have to be read, and not just skipped
//!
//! A missing delimiter is not a missing decoration. `${\paren{a+b}^2}` with
//! its parentheses dropped is `a+b^{2}`, which is a DIFFERENT AND FALSE
//! statement, and under `--katex` it is set by a real math typesetter, so it
//! reads as authoritative rather than as visibly rough. The same goes for a
//! radical: `\frac{-b\pm\sqrt{b^2-4ac}}{2a}` with the sign dropped is not the
//! quadratic formula. Everything below exists because "emit the contents and
//! say so in the README" turned out not to be a safe default for this
//! particular loss.
//!
//! ## What is recovered, and how
//!
//! The glyphs carry real Unicode text — that is what makes the PDF's
//! `ToUnicode` table work — so `∑`, `α` and `≤` arrive as themselves.
//! [`recover`] builds a TREE over them, keyed on the paths:
//!
//! - **Fractions.** A numerator and a denominator have overlapping `dx`, so
//!   plain `dx` order interleaves them: `(a+b)/(c+d)` would come out as
//!   `ac++bd`. The bar survives as a wide, flat `Fill`
//!   (`layout_math_atom`'s `Math::Fraction` arm), and each bar's extent splits
//!   the glyphs it spans into the part above it and the part below.
//! - **Delimiters** ([`Atom::Delim`]). `\paren`, `\sqbracket`, `\brace`,
//!   `\floor`, `\ceil`, `\abs`, `\norm` and `\angle-bracket` are all
//!   `math-paren` applied to a pair of closures out of `math.satyh`, and every
//!   one of them draws its delimiter as a PATH — there is no character
//!   anywhere in `glyphs` to key on. They are recovered from the shape of the
//!   path instead; see [`delim_shape`] for the signature of each and
//!   [`pair_delims`] for how the two halves find each other.
//! - **Radicals** ([`Atom::Radical`]). `Math::Radical` emits a checkmark
//!   `Fill` and an overbar `Fill` whose left edge and top edge coincide with
//!   the sign's EXACTLY — same two additions in the same arm — so the pair is
//!   identifiable without a threshold. That coincidence is also what stops the
//!   overbar being read as a fraction bar, which is what used to make
//!   `\frac{-b\pm\sqrt{b^2-4ac}}{2a}` lose its denominator.
//! - **Scripts.** A glyph set smaller than the run's base size and offset
//!   vertically FROM ITS OWN REGION'S BASELINE is a superscript or a subscript
//!   ([`Atom::Glyph`]'s `script`). The region matters: a fraction's halves sit
//!   on displaced baselines, so measuring against the run's own `dy = 0` reads
//!   a denominator's superscript as a subscript.
//! - **Limits vs. scripts.** `layout_math_list`'s `UpperLimit`/`LowerLimit`
//!   arms CENTRE a limit on its base (`center_offsets` pads the narrower of
//!   the two by half the difference), while `Sup`/`Sub` set the script AT the
//!   base's advance. So a limit's midpoint and its base's midpoint coincide
//!   exactly, and [`limit_groups`] recovers the pairing by looking for that
//!   coincidence — over a RUN of base glyphs, which is what `\lim` needs and
//!   what a per-glyph test cannot give.
//! - **Spaces.** A gap wider than a fraction of the base size is the
//!   inter-atom spacing the math layout inserted around a relation or a binary
//!   operator, and is the only surviving evidence that a space was set at all.
//!
//! ## What is NOT recovered, at this layer or any other
//!
//! Matrices and aligned environments (their row/column arrangement flattens),
//! a `\sqrt`'s DEGREE (`layout_math_atom` carries it in the `Math` value and
//! deliberately does not draw it, so there is nothing in the box stream to
//! read), the middle separator of a `math-paren-with-middle` (`\setsep`'s bar
//! is drawn by the same closure shape an `\abs` uses, and pairing it would
//! close the wrong group — it is dropped instead of guessed at), a delimiter
//! whose path matches none of the signatures below (its BODY is still
//! recovered and grouped, so a script still binds correctly; only the
//! delimiter character is lost), and anything whose meaning is carried by
//! position rather than by its characters. None of that can be had by trying
//! harder here — it would have to be captured before layout, the way
//! `ListMark`/`InlineMark` capture list and emphasis structure. Each writer
//! states the consequence in its own vocabulary; this module states the cause
//! once.

use rustyfi_backend::{graphics_bbox, Closing, GraphicsElem, Length, MathGlyph, Path, PathSeg};

/// A `rules` fill this flat (its width at least this many times its height)
/// is a horizontal bar rather than a drawn shape.
const BAR_ASPECT: f64 = 3.0;

/// …and no taller than this many points. A fraction bar is a hairline; a
/// filled box that happens to be wide is not one.
const BAR_MAX_HEIGHT_PT: f64 = 2.5;

/// The mirror of [`BAR_ASPECT`] for a delimiter: a `rules` path at least this
/// many times taller than it is wide is a drawn delimiter rather than a rule.
/// Measured over `math.satyh`'s own eight: the flattest is `\brace` at 3.7 and
/// the sharpest `\norm` at 5.7, so the bar between the two populations is
/// wide and this sits well inside it.
const DELIM_ASPECT: f64 = 2.0;

/// …and at least this fraction of the run's base size tall. `half-length`
/// (`math.satyh:1019`) floors a delimiter at `fontsize *' 0.5` either side of
/// the axis, so even the shortest one is a whole em; the margin is for a
/// delimiter set at script size inside a run whose base is full size.
const DELIM_MIN_HEIGHT_RATIO: f64 = 0.5;

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

/// Two coordinates this close, in points, came out of the same arithmetic.
///
/// Used only where the layout makes two numbers EQUAL rather than merely
/// similar — a radical's sign and its overbar share `sign_w` and
/// `h_bar + t_bar` (`layout_math_atom`'s `Math::Radical` arm), a delimiter
/// pair shares one `half-length` — so this is a float-noise tolerance and not
/// a threshold. Kept absolute because the quantities it compares are absolute
/// point coordinates that have been through the same additive shifts.
const COINCIDE_PT: f64 = 0.05;

/// Upstream's `superscript_shift_up` / `subscript_shift_down` fallbacks
/// (`MathC::sup_shift`/`sub_shift`, `primitives.rs`'s `SUP_SHIFT` = 0.5 and
/// `SUB_SHIFT` = 0.25), as fractions of the base size.
///
/// Read for ONE purpose: deciding which half of a fraction a script-size glyph
/// belongs to when both halves could claim it. In `${\frac{a_1}{b^1}}` the
/// numerator's subscript lands at `dy = 0.96` and the denominator's
/// superscript at `dy = 2.04` — both below the bar, both in the band where
/// "above or below the bar" gives the wrong answer for one of them. A
/// superscript is set about twice as far from its baseline as a subscript, so
/// asking which of the four (half, direction) slots the glyph is nearest
/// separates them. See [`half_of_script`].
const SUP_SHIFT_RATIO: f64 = 0.5;
const SUB_SHIFT_RATIO: f64 = 0.25;

/// Which of the two vertical positions a script sits in, and whether it is set
/// BESIDE its base or centred OVER/UNDER it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) struct Script {
    /// A superscript / upper limit, as opposed to a subscript / lower limit.
    pub(crate) up: bool,
    /// Centred on the base's own advance rather than set after it — a big
    /// operator's limit. See this module's doc comment for why the test is a
    /// coincidence rather than a threshold.
    pub(crate) limit: bool,
}

/// Which delimiter a drawn path is a picture of.
///
/// One variant per `math.satyh` closure pair, plus [`DelimKind::Unknown`] for
/// a path that is delimiter-SHAPED (tall, narrow, flanking a sub-run) but
/// matches none of the signatures — a third-party package's own
/// `math-paren` argument. An unknown delimiter still groups its body, so a
/// script binds to the whole group; it just has no character to write.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum DelimKind {
    Paren,
    Bracket,
    Brace,
    Floor,
    Ceil,
    Abs,
    Norm,
    Angle,
    Unknown,
}

/// One recovered piece of a math run, in reading order.
///
/// A tree, not a list: [`Atom::Frac`]'s halves, [`Atom::Delim`]'s body and
/// [`Atom::Radical`]'s radicand are each recovered to the same depth as the
/// run itself, so a fraction inside a fraction inside a `\paren` comes back
/// whole. A writer folds over a `&[Atom]` and never has to know a threshold.
pub(crate) enum Atom<'a> {
    /// One glyph record's characters, and whether the layout set them as a
    /// script. A record may hold MANY characters: `math_boxes_of_inline_boxes`
    /// folds a whole `text-in-math` run into one `MathGlyph`.
    Glyph {
        g: &'a MathGlyph,
        script: Option<Script>,
    },
    /// A fraction bar's two halves.
    Frac {
        above: Vec<Atom<'a>>,
        below: Vec<Atom<'a>>,
    },
    /// A `math-paren` group: whichever of the two delimiters was drawn and
    /// identified, and everything between them.
    ///
    /// Either side is `None` when the closure drew nothing (`empty-paren`,
    /// which `\cases` passes as its right delimiter) or when the path matched
    /// no signature. The BODY is what makes this variant worth having even
    /// then: `{a+b}^{2}` binds the script to the sum, `a+b^{2}` does not.
    Delim {
        open: Option<DelimKind>,
        close: Option<DelimKind>,
        body: Vec<Atom<'a>>,
    },
    /// `\sqrt`'s radicand. The degree of a `\sqrt[n]` is not here because it
    /// is not in the box stream — `layout_math_atom` carries it and does not
    /// draw it.
    Radical { body: Vec<Atom<'a>> },
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
    let structures = structures(rules, base);
    let refs: Vec<&'a MathGlyph> = glyphs.iter().collect();
    build(&refs, &structures, base, 0.0)
}

// ---------------------------------------------------------------------------
// Reading the paths
// ---------------------------------------------------------------------------

/// An axis-aligned extent in the math box's own baseline-relative, y-up
/// coordinates.
#[derive(Copy, Clone)]
struct Rect {
    x0: f64,
    x1: f64,
    y0: f64,
    y1: f64,
}

impl Rect {
    fn w(&self) -> f64 {
        self.x1 - self.x0
    }
    fn h(&self) -> f64 {
        self.y1 - self.y0
    }
}

/// Which end of a `math-paren` pair a drawn delimiter is.
///
/// Read off the shape rather than off the order, because two sibling groups
/// (`${\paren{a+b}\paren{a-b}}`) put four delimiters of identical height in
/// one run and the order alone cannot say which two belong together.
/// [`Side::Symmetric`] is a bar with no handedness at all (`\abs`, `\norm`),
/// which has to be paired by alternation instead.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Side {
    Left,
    Right,
    Symmetric,
}

/// One structural thing recovered from the paths, with the two horizontal
/// extents the tree is built out of.
#[derive(Clone, Copy)]
struct Structure {
    /// The whole thing's footprint — what decides where it sorts into reading
    /// order, and what an enclosing structure must contain for it to nest.
    foot: (f64, f64),
    /// Where this structure's CHILDREN live: inside the delimiters, under the
    /// overbar, across the fraction bar.
    body: (f64, f64),
    /// One representative height, for deciding which half of an enclosing
    /// fraction this is in: a bar's own height, an overbar's underside, a
    /// delimiter pair's vertical centre.
    y: f64,
    kind: StructKind,
}

#[derive(Clone, Copy)]
enum StructKind {
    Frac,
    /// A radical, carrying the underside of its overbar — the ceiling its
    /// radicand sits under.
    Radical { bar_y0: f64 },
    Delim {
        open: Option<DelimKind>,
        close: Option<DelimKind>,
    },
}

/// Every structure the `rules` of one math run describe, in no particular
/// order.
///
/// Ordered by how tightly each test binds, because the tests overlap: a
/// radical's sign is tall and narrow enough to read as a delimiter, and its
/// overbar is flat enough to read as a fraction bar, so radicals are matched
/// FIRST and their two paths withdrawn before anything else looks at them.
fn structures(rules: &[GraphicsElem], base: f64) -> Vec<Structure> {
    let mut paths = Vec::new();
    collect_paths(rules, &mut paths);

    let mut out = Vec::new();
    let mut used = vec![false; paths.len()];

    // 1. Radicals. `Math::Radical` builds the sign from `radical_sign_geometry`
    //    and then the overbar as `rect_path((sign_w, h_bar), (inner_w, t_bar))`
    //    — so the bar's left edge IS `sign_w`, the sign's own right edge, and
    //    the two tops are both `h_bar + t_bar`. Both equalities are exact, and
    //    requiring both of them together is what keeps an ordinary fraction
    //    bar that happens to abut a delimiter from being read as an overbar.
    for (bi, bar) in paths.iter().enumerate() {
        if !is_bar(bar) {
            continue;
        }
        let sign = paths.iter().enumerate().find(|(si, s)| {
            *si != bi
                && !used[*si]
                && s.filled
                && !is_bar(s)
                && (s.rect.x1 - bar.rect.x0).abs() <= COINCIDE_PT
                && (s.rect.y1 - bar.rect.y1).abs() <= COINCIDE_PT
                && s.rect.y0 < bar.rect.y0 - COINCIDE_PT
        });
        if let Some((si, s)) = sign {
            used[si] = true;
            used[bi] = true;
            out.push(Structure {
                foot: (s.rect.x0, bar.rect.x1),
                body: (bar.rect.x0, bar.rect.x1),
                y: bar.rect.y0,
                kind: StructKind::Radical {
                    bar_y0: bar.rect.y0,
                },
            });
        }
    }

    // 2. Delimiters, paired.
    let mut delims: Vec<(Rect, DelimKind, Side)> = Vec::new();
    for (i, p) in paths.iter().enumerate() {
        if used[i] || is_bar(p) {
            continue;
        }
        if p.rect.h() <= BAR_MAX_HEIGHT_PT
            || p.rect.h() < p.rect.w() * DELIM_ASPECT
            || p.rect.h() < base * DELIM_MIN_HEIGHT_RATIO
        {
            continue;
        }
        used[i] = true;
        let (kind, side) = delim_shape(p);
        delims.push((p.rect, kind, side));
    }
    out.extend(pair_delims(&mut delims));

    // 3. Whatever bars are left are fraction bars.
    for (i, p) in paths.iter().enumerate() {
        if used[i] || !is_bar(p) {
            continue;
        }
        out.push(Structure {
            foot: (p.rect.x0, p.rect.x1),
            body: (p.rect.x0, p.rect.x1),
            y: (p.rect.y0 + p.rect.y1) / 2.0,
            kind: StructKind::Frac,
        });
    }
    out
}

/// A wide, flat FILLED path — a fraction bar or a radical's overbar.
///
/// `Stroke`s are excluded deliberately: `layout_math_atom` draws both of those
/// as filled rectangles, and a stroked horizontal line in a math run is a
/// user's own `\overline`-style drawing, whose two sides are not a numerator
/// and a denominator.
fn is_bar(p: &PathShape) -> bool {
    let r = &p.rect;
    p.filled && r.h() <= BAR_MAX_HEIGHT_PT && r.w() >= r.h() * BAR_ASPECT && r.w() > 0.0
}

/// One `rules` path, reduced to what the recovery reads off it.
struct PathShape {
    rect: Rect,
    /// The path's ON-CURVE points, subpath by subpath, each list closed back
    /// to its own start when the subpath was. Bezier CONTROL points are left
    /// out deliberately: they are off the drawn outline, so including them
    /// would move a `\paren`'s apparent mid-height extreme outward and blur
    /// exactly the measurement [`side_of`] takes.
    outline: Vec<Vec<(f64, f64)>>,
    curved: bool,
    filled: bool,
}

fn collect_paths(rules: &[GraphicsElem], out: &mut Vec<PathShape>) {
    for elem in rules {
        match elem {
            GraphicsElem::Fill(_, p)
            | GraphicsElem::Stroke(_, _, p)
            | GraphicsElem::DashedStroke(_, _, _, p) => {
                let Some(((Length(x0), Length(y0)), (Length(x1), Length(y1)))) =
                    graphics_bbox(elem)
                else {
                    continue;
                };
                out.push(PathShape {
                    rect: Rect { x0, x1, y0, y1 },
                    outline: outline_of(p),
                    curved: p.subpaths.iter().any(|s| {
                        s.segs.iter().any(|g| matches!(g, PathSeg::Bezier(..)))
                            || matches!(s.closing, Closing::Bezier(..))
                    }),
                    filled: matches!(elem, GraphicsElem::Fill(..)),
                });
            }
            GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => collect_paths(inner, out),
            _ => {}
        }
    }
}

fn outline_of(p: &Path) -> Vec<Vec<(f64, f64)>> {
    p.subpaths
        .iter()
        .map(|s| {
            let mut pts = vec![(s.start.0 .0, s.start.1 .0)];
            for seg in &s.segs {
                let end = match seg {
                    PathSeg::Line(e) => e,
                    PathSeg::Bezier(_, _, e) => e,
                };
                pts.push((end.0 .0, end.1 .0));
            }
            if !matches!(s.closing, Closing::Open) {
                pts.push(pts[0]);
            }
            pts
        })
        .collect()
}

/// Which delimiter this path draws, and which end of the pair it is.
///
/// Every signature below is a statement about the SHAPE, taken from
/// `math.satyh`'s own closures (`angle-left` :1024, `paren-left` :1071,
/// `brace-left` :1157, `bracket-left` :1430 with `bracket-path` :1392,
/// `floor-path` :1406, `ceil-path` :1418, `abs-left` :1479, `norm-left`
/// :1509) but phrased so that a path drawn some other way still lands in the
/// right place if it looks the same:
///
/// - **two subpaths, each a straight line** — `\norm`'s double rule.
/// - **one straight segment** — `\abs`'s single rule. (`bar-middle`, a
///   `math-paren-with-middle` separator, is the same shape; it is the reason
///   [`pair_delims`] must be able to leave a symmetric bar unpaired.)
/// - **two straight segments** — `\angle-bracket`'s chevron.
/// - **no curves, and a full-width arm at one or both ends** — the bracket
///   family, told apart by WHICH ends carry the arm: both is `[`, the bottom
///   alone is `⌊`, the top alone is `⌈`.
/// - **curved, with a monotone outer edge** — `\paren`. From its midpoint to
///   its tip a parenthesis is one arc, so the outer boundary's x moves one
///   way only.
/// - **curved, with a reversal in the outer edge** — `\brace`, whose outer
///   boundary runs out to the shoulder, back IN along the straight, and out
///   again to the tip. That reversal is the cusp that makes a brace a brace.
fn delim_shape(p: &PathShape) -> (DelimKind, Side) {
    let straight_lines = p
        .outline
        .iter()
        .all(|s| (s.len() == 3 && s[0] == s[2]) || s.len() == 2);
    if !p.curved && straight_lines {
        match p.outline.len() {
            2 => return (DelimKind::Norm, Side::Symmetric),
            1 => return (DelimKind::Abs, Side::Symmetric),
            _ => {}
        }
    }
    if !p.curved && p.outline.len() == 1 && p.outline[0].len() == 3 && p.outline[0][0] != p.outline[0][2]
    {
        return (DelimKind::Angle, side_of(p));
    }
    // Filled or stroked alike: `math.satyh`'s brackets are outlines and
    // `azmath`'s `\pB` is a three-segment stroked polyline, and the arms mean
    // the same thing in both.
    if !p.curved {
        let top = end_span(p, true);
        let bot = end_span(p, false);
        let full = |s: f64| s >= p.rect.w() * 0.5;
        let kind = match (full(top), full(bot)) {
            (true, true) => DelimKind::Bracket,
            (false, true) => DelimKind::Floor,
            (true, false) => DelimKind::Ceil,
            (false, false) => DelimKind::Unknown,
        };
        return (kind, side_of(p));
    }
    if p.curved {
        let side = side_of(p);
        let kind = if outer_edge_reverses(p, side) {
            DelimKind::Brace
        } else {
            DelimKind::Paren
        };
        return (kind, side);
    }
    (DelimKind::Unknown, side_of(p))
}

/// The horizontal span of the path's topmost (or bottommost) points — a
/// bracket's arm, or the bare thickness of its stem where there is no arm.
fn end_span(p: &PathShape, top: bool) -> f64 {
    let want = if top { p.rect.y1 } else { p.rect.y0 };
    let xs: Vec<f64> = p
        .outline
        .iter()
        .flatten()
        .filter(|(_, y)| (y - want).abs() <= COINCIDE_PT)
        .map(|(x, _)| *x)
        .collect();
    match (
        xs.iter().cloned().reduce(f64::min),
        xs.iter().cloned().reduce(f64::max),
    ) {
        (Some(lo), Some(hi)) => hi - lo,
        _ => 0.0,
    }
}

/// Which end of a pair this delimiter is, from where it is THICK at its own
/// mid-height.
///
/// Every left delimiter in `math.satyh` reaches its outer extreme at the axis
/// and its inner extreme at the tips; every right one is the mirror. So
/// slicing the outline at mid-height and asking which side of the bounding box
/// the slice hugs answers it, for a curve (`\paren`, `\brace`), a polygon
/// (`\sqbracket`, `\floor`, `\ceil`) and a chevron (`\angle-bracket`) alike —
/// no per-shape special case, and nothing that depends on the drawing order.
/// A slice that hugs both sides equally is a bar with no handedness.
fn side_of(p: &PathShape) -> Side {
    let ymid = (p.rect.y0 + p.rect.y1) / 2.0;
    let xs = crossings_at(p, ymid);
    let (Some(lo), Some(hi)) = (
        xs.iter().cloned().reduce(f64::min),
        xs.iter().cloned().reduce(f64::max),
    ) else {
        return Side::Symmetric;
    };
    let left_gap = lo - p.rect.x0;
    let right_gap = p.rect.x1 - hi;
    if (left_gap - right_gap).abs() <= p.rect.w() * 0.1 {
        Side::Symmetric
    } else if left_gap < right_gap {
        Side::Left
    } else {
        Side::Right
    }
}

/// Every x at which the outline crosses the horizontal line `y`, the outline
/// taken as the polyline through its on-curve points.
fn crossings_at(p: &PathShape, y: f64) -> Vec<f64> {
    let mut out = Vec::new();
    for sub in &p.outline {
        for w in sub.windows(2) {
            let (ax, ay) = w[0];
            let (bx, by) = w[1];
            if (ay - y).abs() <= f64::EPSILON {
                out.push(ax);
            }
            if (by - y).abs() <= f64::EPSILON {
                out.push(bx);
            }
            if (ay < y) != (by < y) && (by - ay).abs() > f64::EPSILON {
                out.push(ax + (bx - ax) * (y - ay) / (by - ay));
            }
        }
    }
    out
}

/// Does the outer boundary turn back on itself between the delimiter's
/// mid-height and its top? — the brace's cusp, and the one thing that
/// distinguishes `\brace` from `\paren` without counting vertices.
fn outer_edge_reverses(p: &PathShape, side: Side) -> bool {
    let ymid = (p.rect.y0 + p.rect.y1) / 2.0;
    const SAMPLES: usize = 8;
    let mut prev: Option<f64> = None;
    let mut worst = 0.0f64;
    for i in 1..SAMPLES {
        let y = ymid + (p.rect.y1 - ymid) * (i as f64) / (SAMPLES as f64);
        let xs = crossings_at(p, y);
        let outer = match side {
            Side::Right => xs.iter().cloned().reduce(f64::max),
            _ => xs.iter().cloned().reduce(f64::min),
        };
        let Some(outer) = outer else { continue };
        if let Some(prev) = prev {
            let step = match side {
                Side::Right => prev - outer,
                _ => outer - prev,
            };
            // Moving back toward the middle of the box is the reversal.
            worst = worst.max(-step);
        }
        prev = Some(outer);
    }
    worst > p.rect.w() * 0.03
}

/// Match the drawn delimiters into `math-paren` pairs.
///
/// A stack walk in `dx` order, with two rules that come straight from how the
/// layout builds a pair: `Math::Paren` runs BOTH closures against ONE
/// `(h_in, d_in)`, so a pair's two halves have the same vertical extent to the
/// last bit, and it splices them as `left ++ inner ++ right`, so they nest.
/// A closing delimiter therefore closes the innermost open one of the same
/// height, and anything skipped over on the way (`\setsep`'s middle bar) is
/// dropped rather than guessed at.
///
/// A [`Side::Symmetric`] bar has no handedness, so it opens when nothing
/// matching is open and closes when something is: `${\abs{x}\abs{y}}` pairs by
/// alternation, and `${\abs{a\abs{b}c}}` by height, since `half-length` gives
/// the outer bar the inner one's ink plus a gap and it is therefore strictly
/// taller.
///
/// An unpaired delimiter is dropped. `\cases`' `empty-paren` right side means
/// a left brace can legitimately have no partner, and inventing a closing
/// delimiter for it would guess at where the group ends.
fn pair_delims(delims: &mut [(Rect, DelimKind, Side)]) -> Vec<Structure> {
    delims.sort_by(|a, b| a.0.x0.total_cmp(&b.0.x0));
    let mut open: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    let same_height = |a: &Rect, b: &Rect| {
        (a.y0 - b.y0).abs() <= COINCIDE_PT && (a.y1 - b.y1).abs() <= COINCIDE_PT
    };
    for i in 0..delims.len() {
        let (rect, kind, side) = delims[i];
        let closes = match side {
            Side::Left => None,
            Side::Right => open
                .iter()
                .rposition(|&j| delims[j].2 == Side::Left && same_height(&delims[j].0, &rect)),
            Side::Symmetric => open.iter().rposition(|&j| {
                delims[j].2 == Side::Symmetric
                    && delims[j].1 == kind
                    && same_height(&delims[j].0, &rect)
            }),
        };
        match closes {
            Some(at) => {
                let j = open[at];
                open.truncate(at);
                out.push(Structure {
                    foot: (delims[j].0.x0, rect.x1),
                    body: (delims[j].0.x1, rect.x0),
                    y: (rect.y0 + rect.y1) / 2.0,
                    kind: StructKind::Delim {
                        open: named(delims[j].1),
                        close: named(kind),
                    },
                });
            }
            None => open.push(i),
        }
    }
    out
}

fn named(k: DelimKind) -> Option<DelimKind> {
    (k != DelimKind::Unknown).then_some(k)
}

// ---------------------------------------------------------------------------
// Building the tree
// ---------------------------------------------------------------------------

/// Recover one REGION — a set of glyphs, the structures that live in it, and
/// the baseline they are all measured against.
///
/// `base` stays the WHOLE RUN's base size all the way down, because it is what
/// the script SIZE test compares against: a fraction's numerator is typeset
/// smaller than the surrounding text, and measuring its scripts against its
/// own largest glyph would find scripts in ordinary characters. `baseline`,
/// by contrast, is per-region: it is the one thing a fraction's half really
/// does change.
fn build<'a>(
    glyphs: &[&'a MathGlyph],
    structures: &[Structure],
    base: f64,
    baseline: f64,
) -> Vec<Atom<'a>> {
    let mut live: Vec<Structure> = structures.to_vec();
    loop {
        match plan(glyphs, &live, base, baseline) {
            Ok(pieces) => return emit(pieces, base, baseline),
            // A bar with nothing on one side of it is not a fraction after
            // all. Dropping it and RE-RUNNING the assignment is what makes
            // the glyphs it wrongly claimed rejoin whatever else was there,
            // rather than being flushed to the top level: a radical's overbar
            // inside a fraction used to take the denominator's glyphs out of
            // the fraction entirely on its way past.
            Err(i) => {
                live.remove(i);
            }
        }
    }
}

/// One piece of a region in reading order, with the horizontal extent that
/// decides where it sorts and how wide the gap before it is.
struct Piece<'a> {
    x0: f64,
    x1: f64,
    what: What<'a>,
}

enum What<'a> {
    Glyph(&'a MathGlyph),
    Built(Atom<'a>),
}

/// Assign a region's glyphs and structures to the structures that enclose
/// them, recursing into each. `Err(i)` names a structure that turned out not
/// to be one and must be dropped before trying again.
fn plan<'a>(
    glyphs: &[&'a MathGlyph],
    structures: &[Structure],
    base: f64,
    baseline: f64,
) -> Result<Vec<Piece<'a>>, usize> {
    let outer: Vec<usize> = (0..structures.len())
        .filter(|&i| {
            !(0..structures.len()).any(|j| j != i && contains(&structures[j], &structures[i], baseline))
        })
        .collect();

    // Each glyph goes to the innermost outermost-structure whose BODY spans
    // it; a radical additionally has to have it under the overbar, because
    // the sign's own footprint reaches out to the left of the radicand and a
    // script sitting above the bar belongs to the radical's own base, not to
    // the radicand.
    let mut owner: Vec<Option<usize>> = vec![None; glyphs.len()];
    for (gi, g) in glyphs.iter().enumerate() {
        let centre = g.dx.0 + g.width.0 / 2.0;
        owner[gi] = outer
            .iter()
            .copied()
            .filter(|&i| {
                let s = &structures[i];
                centre >= s.body.0 && centre <= s.body.1 && glyph_fits(s, g)
            })
            .min_by(|&a, &b| {
                let wa = structures[a].body.1 - structures[a].body.0;
                let wb = structures[b].body.1 - structures[b].body.0;
                wa.total_cmp(&wb)
            });
    }

    let mut pieces: Vec<Piece<'a>> = Vec::new();
    for (gi, g) in glyphs.iter().enumerate() {
        if owner[gi].is_none() {
            pieces.push(Piece {
                x0: g.dx.0,
                x1: g.dx.0 + g.width.0,
                what: What::Glyph(g),
            });
        }
    }

    for &i in &outer {
        let s = structures[i];
        let mine: Vec<&'a MathGlyph> = glyphs
            .iter()
            .enumerate()
            .filter(|(gi, _)| owner[*gi] == Some(i))
            .map(|(_, g)| *g)
            .collect();
        let kids: Vec<Structure> = structures
            .iter()
            .enumerate()
            .filter(|(j, t)| *j != i && contains(&s, t, baseline))
            .map(|(_, t)| *t)
            .collect();
        let atom = match s.kind {
            StructKind::Frac => {
                let Some(atom) = frac_atom(&mine, &kids, base, s.y) else {
                    return Err(i);
                };
                atom
            }
            StructKind::Radical { .. } => Atom::Radical {
                body: build(&mine, &kids, base, baseline),
            },
            StructKind::Delim { open, close } => {
                let body = build(&mine, &kids, base, baseline);
                // Delimiters around NOTHING are dropped rather than written.
                // The body is empty when the run genuinely holds no readable
                // content — a `\pmatrix`'s cells are a `text-in-math` tabular,
                // which contributes advance and no glyph — and `\left( \right)`
                // there would say the document set an empty pair of
                // parentheses, which it did not. Losing the delimiters is the
                // pre-existing behaviour; inventing an empty group is not.
                if body.is_empty() {
                    continue;
                }
                Atom::Delim { open, close, body }
            }
        };
        pieces.push(Piece {
            x0: s.foot.0,
            x1: s.foot.1,
            what: What::Built(atom),
        });
    }
    pieces.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    Ok(pieces)
}

/// May this structure hold this glyph? Only a radical says no: everything
/// under its overbar is the radicand, everything above it is not.
fn glyph_fits(s: &Structure, g: &MathGlyph) -> bool {
    match s.kind {
        StructKind::Radical { bar_y0 } => g.dy.0 < bar_y0 - COINCIDE_PT,
        _ => true,
    }
}

/// Is `inner` nested inside `outer`?
///
/// Horizontal containment does nearly all of it, because every structure here
/// spans exactly the content it owns and siblings never overlap. The one
/// ambiguous case is a fraction inside a fraction whose halves are the same
/// width, where the two bars have IDENTICAL x extents — `${\frac{a}{\frac{b}
/// {c}}}` is exactly that — and x cannot say which way the nesting runs.
/// [`frac_rank`] settles it.
fn contains(outer: &Structure, inner: &Structure, baseline: f64) -> bool {
    if !(inner.foot.0 >= outer.body.0 - COINCIDE_PT && inner.foot.1 <= outer.body.1 + COINCIDE_PT) {
        return false;
    }
    match (outer.kind, inner.kind) {
        (StructKind::Frac, StructKind::Frac) => {
            let same_width =
                ((outer.body.1 - outer.body.0) - (inner.foot.1 - inner.foot.0)).abs()
                    <= COINCIDE_PT;
            !same_width || frac_rank(inner.y, baseline) > frac_rank(outer.y, baseline)
        }
        _ => true,
    }
}

/// How deeply nested a fraction bar at `y` is, within a region whose baseline
/// is `baseline` — smaller is further out.
///
/// A fraction's bar is drawn at its own baseline PLUS the axis height
/// (`layout_math_atom`: `rect_path((0, axis), (w, rule))`), so the outermost
/// fraction of a region has its bar a little way ABOVE the region's baseline
/// and nothing else does: a fraction nested in a numerator is lifted by
/// `frac_numer_shift`, which is at least `0.33 em` and always positive, and
/// one nested in a denominator is dropped by `frac_denom_shift`, which is at
/// least `0.33 em` and always exceeds the axis height (`0.25 em` with no MATH
/// table; `DenominatorShiftDown` well over `AxisHeight` in any real math
/// font). So "just above the baseline" identifies the outer bar, "further
/// above" a numerator's, and "below" a denominator's.
fn frac_rank(y: f64, baseline: f64) -> (u8, f64) {
    if y > baseline {
        (0, y - baseline)
    } else {
        (1, baseline - y)
    }
}

/// A fraction's two halves, or `None` when one of them is empty and the bar
/// was therefore not a fraction bar.
fn frac_atom<'a>(
    glyphs: &[&'a MathGlyph],
    kids: &[Structure],
    base: f64,
    bar_y: f64,
) -> Option<Atom<'a>> {
    let mut kids_above: Vec<Structure> = Vec::new();
    let mut kids_below: Vec<Structure> = Vec::new();
    for k in kids {
        let up = match k.kind {
            // A nested fraction's bar stands off from this one by exactly the
            // `frac_numer_shift`/`frac_denom_shift` that displaced the half it
            // is in, so which side of this bar it was drawn on is the answer.
            StructKind::Frac => k.y >= bar_y,
            // A radical or a delimited group sits ON its half's baseline and
            // so does its CONTENT — but the drawn parts do not: an overbar
            // clears its radicand, so a radical in a denominator can put one
            // above the fraction bar. The content is what is measured.
            _ => {
                let inside: Vec<f64> = glyphs
                    .iter()
                    .filter(|g| owns(k, g))
                    .map(|g| g.dy.0)
                    .collect();
                median(&inside).unwrap_or(k.y) >= bar_y
            }
        };
        if up {
            kids_above.push(*k);
        } else {
            kids_below.push(*k);
        }
    }

    // Full-size glyphs split AT THE BAR, including a nested fraction's own:
    // `frac_numer_shift`/`frac_denom_shift` clear the bar by the numerator's
    // depth and the denominator's height (`math.ml:574-594`), so whatever a
    // half contains — a plain letter or a whole fraction — every bit of its
    // ink is on its own side.
    //
    // (Only with NO MATH table does that stop being true: the fallback shift
    // is a flat `0.33 em` that ignores the half's extent, so a fraction nested
    // in a numerator hangs its own denominator across the bar and is read as
    // this fraction's. Reflecting the nested bar's standoff through this one
    // was tried as a fix for that and REVERTED: with a real MATH font the
    // numerator shift is three times the fallback, and the reflection then
    // reached far enough to swallow the denominator instead. The base-14
    // geometry is not separable, and a rule tuned to it costs the real one.)
    let local = glyphs
        .iter()
        .map(|g| g.info.size.0)
        .fold(0.0f64, f64::max)
        .max(1.0);
    let mut above: Vec<&'a MathGlyph> = Vec::new();
    let mut below: Vec<&'a MathGlyph> = Vec::new();
    let mut small: Vec<&'a MathGlyph> = Vec::new();
    for g in glyphs {
        if g.info.size.0 <= local * SCRIPT_SIZE_RATIO {
            // Script-size glyphs are the ones that need [`half_of_script`],
            // once the baselines it needs have been measured.
            small.push(g);
        } else if g.dy.0 >= bar_y {
            above.push(g);
        } else {
            below.push(g);
        }
    }

    let up_base = median(&above.iter().map(|g| g.dy.0).collect::<Vec<_>>());
    let down_base = median(&below.iter().map(|g| g.dy.0).collect::<Vec<_>>());
    for g in small {
        let up = match (up_base, down_base) {
            (Some(u), Some(d)) => half_of_script(g.dy.0, u, d, base),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => g.dy.0 >= bar_y,
        };
        if up {
            above.push(g);
        } else {
            below.push(g);
        }
    }
    if (above.is_empty() && kids_above.is_empty()) || (below.is_empty() && kids_below.is_empty()) {
        return None;
    }
    // With no full-size glyph of its own — a half whose whole content is a
    // nested structure — the half's baseline is unmeasurable here, and the
    // enclosing bar's own height is the closest thing to it. Nothing in such a
    // half is a loose script, so nothing reads it.
    let bn = up_base.unwrap_or(bar_y);
    let bd = down_base.unwrap_or(bar_y);
    Some(Atom::Frac {
        above: build(&above, &kids_above, base, bn),
        below: build(&below, &kids_below, base, bd),
    })
}

/// Would this structure hold this glyph, were the glyph in its half?
fn owns(s: &Structure, g: &MathGlyph) -> bool {
    let centre = g.dx.0 + g.width.0 / 2.0;
    centre >= s.body.0 && centre <= s.body.1 && glyph_fits(s, g)
}

/// Is a script-size glyph at `dy` the upper half's, or the lower's?
///
/// It cannot be settled by which side of the bar the glyph is on: in
/// `${\frac{a_1}{b^1}}` the numerator's subscript and the denominator's
/// superscript are BOTH below the bar, 1.08pt apart, at the same `dx`. What
/// does settle it is that the four slots — each half's superscript and
/// subscript — are at four different heights, because a superscript is set
/// about twice as far from its baseline as a subscript. So the glyph is
/// assigned to whichever half has a slot nearer to it.
fn half_of_script(dy: f64, up_base: f64, down_base: f64, base: f64) -> bool {
    let cost = |b: f64| {
        let rel = dy - b;
        (rel - base * SUP_SHIFT_RATIO)
            .abs()
            .min((rel + base * SUB_SHIFT_RATIO).abs())
    };
    cost(up_base) <= cost(down_base)
}

fn median(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    Some(v[v.len() / 2])
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

// ---------------------------------------------------------------------------
// Reading order
// ---------------------------------------------------------------------------

/// Fold one region's pieces into reading order, putting each script after the
/// base it belongs to and each measured gap where it was measured.
fn emit<'a>(pieces: Vec<Piece<'a>>, base: f64, baseline: f64) -> Vec<Atom<'a>> {
    let dir: Vec<Option<bool>> = pieces
        .iter()
        .map(|p| match &p.what {
            What::Glyph(g) => script_dir(g, base, baseline),
            What::Built(_) => None,
        })
        .collect();

    let line: Vec<usize> = (0..pieces.len()).filter(|&i| dir[i].is_none()).collect();
    let limits = limit_groups(&pieces, &dir, &line);
    let held: Vec<bool> = (0..pieces.len())
        .map(|i| limits.iter().any(|&(j, _)| j == i))
        .collect();

    let mut slots: Vec<Option<Piece<'a>>> = pieces.into_iter().map(Some).collect();
    let mut state = Emit {
        out: Vec::new(),
        prev_right: None,
        base_span: None,
    };
    for i in 0..slots.len() {
        if held[i] {
            continue;
        }
        push_piece(&mut state, &mut slots, i, dir[i], false, base);
        // A base run's limits are written straight after the run, wherever in
        // `dx` order they actually fell — which for a centred limit is
        // BEFORE some of the run's own glyphs. `${\lim_{x \to 0}}` is the
        // shape: its limit is wider than `lim`, so a plain `dx` walk emits
        // `l`, `x`, `i`, `→`, `m`, `0`.
        let mine: Vec<usize> = limits
            .iter()
            .filter(|&&(_, at)| at == i)
            .map(|&(j, _)| j)
            .collect();
        for j in mine {
            push_piece(&mut state, &mut slots, j, dir[j], true, base);
        }
    }
    state.out
}

struct Emit<'a> {
    out: Vec<Atom<'a>>,
    prev_right: Option<f64>,
    /// The advance of the most recent base-line piece — what the per-glyph
    /// fallback in [`push_piece`] measures a script's centre against.
    base_span: Option<(f64, f64)>,
}

fn push_piece<'a>(
    st: &mut Emit<'a>,
    slots: &mut [Option<Piece<'a>>],
    i: usize,
    dir: Option<bool>,
    limit: bool,
    base: f64,
) {
    let Some(p) = slots[i].take() else { return };
    if let Some(pr) = st.prev_right {
        if p.x0 - pr > base * SPACE_GAP_RATIO && !st.out.is_empty() {
            st.out.push(Atom::Space);
        }
    }
    match p.what {
        What::Glyph(g) => {
            let script = dir.map(|up| Script {
                up,
                // [`limit_groups`] finds a limit by the midpoint its base RUN
                // shares with it, which is what a multi-glyph operator needs.
                // A single-glyph base has a second, older witness that costs
                // nothing to keep: `Sup`/`Sub` set a script AT the base's
                // advance, so a script whose centre is still INSIDE that
                // advance was centred on it and is a limit.
                limit: limit
                    || st
                        .base_span
                        .is_some_and(|(lo, hi)| {
                            let c = p.x0 + (p.x1 - p.x0) / 2.0;
                            c > lo && c < hi
                        }),
            });
            if script.is_none() {
                st.base_span = Some((p.x0, p.x1));
            }
            st.out.push(Atom::Glyph { g, script });
        }
        What::Built(atom) => {
            st.base_span = Some((p.x0, p.x1));
            st.out.push(atom);
        }
    }
    st.prev_right = Some(p.x1);
}

/// Is this glyph a script, and which way? — `Some(true)` for a superscript or
/// upper limit, `Some(false)` for a subscript or lower limit.
///
/// Measured against `baseline`, the region's own, rather than against the
/// run's `dy = 0`. In a fraction's denominator the two differ by the whole
/// `frac_denom_shift`, which is enough to read `\frac{1}{x^2}`'s superscript
/// as a subscript.
fn script_dir(g: &MathGlyph, base: f64, baseline: f64) -> Option<bool> {
    if g.info.size.0 > base * SCRIPT_SIZE_RATIO {
        return None;
    }
    let shift = g.dy.0 - baseline;
    if shift > base * SCRIPT_SHIFT_RATIO {
        Some(true)
    } else if shift < -base * SCRIPT_SHIFT_RATIO {
        Some(false)
    } else {
        None
    }
}

/// Which script pieces are LIMITS, and after which base-line piece each should
/// be written.
///
/// `layout_math_list`'s `UpperLimit`/`LowerLimit` arms pad the narrower of the
/// base and the limit by half the difference (`center_offsets`), so the two
/// share a midpoint EXACTLY. Searching contiguous runs of base-line pieces for
/// that shared midpoint is what tells `\sum`'s bounds from `x²`, and — the
/// part a per-glyph test cannot do — what keeps `\lim`'s three letters
/// together, since `lim` is three glyph records and its limit is centred on
/// all three.
///
/// The overlap requirement is the guard against a coincidence: a limit is
/// centred on its base so the two always overlap horizontally, while an
/// ordinary script begins exactly where its base's advance ends and so never
/// does.
fn limit_groups(
    pieces: &[Piece<'_>],
    dir: &[Option<bool>],
    line: &[usize],
) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    if line.is_empty() {
        return out;
    }
    for up in [true, false] {
        let side: Vec<usize> = (0..pieces.len())
            .filter(|&i| dir[i] == Some(up))
            .collect();
        // Maximal runs, split wherever the next script does not begin where
        // the previous one left off. The tolerance is float noise and not a
        // gap allowance: a limit's glyphs are laid out consecutively, so each
        // begins at the previous one's advance — but `x1` is recomputed here
        // as `dx + width` while `dx` came out of the layout's own running
        // sum, and the two agree only to the last bit. An exact test split
        // `\sum_{k=0}` after the `=`, which cost the whole run its limit.
        let mut start = 0;
        while start < side.len() {
            let mut end = start + 1;
            while end < side.len()
                && pieces[side[end]].x0 <= pieces[side[end - 1]].x1 + COINCIDE_PT
            {
                end += 1;
            }
            let run = &side[start..end];
            let r0 = run.iter().map(|&i| pieces[i].x0).fold(f64::MAX, f64::min);
            let r1 = run.iter().map(|&i| pieces[i].x1).fold(f64::MIN, f64::max);
            if let Some(at) = centred_on(pieces, line, (r0 + r1) / 2.0, r0, r1) {
                for &i in run {
                    out.push((i, at));
                }
            }
            start = end;
        }
    }
    out
}

/// The last piece of the contiguous base-line run whose midpoint is `centre`
/// and whose INK the span `[s0, s1]` overlaps, if there is one.
///
/// Both conditions are load-bearing, and the second is the one that took two
/// attempts to get right. A midpoint can coincide by arithmetic without
/// meaning anything: in `${x^2+y^2+z^2-xy-yz-zx}` the `z`'s superscript sits
/// at the exact midpoint of the two-glyph run `z −`, and in
/// `${\frac{4k^2}{4k^2-1}}` the denominator's superscript sits at the midpoint
/// of `4k−`. Overlapping the run's SPAN does not rule either out — the second
/// one lands squarely in the gap the binary operator's spacing opens between
/// `k` and `−`. Overlapping a member PIECE does rule both out, and rules
/// nothing real out with it: a limit is centred on a run of adjacent glyphs,
/// which has no internal gap for it to hide in.
fn centred_on(
    pieces: &[Piece<'_>],
    line: &[usize],
    centre: f64,
    s0: f64,
    s1: f64,
) -> Option<usize> {
    for a in 0..line.len() {
        for b in a..line.len() {
            let x0 = pieces[line[a]].x0;
            let x1 = pieces[line[b]].x1;
            if ((x0 + x1) / 2.0 - centre).abs() > COINCIDE_PT {
                continue;
            }
            let touches = line[a..=b]
                .iter()
                .any(|&i| s0 < pieces[i].x1 && s1 > pieces[i].x0);
            if touches {
                return Some(line[b]);
            }
        }
    }
    None
}

/// `pub(crate)` so the three WRITERS' own test modules can build a `MathGlyph`
/// through [`tests::glyph`] instead of each repeating the ten-field literal —
/// and, now that delimiters are recovered from paths, so they can build those
/// too ([`tests::paren`] and friends). The recovery is shared, so the fixtures
/// for it should be.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rustyfi_backend::{Color, FontKey, HorzStringInfo, Subpath};

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

    /// [`glyph`] with a real advance rather than the fixture's half-size one
    /// — needed wherever a test's point is a MIDPOINT, since a limit and its
    /// base share theirs exactly and an invented width would not.
    pub(crate) fn wide(text: &str, dx: f64, dy: f64, size: f64, width: f64) -> MathGlyph {
        MathGlyph {
            width: Length(width),
            ..glyph(text, dx, dy, size)
        }
    }

    fn pt(x: f64, y: f64) -> (Length, Length) {
        (Length(x), Length(y))
    }

    fn closed(start: (f64, f64), rest: &[(f64, f64)]) -> Subpath {
        Subpath {
            start: pt(start.0, start.1),
            segs: rest.iter().map(|&(x, y)| PathSeg::Line(pt(x, y))).collect(),
            closing: Closing::Line,
        }
    }

    fn fill(subpaths: Vec<Subpath>) -> GraphicsElem {
        GraphicsElem::Fill(Color::Gray(0.0), Path { subpaths })
    }

    /// A horizontal rule spanning `x0..x1` at `y` — a fraction bar, and the
    /// same shape a radical's overbar has.
    pub(crate) fn bar(x0: f64, x1: f64, y: f64) -> GraphicsElem {
        fill(vec![closed(
            (x0, y),
            &[(x1, y), (x1, y + 0.5), (x0, y + 0.5)],
        )])
    }

    /// One half of a `\paren` pair, as `math.satyh:1071`'s `paren-left` draws
    /// it: one closed outline of cubics whose outer edge reaches the axis and
    /// whose tips reach the other side. `left = false` mirrors it.
    pub(crate) fn paren(x0: f64, x1: f64, y0: f64, y1: f64, left: bool) -> GraphicsElem {
        let ymid = (y0 + y1) / 2.0;
        let (outer, inner, tip) = if left {
            (x0, x0 + (x1 - x0) * 0.33, x1)
        } else {
            (x1, x1 - (x1 - x0) * 0.33, x0)
        };
        let c = |x: f64, y: f64| pt(x, y);
        GraphicsElem::Fill(
            Color::Gray(0.0),
            Path {
                subpaths: vec![Subpath {
                    start: c(tip, y1),
                    segs: vec![
                        PathSeg::Bezier(c(outer, y1), c(outer, ymid), c(outer, ymid)),
                        PathSeg::Bezier(c(outer, ymid), c(outer, y0), c(tip, y0)),
                        PathSeg::Bezier(c(inner, y0), c(inner, ymid), c(inner, ymid)),
                        PathSeg::Bezier(c(inner, ymid), c(inner, y1), c(tip, y1)),
                    ],
                    closing: Closing::Line,
                }],
            },
        )
    }

    /// `math.satyh:1392`'s `bracket-path`: a rectilinear `[` with a
    /// full-width arm at each end. `left = false` mirrors it.
    pub(crate) fn sqbracket(x0: f64, x1: f64, y0: f64, y1: f64, left: bool) -> GraphicsElem {
        let t = (y1 - y0) * 0.05;
        let (stem, mid, arm) = if left {
            (x0, x0 + (x1 - x0) * 0.25, x1)
        } else {
            (x1, x1 - (x1 - x0) * 0.25, x0)
        };
        fill(vec![closed(
            (arm, y1),
            &[
                (stem, y1),
                (stem, y0),
                (arm, y0),
                (arm, y0 + t),
                (mid, y0 + t),
                (mid, y1 - t),
                (arm, y1 - t),
            ],
        )])
    }

    /// `math.satyh:1479`'s `abs-left`: one stroked vertical rule, the same
    /// shape at both ends of the pair.
    pub(crate) fn absbar(x: f64, y0: f64, y1: f64) -> GraphicsElem {
        GraphicsElem::Stroke(
            Length(0.5),
            Color::Gray(0.0),
            Path {
                subpaths: vec![Subpath {
                    start: pt(x, y1),
                    segs: vec![PathSeg::Line(pt(x, y0))],
                    closing: Closing::Line,
                }],
            },
        )
    }

    /// A radical, as `layout_math_atom`'s `Math::Radical` arm emits it: a
    /// checkmark `Fill` whose right edge and top edge coincide EXACTLY with
    /// the overbar's left and top, plus that overbar.
    pub(crate) fn radical(
        sign_x0: f64,
        bar_x0: f64,
        bar_x1: f64,
        bar_y: f64,
        depth: f64,
    ) -> Vec<GraphicsElem> {
        let top = bar_y + 0.5;
        vec![
            fill(vec![closed(
                (bar_x0, top),
                &[
                    (sign_x0 + (bar_x0 - sign_x0) * 0.6, -depth),
                    (sign_x0, -depth + (top + depth) * 0.4),
                    (sign_x0 + (bar_x0 - sign_x0) * 0.5, bar_y),
                ],
            )]),
            bar(bar_x0, bar_x1, bar_y),
        ]
    }

    /// A recovered tree as one line, so a test can state the whole shape it
    /// expects rather than pattern-matching three levels down. `^`/`_` mark a
    /// script and `^^`/`__` a limit.
    fn kinds(atoms: &[Atom<'_>]) -> String {
        let mut out = String::new();
        for a in atoms {
            match a {
                Atom::Glyph { g, script } => {
                    out.push_str(&g.text);
                    match script {
                        Some(Script { up: true, limit }) => {
                            out.push_str(if *limit { "^^" } else { "^" })
                        }
                        Some(Script { up: false, limit }) => {
                            out.push_str(if *limit { "__" } else { "_" })
                        }
                        None => {}
                    }
                }
                Atom::Frac { above, below } => {
                    out.push_str(&format!("frac({}|{})", kinds(above), kinds(below)))
                }
                Atom::Delim { open, close, body } => {
                    out.push_str(&format!("delim({open:?},{close:?}|{})", kinds(body)))
                }
                Atom::Radical { body } => out.push_str(&format!("sqrt({})", kinds(body))),
                Atom::Space => out.push(' '),
            }
        }
        out
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

    /// A bar with glyphs on only one side is not a fraction — an underline
    /// rule is the shape that reaches this — and its glyphs stay in the flat
    /// run rather than becoming an empty denominator.
    #[test]
    fn a_one_sided_bar_is_not_a_fraction() {
        // Both glyphs sit ABOVE the bar, so there is no denominator.
        let gs = vec![glyph("a", 2.0, 8.0, 10.0), glyph("b", 8.0, 8.0, 10.0)];
        let atoms = recover(&gs, &[bar(0.0, 20.0, 5.0)]);
        assert_eq!(atoms.len(), 2, "no fraction should have been formed");
        assert!(atoms.iter().all(|a| matches!(a, Atom::Glyph { .. })));
    }

    /// The defect this module was reopened for: `\paren` draws its delimiters
    /// as paths, so `${\paren{a+b}^2}` used to recover as `a+b^{2}` — not an
    /// ugly rendering of the formula but a different and false one.
    #[test]
    fn a_drawn_paren_comes_back_as_a_group() {
        let gs = vec![
            glyph("a", 5.0, 0.0, 10.0),
            glyph("+", 11.0, 0.0, 10.0),
            glyph("b", 17.0, 0.0, 10.0),
            glyph("2", 27.0, 5.0, 7.0),
        ];
        let rules = vec![
            paren(1.0, 4.0, -4.0, 9.0, true),
            paren(23.0, 26.0, -4.0, 9.0, false),
        ];
        assert_eq!(
            kinds(&recover(&gs, &rules)),
            "delim(Some(Paren),Some(Paren)|a+b)2^"
        );
    }

    /// Two groups side by side, of identical height — the shape that proves
    /// the pairing reads each delimiter's own HANDEDNESS rather than merely
    /// alternating: `${\paren{a+b}\paren{a-b}}`.
    #[test]
    fn two_sibling_groups_of_the_same_height_pair_up_correctly() {
        let gs = vec![
            glyph("a", 5.0, 0.0, 10.0),
            glyph("+", 11.0, 0.0, 10.0),
            glyph("b", 17.0, 0.0, 10.0),
            glyph("a", 31.0, 0.0, 10.0),
            glyph("-", 37.0, 0.0, 10.0),
            glyph("b", 43.0, 0.0, 10.0),
        ];
        let rules = vec![
            paren(1.0, 4.0, -4.0, 9.0, true),
            paren(23.0, 26.0, -4.0, 9.0, false),
            paren(27.0, 30.0, -4.0, 9.0, true),
            paren(49.0, 52.0, -4.0, 9.0, false),
        ];
        assert_eq!(
            kinds(&recover(&gs, &rules)),
            "delim(Some(Paren),Some(Paren)|a+b)delim(Some(Paren),Some(Paren)|a-b)"
        );
    }

    /// Each delimiter family has its own signature; a `[` must not come back
    /// as a `(`, and a bar with no handedness has to pair by alternation.
    #[test]
    fn the_delimiter_families_are_told_apart_by_shape() {
        let gs = vec![glyph("a", 6.0, 0.0, 10.0)];
        let square = vec![
            sqbracket(1.0, 4.0, -4.0, 9.0, true),
            sqbracket(12.0, 15.0, -4.0, 9.0, false),
        ];
        assert_eq!(
            kinds(&recover(&gs, &square)),
            "delim(Some(Bracket),Some(Bracket)|a)"
        );
        let bars = vec![absbar(2.0, -4.0, 9.0), absbar(13.0, -4.0, 9.0)];
        assert_eq!(kinds(&recover(&gs, &bars)), "delim(Some(Abs),Some(Abs)|a)");
    }

    /// `\sqrt` used to be listed as unrecoverable, and a radical INSIDE a
    /// fraction was worse than that: its overbar was read as a second, narrower
    /// fraction bar, which claimed the denominator's glyphs on its way past and
    /// left the quadratic formula as `x=\frac{-b\pm}{2}ba^{2}-4ac`.
    #[test]
    fn a_radical_inside_a_fraction_keeps_the_denominator() {
        // `\frac{-b + \sqrt{c}}{2a}`, with the radical in the numerator and
        // the fraction bar spanning the whole thing.
        let gs = vec![
            glyph("-", 0.0, 8.0, 10.0),
            glyph("b", 6.0, 8.0, 10.0),
            glyph("+", 12.0, 8.0, 10.0),
            glyph("c", 24.0, 8.0, 10.0),
            glyph("2", 8.0, -8.0, 10.0),
            glyph("a", 14.0, -8.0, 10.0),
        ];
        let mut rules = vec![bar(0.0, 30.0, 3.0)];
        rules.extend(radical(18.0, 23.0, 30.0, 16.0, 4.0));
        assert_eq!(kinds(&recover(&gs, &rules)), "frac(-b+sqrt(c)|2a)");
    }

    /// `\frac{a}{\frac{b}{c}}` is `a/(b/c)`, and the two bars have IDENTICAL
    /// horizontal extents, so which contains which cannot be read off x at
    /// all. Getting it backwards produced `\frac{ab}{c}` — a different number.
    #[test]
    fn a_fraction_nested_in_a_denominator_keeps_its_direction() {
        // The measured 12pt geometry of `${\frac{a}{\frac{b}{c}}}` under
        // `latinmodern-math`: the outer bar sits at the axis, 3.240, and the
        // inner one is dropped a whole `frac_denom_shift` below the baseline.
        let gs = vec![
            wide("a", 0.0, 11.472, 12.0, 6.35),
            wide("b", 0.0, -4.812, 12.0, 5.15),
            wide("c", 0.0, -17.976, 12.0, 5.2),
        ];
        let rules = vec![bar(0.0, 6.35, 3.24), bar(0.0, 6.35, -9.804)];
        assert_eq!(kinds(&recover(&gs, &rules)), "frac(a|frac(b|c))");
        // …and the mirror image, `${\frac{\frac{a}{b}}{c}}`, which was right
        // before only by the accident that `(a/b)/c` and `a/(b/c)` agree when
        // the flattening goes the other way. The two bars have IDENTICAL
        // horizontal extents in both, so only [`frac_rank`] tells them apart.
        let gs = vec![
            wide("a", 0.0, 21.168, 12.0, 6.35),
            wide("b", 0.6, 4.812, 12.0, 5.15),
            wide("c", 0.58, -8.232, 12.0, 5.2),
        ];
        let rules = vec![bar(0.0, 6.35, 3.24), bar(0.0, 6.35, 16.284)];
        assert_eq!(kinds(&recover(&gs, &rules)), "frac(frac(a|b)|c)");
    }

    /// A fraction half sits on a displaced baseline, so a script inside one
    /// has to be measured against THAT baseline: against the run's own `dy`
    /// the numerator's subscript reads as neither, and lands in the
    /// denominator.
    #[test]
    fn a_script_inside_a_fraction_is_measured_against_its_own_half() {
        // `\frac{a_1}{b^1}`: the numerator's subscript (dy 0.8) and the
        // denominator's superscript (dy 1.7) are BOTH below the bar and
        // 0.9pt apart. Only the shift MAGNITUDES separate them.
        let gs = vec![
            glyph("a", 0.0, 3.3, 10.0),
            glyph("1", 5.0, 0.8, 7.0),
            glyph("b", 0.0, -3.3, 10.0),
            glyph("1", 5.0, 1.7, 7.0),
        ];
        let rules = vec![bar(0.0, 9.0, 2.7)];
        assert_eq!(kinds(&recover(&gs, &rules)), "frac(a1_|b1^)");
    }

    /// A centred limit starts to the LEFT of a multi-glyph operator, so a
    /// plain `dx` walk interleaves the two: `${\lim_{x \to 0}}` came out as
    /// `l_{x}i_{\to}m_{0}`.
    #[test]
    fn a_centred_limit_does_not_interleave_with_its_operator() {
        // `${\lim_{x \to 0}}` at 12pt, as the layout really places it: `lim`
        // spans [0, 15.324] and the limit `x→0` spans [1.127, 14.197], which
        // share the midpoint 7.662 to the last bit — `center_offsets`
        // guarantees that, and it is the whole basis of the recovery.
        let mut gs = vec![
            wide("l", 0.0, 0.0, 12.0, 2.664),
            wide("i", 2.664, 0.0, 12.0, 2.664),
            wide("m", 5.328, 0.0, 12.0, 9.996),
        ];
        for (t, dx, w) in [("x", 1.127, 4.2), ("\u{2192}", 5.327, 4.2), ("0", 9.527, 4.67)] {
            gs.push(wide(t, dx, -3.0, 8.4, w));
        }
        assert_eq!(kinds(&recover(&gs, &[])), "limx__\u{2192}__0__");
    }

    /// A midpoint can COINCIDE by arithmetic without meaning anything, so the
    /// centre search alone is not enough — it has to touch the base's ink.
    /// Both of these are real, and both came out of `azmath`.
    #[test]
    fn a_midpoint_coincidence_is_not_a_limit() {
        // `${x^2+y^2+z^2-xy-yz-zx}`: `2`'s centre (8.335) is the midpoint of
        // the run `z −` ([0, 16.67]) exactly. Read as that run's limit, the
        // identity came out as `z-x^{2}\ y`.
        let gs = vec![
            wide("z", 0.0, 0.0, 12.0, 6.0),
            wide("2", 6.0, 6.0, 8.4, 4.67),
            wide("-", 13.67, 0.0, 12.0, 3.0),
            wide("x", 16.67, 0.0, 12.0, 6.0),
        ];
        assert_eq!(kinds(&recover(&gs, &[])), "z2^ -x");

        // `${\frac{4k^2}{4k^2-1}}`'s denominator, measured: `2`'s centre
        // (14.822) is the midpoint of `4 k −` ([0, 29.548]) to within 0.048pt,
        // and — the part the earlier "starts at its base's advance" guard
        // missed — the italic correction after `k` means it does not start at
        // an advance either. It sits in the GAP the binary operator's spacing
        // opens, touching neither `k` nor `−`.
        let gs = vec![
            wide("4", 0.0, 0.0, 12.0, 6.0),
            wide("k", 6.0, 0.0, 12.0, 6.252),
            wide("2", 12.432, 5.328, 8.4, 4.78),
            wide("\u{2212}", 20.212, 0.0, 12.0, 9.336),
            wide("1", 32.548, 0.0, 12.0, 6.0),
        ];
        // (The two spaces are the binary operator's own inter-atom spacing;
        // `latex::latex_spaces_itself` drops them again, so what `--katex`
        // writes is `4k^{2}-1`.)
        assert_eq!(kinds(&recover(&gs, &[])), "4k2^ \u{2212} 1");
    }

    /// A delimiter drawn as a STROKED polyline rather than a filled outline —
    /// `azmath`'s `\pB` (`parens.satyh:303`) — is the same bracket and must
    /// read as one. It used to fall through to `Unknown`.
    #[test]
    fn a_stroked_bracket_is_still_a_bracket() {
        let arm = |x0: f64, x1: f64, left: bool| {
            let (stem, tip) = if left { (x0, x1) } else { (x1, x0) };
            GraphicsElem::Stroke(
                Length(0.5),
                Color::Gray(0.0),
                Path {
                    subpaths: vec![Subpath {
                        start: pt(tip, 9.0),
                        segs: vec![
                            PathSeg::Line(pt(stem, 9.0)),
                            PathSeg::Line(pt(stem, -4.0)),
                            PathSeg::Line(pt(tip, -4.0)),
                        ],
                        closing: Closing::Open,
                    }],
                },
            )
        };
        let gs = vec![glyph("a", 6.0, 0.0, 10.0)];
        let rules = vec![arm(1.0, 4.0, true), arm(12.0, 15.0, false)];
        assert_eq!(
            kinds(&recover(&gs, &rules)),
            "delim(Some(Bracket),Some(Bracket)|a)"
        );
    }

    /// The control for the one above: a big operator with narrow limits is
    /// already recovered correctly and must stay so, INCLUDING the ordinary
    /// superscript that follows it and is not a limit.
    #[test]
    fn a_big_operator_with_its_limits_and_a_following_script_is_unchanged() {
        // `${\sum_{k=1}^{n} k^2}` at 12pt, again at the measured positions.
        // The lower limit is wider than `∑` and so begins to its left, which
        // is why a plain `dx` walk used to write `{}_{k}\sum\limits_{=1}^{n}`.
        let gs = vec![
            wide("\u{2211}", 3.888, 0.0, 12.0, 6.0),
            wide("k", 0.0, -3.0, 8.4, 4.2),
            wide("=", 4.2, -3.0, 8.4, 4.906),
            wide("1", 9.106, -3.0, 8.4, 4.67),
            wide("n", 4.553, 6.0, 8.4, 4.67),
            wide("k", 15.276, 0.0, 12.0, 6.0),
            wide("2", 21.276, 6.0, 8.4, 4.67),
        ];
        assert_eq!(kinds(&recover(&gs, &[])), "\u{2211}n^^k__=__1__k2^");
    }
}

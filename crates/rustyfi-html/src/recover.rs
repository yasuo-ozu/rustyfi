//! Structure recovery shared by the two output backends.
//!
//! `--format markdown` is a SUBSET of `--format html`: it recovers the same
//! headings, lists, tables, links and emphasis out of the same flat box
//! stream, and then writes less. The recovery itself must therefore be ONE
//! implementation, not two — every rule collected here was got wrong at least
//! once before it was got right, and a forked copy would rot silently the
//! next time one of them is corrected:
//!
//! - [`wants_space`] — glue is not a space. The box stream puts glue between
//!   every pair of CJK characters, so "glue means space" renders Japanese as
//!   `研 究 計 画`. `reflow/text.rs`'s doc comment has the full argument.
//! - [`line_join`] — a `VertBox::Line` boundary is the LINE BREAKER's
//!   decision, so it is undone; but the hyphen at the end of the line may be
//!   the AUTHOR's, and deleting it turns `code-printer` into `codeprinter`.
//!   `InlineMarkKind::BreakHyphen` is what tells the two apart.
//! - [`find_heading_level`] — a heading is found by correlating
//!   `extras.outline` to a destination frame through `dest_name`, a
//!   STRUCTURAL id match. `reflow/structure.rs`'s doc comment explains why a
//!   font-size guess was refused, and why `InlineFrameMarker` (not just
//!   `Frame`) is what every bundled doc class actually emits.
//! - [`table_rows`] — a `TabularBox` carries no grid, only cells; rows are
//!   recovered from each cell's `x`.
//! - [`Borders`] — and no grid LINES either; which boundaries a table
//!   actually rules is recovered from the shapes in `TabularBox::rules`.
//! - [`is_monospace`] — the one signal that says a `Line` boundary is the
//!   AUTHOR's rather than the breaker's (a `+code` block and a wrapped
//!   paragraph are structurally identical otherwise).
//!
//! Nothing here emits markup. Each function answers a question about the box
//! stream and lets its caller decide what to write, which is exactly the line
//! between "the same document structure" and "two different serializations of
//! it".

use std::collections::HashMap;

use rustyfi_backend::{
    graphics_bbox, Color, DecoId, FontKey, GraphicsElem, Length, OutlineEntry, PureHorzBox,
    TabularBox, TabularCellBox,
};
use rustyfi_pdf::TtfFontStore;

/// Anything below this (in pt) counts as a zero-width glue — a break
/// opportunity or a kern, never spacing. Deliberately a hair above 0 rather
/// than an exact comparison: the CJK pair glue's natural part is computed
/// through several `f64` multiplications, and a 0.01pt residue is not a
/// space.
const GLUE_EPSILON_PT: f64 = 0.05;

/// Is `c` set solid, with no inter-character space — Han, kana, Hangul, the
/// CJK punctuation block and the full-width forms? Used only by
/// [`wants_space`]; a false negative costs one spurious space and a false
/// positive one missing one, so the ranges are the broad blocks rather than
/// a Unicode script database.
pub(crate) fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF     // Hangul Jamo
        | 0x2E80..=0x2EFF   // CJK radicals supplement
        | 0x3000..=0x303F   // CJK symbols and punctuation (incl. U+3000)
        | 0x3040..=0x30FF   // Hiragana + Katakana
        | 0x3100..=0x312F   // Bopomofo
        | 0x3190..=0x319F   // Kanbun
        | 0x31F0..=0x31FF   // Katakana phonetic extensions
        | 0x3400..=0x4DBF   // CJK ext A
        | 0x4E00..=0x9FFF   // CJK unified ideographs
        | 0xAC00..=0xD7AF   // Hangul syllables
        | 0xF900..=0xFAFF   // CJK compatibility ideographs
        | 0xFF00..=0xFF60   // full-width forms
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x2FA1F // CJK ext B..F + compatibility supplement
    )
}

/// Does a glue box of natural width `natural_pt`, sitting between `prev` and
/// `next`, become a space? See `reflow/text.rs`'s doc comment for the
/// reasoning behind each clause. `prev`/`next` are `None` at a paragraph edge
/// and either side of an opaque box (a drawing, an image, a table), which
/// count as non-CJK: a formula or figure set into Japanese prose takes the
/// same space its inter-script glue was asking for.
pub(crate) fn wants_space(prev: Option<char>, next: Option<char>, natural_pt: f64) -> bool {
    if natural_pt <= GLUE_EPSILON_PT {
        return false;
    }
    // Nothing to separate FROM: a leading space is trimmed by the paragraph
    // flush anyway, and emitting one here would defeat that trim inside a
    // table cell.
    let Some(p) = prev else { return false };
    !(is_cjk(p) && next.is_some_and(is_cjk))
}

/// What to do at a `VertBox::Line` boundary inside one paragraph — the
/// decision both backends make, spelled once.
///
/// A `Line` boundary is the PORT's own wrapping decision, never the author's
/// (except inside a code block, which both callers detect with
/// [`is_monospace`] and handle before asking). So the two lines are rejoined;
/// the only question is what happens to the hyphen at the join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineJoin {
    /// An ordinary wrap: rejoin with a word space (which the CJK rule may
    /// still suppress — the two characters either side of the break must not
    /// gain one).
    Space,
    /// The LINE BREAKER hyphenated here (`InlineMarkKind::BreakHyphen`).
    /// Delete its hyphen and rejoin the word, with no space.
    DropHyphen,
    /// The line ends with a hyphen the AUTHOR typed, which the breaker was
    /// merely allowed to break after (UAX#14 permits it). The hyphen STAYS —
    /// but the two halves of `code-printer` must not gain a space between
    /// them.
    KeepHyphen,
}

/// Classify a line boundary. `break_hyphen` is whether an
/// `InlineMarkKind::BreakHyphen` was seen on the line just closed;
/// `ends_with_hyphen` is whether the accumulated text ends in one.
///
/// The marker is what makes this exact. The breaker's splice
/// (`linebreak::line_content`) produces an ordinary `InnerString`, so before
/// the marker existed the only available test was the SHAPE of the text —
/// "ends letter+hyphen, next line opens lowercase" — which is also the shape
/// of an authored compound at a line end, and deleted real hyphens.
pub(crate) fn line_join(break_hyphen: bool, ends_with_hyphen: bool) -> LineJoin {
    if break_hyphen {
        LineJoin::DropHyphen
    } else if ends_with_hyphen {
        LineJoin::KeepHyphen
    } else {
        LineJoin::Space
    }
}

/// Is this character the hyphen a line break can fall after? Both backends
/// need the same answer, and it is not just ASCII `-`.
pub(crate) fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{2010}')
}

/// A plain inter-word space, in points, standing in for the line break
/// between two consecutive `Line`s of one paragraph. The exact value is
/// immaterial — it only has to be above [`wants_space`]'s zero-width
/// threshold, since the decision it feeds is "is this a CJK pair" rather than
/// "how wide".
pub(crate) const WORD_SPACE_PT: f64 = 3.0;

/// Whether `font` is a fixed-pitch face, read off the family name the file
/// declares (`fonts::is_monospace_family` — a name heuristic, and labelled as
/// one there). `false` in base-14 mode, where there is no file to ask.
///
/// This is not a styling nicety in either backend: it is the ONLY signal in
/// the box stream that distinguishes a line boundary the renderer should
/// REDO from one it must KEEP. A wrapped paragraph and a `+code` block are
/// structurally identical — `code.satyh` calls `line-break` once per source
/// line exactly as the line breaker does per wrapped line.
pub(crate) fn is_monospace(store: Option<&TtfFontStore>, font: Option<FontKey>) -> bool {
    let (Some(store), Some(font)) = (store, font) else {
        return false;
    };
    store
        .file_family_name(store.file_index(font))
        .is_some_and(|f| crate::fonts::is_monospace_family(&f))
}

/// `dest_name -> level` from `extras.outline` (`register-outline`'s already
/// `Interp::dest_name`-resolved entries) — the lookup table
/// [`find_heading_level`] consults.
pub(crate) fn outline_levels(outline: &[OutlineEntry]) -> HashMap<String, i64> {
    outline
        .iter()
        .map(|entry| (entry.dest_name.clone(), entry.level))
        .collect()
}

/// `register-outline`'s `level` is 0-based (`+section` registers level 0,
/// `+subsection` level 1 — `stdjabook.satyh:548`/`:573`); a heading DEPTH is
/// 1-based and capped at 6, which is both HTML's `<h1>`..`<h6>` and the
/// deepest `######` ATX heading Markdown defines. A deeper-than-6 outline
/// (unusual, but upstream never validates outline depth) collapses onto 6
/// rather than producing something invalid in either format.
pub(crate) fn heading_depth(level: i64) -> u8 {
    (level.max(0) as u64 + 1).min(6) as u8
}

/// Does `bx` (or, recursively, one of its `Frame` descendants) carry the
/// `DecoId` of a `register-location-frame`/`register-destination` call whose
/// resolved name matches a `register-outline` entry? Returns that entry's
/// level on the first match. See `reflow/structure.rs`'s doc comment for why
/// this is a structural match rather than a font-size heuristic.
///
/// **`InlineFrameMarker` is checked too, and that is what makes this work at
/// all on a real document.** `inline-frame-breakable` splices its contents
/// between a marker PAIR rather than building a `Frame`, so that the frame
/// can split across a line break — and that is how every bundled doc class
/// writes a section title (`stdjabook.satyh:551`, `stdjareport.satyh:445`).
/// Matching only `Frame` meant no heading in any `stdjabook` document was
/// ever promoted. Only the START marker is consulted — the `end: true` twin
/// carries the same `DecoId` and would match a second time for nothing.
pub(crate) fn find_heading_level(
    bx: &PureHorzBox,
    dests: &HashMap<DecoId, &str>,
    outline_by_dest: &HashMap<String, i64>,
) -> Option<i64> {
    match bx {
        PureHorzBox::InlineFrameMarker { id, end: false, .. } => {
            level_of_deco(id, dests, outline_by_dest)
        }
        PureHorzBox::Frame { deco, contents, .. } => {
            level_of_deco(deco, dests, outline_by_dest).or_else(|| {
                contents
                    .iter()
                    .find_map(|(_, inner)| find_heading_level(inner, dests, outline_by_dest))
            })
        }
        _ => None,
    }
}

/// `DecoId` -> destination name -> outline level, the two-hop structural
/// lookup both arms of [`find_heading_level`] share.
fn level_of_deco(
    deco: &DecoId,
    dests: &HashMap<DecoId, &str>,
    outline_by_dest: &HashMap<String, i64>,
) -> Option<i64> {
    let name = dests.get(deco)?;
    outline_by_dest.get(*name).copied()
}

/// Regroup a solved `TabularBox`'s flat cell list back into rows.
///
/// Recovered from `TabularCellBox::x` alone: `TabularBox` does not carry the
/// solver's `xs`/`ys` grid-line lists (those exist only on the transient
/// `tabular::Solved` the lang-side rule callback consumes), but
/// `tabular::solidify_tabular` pushes cells in strict row-major order (outer
/// loop over rows, inner over columns, `Cell::Empty` slots producing no entry
/// at all) — so within one row, `x` (each cell's box-local left edge) is
/// monotonically non-decreasing, and a new row begins exactly when `x` fails
/// to increase. This is exact for the common case; a pathological grid whose
/// first visible cell in a row happens to sit further right than the previous
/// row's last visible cell would mis-group.
pub(crate) fn table_rows(tab: &TabularBox) -> Vec<Vec<&TabularCellBox>> {
    let mut rows: Vec<Vec<&TabularCellBox>> = Vec::new();
    let mut last_x: Option<f64> = None;
    for cell in &tab.cells {
        let x = cell.x.0;
        let starts_new_row = match last_x {
            None => true,
            Some(lx) => x <= lx,
        };
        if starts_new_row {
            rows.push(Vec::new());
        }
        rows.last_mut().expect("just pushed if empty").push(cell);
        last_x = Some(x);
    }
    rows
}

/// Which grid lines a table actually draws, recovered from
/// `TabularBox::rules`.
///
/// A stylesheet cannot know this, and neither can a column specification.
/// `rules` is whatever the document's own rule callback drew, and the
/// conventions differ completely: `easytable`'s default draws three
/// horizontal rules and no verticals (the booktabs look), while a
/// `\easytable` with explicit column separators draws a full grid. Giving
/// every cell the same border made the first render as the second, and no
/// table in the corpus looked like its PDF.
///
/// The rules are ordinary graphics — thin filled rectangles or strokes — so
/// each one's bounding box says where it lies, and its position against the
/// cell origins says which boundary it is. Rules the geometry cannot place
/// (a diagonal, a decorative flourish) are simply not reproduced; they draw
/// nothing rather than something wrong.
pub(crate) struct Borders {
    /// `horizontal[r]` is the rule ABOVE row `r`; the extra last entry is the
    /// rule below the final row.
    pub(crate) horizontal: Vec<Option<Rule>>,
    /// `vertical[c]` is the rule LEFT of column `c`; the extra last entry is
    /// the rule right of the final column.
    pub(crate) vertical: Vec<Option<Rule>>,
}

/// One recovered grid line: how thick, and in what colour.
#[derive(Clone, Copy)]
pub(crate) struct Rule {
    pub(crate) width: f64,
    pub(crate) color: Color,
}

/// A rule thinner than this (pt) is invisible in a browser anyway; a
/// coordinate closer than this to a boundary counts as being on it.
pub(crate) const RULE_EPS_PT: f64 = 0.05;

impl Borders {
    pub(crate) fn solve(rows: &[Vec<&TabularCellBox>], rules: &[GraphicsElem]) -> Self {
        let ncols = rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut borders = Borders {
            horizontal: vec![None; rows.len() + 1],
            vertical: vec![None; ncols + 1],
        };
        // Row baselines DESCEND (`Solved::ys` runs from the table's height
        // down to 0), so a rule sits above row `r` when its y is above that
        // row's baseline and below the previous row's.
        let baselines: Vec<f64> = rows
            .iter()
            .map(|row| row.first().map_or(0.0, |c| c.baseline_y.0))
            .collect();
        let lefts: Vec<f64> = (0..ncols)
            .map(|c| {
                rows.iter()
                    .filter_map(|row| row.get(c).map(|cell| cell.x.0))
                    .fold(f64::INFINITY, f64::min)
            })
            .collect();
        for elem in rules {
            collect_rule(elem, &baselines, &lefts, &mut borders);
        }
        borders
    }
}

/// Every rules-bearing `TabularBox` reachable through a text-only graphics
/// overlay, as `(width, height, rules)`.
///
/// **The phantom table.** `easytable` builds every table TWICE
/// (`table-builder.satyh`'s `build`): once with the real cell text and no
/// rules, and once as the same grid of EMPTY cells carrying only the rule
/// callbacks, drawing both into one `inline-graphics`. In the PDF the second
/// is invisible — it is nothing but the lines. The two halves are visible
/// together only at the graphics box that holds them, so a backend that
/// wants a real table's rules has to collect them HERE and pair them by
/// size when it reaches the text-bearing twin. Both the HTML and the LaTeX
/// writer do exactly that, which is why the traversal is shared.
pub(crate) fn overlaid_table_rules(
    elems: &[GraphicsElem],
) -> Vec<(f64, f64, Vec<GraphicsElem>)> {
    let mut out = Vec::new();
    walk_tabulars(elems, &mut |tab| {
        if !tab.rules.is_empty() {
            out.push((tab.width.0, tab.height.0, tab.rules.clone()));
        }
    });
    out
}

/// Visit every `Tabular` reachable through a text-only graphics group's
/// nested boxes.
fn walk_tabulars(elems: &[GraphicsElem], f: &mut impl FnMut(&TabularBox)) {
    for elem in elems {
        match elem {
            GraphicsElem::Text { contents, .. } => {
                for (_, bx) in contents {
                    if let PureHorzBox::Tabular(tab) = bx {
                        f(tab);
                    }
                }
            }
            GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => walk_tabulars(inner, f),
            _ => {}
        }
    }
}

/// Place one rule graphic on the grid, recursing through `Group`/`Clip` so a
/// united rule set is read the same way a flat one is.
fn collect_rule(elem: &GraphicsElem, baselines: &[f64], lefts: &[f64], borders: &mut Borders) {
    let (color, stroke_w) = match elem {
        GraphicsElem::Fill(c, _) => (*c, None),
        GraphicsElem::Stroke(w, c, _) | GraphicsElem::DashedStroke(w, _, c, _) => (*c, Some(w.0)),
        GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => {
            for e in inner {
                collect_rule(e, baselines, lefts, borders);
            }
            return;
        }
        _ => return,
    };
    let Some((lo, hi)) = graphics_bbox(elem) else {
        return;
    };
    let (Length(x0), Length(y0)) = lo;
    let (Length(x1), Length(y1)) = hi;
    let (w, h) = (x1 - x0, y1 - y0);
    if w >= h {
        // Horizontal: above row `r` = the number of rows whose baseline is
        // above this rule's own centre line.
        let y = (y0 + y1) / 2.0;
        let above = baselines.iter().filter(|b| **b > y + RULE_EPS_PT).count();
        let rule = Rule {
            width: stroke_w.unwrap_or(h).max(RULE_EPS_PT),
            color,
        };
        if let Some(slot) = borders.horizontal.get_mut(above) {
            *slot = Some(rule);
        }
    } else {
        let x = (x0 + x1) / 2.0;
        let left_of = lefts.iter().filter(|l| **l < x - RULE_EPS_PT).count();
        let rule = Rule {
            width: stroke_w.unwrap_or(w).max(RULE_EPS_PT),
            color,
        };
        if let Some(slot) = borders.vertical.get_mut(left_of) {
            *slot = Some(rule);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // [`wants_space`]/[`is_cjk`] are exercised by `reflow/text.rs`'s own
    // suite, which is where the rule was worked out and stays the statement
    // of record for it. Only the rules that are new HERE are tested here.

    /// The whole point of `InlineMarkKind::BreakHyphen`: the two hyphens are
    /// distinguishable only by the marker, never by the text's shape.
    #[test]
    fn only_the_marker_deletes_a_hyphen() {
        assert_eq!(line_join(true, true), LineJoin::DropHyphen);
        // Same text shape, no marker: the hyphen is the author's and stays,
        // but the halves of `code-printer` still must not gain a space.
        assert_eq!(line_join(false, true), LineJoin::KeepHyphen);
        assert_eq!(line_join(false, false), LineJoin::Space);
    }

    #[test]
    fn heading_depth_is_one_based_and_capped_at_six() {
        assert_eq!(heading_depth(0), 1);
        assert_eq!(heading_depth(1), 2);
        assert_eq!(heading_depth(5), 6);
        // Deeper than the format allows collapses rather than overflowing.
        assert_eq!(heading_depth(9), 6);
        assert_eq!(heading_depth(-3), 1);
    }
}

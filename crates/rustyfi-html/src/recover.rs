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
//! - [`is_aligned_equation`] — and not every `TabularBox` is a TABLE. The
//!   `math` package builds `+align` out of one, so an aligned equation and a
//!   spreadsheet arrive in the same box.
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
    DecoId, FontKey, OutlineEntry, PureHorzBox, TabularBox, TabularCellBox,
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

/// Is this `TabularBox` an ALIGNED EQUATION rather than tabular data?
///
/// `+align` — the `math` package's multi-line equation block — is built out of
/// a `tabular` (`lib-rustyfi/dist/packages/math.satyh:541-574`, upstream
/// `math.satyh`'s own `% temporary`), so an equation and a spreadsheet reach a
/// backend in the same box. That matters because the two want opposite
/// renderings: a grid is the whole point of one and an artefact of the other.
///
/// **The signal is the construction, not a resemblance.** Three clauses, each
/// a fact about how `+align` builds its grid, and each one on its own able to
/// keep a real table out:
///
/// 1. **It draws no rules.** `+align` passes `(fun _ _ -> [])` as the rule
///    callback. A grid that draws its own lines is asserting that it IS a
///    grid, whatever is in the cells.
/// 2. **Every cell's only ink is math** ([`cell_ink`]). This is the clause
///    that excludes the real tables: `easytable`'s cells hold text, and its
///    content half draws no rules either (they live in the phantom twin), so
///    clause 1 alone would not have.
/// 3. **The columns alternate RIGHT, LEFT, RIGHT, …** ([`cell_align`]), which
///    is `+align`'s `if index mod 2 == 0 then inline-fil ++ ib else ib ++
///    inline-fil` read back out of the box stream. It is also exactly the
///    column pattern LaTeX's `aligned` means, which is what makes the
///    Markdown backend's `\begin{aligned}` a translation rather than a guess.
///
/// Clause 3 is what separates an ALIGNMENT from a MATRIX. A matrix built the
/// same way (`satysfi-base`'s `math-ext.satyh:1585`, `azmath`'s
/// `matrices.satyh`) is also a rule-less grid of math-only cells, but its
/// cells are CENTRED — a fil on both sides — and its meaning is carried by
/// delimiters drawn outside the `tabular`, where this cannot see them.
/// Rendering one as `aligned` would silently restyle it, so a matrix declines
/// here and keeps whatever the caller does with an ordinary table. That is a
/// known, deliberate boundary rather than an oversight.
///
/// An EMPTY cell is allowed anywhere and constrains nothing: a ragged
/// `+align` row is padded, and a padded slot has neither ink nor alignment to
/// check. At least one cell must hold math, so the rules-only phantom grid
/// `easytable` overlays on every table is not claimed.
pub(crate) fn is_aligned_equation(tab: &TabularBox) -> bool {
    if !tab.rules.is_empty() {
        return false;
    }
    let mut any_math = false;
    for row in table_rows(tab) {
        for (col, cell) in row.iter().enumerate() {
            match cell_ink(&cell.contents) {
                CellInk::Other => return false,
                // A padded slot: no ink, and so no alignment either.
                CellInk::None => continue,
                CellInk::Math => any_math = true,
            }
            let wanted = if col % 2 == 0 {
                CellAlign::Right
            } else {
                CellAlign::Left
            };
            if cell_align(&cell.contents) != wanted {
                return false;
            }
        }
    }
    any_math
}

/// What a cell holds that a reader would SEE — the question
/// [`is_aligned_equation`] asks of every cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CellInk {
    /// Glue, fils, skips and markers only.
    None,
    /// At least one `PureHorzBox::Math`, and nothing else that shows.
    Math,
    /// Anything a reader would see that is not an equation.
    Other,
}

/// [`CellInk`] for one cell's contents, recursing through `Frame` (a link or a
/// decoration wrapped around the equation contributes no ink of its own).
///
/// Everything not explicitly inert counts as `Other`, which is the safe
/// direction: a misread cell leaves the box a TABLE, which is what it already
/// was. In particular a `Graphics` box is `Other` even when it holds nothing
/// but `draw-text` — a `+align` cell never has one, and recognising the
/// wrapper shape here would mean a second copy of `markdown::inline`'s
/// `is_pure_text` living in a module that emits no markup.
fn cell_ink(contents: &[(rustyfi_backend::Length, PureHorzBox)]) -> CellInk {
    let mut ink = CellInk::None;
    for (_, bx) in contents {
        match box_ink(bx) {
            CellInk::Other => return CellInk::Other,
            CellInk::Math => ink = CellInk::Math,
            CellInk::None => {}
        }
    }
    ink
}

/// [`cell_ink`] for one box. See there for why the default is `Other`.
fn box_ink(bx: &PureHorzBox) -> CellInk {
    match bx {
        PureHorzBox::Math { .. } => CellInk::Math,
        PureHorzBox::InnerString { text, .. } => {
            if text.trim().is_empty() {
                CellInk::None
            } else {
                CellInk::Other
            }
        }
        PureHorzBox::Frame { contents, .. } => cell_ink(contents),
        PureHorzBox::OuterEmpty { .. }
        | PureHorzBox::OuterFil
        | PureHorzBox::FixedEmpty { .. }
        | PureHorzBox::Discretionary { .. }
        | PureHorzBox::InlineMark(_)
        | PureHorzBox::InlineFrameMarker { .. }
        | PureHorzBox::HookPageBreak { .. }
        | PureHorzBox::FrameMarker { .. } => CellInk::None,
        _ => CellInk::Other,
    }
}

/// How a `tabular` cell's content is aligned in its column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CellAlign {
    /// An `inline-fil` after the content and none before: the content is
    /// pushed to the column's LEFT edge.
    Left,
    /// The mirror — a fil before and none after.
    Right,
    /// A fil on both sides.
    Centre,
    /// Neither, so the content simply fills the cell.
    Fill,
}

/// Read a cell's alignment off where its `inline-fil`s sit.
///
/// A `tabular` cell has no alignment field: `NormalCell` takes padding and an
/// inline box, and every package that aligns a cell does it by putting an
/// `inline-fil` on the side the content should move AWAY from. So the fils
/// relative to the inked boxes ARE the alignment, and reading them back is
/// exact rather than inferred from where the cell was placed.
///
/// Zero-width `inline-skip`s (the cell padding `+align` writes on both sides)
/// are not ink and do not move the boundary.
fn cell_align(contents: &[(rustyfi_backend::Length, PureHorzBox)]) -> CellAlign {
    let mut inked = contents
        .iter()
        .enumerate()
        .filter(|(_, (_, bx))| box_ink(bx) != CellInk::None)
        .map(|(i, _)| i);
    let Some(first) = inked.next() else {
        return CellAlign::Fill;
    };
    let last = inked.next_back().unwrap_or(first);
    let fil_at = |range: std::ops::Range<usize>| {
        contents[range]
            .iter()
            .any(|(_, bx)| matches!(bx, PureHorzBox::OuterFil))
    };
    match (fil_at(0..first), fil_at(last + 1..contents.len())) {
        (true, true) => CellAlign::Centre,
        (true, false) => CellAlign::Right,
        (false, true) => CellAlign::Left,
        (false, false) => CellAlign::Fill,
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

    /// A `tabular` cell has no alignment FIELD — every package aligns a cell
    /// by putting an `inline-fil` on the side the content moves away from
    /// (`lib-rustyfi/dist/packages/table.satyh:22-29` writes `l`, `r` and `c`
    /// as exactly these three shapes). So reading the fils back is the
    /// alignment, and the padding skips around them do not move it.
    #[test]
    fn a_cells_alignment_is_read_off_its_fils_and_not_off_the_padding() {
        let pad = || {
            (
                rustyfi_backend::Length::ZERO,
                PureHorzBox::FixedEmpty {
                    width: rustyfi_backend::Length::ZERO,
                },
            )
        };
        let fil = || (rustyfi_backend::Length::ZERO, PureHorzBox::OuterFil);
        let ink = || {
            (
                rustyfi_backend::Length::ZERO,
                PureHorzBox::Math {
                    width: rustyfi_backend::Length::ZERO,
                    height: rustyfi_backend::Length::ZERO,
                    depth: rustyfi_backend::Length::ZERO,
                    glyphs: Vec::new(),
                    rules: Vec::new(),
                },
            )
        };
        assert_eq!(
            cell_align(&[pad(), fil(), ink(), pad()]),
            CellAlign::Right,
            "`r`: a fil before the content pushes it right",
        );
        assert_eq!(
            cell_align(&[pad(), ink(), fil(), pad()]),
            CellAlign::Left,
            "`l`: a fil after it pushes it left",
        );
        assert_eq!(
            cell_align(&[pad(), fil(), ink(), fil(), pad()]),
            CellAlign::Centre,
            "`c`: a fil on both sides — a MATRIX cell, never an alignment's",
        );
        assert_eq!(cell_align(&[pad(), ink(), pad()]), CellAlign::Fill);
        // No ink at all: a padded slot in a ragged row, which constrains
        // nothing.
        assert_eq!(cell_align(&[pad(), fil()]), CellAlign::Fill);
    }
}

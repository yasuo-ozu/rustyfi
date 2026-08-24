//! Structure recovery shared by the three output backends.
//!
//! `--format markdown` is a SUBSET of `--format html`: it recovers the same
//! headings, lists, tables, links and emphasis out of the same flat box
//! stream, and then writes less. `--format latex` — the `rustyfi-latex`
//! crate, which is why this module is `pub` — recovers the same structure
//! again and writes it for another typesetter. The recovery itself must
//! therefore be ONE implementation, not three — every rule collected here
//! was got wrong at least once before it was got right, and a forked copy
//! would rot silently the next time one of them is corrected.
//!
//! **That is achieved for the rules below, and NOT yet for everything in this
//! module** — see "Still forked" at the end, which names each exception. Read
//! this list as what the module is FOR, not as an inventory of what it has
//! already finished collecting.
//!
//! - [`wants_space`] — glue is not a space. The box stream puts glue between
//!   every pair of CJK characters, so "glue means space" renders Japanese as
//!   `研 究 計 画`. `reflow/text.rs`'s doc comment has the full argument.
//! - [`line_join`] — a `VertBox::Line` boundary is the LINE BREAKER's
//!   decision, so it is undone; but the hyphen at the end of the line may be
//!   the AUTHOR's, and deleting it turns `code-printer` into `codeprinter`.
//!   `InlineMarkKind::BreakHyphen` is what tells the two apart.
//! - [`find_heading`] — a heading is found by correlating
//!   `extras.outline` to a destination frame through `dest_name`, a
//!   STRUCTURAL id match. `reflow/structure.rs`'s doc comment explains why a
//!   font-size guess was refused, and why `InlineFrameMarker` (not just
//!   `Frame`) is what every bundled doc class actually emits.
//! - [`table_rows`] — a `TabularBox` carries no grid, only cells; rows are
//!   recovered from each cell's `x`.
//! - [`Borders`] — and no grid LINES either; which boundaries a table
//!   actually rules is recovered from the shapes in `TabularBox::rules`.
//! - [`MonoFiles::is_monospace`] — the one signal that says a `Line`
//!   boundary is the AUTHOR's rather than the breaker's (a `+code` block
//!   and a wrapped paragraph are structurally identical otherwise).
//!
//! Nothing here emits markup. Each function answers a question about the box
//! stream and lets its caller decide what to write, which is exactly the line
//! between "the same document structure" and "two different serializations of
//! it".
//!
//! ## Still forked — a known debt, listed so it cannot be mistaken for done
//!
//! The commit that hoisted the box-stream helpers here ("hoist the box-stream
//! rules the LaTeX backend was the third copy of") wrote a fresh copy into
//! this module and **left the existing copies where they were**. So for the
//! items below, this module is not the shared implementation: it is the
//! LaTeX backend's copy, which happens to live in the HTML crate. Every one
//! of them has ZERO callers in `rustyfi-html` itself.
//!
//! | item | definitions in the tree |
//! |--|--|
//! | [`is_pure_text`] | 3 — here, `markdown/inline.rs`, `reflow/inline.rs` |
//! | [`pre_break_carries_text`] | 3 — the same three |
//! | [`glue_width`] | 3 — the same three |
//! | [`HSKIP_MIN_PT`] | 3 — the same three |
//! | [`GRAPHIC_MIN_PT`] | 2 — here and `markdown/inline.rs` |
//! | [`gap_spaces`] | 2 — here and `markdown/para.rs`, doc comment and unit test copied verbatim too |
//! | [`is_code_paragraph`] | 2 — here and `markdown/para.rs`'s `Para::is_code`, spelled out inline |
//! | [`ink_bbox`] | 2 — here and inlined into `svg.rs`'s `graphics_block` |
//!
//! **Two of those are TUNED THRESHOLDS, which is what makes this worth
//! writing down rather than filing away.** [`HSKIP_MIN_PT`] and
//! [`GRAPHIC_MIN_PT`] are numbers somebody measured against the corpus; the
//! copies agree today, and nothing makes them agree tomorrow. Drift there
//! does not produce an error — it produces three output formats that disagree
//! about what counts as a figure, in a document nobody is looking at.
//!
//! The fix is one line per site and is deliberately NOT done here: the sites
//! are `markdown/inline.rs`, `reflow/inline.rs`, `markdown/para.rs` and
//! `svg.rs`, all four of which have other work in flight in them, and the
//! commit that introduced the duplication is not the commit that should
//! resolve it. Delete the local copy, call the one here, and cross the row
//! off.

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
pub fn is_cjk(c: char) -> bool {
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
pub fn wants_space(prev: Option<char>, next: Option<char>, natural_pt: f64) -> bool {
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
/// (except inside a code block, which every caller detects with
/// [`MonoFiles::is_monospace`] and handles before asking). So the two lines
/// are rejoined; the only question is what happens to the hyphen at the join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineJoin {
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
pub fn line_join(break_hyphen: bool, ends_with_hyphen: bool) -> LineJoin {
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
pub fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{2010}')
}

/// A plain inter-word space, in points, standing in for the line break
/// between two consecutive `Line`s of one paragraph. The exact value is
/// immaterial — it only has to be above [`wants_space`]'s zero-width
/// threshold, since the decision it feeds is "is this a CJK pair" rather than
/// "how wide".
pub const WORD_SPACE_PT: f64 = 3.0;

/// Whether `font` is a fixed-pitch face, read off the family name the file
/// declares (`fonts::is_monospace_family` — a name heuristic, and labelled as
/// one there). `false` in base-14 mode, where there is no file to ask.
///
/// This is not a styling nicety in either backend: it is the ONLY signal in
/// the box stream that distinguishes a line boundary the renderer should
/// REDO from one it must KEEP. A wrapped paragraph and a `+code` block are
/// structurally identical — `code.satyh` calls `line-break` once per source
/// line exactly as the line breaker does per wrapped line.
/// **Answered once per FILE, not once per run**, and that is the difference
/// between this backend costing 2ms and costing 145ms.
/// `TtfFontStore::file_family_name` re-`Face::parse`s the whole font file and
/// decodes a UTF-16 `name` record on every call; the box stream emits one
/// text run per CJK CHARACTER, so `latexcmds` asked the same question of the
/// same 7.8MB `ipaexm.ttf` about 5400 times and got the same answer. There
/// are at most `num_files()` distinct answers, and they are all computed
/// here, up front.
pub struct MonoFiles(Vec<bool>);

impl MonoFiles {
    pub fn new(store: Option<&TtfFontStore>) -> Self {
        MonoFiles(match store {
            None => Vec::new(),
            Some(store) => (0..store.num_files())
                .map(|i| {
                    store
                        .file_family_name(i)
                        .is_some_and(|f| crate::fonts::is_monospace_family(&f))
                })
                .collect(),
        })
    }

    /// Whether `font` is a fixed-pitch face. `false` in base-14 mode, where
    /// there is no file to ask — see [`MonoFiles`].
    pub fn is_monospace(&self, store: Option<&TtfFontStore>, font: Option<FontKey>) -> bool {
        let (Some(store), Some(font)) = (store, font) else {
            return false;
        };
        self.0
            .get(store.file_index(font))
            .copied()
            .unwrap_or(false)
    }
}

/// A `FixedEmpty` (`inline-skip`) at least this wide (pt) is a deliberate gap
/// worth one space; anything narrower is a KERN and renders as nothing.
///
/// The two populations it separates: a paragraph indent or a table cell's
/// padding above, the `\LaTeX` logo's own four kerns and a table-of-contents
/// leader's dot spacing below.
pub const HSKIP_MIN_PT: f64 = 2.0;

/// Smaller than this in either dimension (pt) and a drawing's INK is a rule,
/// a leader dot or a piece of underlining, not a figure.
///
/// The size measured is the INK's, not the box's, and that distinction is
/// load-bearing: `stdjabook` draws the rule under a section heading as a
/// 440pt x 1pt line inside a 440pt x 4pt box. Judged by the box it is a
/// figure, and `easytable`'s manual grew a placeholder above and below every
/// heading in it.
pub const GRAPHIC_MIN_PT: f64 = 4.0;

/// How many spaces a gap of `pt` points is, in a document whose fixed-pitch
/// character advances `advance` points.
///
/// `code.satyh` sizes both the leading indent and every inter-word space in
/// exact multiples of that advance (`set-space-ratio (charwid /' fontsize)`),
/// so this division is not an estimate — it recovers the source's own column
/// count. With no advance measured (a base-14 render, where no font file says
/// whether a face is fixed-pitch at all) it degrades to one space, which
/// keeps the words apart and loses only the indentation.
pub fn gap_spaces(pt: f64, advance: Option<f64>) -> usize {
    match advance {
        Some(a) if a > 0.0 => (pt / a).round().max(0.0) as usize,
        // No measurement: any positive gap is one space.
        _ => usize::from(pt > 0.0),
    }
}

/// The fixed-pitch character advance a run measures, if it is one this can be
/// measured from.
///
/// In a fixed-pitch face every character is one advance wide, so a run's own
/// width divided by its character count IS the column width a code block's
/// indentation is counted in. Only a run of plain ASCII is measured — a
/// fixed-pitch Latin face still sets a stray CJK character full-width, which
/// would halve the estimate.
pub fn mono_advance(text: &str, width: f64) -> Option<f64> {
    let n = text.chars().count();
    (n > 0 && width > 0.0 && text.chars().all(|c| c.is_ascii_graphic()))
        .then(|| width / n as f64)
}

/// Whether `elem` contributes no ink of its own — a `draw-text`, or a group
/// containing only those. `Group`/`Clip` recurse so a `unite-graphics` of
/// text runs is recognised too.
pub fn is_pure_text(elem: &GraphicsElem) -> bool {
    match elem {
        GraphicsElem::Text { .. } => true,
        GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => inner.iter().all(is_pure_text),
        _ => false,
    }
}

/// The union of every element's ink bounding box, or `None` when nothing in
/// `elems` draws.
pub fn ink_bbox(elems: &[GraphicsElem]) -> Option<((Length, Length), (Length, Length))> {
    elems
        .iter()
        .filter_map(graphics_bbox)
        .reduce(|(alo, ahi), (blo, bhi)| {
            (
                (
                    Length(alo.0 .0.min(blo.0 .0)),
                    Length(alo.1 .0.min(blo.1 .0)),
                ),
                (
                    Length(ahi.0 .0.max(bhi.0 .0)),
                    Length(ahi.1 .0.max(bhi.1 .0)),
                ),
            )
        })
}

/// Does a `Discretionary`'s `pre_break` carry a visible character (the
/// hyphenation dictionary's hyphen), as opposed to bare glue?
pub fn pre_break_carries_text(pre_break: &[PureHorzBox]) -> bool {
    pre_break.iter().any(|b| match b {
        PureHorzBox::InnerString { text, .. } => !text.is_empty(),
        _ => false,
    })
}

/// The total natural width of a `Discretionary`'s `pre_break` glue, fed to
/// the ordinary glue rule when there is no hyphen to show.
pub fn glue_width(pre_break: &[PureHorzBox]) -> f64 {
    pre_break
        .iter()
        .map(|b| match b {
            PureHorzBox::OuterEmpty { natural, .. } => natural.0,
            PureHorzBox::FixedEmpty { width } => width.0,
            _ => 0.0,
        })
        .sum()
}

/// Is a paragraph with these tallies a code block — one whose line breaks are
/// the AUTHOR's and whose whitespace is significant?
///
/// The obvious test, `all_mono`, is not enough: a `+code` block containing any
/// Japanese fails it, because a fixed-pitch Latin face has no CJK glyphs and
/// SATySFi sets those characters in the document's own gothic/mincho face, so
/// the paragraph reads as MIXED. In `latexcmds`' manual, whose code samples
/// are full of Japanese string literals, that is most of the code blocks in
/// the document.
///
/// The reliable signal is structural: `code.satyh` builds a block as ONE
/// `line-break` over a sequence of
/// `inline-skip ++ line ++ inline-fil ++ discretionary`, one per source line,
/// so EVERY line of a code block ends with an `inline-fil`. A justified prose
/// paragraph ends only its LAST line that way.
///
/// A single line cannot be told apart this way (one line is always "all its
/// lines"), so a one-line paragraph falls back to `all_mono` — which is
/// exactly right for it, and is kept as an alternative at every length.
///
/// The count is a MAJORITY, not "all", because a code line too long for the
/// measure is broken by the paragraph breaker like any other and ends at a
/// hyphenation point rather than at its fil. One overflowing line in `xpath`'s
/// API listing was enough to make the whole block prose under an "all" test.
pub fn is_code_paragraph(all_mono: bool, has_mono: bool, lines: usize, fil_lines: usize) -> bool {
    all_mono || (lines >= 2 && has_mono && fil_lines * 2 > lines)
}

/// `dest_name -> level` from `extras.outline` (`register-outline`'s already
/// `Interp::dest_name`-resolved entries) — the lookup table
/// [`find_heading`] consults.
pub fn outline_levels(outline: &[OutlineEntry]) -> HashMap<String, i64> {
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
pub fn heading_depth(level: i64) -> u8 {
    (level.max(0) as u64 + 1).min(6) as u8
}

/// [`find_heading`], for a caller that wants only the level.
///
/// `pub(crate)` where its sibling is `pub`, and that asymmetry is the point:
/// `rustyfi-latex` needs the destination NAME as well (a `\hypertarget` a
/// `\ref` reaches by `\hyperlink`), so it calls [`find_heading`]; the two
/// in-crate backends want only the level. Widening this one as well would
/// publish API on the strength of a doc-comment mention rather than a caller.
pub(crate) fn find_heading_level(
    bx: &PureHorzBox,
    dests: &HashMap<DecoId, &str>,
    outline_by_dest: &HashMap<String, i64>,
) -> Option<i64> {
    find_heading(bx, dests, outline_by_dest).map(|(level, _)| level)
}

/// Does `bx` (or, recursively, one of its `Frame` descendants) carry the
/// `DecoId` of a `register-location-frame`/`register-destination` call whose
/// resolved name matches a `register-outline` entry? Returns that entry's
/// level AND the destination name on the first match. See
/// `reflow/structure.rs`'s doc comment for why this is a structural match
/// rather than a font-size heuristic.
///
/// **The name comes back with the level, rather than from a second lookup.**
/// The LaTeX backend needs both — the level to pick `\section*` and the name
/// for the `\hypertarget` a `\ref` reaches it by — and it originally asked
/// twice, walking this same recursion again a line later. The refinement
/// below (that `InlineFrameMarker`, not only `Frame`, has to be matched or no
/// heading in any `stdjabook` document is ever promoted) then had to hold in
/// two traversals, and if they ever disagreed a heading would keep its
/// `\section*` and silently lose its anchor — turning every cross-reference
/// to it into a dead link that still compiles.
///
/// **`InlineFrameMarker` is checked too, and that is what makes this work at
/// all on a real document.** `inline-frame-breakable` splices its contents
/// between a marker PAIR rather than building a `Frame`, so that the frame
/// can split across a line break — and that is how every bundled doc class
/// writes a section title (`stdjabook.satyh:551`, `stdjareport.satyh:445`).
/// Matching only `Frame` meant no heading in any `stdjabook` document was
/// ever promoted. Only the START marker is consulted — the `end: true` twin
/// carries the same `DecoId` and would match a second time for nothing.
pub fn find_heading<'a>(
    bx: &PureHorzBox,
    dests: &HashMap<DecoId, &'a str>,
    outline_by_dest: &HashMap<String, i64>,
) -> Option<(i64, &'a str)> {
    match bx {
        PureHorzBox::InlineFrameMarker { id, end: false, .. } => {
            level_of_deco(id, dests, outline_by_dest)
        }
        PureHorzBox::Frame { deco, contents, .. } => {
            level_of_deco(deco, dests, outline_by_dest).or_else(|| {
                contents
                    .iter()
                    .find_map(|(_, inner)| find_heading(inner, dests, outline_by_dest))
            })
        }
        _ => None,
    }
}

/// `DecoId` -> destination name -> outline level, the two-hop structural
/// lookup both arms of [`find_heading`] share.
fn level_of_deco<'a>(
    deco: &DecoId,
    dests: &HashMap<DecoId, &'a str>,
    outline_by_dest: &HashMap<String, i64>,
) -> Option<(i64, &'a str)> {
    let name = *dests.get(deco)?;
    outline_by_dest.get(name).copied().map(|l| (l, name))
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
pub fn table_rows(tab: &TabularBox) -> Vec<Vec<&TabularCellBox>> {
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
/// **The two vectors are private, and deliberately so.** Both consumers ask
/// only "is there a rule at boundary `i`", through the accessors below; the
/// `Vec<Option<Rule>>`-with-a-trailing-entry layout is an implementation
/// detail that a table with row or column SPANS would have to change, and
/// publishing it would make that change breaking for no one's benefit.
pub struct Borders {
    /// `horizontal[r]` is the rule ABOVE row `r`; the extra last entry is the
    /// rule below the final row.
    horizontal: Vec<Option<Rule>>,
    /// `vertical[c]` is the rule LEFT of column `c`; the extra last entry is
    /// the rule right of the final column.
    vertical: Vec<Option<Rule>>,
}

/// One recovered grid line: how thick, and in what colour.
///
/// `pub` because [`Borders::horizontal`]/[`Borders::vertical`] hand it out,
/// with `pub(crate)` fields because nothing outside this crate reads them:
/// the HTML backend writes `border-…: {width}pt solid {color}`, and LaTeX's
/// `\hline` has one width for the whole table and no colour without
/// `colortbl`, so `rustyfi-latex` only ever asks whether the `Option` is
/// `Some`. Should it grow a `colortbl` mode, widen these then.
#[derive(Clone, Copy)]
pub struct Rule {
    pub(crate) width: f64,
    pub(crate) color: Color,
}

/// A rule thinner than this (pt) is invisible in a browser anyway; a
/// coordinate closer than this to a boundary counts as being on it.
pub const RULE_EPS_PT: f64 = 0.05;

impl Borders {
    /// The rule at horizontal boundary `i`, counting from the top: `0` is
    /// above the first row and [`Borders::rows`] is below the last. Out of
    /// range is `None`, which is also what "no rule here" is — a caller
    /// asking about a boundary that does not exist wants the same answer.
    pub fn horizontal(&self, i: usize) -> Option<Rule> {
        self.horizontal.get(i).copied().flatten()
    }

    /// The rule at vertical boundary `i`, counting from the left: `0` is left
    /// of the first column and [`Borders::cols`] is right of the last.
    pub fn vertical(&self, i: usize) -> Option<Rule> {
        self.vertical.get(i).copied().flatten()
    }

    /// How many rows the grid was solved for — so `horizontal(rows())` is the
    /// bottom rule.
    pub fn rows(&self) -> usize {
        self.horizontal.len().saturating_sub(1)
    }

    /// How many columns the grid was solved for — so `vertical(cols())` is
    /// the right-hand rule.
    pub fn cols(&self) -> usize {
        self.vertical.len().saturating_sub(1)
    }

    pub fn solve(rows: &[Vec<&TabularCellBox>], rules: &[GraphicsElem]) -> Self {
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
pub fn overlaid_table_rules(elems: &[GraphicsElem]) -> Vec<(f64, f64, Vec<GraphicsElem>)> {
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

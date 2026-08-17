//! The `tabular` grid solver: row/column metrics, `MultiCell` span
//! bookkeeping, and cell content fitting. A faithful port of v0.0.6's
//! `src/backend/tabular.ml` (`main`/`determine_row_metrics`/
//! `determine_column_width`/`normalize_tabular`/`transpose_tabular`/
//! `solidify_tabular`, cited by name below), adapted two ways:
//!
//! - **Depth sign.** `tabular.ml` threads *negative* depths (more negative =
//!   deeper) through `Length.min`/`Length.negate`. This port's
//!   [`natural_metrics`](crate::linebreak::natural_metrics) already returns a
//!   non-negative "how far below the baseline" magnitude (see `hbox.rs`), so
//!   every upstream `min`/`negate` pair becomes a plain `max`/`+` here — no
//!   sign flip is needed at the call site, only at the *shape* of the
//!   formula (documented at each function below).
//! - **Row/column indexing.** `normalize_tabular` always produces a
//!   rectangular grid (every row padded to the widest row's length), so a
//!   positional transpose (by column index) replaces upstream's recursive
//!   `chop_column`/`transpose_tabular` — same result, simpler in Rust.
//!
//! **Malformed grids degrade, they don't panic** (docs/plans/
//! table-subsystem.md Risks: "Span bookkeeping"): where upstream asserts
//! false on a cell that should have been an `EmptyCell` continuing a
//! pending span (or a span declared with `numrow`/`numcol` < 1), this port
//! drops the bogus pending state / clamps to 1 and keeps going, so a table
//! author's off-by-one never aborts the whole document.
//!
//! docs/plans/table-subsystem.md §Slice 1.

use crate::graphics::GraphicsElem;
use crate::hbox::{HorzBox, PureHorzBox};
use crate::length::Length;
use crate::linebreak::{fit_cell, natural_metrics};

/// `paddings` (horzBox.ml's `paddingL/R/T/B`; `prim_types::t_paddings`'s
/// runtime shape once extracted by `as_paddings`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paddings {
    pub l: Length,
    pub r: Length,
    pub t: Length,
    pub b: Length,
}

/// `cell` (horzBox.ml:447). Content is already-measured pure boxes (the
/// package's `read-inline ctx it` produced them via `cellf`/`multif`), so
/// the solver never threads a `Context`.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    /// `NormalCell(pads, hblst)`.
    Normal(Paddings, Vec<HorzBox>),
    /// `EmptyCell` — a blank grid slot, and also how a `MultiCell`'s spanned
    /// (non-anchor) slots must be filled in, both across columns (in the
    /// same row) and down rows (in later rows at the same column).
    Empty,
    /// `MultiCell(numrow, numcol, pads, hblst)`.
    Multi(usize, usize, Paddings, Vec<HorzBox>),
}

/// One placed cell inside a solved [`TabularBox`]: its box-local anchor
/// (`x` = left edge, `baseline_y` = content baseline, both y-**up** from the
/// box's own baseline-left origin) and its content already fitted to the
/// cell's (or, for a span, combined) column width — exactly what a
/// `VertBox::Line` carries. `EmptyCell`s produce no entry at all.
#[derive(Clone, Debug, PartialEq)]
pub struct TabularCellBox {
    pub x: Length,
    pub baseline_y: Length,
    pub contents: Vec<(Length, PureHorzBox)>,
}

/// The solved `PHGFixedTabular` payload (horzBox.ml:279), minus `rules`
/// (filled in lang-side once the rule callback runs — `primitives.rs`'s
/// `prim_tabular`).
#[derive(Clone, Debug, PartialEq)]
pub struct TabularBox {
    pub width: Length,
    pub height: Length,
    /// Always `Length::ZERO` (upstream `dpttotal`, tabular.ml:340).
    pub depth: Length,
    pub cells: Vec<TabularCellBox>,
    pub rules: Vec<GraphicsElem>,
}

/// `Tabular.main`'s result (tabular.ml:309): geometry, solidified cells, and
/// the grid-line coordinates the rule callback wants — `xs` ascending from
/// `0` (column boundaries), `ys` **descending** from `height` (row *tops*,
/// `handlePdf.ml:214-220`) down to `0`.
#[derive(Clone, Debug, PartialEq)]
pub struct Solved {
    pub width: Length,
    pub height: Length,
    pub cells: Vec<TabularCellBox>,
    pub xs: Vec<Length>,
    pub ys: Vec<Length>,
}

/// Per-column pending multi-**row** span state, threaded row-to-row
/// top-to-bottom (`rest_row` in tabular.ml): `Some((rows_remaining,
/// extra_len_needed))` at column `i` means an earlier row's `MultiCell`
/// still owns this column for `rows_remaining` more rows.
type RestRow = Vec<Option<(usize, Length)>>;

/// Per-row pending multi-**column** span state, threaded column-to-column
/// left-to-right (`rest_column` in tabular.ml).
type RestCol = Vec<Option<(usize, Length)>>;

/// `normalize_tabular` (tabular.ml): pad every row to the widest row's
/// length with trailing `EmptyCell`s. A short row is filled on the
/// *right*, never in the middle — a mid-row gap under a span is the table
/// author's job (an explicit `EmptyCell` at the spanned position).
fn normalize_tabular(rows: Vec<Vec<Cell>>) -> (usize, Vec<Vec<Cell>>) {
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let htabular = rows
        .into_iter()
        .map(|mut row| {
            while row.len() < ncols {
                row.push(Cell::Empty);
            }
            row
        })
        .collect();
    (ncols, htabular)
}

/// Column-major view of a (rectangular, post-`normalize_tabular`) grid —
/// replaces `transpose_tabular`'s recursive `chop_column` (see module doc
/// comment).
fn transpose(rows: &[Vec<Cell>], ncols: usize) -> Vec<Vec<&Cell>> {
    (0..ncols)
        .map(|c| rows.iter().map(|row| &row[c]).collect())
        .collect()
    // Every `row` has exactly `ncols` entries post-normalize, so `row[c]`
    // never panics.
}

/// `determine_row_metrics` (tabular.ml:10): one row's `(height, depth
/// magnitude)`, plus the updated `rest_row` for the next row down.
fn determine_row_metrics(restprev: &RestRow, row: &[Cell]) -> (RestRow, Length, Length) {
    let mut restacc: RestRow = Vec::with_capacity(row.len());
    let mut hgt_max = Length::ZERO;
    let mut dpt_mag_max = Length::ZERO;
    for (slot, cell) in restprev.iter().zip(row.iter()) {
        match (slot, cell) {
            (None, Cell::Normal(pads, content)) => {
                let (_, hgt, dpt) = natural_metrics(content);
                hgt_max = hgt_max.max(hgt + pads.t);
                dpt_mag_max = dpt_mag_max.max(dpt + pads.b);
                restacc.push(None);
            }
            (None, Cell::Empty) => restacc.push(None),
            // A span-anchor `MultiCell` does not affect `hgt_max`/
            // `dpt_mag_max` itself (only `len`, for a *continuing* span) —
            // faithful to tabular.ml:34-42, where the analogous `aux` call
            // passes `hgtmax dptmin` through unchanged in this branch too.
            (None, Cell::Multi(nr, _nc, pads, content)) => {
                let (_, hgt, dpt) = natural_metrics(content);
                let len = (hgt + pads.t) + (dpt + pads.b);
                let nr = (*nr).max(1);
                let restelem = if nr == 1 { None } else { Some((nr, len)) };
                restacc.push(restelem);
            }
            // A continuing multi-row span: the slot must be `EmptyCell`.
            (Some((numrow, len)), Cell::Empty) => {
                restacc.push(Some((*numrow, *len)));
            }
            // Malformed grid (upstream `assert false`, tabular.ml:54) — a
            // real cell where a span's continuation was expected. Degrade:
            // drop the stale pending span and treat this cell at face
            // value, as if the slot had been `None`.
            (Some(_), Cell::Normal(pads, content)) => {
                let (_, hgt, dpt) = natural_metrics(content);
                hgt_max = hgt_max.max(hgt + pads.t);
                dpt_mag_max = dpt_mag_max.max(dpt + pads.b);
                restacc.push(None);
            }
            (Some(_), Cell::Multi(nr, _nc, pads, content)) => {
                let (_, hgt, dpt) = natural_metrics(content);
                let len = (hgt + pads.t) + (dpt + pads.b);
                let nr = (*nr).max(1);
                let restelem = if nr == 1 { None } else { Some((nr, len)) };
                restacc.push(restelem);
            }
        }
    }
    let rest = restacc
        .into_iter()
        .map(|slot| match slot {
            None => None,
            Some((1, _)) => None,
            Some((numrow, len)) => Some((numrow - 1, len - hgt_max - dpt_mag_max)),
        })
        .collect();
    (rest, hgt_max, dpt_mag_max)
}

/// `determine_column_width` (tabular.ml:83): one column's width, plus the
/// updated `rest_column` for the next column right.
fn determine_column_width(restprev: &RestCol, col: &[&Cell]) -> (RestCol, Length) {
    let mut restacc: RestCol = Vec::with_capacity(col.len());
    let mut wid_max = Length::ZERO;
    for (slot, cell) in restprev.iter().zip(col.iter()) {
        match (slot, cell) {
            (None, Cell::Normal(pads, content)) => {
                let (wid, _, _) = natural_metrics(content);
                wid_max = wid_max.max(pads.l + wid + pads.r);
                restacc.push(None);
            }
            (None, Cell::Empty) => restacc.push(None),
            (None, Cell::Multi(_nr, nc, pads, content)) => {
                let (widraw, _, _) = natural_metrics(content);
                let wid = pads.l + widraw + pads.r;
                let nc = (*nc).max(1);
                if nc == 1 {
                    wid_max = wid_max.max(wid);
                }
                restacc.push(Some((nc, wid)));
            }
            (Some((numcol, widrest)), Cell::Empty) => {
                let numcol = *numcol;
                if numcol == 1 {
                    wid_max = wid_max.max(*widrest);
                }
                restacc.push(Some((numcol, *widrest)));
            }
            // Malformed grid (upstream `assert false`, tabular.ml:119) —
            // degrade like `determine_row_metrics` above.
            (Some(_), Cell::Normal(pads, content)) => {
                let (wid, _, _) = natural_metrics(content);
                wid_max = wid_max.max(pads.l + wid + pads.r);
                restacc.push(None);
            }
            (Some(_), Cell::Multi(_nr, nc, pads, content)) => {
                let (widraw, _, _) = natural_metrics(content);
                let wid = pads.l + widraw + pads.r;
                let nc = (*nc).max(1);
                if nc == 1 {
                    wid_max = wid_max.max(wid);
                }
                restacc.push(Some((nc, wid)));
            }
        }
    }
    let rest = restacc
        .into_iter()
        .map(|slot| match slot {
            None => None,
            Some((1, _)) => None,
            Some((numcol, wid)) => Some((numcol - 1, wid - wid_max)),
        })
        .collect();
    (rest, wid_max)
}

/// `multi_cell_width` (tabular.ml:207): the combined width of `nc` columns
/// starting at `index_c`, clamped to the grid's actual column count (a
/// malformed span that overruns the grid degrades to "the rest of the
/// grid" instead of panicking).
fn multi_cell_width(widlst: &[Length], index_c: usize, nc: usize) -> Length {
    if widlst.is_empty() {
        return Length::ZERO;
    }
    let end = (index_c + nc).saturating_sub(1).min(widlst.len() - 1);
    widlst[index_c.min(end)..=end]
        .iter()
        .fold(Length::ZERO, |acc, w| acc + *w)
}

/// `multi_cell_vertical` (tabular.ml:220): the combined `height + depth
/// magnitude` of `nr` rows starting at `index_r`, clamped like
/// `multi_cell_width` above.
fn multi_cell_vertical(vmetrlst: &[(Length, Length)], index_r: usize, nr: usize) -> Length {
    if vmetrlst.is_empty() {
        return Length::ZERO;
    }
    let end = (index_r + nr).saturating_sub(1).min(vmetrlst.len() - 1);
    vmetrlst[index_r.min(end)..=end]
        .iter()
        .fold(Length::ZERO, |acc, (hgt, dpt)| acc + *hgt + *dpt)
}

/// Wrap a cell's content with its left/right padding (tabular.ml:263-268's
/// `hblstwithpads`) — top/bottom padding never enters the horizontal box
/// list; it only ever affects `determine_row_metrics`'s row-height
/// arithmetic above.
fn pad_content(pads: Paddings, content: Vec<HorzBox>) -> Vec<HorzBox> {
    let mut out = Vec::with_capacity(content.len() + 2);
    out.push(HorzBox::Pure(PureHorzBox::FixedEmpty { width: pads.l }));
    out.extend(content);
    out.push(HorzBox::Pure(PureHorzBox::FixedEmpty { width: pads.r }));
    out
}

/// `solidify_tabular` (tabular.ml:229): fit every non-`Empty` cell's content
/// to its (possibly combined, for a span) column width and place it at its
/// box-local anchor.
fn solidify_tabular(
    vmetrlst: &[(Length, Length)],
    widlst: &[Length],
    xs: &[Length],
    ys: &[Length],
    htabular: Vec<Vec<Cell>>,
) -> Vec<TabularCellBox> {
    let mut cells = Vec::new();
    for (index_r, row) in htabular.into_iter().enumerate() {
        // Only the row's *height* is ever used for placement (Slice 1
        // never surfaces a per-cell depth — see `TabularCellBox`'s doc
        // comment; upstream's own `dpt` only ever feeds the roadmap-F
        // `warn_ratios` diagnostic, not placement).
        let hgt_row = vmetrlst
            .get(index_r)
            .map(|(h, _)| *h)
            .unwrap_or(Length::ZERO);
        let row_top = ys.get(index_r).copied().unwrap_or(Length::ZERO);
        for (index_c, cell) in row.into_iter().enumerate() {
            let x = xs.get(index_c).copied().unwrap_or(Length::ZERO);
            match cell {
                Cell::Empty => {}
                Cell::Normal(pads, content) => {
                    let wid = widlst.get(index_c).copied().unwrap_or(Length::ZERO);
                    let padded = pad_content(pads, content);
                    // Discard `fit_cell`'s own (height, depth): a
                    // `NormalCell` is placed at the *row's* shared metrics
                    // directly (tabular.ml:271's `ImNormalCell(ratios,
                    // (wid, hgtnmlcell, dptnmlcell), imhbs)` — the fitted
                    // content's own hgt/dpt never appear in that tuple).
                    let (contents, _fit_hgt, _fit_dpt) = fit_cell(padded, wid);
                    let baseline_y = row_top - hgt_row;
                    cells.push(TabularCellBox {
                        x,
                        baseline_y,
                        contents,
                    });
                }
                Cell::Multi(nr, nc, pads, content) => {
                    let nr = nr.max(1);
                    let nc = nc.max(1);
                    let wid = multi_cell_width(widlst, index_c, nc);
                    let padded = pad_content(pads, content);
                    let (contents, fit_hgt, fit_dpt) = fit_cell(padded, wid);
                    // A single-row span places like `NormalCell` (the row's
                    // own metrics); a multi-row span instead centers the
                    // *fitted content's own* extent within the combined
                    // span's vertical space (tabular.ml:288-297) — note the
                    // sign: upstream's `(hgt +% lenspace, dpt -% lenspace)`
                    // on a *negative* dpt is `(hgt + lenspace, dpt_mag +
                    // lenspace)` on our non-negative magnitude.
                    let hgt_cell = if nr == 1 {
                        hgt_row
                    } else {
                        let vlen_cell = multi_cell_vertical(vmetrlst, index_r, nr);
                        let vlen_content = fit_hgt + fit_dpt;
                        let lenspace = (vlen_cell - vlen_content) * 0.5;
                        fit_hgt + lenspace
                    };
                    let baseline_y = row_top - hgt_cell;
                    cells.push(TabularCellBox {
                        x,
                        baseline_y,
                        contents,
                    });
                }
            }
        }
    }
    cells
}

/// `Tabular.main` (tabular.ml:309): solve the whole grid — geometry,
/// solidified cells, and the grid-line coordinates a rule callback wants.
pub fn main(rows: Vec<Vec<Cell>>) -> Solved {
    let nrows = rows.len();
    let (ncols, htabular) = normalize_tabular(rows);

    // Row metrics, top-to-bottom, threading multi-row span bookkeeping
    // column-by-column.
    let mut restrow: RestRow = vec![None; ncols];
    let mut vmetrlst: Vec<(Length, Length)> = Vec::with_capacity(nrows);
    for row in &htabular {
        let (rest, hgt, dpt) = determine_row_metrics(&restrow, row);
        restrow = rest;
        vmetrlst.push((hgt, dpt));
    }

    // Column widths, left-to-right, threading multi-col span bookkeeping
    // row-by-row.
    let vtabular = transpose(&htabular, ncols);
    let mut restcol: RestCol = vec![None; nrows];
    let mut widlst: Vec<Length> = Vec::with_capacity(ncols);
    for col in &vtabular {
        let (rest, wid) = determine_column_width(&restcol, col);
        restcol = rest;
        widlst.push(wid);
    }

    let width = widlst.iter().fold(Length::ZERO, |acc, w| acc + *w);
    let height = vmetrlst
        .iter()
        .fold(Length::ZERO, |acc, (h, d)| acc + *h + *d);

    // Grid-line coordinates for the rule callback (handlePdf.ml's
    // `ops_of_evaled_tabular`): `xs` ascending from 0 (`ncols + 1` entries),
    // `ys` descending from `height` (row *tops*) to 0 (`nrows + 1` entries).
    let mut xs = Vec::with_capacity(ncols + 1);
    xs.push(Length::ZERO);
    let mut x = Length::ZERO;
    for w in &widlst {
        x = x + *w;
        xs.push(x);
    }
    let mut ys = Vec::with_capacity(nrows + 1);
    ys.push(height);
    let mut y = height;
    for (hgt, dpt) in &vmetrlst {
        y = y - (*hgt + *dpt);
        ys.push(y);
    }

    let cells = solidify_tabular(&vmetrlst, &widlst, &xs, &ys, htabular);

    Solved {
        width,
        height,
        cells,
        xs,
        ys,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cell whose `natural_metrics` are exactly `(w, h, d)` and whose own
    /// content width contributes nothing extra beyond that — a
    /// `Graphics` box with no elements, so tests don't need real glyph
    /// metrics to get deterministic geometry.
    fn probe(w: f64, h: f64, d: f64) -> Vec<HorzBox> {
        vec![HorzBox::Pure(PureHorzBox::Graphics {
            width: Length::pt(w),
            height: Length::pt(h),
            depth: Length::pt(d),
            elems: Vec::new(),
        })]
    }

    fn zero_pads() -> Paddings {
        Paddings {
            l: Length::ZERO,
            r: Length::ZERO,
            t: Length::ZERO,
            b: Length::ZERO,
        }
    }

    #[test]
    fn two_by_two_normal_grid_geometry() {
        // col0 widths 30/25 -> 30; col1 widths 20/15 -> 20.
        // row0 (12,3)/(8,2) -> hgt 12, dpt 3, vlen 15.
        // row1 (10,4)/(6,1) -> hgt 10, dpt 4, vlen 14.
        let rows = vec![
            vec![
                Cell::Normal(zero_pads(), probe(30.0, 12.0, 3.0)),
                Cell::Normal(zero_pads(), probe(20.0, 8.0, 2.0)),
            ],
            vec![
                Cell::Normal(zero_pads(), probe(25.0, 10.0, 4.0)),
                Cell::Normal(zero_pads(), probe(15.0, 6.0, 1.0)),
            ],
        ];
        let solved = main(rows);

        assert_eq!(solved.width, Length::pt(50.0));
        assert_eq!(solved.height, Length::pt(29.0));
        assert_eq!(
            solved.xs,
            vec![Length::pt(0.0), Length::pt(30.0), Length::pt(50.0)]
        );
        assert_eq!(
            solved.ys,
            vec![Length::pt(29.0), Length::pt(14.0), Length::pt(0.0)]
        );
        assert_eq!(solved.cells.len(), 4);

        // cellA (row0,col0): x=0, baseline = 29 - 12 = 17.
        assert_eq!(solved.cells[0].x, Length::pt(0.0));
        assert_eq!(solved.cells[0].baseline_y, Length::pt(17.0));
        // cellB (row0,col1): x=30, baseline = 17.
        assert_eq!(solved.cells[1].x, Length::pt(30.0));
        assert_eq!(solved.cells[1].baseline_y, Length::pt(17.0));
        // cellC (row1,col0): x=0, baseline = 14 - 10 = 4.
        assert_eq!(solved.cells[2].x, Length::pt(0.0));
        assert_eq!(solved.cells[2].baseline_y, Length::pt(4.0));
        // cellD (row1,col1): x=30, baseline = 4.
        assert_eq!(solved.cells[3].x, Length::pt(30.0));
        assert_eq!(solved.cells[3].baseline_y, Length::pt(4.0));
    }

    #[test]
    fn empty_cell_produces_no_box() {
        let rows = vec![vec![
            Cell::Normal(zero_pads(), probe(10.0, 5.0, 1.0)),
            Cell::Empty,
        ]];
        let solved = main(rows);
        assert_eq!(solved.cells.len(), 1);
        assert_eq!(solved.xs.len(), 3);
    }

    #[test]
    fn multi_column_span_absorbs_following_empty() {
        // row0: Multi(1,2, w=50) | Empty
        // row1: Normal(w=20)     | Normal(w=25)
        // col0 width is forced to 20 by row1; col1 must then absorb the
        // multi-cell's remaining 50 - 20 = 30 (tabular.ml:119's `rest`
        // bookkeeping — this is exactly the "following Empty is absorbed"
        // acceptance case).
        let rows = vec![
            vec![
                Cell::Multi(1, 2, zero_pads(), probe(50.0, 10.0, 2.0)),
                Cell::Empty,
            ],
            vec![
                Cell::Normal(zero_pads(), probe(20.0, 5.0, 1.0)),
                Cell::Normal(zero_pads(), probe(25.0, 6.0, 1.0)),
            ],
        ];
        let solved = main(rows);

        assert_eq!(
            solved.xs,
            vec![Length::pt(0.0), Length::pt(20.0), Length::pt(50.0)]
        );
        // 3 boxes: the Multi cell + the two row1 Normals; the row0 Empty
        // (the span's reserved slot) produces none.
        assert_eq!(solved.cells.len(), 3);
        // The multi-cell starts at column 0.
        assert_eq!(solved.cells[0].x, Length::pt(0.0));
    }

    #[test]
    fn tabular_box_measures_as_a_single_leaf() {
        let rows = vec![vec![Cell::Normal(zero_pads(), probe(30.0, 12.0, 3.0))]];
        let solved = main(rows);
        let tab = TabularBox {
            width: solved.width,
            height: solved.height,
            depth: Length::ZERO,
            cells: solved.cells,
            rules: Vec::new(),
        };
        let bx = HorzBox::Pure(PureHorzBox::Tabular(tab.clone()));
        assert_eq!(
            crate::linebreak::natural_metrics(std::slice::from_ref(&bx)),
            (tab.width, tab.height, Length::ZERO)
        );
        let HorzBox::Pure(p) = &bx;
        assert!(!p.is_glue());
        assert_eq!(p.natural_width(), tab.width);
    }
}

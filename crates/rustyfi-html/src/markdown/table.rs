//! `PureHorzBox::Tabular` -> a GFM pipe table, or — when it is an aligned
//! equation rather than a table — something that is not a grid at all
//! ([`render_aligned_equation`]).
//!
//! Row grouping is `crate::recover::table_rows`, shared with the HTML
//! backend. What differs is everything after it, because a pipe table is a
//! much narrower thing than a `<table>`:
//!
//! - **Rules are dropped.** GFM has one table style and no way to say which
//!   grid lines a table draws, so `TabularBox::rules` — which the HTML
//!   backend reads carefully, since `easytable`'s booktabs default and a
//!   fully-ruled grid must not render alike — has nowhere to go.
//! - **The first row becomes the header**, because GFM's grammar requires a
//!   header and a delimiter row; a table with no header row in the document's
//!   own sense simply gets its first row emphasized by the renderer. The one
//!   exception is a recovered `+align`
//!   (`crate::recover::is_aligned_equation`), which gets an EMPTY header
//!   instead — promoting an EQUATION to a column heading is wrong in a way
//!   that promoting a data row is not.
//! - **A cell is one line.** Anything multi-line collapses to spaces
//!   (`escape::table_cell`), since a pipe row ends at the newline.
//! - **Ragged rows are padded** to the widest, since a row with fewer cells
//!   than the header is not a table at all to most parsers.
//!
//! ## The phantom table
//!
//! `easytable` builds every table TWICE (`table-builder.satyh`'s `build`):
//! once with the real cell text and no rules, and once as the same grid of
//! EMPTY cells carrying only the rule callbacks, drawing both into one
//! `inline-graphics`. In the PDF the second is invisible — it is nothing but
//! the lines. Rendered literally it becomes an empty grid above every real
//! table, forty of them in `easytable`'s own manual. Since this backend draws
//! no rules at all, the rules-only twin holds nothing whatever, and
//! [`render_table`] returns `None` for a table no cell of which has text.

use std::fmt::Write as _;

use rustyfi_backend::{PureHorzBox, TabularBox};

use super::escape;
use super::para::{Para, Piece};
use super::Ctx;

/// One table as a GFM pipe table, or `None` when it holds no text at all —
/// see this module's doc comment on the phantom table.
pub(super) fn render_table(tab: &TabularBox, ctx: &Ctx) -> Option<String> {
    let rows = crate::recover::table_rows(tab);
    if rows.is_empty() {
        return None;
    }
    let mut text: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    let mut any_content = false;
    for row in &rows {
        let mut cells = Vec::with_capacity(row.len());
        for cell in row {
            // A cell is a hard flow boundary in both directions: glue left
            // pending by the previous cell must not open this one with a
            // space, and this cell's last character must not decide the
            // spacing of the next.
            ctx.reset_flow();
            let mut para = Para {
                open: true,
                ..Para::default()
            };
            for (_, bx) in &cell.contents {
                match bx {
                    // A cell wide enough to WRAP holds a whole nested block
                    // rather than an inline run, and `emit_inline`'s
                    // `EmbeddedBlock` arm is inert because only the block
                    // walker can close a paragraph. Left at that the cell
                    // comes out EMPTY — `easytable`'s own `lw 120pt` example
                    // loses its third column entirely, which is what that
                    // section of the manual is demonstrating. (The HTML
                    // backend has the same hole; fixing it there would change
                    // its output, and this backend is meant to be purely
                    // additive.)
                    PureHorzBox::EmbeddedBlock { block, .. } => {
                        let inner = super::block::render_block(block, ctx);
                        // Already Markdown, so it is not escaped again — but
                        // a `|` in it would end the row it is sitting in.
                        let inner = inner.trim().replace('|', "\\|");
                        para.push_markup(inner.clone(), inner);
                    }
                    _ => super::inline::emit_inline(&mut para, bx, ctx),
                }
            }
            ctx.reset_flow();
            // Rendered as prose whatever its face: a fixed-pitch cell becomes
            // a code SPAN, never a fence, because a fence inside a pipe cell
            // is not expressible.
            let rendered = Para {
                mono: false,
                has_mono: false,
                ..para
            }
            .render(None, true)
            .map(|r| r.text)
            .unwrap_or_default();
            let cell_text = escape::table_cell(&rendered);
            any_content |= !cell_text.is_empty();
            cells.push(cell_text);
        }
        text.push(cells);
    }
    if !any_content {
        return None;
    }
    let width = text.iter().map(Vec::len).max().unwrap_or(0);
    if width == 0 {
        return None;
    }

    let mut out = String::new();
    // **An aligned equation gets an EMPTY header row.** GFM's grammar has no
    // headerless table — the delimiter row is mandatory and it always follows
    // row one — so "this grid has no heading" can only be said by writing a
    // header with nothing in it. Without that, a two-row `+align` renders as a
    // ONE-row table whose column heading is an equation, which is not a
    // cosmetic loss: a renderer sets a header cell bold and centred, and
    // `pandoc`, GitHub and every reading tool downstream then treat the first
    // equation as a label for the second.
    //
    // Only for a recovered `+align` (`crate::recover::is_aligned_equation`),
    // not for tables in general. A real table is far more often headed than
    // not — `easytable`'s manual is nothing but headed tables — and NOTHING in
    // the box stream distinguishes a headed one from a headless one, so the
    // documented first-row-is-the-header rule stays exactly as it was for
    // everything this classifier does not claim. This mode is the only place
    // an aligned equation still reaches this function at all: the drawing
    // modes divert it before here (`markdown::inline`'s `Tabular` arm).
    let headless = crate::recover::is_aligned_equation(tab);
    if headless {
        push_row(&mut out, &[], width);
        push_delimiter(&mut out, width);
    }
    for (r, row) in text.iter().enumerate() {
        push_row(&mut out, row, width);
        if r == 0 && !headless {
            push_delimiter(&mut out, width);
        }
    }
    // A one-row table still needs its delimiter row, which the loop above has
    // already written; nothing more to do.
    Some(out)
}

/// One pipe row, padded to `width` cells — a row with fewer than the header's
/// is not a table at all to most parsers.
fn push_row(out: &mut String, row: &[String], width: usize) {
    out.push('|');
    for c in 0..width {
        let _ = write!(out, " {} |", row.get(c).map_or("", String::as_str));
    }
    out.push('\n');
}

/// GFM's delimiter row. No alignment colons: `TabularCellBox` records where a
/// cell was PLACED, not how its column was declared to align, and inferring
/// alignment from the placement would guess wrong for every column whose
/// content happens to be the same width.
fn push_delimiter(out: &mut String, width: usize) {
    out.push('|');
    for _ in 0..width {
        out.push_str(" --- |");
    }
    out.push('\n');
}

/// A recovered `+align` (`crate::recover::is_aligned_equation`) as one or more
/// Markdown BLOCKS, for the modes that draw or typeset an equation.
///
/// Called instead of [`render_table`], never as well — see `markdown::inline`'s
/// `Tabular` arm for the mode test. `--unicode-math` does not come here at
/// all: it writes characters, and a two-column text table is a defensible way
/// to SHOW an alignment in plain text, so there the grid stays (with an empty
/// header row, so no equation is promoted to a heading).
///
/// ## `--katex`: one `\begin{aligned}`, and why it is a translation
///
/// A cell boundary in `+align` is an alignment point, and `&` is LaTeX's
/// spelling for exactly that. `crate::recover::is_aligned_equation` has
/// already established that the columns run RIGHT, LEFT, RIGHT, … — which is
/// the column pattern `aligned` (and `align`, which `math.satyh`'s `+align`
/// exists to imitate) is DEFINED as — so joining a row's cells with `&` and
/// its rows with `\\` reproduces the document's own alignment rather than
/// approximating it. Every alignment point survives, including the second and
/// later pairs of a multi-column `+align`, which `aligned` handles natively.
///
/// This does not contradict `crate::latex`'s "matrices and `\begin{aligned}`
/// do not come back". That is true INSIDE one math box, where the row and
/// column arrangement is carried by glyph positions and nothing delimits it.
/// Here the arrangement is not inferred at all: the document built a real
/// `tabular` and the cells are still cells.
///
/// ## The drawing modes: one block per ROW
///
/// A row is one equation — `${x + y}` and `${= a^2 + 2ab}` are two halves of
/// it, split at the alignment point — so the row's cells are joined into a
/// single paragraph, and each row becomes its own block, which is what the
/// document does with them.
///
/// **The column alignment BETWEEN rows is lost, and that is a deliberate
/// trade.** It could be kept: the `tabular` has already been solved, so each
/// cell carries the `x` its column was placed at, and one `<svg>` per row at
/// the full grid width with each cell translated to its own `x` would line the
/// `=` signs up exactly. It is not done because it would put a second copy of
/// the layout's geometry in this backend and mint a kind of drawing the HTML
/// backend has no counterpart for — to buy an alignment that only survives for
/// a reader whose renderer keeps the `<svg>` at all. A sanitizing renderer
/// drops it, a terminal shows nothing, and both of those are ordinary ways to
/// read a `.md`. Two centred equations, each whole, degrade better than a
/// grid that is either perfect or absent.
pub(super) fn render_aligned_equation(tab: &TabularBox, ctx: &Ctx) -> Vec<String> {
    let rows = crate::recover::table_rows(tab);
    match ctx.math {
        // Both drawing modes: one block per row, the row's cells joined.
        crate::MathMode::SvgOutline | crate::MathMode::SvgText => rows
            .iter()
            .filter_map(|row| render_row(row, ctx))
            .collect(),
        crate::MathMode::Katex => {
            let body: Vec<String> = rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| cell_latex(cell, ctx))
                        .collect::<Vec<_>>()
                        .join(" & ")
                })
                .filter(|r| !r.trim().is_empty())
                .collect();
            if body.is_empty() {
                return Vec::new();
            }
            // One LINE, not one line per row. Every other block this backend
            // writes is a single line (`Para::render` collapses one), and a
            // `$$` block broken over several lines has to survive whatever
            // indentation the enclosing list adds to each of them.
            vec![format!(
                "$$\\begin{{aligned}} {} \\end{{aligned}}$$",
                body.join(" \\\\ ")
            )]
        }
        // Diverted before this is called; see the doc comment.
        crate::MathMode::Unicode => Vec::new(),
    }
}

/// One row of a `+align` as one paragraph — the cells joined, in order.
fn render_row(row: &[&rustyfi_backend::TabularCellBox], ctx: &Ctx) -> Option<String> {
    let mut para = Para {
        open: true,
        ..Para::default()
    };
    for (i, cell) in row.iter().enumerate() {
        // A cell is a hard flow boundary, exactly as in [`render_table`]: the
        // fil that aligned it must not become a space, and the previous cell's
        // last character must not decide this one's spacing.
        ctx.reset_flow();
        if i > 0 {
            // One space at the alignment point, written rather than inferred.
            // The boundary is real — the second cell of `+align`'s pair opens
            // with the relation — and the glue rule cannot supply it, because
            // both sides of the join are `inline-fil`s worth no width.
            para.push_text(" ", false);
        }
        for (_, bx) in &cell.contents {
            super::inline::emit_inline(&mut para, bx, ctx);
        }
    }
    ctx.reset_flow();
    // `in_cell: false` — this IS a block of its own now, so a row holding one
    // drawing may be pretty-printed, which is the whole point of not being a
    // table any more.
    para.render(None, false).map(|r| r.text)
}

/// One cell's LaTeX, for the `aligned` body. The classifier has already
/// established that a cell holds nothing but equations, so the pieces are
/// concatenated with no delimiters of their own — `Piece::Math` is
/// undelimited by construction (`crate::latex`'s "delimiters are the
/// CALLER's"), and the caller here is the `$$…$$` around the whole block.
fn cell_latex(cell: &rustyfi_backend::TabularCellBox, ctx: &Ctx) -> String {
    let mut para = Para {
        open: true,
        ..Para::default()
    };
    ctx.reset_flow();
    for (_, bx) in &cell.contents {
        super::inline::emit_inline(&mut para, bx, ctx);
    }
    ctx.reset_flow();
    para.pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Math { latex, .. } => Some(latex.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

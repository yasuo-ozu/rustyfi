//! `PureHorzBox::Tabular` -> a GFM pipe table.
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
//!   own sense simply gets its first row emphasized by the renderer.
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

use rustyfi_backend::TabularBox;

use super::escape;
use super::para::Para;
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
                super::inline::emit_inline(&mut para, bx, ctx);
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
            .render(None)
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
    for (r, row) in text.iter().enumerate() {
        out.push('|');
        for c in 0..width {
            let _ = write!(out, " {} |", row.get(c).map_or("", String::as_str));
        }
        out.push('\n');
        if r == 0 {
            // GFM's delimiter row. No alignment colons: `TabularCellBox`
            // records where a cell was PLACED, not how its column was
            // declared to align, and inferring alignment from the placement
            // would guess wrong for every column whose content happens to be
            // the same width.
            out.push('|');
            for _ in 0..width {
                out.push_str(" --- |");
            }
            out.push('\n');
        }
    }
    // A one-row table still needs its delimiter row, which the loop above has
    // already written; nothing more to do.
    Some(out)
}

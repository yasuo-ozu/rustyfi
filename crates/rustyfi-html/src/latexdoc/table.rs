//! `PureHorzBox::Tabular` -> a LaTeX `tabular`.
//!
//! Row grouping is [`crate::recover::table_rows`] and the grid lines are
//! [`crate::recover::Borders`] — both shared with the HTML backend, both
//! documented there.
//!
//! ## The document's own rules, not a blanket grid
//!
//! `{|l|l|l|}` with an `\hline` between every row is what a naive writer
//! emits and it is wrong for most of the corpus: `easytable`'s default draws
//! three horizontal rules and no verticals (the booktabs look), and a
//! `\easytable` with explicit column separators draws a full grid. The two
//! must not render alike. So the column specification carries a `|` only
//! where a vertical rule was actually recovered, and an `\hline` goes only
//! where a horizontal one was.
//!
//! Thickness and colour are DROPPED. `\hline` has one width for the whole
//! table (`\arrayrulewidth`) and no colour at all without `colortbl`, and a
//! per-rule `\specialrule` would need `booktabs` for a distinction no
//! document in the corpus makes — every rule in every table it draws is the
//! same hairline. The HTML backend keeps both because CSS costs nothing to
//! say them in.
//!
//! ## Alignment is not recoverable, so every column is `l`
//!
//! `TabularCellBox` records where a cell was PLACED, not how its column was
//! declared to align. Inferring `c` from "the cell is centred in its column"
//! guesses wrong for every column whose content happens to fill it, and
//! guesses right only by accident otherwise. The Markdown backend declines to
//! emit alignment colons for the same reason.
//!
//! ## The phantom table
//!
//! `easytable` builds every table TWICE — see
//! [`crate::recover::overlaid_table_rules`]. The rules-only twin holds no
//! text at all, so [`render_table`] returns `None` for a table no cell of
//! which has any, and the real twin picks the rules up by matching its own
//! width and height against what the enclosing graphics box recorded.

use std::fmt::Write as _;

use rustyfi_backend::{GraphicsElem, PureHorzBox, TabularBox};

use super::para::Para;
use super::Ctx;

/// One table as a LaTeX `tabular`, or `None` when it holds no text at all —
/// see this module's doc comment on the phantom table.
pub(super) fn render_table(tab: &TabularBox, ctx: &Ctx) -> Option<String> {
    let rows = crate::recover::table_rows(tab);
    if rows.is_empty() {
        return None;
    }

    // A table with no rules of its own may be the text-bearing half of an
    // `easytable` pair; the rules travelled with the invisible twin, and the
    // enclosing graphics box is where they were seen.
    let paired;
    let rules: &[GraphicsElem] = if tab.rules.is_empty() {
        paired = ctx
            .tabular_rules
            .borrow()
            .iter()
            .rev()
            .find(|(w, h, _)| {
                (w - tab.width.0).abs() < crate::recover::RULE_EPS_PT
                    && (h - tab.height.0).abs() < crate::recover::RULE_EPS_PT
            })
            .map(|(_, _, r)| r.clone());
        paired.as_deref().unwrap_or(&[])
    } else {
        &tab.rules
    };
    let borders = crate::recover::Borders::solve(&rows, rules);

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
            let cell_text = match cell.contents.as_slice() {
                // A cell wide enough to WRAP holds one whole nested block
                // rather than an inline run, and the document told us the
                // measure it wrapped at. A `minipage` of that measure is the
                // LaTeX for exactly that — and it is also the only container
                // inside a `tabular` where a `Verbatim`, a list or a `\par`
                // is legal at all, everything else being LR mode.
                //
                // `emit_inline`'s `EmbeddedBlock` arm is inert (only the
                // block walker can close a paragraph), so without this the
                // cell comes out EMPTY: `easytable`'s own `lw 120pt` example
                // loses its third column entirely, which is what that
                // section of the manual is demonstrating.
                [(_, PureHorzBox::EmbeddedBlock { block, width, .. })] => {
                    let inner = super::block::render_block(block, ctx);
                    let inner = inner.trim();
                    if inner.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "\\begin{{minipage}}[t]{{{:.3}bp}}\n{inner}\n\\end{{minipage}}",
                            width.0.max(1.0)
                        )
                    }
                }
                contents => {
                    let mut para = Para {
                        open: true,
                        ..Para::default()
                    };
                    for (_, bx) in contents {
                        match bx {
                            // An embedded block sharing its cell with inline
                            // content has no measure of its own to give a
                            // minipage, so it goes in beside the text — and
                            // must therefore open no environment.
                            PureHorzBox::EmbeddedBlock { block, .. } => {
                                let was = ctx.inline_only.replace(true);
                                let inner = super::block::render_block(block, ctx);
                                ctx.inline_only.set(was);
                                let inner = inner.trim().to_string();
                                para.push_markup(inner.clone(), inner);
                            }
                            _ => super::inline::emit_inline(&mut para, bx, ctx),
                        }
                    }
                    // Rendered as prose whatever its face: a fixed-pitch cell
                    // becomes `\texttt{}`, never a `Verbatim`, because a
                    // verbatim environment in an alignment cell reads the `&`
                    // and `\\` that delimit the cell itself.
                    let rendered = Para {
                        mono: false,
                        has_mono: false,
                        ..para
                    }
                    .render(None)
                    .map(|r| r.text)
                    .unwrap_or_default();
                    cell_body(&rendered)
                }
            };
            ctx.reset_flow();
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
    let _ = writeln!(out, "\\begin{{tabular}}{{{}}}", colspec(&borders, width));
    for (r, row) in text.iter().enumerate() {
        if borders.horizontal.get(r).copied().flatten().is_some() {
            out.push_str("\\hline\n");
        }
        for c in 0..width {
            if c > 0 {
                out.push_str(" & ");
            }
            out.push_str(row.get(c).map_or("", String::as_str));
        }
        // Every row ends `\\`, including the last: an `\hline` after the
        // final row needs one before it, and a trailing `\\` on the last row
        // is harmless (it adds no empty row — LaTeX's `tabular` ends at
        // `\end`).
        out.push_str(" \\\\\n");
    }
    if borders.horizontal.last().copied().flatten().is_some() {
        out.push_str("\\hline\n");
    }
    out.push_str("\\end{tabular}");
    Some(out)
}

/// The column specification: `l` per column, with a `|` wherever a vertical
/// rule was recovered — see this module's doc comment on why `l` and not `c`.
fn colspec(borders: &crate::recover::Borders, width: usize) -> String {
    let mut spec = String::with_capacity(width * 2 + 1);
    for c in 0..width {
        if borders.vertical.get(c).copied().flatten().is_some() {
            spec.push('|');
        }
        spec.push('l');
    }
    if borders.vertical.get(width).copied().flatten().is_some() {
        spec.push('|');
    }
    spec
}

/// A cell's body, made safe to sit inside an alignment.
///
/// `escape::text` has already neutralised the document's own `&` and `\`, so
/// the only thing left is the LINE STRUCTURE: a raw newline inside a cell is
/// harmless to `tabular` itself but a blank line is a `\par`, which is
/// `Paragraph ended before \\ was complete` — a hard error. Everything
/// collapses to spaces.
fn cell_body(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for c in s.chars() {
        let c = if c == '\n' || c == '\r' { ' ' } else { c };
        if c == ' ' {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A blank line inside a cell is `Paragraph ended before \\ was
    /// complete`, which stops the compile dead.
    #[test]
    fn a_cell_can_never_break_its_row() {
        assert_eq!(cell_body("a\n\nb"), "a b");
        assert_eq!(cell_body("  spaced   out  "), "spaced out");
    }
}

//! The composite `Tabular` box's PDF writer arm: a page whose only content is
//! a `Tabular` (one text cell + one rule) renders both the cell's `Tj` run and
//! the rule's path ops into the same content stream and coordinate frame — the
//! reentrant `emit_box` reconciling page y-down / box y-up / cell-baseline-y-up
//! in one `ty + cell.baseline_y` expression.

use rustyfi_backend::{
    Closing, Color, FontKey, GraphicsElem, HorzStringInfo, Length, Page, PageGeometry, Path,
    PathSeg, PlacedLine, PureHorzBox, Subpath, TabularBox, TabularCellBox,
};

fn page_with_tabular_box() -> Page {
    let cell = TabularCellBox {
        x: Length::pt(2.0),
        baseline_y: Length::pt(3.0),
        contents: vec![(
            Length::pt(1.0),
            PureHorzBox::InnerString {
                info: HorzStringInfo {
                    font: FontKey(0),
                    size: Length::pt(12.0),
                    rising: Length::ZERO,
                    color: Color::Gray(0.0),
                },
                text: "A".to_string(),
                width: Length::pt(8.0),
                height: Length::pt(9.0),
                depth: Length::pt(2.0),
            },
        )],
    };
    // One vertical rule at the box's own x=0, spanning its full height.
    let rule_path = Path {
        subpaths: vec![Subpath {
            start: (Length::pt(0.0), Length::pt(0.0)),
            segs: vec![PathSeg::Line((Length::pt(0.0), Length::pt(10.0)))],
            closing: Closing::Open,
        }],
    };
    let rules = vec![GraphicsElem::Stroke(
        Length::pt(1.0),
        Color::Gray(0.0),
        rule_path,
    )];
    let tab = TabularBox {
        width: Length::pt(20.0),
        height: Length::pt(10.0),
        depth: Length::ZERO,
        cells: vec![cell],
        rules,
    };
    Page {
            body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(Length::ZERO, PureHorzBox::Tabular(tab))],
        }],
    }
}

#[test]
fn tabular_box_renders_cell_text_and_rule_ops_in_the_same_frame() {
    // Round numbers throughout, so every expected operand is an exact
    // integer string.
    let geometry = PageGeometry {
        paper_width: Length::pt(200.0),
        paper_height: Length::pt(300.0),
        text_origin: (Length::pt(20.0), Length::pt(20.0)),
        text_width: Length::pt(160.0),
        text_height: Length::pt(260.0),
    };
    let page = page_with_tabular_box();
    let bytes = rustyfi_pdf::render_pdf(&geometry, std::slice::from_ref(&page), &[])
        .expect("PDF rendering must succeed");
    let hay = String::from_utf8_lossy(&bytes);

    // The box itself is placed exactly like a `Graphics`/`Image` box: one
    // `cm` translate to `(line.x + dx, paper_h - baseline_y)` = (50, 200).
    let box_ty = geometry.paper_height.0 - 100.0;
    let expected_cm = format!("1 0 0 1 50 {box_ty} cm");
    assert!(
        hay.contains(&expected_cm),
        "missing the tabular box's own placement transform {expected_cm:?}:\n{hay}"
    );

    // Cell text: `Td` to `(tx + cell.x + cdx, ty + cell.baseline_y)` = (50 +
    // 2 + 1, 200 + 3) = (53, 203) — the box's placed anchor plus the cell's
    // own box-local offset, composed without a second y-flip.
    let expected_td = format!("53 {} Td", box_ty + 3.0);
    assert!(
        hay.contains(&expected_td),
        "missing cell text placement {expected_td:?}:\n{hay}"
    );
    assert!(hay.contains("(A) Tj"), "missing cell text run:\n{hay}");

    // The rule renders through the same `place_graphics` a standalone
    // `inline-graphics` box uses, in the box's OWN frame — the same `cm` the
    // cell text's `Td` is offset from.
    for op in ["0 0 m", "0 10 l", "1 w", "0 G", "\nS\n"] {
        assert!(hay.contains(op), "content stream missing rule op {op:?}:\n{hay}");
    }
}

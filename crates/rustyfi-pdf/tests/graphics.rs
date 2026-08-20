//! Integration test for the Slice 1 graphics writer (`place_graphics`): a
//! page whose only content is a `PureHorzBox::Graphics` box (fill + stroke
//! of a rectangle) renders to a PDF whose uncompressed content stream
//! contains the expected path operators, with the box translated to its
//! placed, y-flipped anchor.

use rustyfi_backend::{
    Closing, Color, GraphicsElem, Length, Page, PageGeometry, Path, PathSeg, PlacedLine,
    PureHorzBox, Subpath,
};

/// A 20pt square, traced (0,0) -> (20,0) -> (20,20) -> (0,20) -> close —
/// exactly `start-path (0,0) |> line-to (20,0) |> line-to (20,20) |>
/// line-to (0,20) |> close-with-line` would build (this port has no `|>`
/// yet — see the e2e fixture in `rustyfi/tests/fixtures/graphics.saty`
/// for the real primitive chain).
fn rectangle_path() -> Path {
    Path {
        subpaths: vec![Subpath {
            start: (Length::pt(0.0), Length::pt(0.0)),
            segs: vec![
                PathSeg::Line((Length::pt(20.0), Length::pt(0.0))),
                PathSeg::Line((Length::pt(20.0), Length::pt(20.0))),
                PathSeg::Line((Length::pt(0.0), Length::pt(20.0))),
            ],
            closing: Closing::Line,
        }],
    }
}

fn page_with_graphics_box() -> Page {
    let path = rectangle_path();
    let elems = vec![
        GraphicsElem::Fill(Color::Rgb(1.0, 0.0, 0.0), path.clone()),
        GraphicsElem::Stroke(Length::pt(1.0), Color::Gray(0.0), path),
    ];
    let gbox = PureHorzBox::Graphics {
        width: Length::pt(20.0),
        height: Length::pt(20.0),
        depth: Length::pt(0.0),
        elems,
        origin_independent: false,
    };
    Page {
            body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(Length::ZERO, gbox)],
        }],
    }
}

#[test]
fn graphics_box_renders_path_operators_into_the_content_stream() {
    // Round numbers throughout (not `PageGeometry::default`'s mm-converted
    // values) so the expected `cm` transform below is an exact integer
    // string, not a float-formatting guess.
    let geometry = PageGeometry {
        paper_width: Length::pt(200.0),
        paper_height: Length::pt(300.0),
        text_origin: (Length::pt(20.0), Length::pt(20.0)),
        text_width: Length::pt(160.0),
        text_height: Length::pt(260.0),
    };
    let page = page_with_graphics_box();
    let bytes = rustyfi_pdf::render_pdf(&geometry, std::slice::from_ref(&page), &[])
        .expect("PDF rendering must succeed");
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    let hay = String::from_utf8_lossy(&bytes);

    // Path construction: move-to the subpath start, three line-tos tracing
    // the rectangle, and `close_path` (`h`) on its own line (zero operands).
    for op in ["0 0 m", "20 0 l", "20 20 l", "0 20 l", "\nh\n"] {
        assert!(hay.contains(op), "content stream missing {op:?}:\n{hay}");
    }
    // Fill (even-odd — upstream's `op_f'`, not nonzero) in RGB red, then a
    // stroke at 1pt gray — each paints its own re-emitted copy of the path
    // (a PDF path must be re-specified per paint operator).
    for op in ["1 0 0 rg", "f*", "1 w", "0 G", "\nS\n"] {
        assert!(hay.contains(op), "content stream missing {op:?}:\n{hay}");
    }

    // The box is translated as a whole to its placed, already-y-flipped
    // anchor: `(line.x + dx, paper_h - baseline_y)` = (50, paper_h - 100),
    // via a single `cm` (matching `place_image`'s technique) — never a
    // per-coordinate flip (which would mirror the rectangle vertically).
    let expected_ty = geometry.paper_height.0 - 100.0;
    let expected_cm = format!("1 0 0 1 50 {expected_ty} cm");
    assert!(
        hay.contains(&expected_cm),
        "content stream missing the box's placement transform {expected_cm:?}:\n{hay}"
    );
}

// ============================================================================
// L5b (prim-retype-sweep §3.3, §4.2 test 5): the `Group`/`Clip` container
// arms `place_graphics` gained for 0.1's graphics-collection sweep. Never
// reachable from a 0.0.6 program (no 0.0.6-visible prim constructs either
// variant — see `GraphicsElem`'s doc comment); these are the tripwire that a
// FUTURE 0.1 caller renders the right ops, exercised directly at the
// `GraphicsElem` level exactly like the fixture above.
// ============================================================================

fn render_elems(elems: Vec<GraphicsElem>) -> String {
    let geometry = PageGeometry {
        paper_width: Length::pt(200.0),
        paper_height: Length::pt(300.0),
        text_origin: (Length::pt(20.0), Length::pt(20.0)),
        text_width: Length::pt(160.0),
        text_height: Length::pt(260.0),
    };
    let gbox = PureHorzBox::Graphics {
        width: Length::pt(20.0),
        height: Length::pt(20.0),
        depth: Length::pt(0.0),
        elems,
        origin_independent: false,
    };
    let page = Page {
            body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(Length::ZERO, gbox)],
        }],
    };
    let bytes = rustyfi_pdf::render_pdf(&geometry, std::slice::from_ref(&page), &[])
        .expect("PDF rendering must succeed");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn clip_emits_w_star_n_between_the_clip_path_and_its_contents() {
    let clip_path = rectangle_path();
    let fill = GraphicsElem::Fill(Color::Rgb(1.0, 0.0, 0.0), rectangle_path());
    let hay = render_elems(vec![GraphicsElem::Clip(clip_path, vec![fill])]);

    // `graphicD.ml:323-336`: path ops, then `W*`, then `n` (end-path, no
    // paint), THEN the contents' own paint ops — the per-element q…Q
    // wrapper already supplies the surrounding save/restore.
    let path_pos = hay.find("0 0 m").expect("clip path move-to");
    let w_pos = hay.find("\nW*\n").expect("W* clip op");
    let n_pos = hay.find("\nn\n").expect("n end-path op");
    let fill_pos = hay.find("f*").expect("fill paint op");
    assert!(
        path_pos < w_pos && w_pos < n_pos && n_pos < fill_pos,
        "expected path < W* < n < fill, got positions {path_pos} < {w_pos} < {n_pos} < {fill_pos}:\n{hay}"
    );
}

#[test]
fn group_renders_identically_to_its_flattened_element_sequence() {
    let fill = GraphicsElem::Fill(Color::Rgb(1.0, 0.0, 0.0), rectangle_path());
    let stroke = GraphicsElem::Stroke(Length::pt(1.0), Color::Gray(0.0), rectangle_path());

    let grouped = render_elems(vec![GraphicsElem::Group(vec![fill.clone(), stroke.clone()])]);
    let flat = render_elems(vec![fill, stroke]);

    // The `Group` arm recurses with a zero anchor into the SAME per-element
    // q…Q loop `place_graphics` already runs for a flat `Vec` — byte-compare
    // (up to the harmless extra identity `q`/`cm(0,0)`/`Q` nesting the
    // recursive call adds around the group's own two elements).
    for op in ["1 0 0 rg", "f*", "1 w", "0 G", "\nS\n"] {
        assert!(grouped.contains(op), "grouped stream missing {op:?}:\n{grouped}");
        assert!(flat.contains(op), "flat stream missing {op:?}:\n{flat}");
    }
}

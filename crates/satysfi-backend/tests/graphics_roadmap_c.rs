//! Backend-unit coverage for roadmap C2 (`PureHorzBox::GraphicsOuter`'s
//! fil-width write-back through `fit_cell`/`natural_metrics`) and C3b
//! (`path_bbox`'s exact cubic-extrema bounding box), independent of the
//! lang-side primitives that build these values in practice
//! (`crates/satysfi-lang/tests/stdlib_tier0.rs` covers those end to end).

use satysfi_backend::*;

fn outer(fn_id: usize, h: f64, d: f64) -> HorzBox {
    HorzBox::Pure(PureHorzBox::GraphicsOuter {
        height: Length::pt(h),
        depth: Length::pt(d),
        width: Length::ZERO,
        fn_id: GraphicsFnId(fn_id),
    })
}

fn fixed(w: f64) -> HorzBox {
    HorzBox::Pure(PureHorzBox::FixedEmpty { width: Length::pt(w) })
}

fn fil() -> HorzBox {
    HorzBox::Pure(PureHorzBox::OuterFil)
}

// ============================================================================
// C2: `fit_cell` writes the resolved per-fil width share back into a
// `GraphicsOuter` box, exactly like an `OuterFil` shares the same slack.
// ============================================================================

#[test]
fn fit_cell_writes_the_resolved_width_into_a_lone_graphics_outer() {
    let content = vec![outer(0, 5.0, 2.0), fixed(20.0)];
    let (contents, height, depth) = fit_cell(content, Length::pt(50.0));

    assert_eq!(contents.len(), 2);
    // Slack = 50 - 20 (GraphicsOuter's own natural width is 0) = 30, all of
    // which the lone GraphicsOuter absorbs (fil semantics: it's the only
    // "fil" box on the line).
    let (x0, b0) = &contents[0];
    assert_eq!(*x0, Length::ZERO);
    match b0 {
        PureHorzBox::GraphicsOuter { width, .. } => {
            assert!((width.0 - 30.0).abs() < 1e-9, "expected width 30pt, got {width:?}");
        }
        other => panic!("expected GraphicsOuter, got {other:?}"),
    }
    let (x1, b1) = &contents[1];
    assert!((x1.0 - 30.0).abs() < 1e-9, "expected x offset 30pt, got {x1:?}");
    assert!(matches!(b1, PureHorzBox::FixedEmpty { .. }));

    // Line metrics: height/depth max'd in from the GraphicsOuter's own (h,d).
    assert!((height.0 - 5.0).abs() < 1e-9);
    assert!((depth.0 - 2.0).abs() < 1e-9);
}

#[test]
fn fit_cell_splits_slack_three_ways_across_two_graphics_outers_and_a_fil() {
    // Two `GraphicsOuter`s + one `OuterFil`, all zero natural width, on a
    // 30pt-wide cell: slack = 30, split evenly three ways (10pt each) —
    // `justify_line`'s `fil_count` counts `OuterFil` and `GraphicsOuter`
    // identically (upstream `Fils(nfil)`, roadmap C2's design summary).
    let content = vec![outer(0, 3.0, 0.0), fil(), outer(1, 4.0, 1.0)];
    let (contents, height, depth) = fit_cell(content, Length::pt(30.0));

    assert_eq!(contents.len(), 3);
    match &contents[0].1 {
        PureHorzBox::GraphicsOuter { width, fn_id, .. } => {
            assert_eq!(*fn_id, GraphicsFnId(0));
            assert!((width.0 - 10.0).abs() < 1e-9, "got {width:?}");
        }
        other => panic!("expected GraphicsOuter, got {other:?}"),
    }
    // The `OuterFil` box itself doesn't carry a resolved width field (it's
    // zero-width by definition, `natural_width` returns ZERO for it too),
    // but it still occupies its 1/3 share of the x-advance budget: the third
    // box's x offset proves it.
    match &contents[2].1 {
        PureHorzBox::GraphicsOuter { width, fn_id, .. } => {
            assert_eq!(*fn_id, GraphicsFnId(1));
            assert!((width.0 - 10.0).abs() < 1e-9, "got {width:?}");
        }
        other => panic!("expected GraphicsOuter, got {other:?}"),
    }
    let (x2, _) = &contents[2];
    assert!((x2.0 - 20.0).abs() < 1e-9, "expected x offset 20pt, got {x2:?}");

    assert!((height.0 - 4.0).abs() < 1e-9, "height should max in both GraphicsOuters");
    assert!((depth.0 - 1.0).abs() < 1e-9, "depth should max in both GraphicsOuters");
}

// ============================================================================
// C2: `natural_metrics` treats an unresolved `GraphicsOuter` as fil
// (zero-width), while still folding its height/depth into the run's outer
// metrics.
// ============================================================================

#[test]
fn natural_metrics_gives_a_graphics_outer_zero_width_but_maxes_in_height_depth() {
    let boxes = vec![fixed(10.0), outer(0, 8.0, 3.0), fixed(5.0)];
    let (width, height, depth) = natural_metrics(&boxes);
    assert!((width.0 - 15.0).abs() < 1e-9, "GraphicsOuter contributes 0 width, got {width:?}");
    assert!((height.0 - 8.0).abs() < 1e-9);
    assert!((depth.0 - 3.0).abs() < 1e-9);
}

// ============================================================================
// C3b: `path_bbox`'s exact cubic-extrema bounding box vs. the (looser)
// control-point hull it replaces.
// ============================================================================

/// `Gr.circle (cx, cy) r`'s exact path shape (`gr.satyh:128-134`): 3
/// `bezier-to`s + a `close-with-bezier`, `k = r * 0.55228`.
fn circle_path(cx: f64, cy: f64, r: f64) -> Path {
    let k = r * 0.55228;
    let pt = |x: f64, y: f64| (Length::pt(x), Length::pt(y));
    Path {
        subpaths: vec![Subpath {
            start: pt(cx - r, cy),
            segs: vec![
                PathSeg::Bezier(pt(cx - r, cy + k), pt(cx - k, cy + r), pt(cx, cy + r)),
                PathSeg::Bezier(pt(cx + k, cy + r), pt(cx + r, cy + k), pt(cx + r, cy)),
                PathSeg::Bezier(pt(cx + r, cy - k), pt(cx + k, cy - r), pt(cx, cy - r)),
            ],
            closing: Closing::Bezier(pt(cx - k, cy - r), pt(cx - r, cy - k)),
        }],
    }
}

#[test]
fn path_bbox_of_a_circle_is_exact_within_the_radius_not_the_wider_control_hull() {
    let path = circle_path(50.0, 50.0, 10.0);
    let (pmin, pmax) = path_bbox(&path);
    assert!(
        (pmin.0 .0 - 40.0).abs() < 1e-3
            && (pmin.1 .0 - 40.0).abs() < 1e-3
            && (pmax.0 .0 - 60.0).abs() < 1e-3
            && (pmax.1 .0 - 60.0).abs() < 1e-3,
        "expected exact bbox (40,40)-(60,60), got {pmin:?}-{pmax:?}"
    );
}

/// Companion to the exact circle test above: a single cubic segment whose
/// control points genuinely overshoot the curve's own extent on one axis
/// (a classic "control polygon wider than the curve" shape) — proves
/// `path_bbox` reports the tight curve extent, not the control-point hull it
/// used to.
#[test]
fn path_bbox_of_a_bulging_bezier_is_tighter_than_its_control_hull() {
    let pt = |x: f64, y: f64| (Length::pt(x), Length::pt(y));
    // A cubic from (0,0) to (10,0) whose control points sit at y=20 — the
    // curve itself never reaches anywhere near y=20 (its true max is
    // 3/4 * 20 = 15, at t=0.5, the standard symmetric-cubic peak), but the
    // control-point hull would report y_max = 20.
    let path = Path {
        subpaths: vec![Subpath {
            start: pt(0.0, 0.0),
            segs: vec![PathSeg::Bezier(pt(0.0, 20.0), pt(10.0, 20.0), pt(10.0, 0.0))],
            closing: Closing::Open,
        }],
    };
    let (pmin, pmax) = path_bbox(&path);
    assert!(
        (pmax.1 .0 - 15.0).abs() < 1e-6,
        "expected exact y_max 15pt (control hull would give 20pt), got {:?}",
        pmax.1
    );
    assert!((pmin.1 .0 - 0.0).abs() < 1e-9);
    assert!((pmin.0 .0 - 0.0).abs() < 1e-9 && (pmax.0 .0 - 10.0).abs() < 1e-9);
}

#[test]
fn path_bbox_of_a_straight_line_path_is_unaffected_by_the_bezier_upgrade() {
    // A pure `Line`-only path (no Bezier segments) should behave exactly as
    // before: bbox = the corner extremes of its own points.
    let path = Path {
        subpaths: vec![Subpath {
            start: (Length::pt(0.0), Length::pt(0.0)),
            segs: vec![
                PathSeg::Line((Length::pt(10.0), Length::pt(0.0))),
                PathSeg::Line((Length::pt(10.0), Length::pt(20.0))),
                PathSeg::Line((Length::pt(0.0), Length::pt(20.0))),
            ],
            closing: Closing::Line,
        }],
    };
    let (pmin, pmax) = path_bbox(&path);
    assert_eq!(pmin, (Length::pt(0.0), Length::pt(0.0)));
    assert_eq!(pmax, (Length::pt(10.0), Length::pt(20.0)));
}

//! `register-destination` called from inside an `inline-graphics` callback.
//!
//! Upstream runs that callback DURING page breaking, with the box's true
//! placed point, so a `register-destination` inside it is squarely inside
//! `annotation.ml:15`'s window. This port runs it EAGERLY at construction
//! time (`prim_inline_graphics`'s documented shortcut), where no page exists
//! — so the same call used to hit the gate and abort the whole document.
//! `azmath`'s `equation.satyh` anchors every `\label`ed equation exactly this
//! way (`layout-tests/probes/inline_graphics_dest.saty` is the package-free
//! reproduction), so this was the single thing keeping its manual from
//! building at all.
//!
//! The fix RE-TIMES the call rather than relaxing the gate: the eager
//! application records it, the box carries it as a
//! `GraphicsElem::Destination` marker, and `fire_hooks` replays it once the
//! box has a page and a placed point. These tests pin both halves and the
//! join, plus the fact that the gate is untouched everywhere else.

use rustyfi_backend::{
    FontKey, FontMetrics, GraphicsElem, HorzBox, Length, Page, PageGeometry, PlacedLine,
    PureHorzBox,
};
use rustyfi_lang::ast::Ast;
use rustyfi_lang::eval;
use rustyfi_lang::primitives;
use rustyfi_lang::value::{DocumentValue, Value};
use rustyfi_syntax::Span;
use std::rc::Rc;

struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, _c: char, size: Length) -> Option<Length> {
        Some(size * 0.5)
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

fn var(name: &str) -> Ast {
    Ast::Var(name.to_string(), Span::default())
}

fn apply_all(name: &str, args: Vec<Ast>) -> Ast {
    args.into_iter()
        .fold(var(name), |f, a| Ast::Apply(Box::new(f), Box::new(a)))
}

fn len(pt: f64) -> Ast {
    Ast::Length(Length::pt(pt))
}

/// `fun p -> register-destination <key> p before []` — azmath's
/// `ib-annotation` shape reduced to its essentials: a callback whose ONLY
/// effect is the registration and whose ink is empty.
fn anchor_callback(key: &str) -> Ast {
    Ast::Lambda(
        "p".to_string(),
        Rc::new(Ast::Sequential(
            Box::new(apply_all(
                "register-destination",
                vec![Ast::Str(key.to_string()), var("p")],
            )),
            Box::new(Ast::List(Vec::new())),
        )),
    )
}

/// `inline-graphics 0pt 0pt 0pt <callback>`.
fn anchor_box_ast(key: &str) -> Ast {
    Ast::Apply(
        Box::new(apply_all(
            "inline-graphics",
            vec![len(0.0), len(0.0), len(0.0)],
        )),
        Box::new(anchor_callback(key)),
    )
}

fn eval_ast(interp: &mut eval::Interp, ast: &Ast) -> Result<Value, eval::EvalError> {
    let env = primitives::base_env();
    interp.eval(&env, ast)
}

fn geometry() -> PageGeometry {
    PageGeometry {
        paper_width: Length::pt(400.0),
        paper_height: Length::pt(300.0),
        text_origin: (Length::pt(0.0), Length::pt(0.0)),
        text_width: Length::pt(400.0),
        text_height: Length::pt(300.0),
    }
}

fn doc_with_pages(pages: Vec<Page>) -> DocumentValue {
    DocumentValue {
        geometry: geometry(),
        pages,
        images: Vec::new(),
        extras: Default::default(),
        reflow_source: None,
        reflow_links: Vec::new(),
        reflow_dests: Vec::new(),
    }
}

/// One page holding `bx` on a single line at `(x, baseline_y)`.
fn page_with(bx: PureHorzBox, x: f64, baseline_y: f64) -> Page {
    Page {
        body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::pt(x),
            baseline_y: Length::pt(baseline_y),
            contents: vec![(Length::ZERO, bx)],
        }],
    }
}

/// A hand-built anchor box, so the placement arithmetic can be pinned against
/// a point that is neither zero nor the box's own origin.
fn marker_box(key: &str, pt: (f64, f64), origin_independent: bool) -> PureHorzBox {
    PureHorzBox::Graphics {
        width: Length::ZERO,
        height: Length::ZERO,
        depth: Length::ZERO,
        elems: vec![GraphicsElem::Destination {
            key: key.to_string(),
            pt: (Length::pt(pt.0), Length::pt(pt.1)),
        }],
        origin_independent,
    }
}

// ---------------------------------------------------------------------------
// The primitive half: the eager call records instead of erroring.
// ---------------------------------------------------------------------------

/// THE regression. Before the fix this errored with "register-destination can
/// only be called during page breaking", which is what azmath's manual and
/// `layout-tests/probes/inline_graphics_dest.saty` both died on.
#[test]
fn register_destination_inside_an_inline_graphics_callback_does_not_error() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let v = eval_ast(&mut interp, &anchor_box_ast("eq:pythagoras"))
        .expect("an anchor callback must not hit the during-page-break gate");
    let boxes = match v {
        Value::InlineBoxes(bs) => bs,
        other => panic!("expected inline-boxes, got {other:?}"),
    };
    let elems = match &boxes[..] {
        [HorzBox::Pure(PureHorzBox::Graphics { elems, .. })] => elems,
        other => panic!("expected one Graphics box, got {other:?}"),
    };
    assert_eq!(
        elems,
        &vec![GraphicsElem::Destination {
            key: "eq:pythagoras".to_string(),
            pt: (Length::ZERO, Length::ZERO),
        }],
        "the deferred registration must ride in the box as a marker at the \
         box-local point the callback named"
    );
    // Nothing was committed yet: the point is not known until the box is placed.
    assert!(
        interp.destinations.is_empty(),
        "an eager callback must DEFER, not register"
    );
}

/// The probe run's copy of the registration is discarded, not committed
/// twice: `prim_inline_graphics` applies the callback a second time at a
/// far-off point to classify it.
#[test]
fn the_origin_independence_probe_does_not_duplicate_the_registration() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let v = eval_ast(&mut interp, &anchor_box_ast("k")).expect("must evaluate");
    let elems = match &v {
        Value::InlineBoxes(bs) => match &bs[..] {
            [HorzBox::Pure(PureHorzBox::Graphics { elems, .. })] => elems,
            other => panic!("expected one Graphics box, got {other:?}"),
        },
        other => panic!("expected inline-boxes, got {other:?}"),
    };
    assert_eq!(elems.len(), 1, "one call, one marker: {elems:?}");
}

/// An anchor-only callback draws NOTHING, so on ink alone the origin and probe
/// runs look identical and the box would be classified page-absolute — which
/// would then place every anchor at the callback's raw argument instead of
/// where the box actually landed. The classifier compares the marker-bearing
/// elements for exactly this reason.
///
/// Mutation check: drop the markers from either side of the
/// `origin_independent` comparison in `prim_inline_graphics` and this fails.
#[test]
fn an_anchor_only_callback_is_not_classified_page_absolute() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let v = eval_ast(&mut interp, &anchor_box_ast("k")).expect("must evaluate");
    match &v {
        Value::InlineBoxes(bs) => match &bs[..] {
            [HorzBox::Pure(PureHorzBox::Graphics {
                origin_independent, ..
            })] => assert!(
                !origin_independent,
                "a callback whose registration point tracks its argument is \
                 position-RELATIVE, however empty its ink"
            ),
            other => panic!("expected one Graphics box, got {other:?}"),
        },
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The firing half: `fire_hooks` replays a marker at the box's placed point.
// ---------------------------------------------------------------------------

/// Mutation check: change either term of `paper_height - anchor_y + pt.1` in
/// `fire_nested_in_graphics` and this fails.
#[test]
fn fire_hooks_resolves_a_marker_to_the_boxs_placed_page_and_point() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    // Page 2 of 2, so a wrong page index cannot pass by defaulting to 0.
    let blank = Page {
        body_lines: usize::MAX,
        lines: Vec::new(),
    };
    let doc = doc_with_pages(vec![
        blank,
        page_with(marker_box("eq:1", (3.0, 7.0), false), 50.0, 100.0),
    ]);

    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks must succeed");

    assert_eq!(interp.destinations.len(), 1);
    let d = &interp.destinations[0];
    assert_eq!(d.page, 1, "the page the box was PLACED on");
    assert_eq!(d.name, "nameddest0", "minted through the shared dest_names");
    assert_eq!(d.x, Length::pt(53.0), "line x (50) + box-local x (3)");
    // A `NamedDest`'s y is PDF y-UP; the walk's `baseline_y` is y-down from the
    // top of a 300pt page, so the baseline is at y-up 200 and the marker 7
    // above it.
    assert_eq!(d.y, Length::pt(207.0));
}

/// The page-absolute counterpart: a callback that ignores its point already
/// produced final coordinates, so the writers emit its ink under an identity
/// `cm` and its anchor must not be translated either.
#[test]
fn a_page_absolute_boxs_marker_is_taken_as_already_final() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let doc = doc_with_pages(vec![page_with(
        marker_box("fixed", (3.0, 7.0), true),
        50.0,
        100.0,
    )]);

    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks must succeed");

    assert_eq!(interp.destinations.len(), 1);
    let d = &interp.destinations[0];
    assert_eq!((d.x, d.y), (Length::pt(3.0), Length::pt(7.0)));
}

/// End to end across the two halves, which is what azmath actually needs: the
/// value the primitive builds, placed and fired, yields the destination.
#[test]
fn an_evaluated_anchor_box_becomes_a_named_destination_once_placed() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let v = eval_ast(&mut interp, &anchor_box_ast("eq:euler")).expect("must evaluate");
    let bx = match v {
        Value::InlineBoxes(mut bs) => {
            let HorzBox::Pure(p) = bs.remove(0);
            p
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    };
    let doc = doc_with_pages(vec![page_with(bx, 120.0, 60.0)]);

    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks must succeed");

    assert_eq!(interp.destinations.len(), 1);
    let d = &interp.destinations[0];
    assert_eq!(d.page, 0);
    assert_eq!(d.name, "nameddest0");
    // The callback registered at its own argument, so the anchor is the box's
    // placed origin exactly: (120, 300 - 60).
    assert_eq!((d.x, d.y), (Length::pt(120.0), Length::pt(240.0)));
}

// ---------------------------------------------------------------------------
// The gate is re-timed, not relaxed.
// ---------------------------------------------------------------------------

/// Outside an `inline-graphics` callback nothing changed: no page, no
/// destination, loudly. (The eager window must not leak past the callback.)
#[test]
fn the_during_page_break_gate_still_refuses_a_bare_call() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    // Evaluate an anchor box FIRST, so a window that failed to close would be
    // open for the call below.
    eval_ast(&mut interp, &anchor_box_ast("k")).expect("must evaluate");
    let bare = apply_all(
        "register-destination",
        vec![
            Ast::Str("chapter1".to_string()),
            Ast::Tuple(vec![len(10.0), len(20.0)]),
        ],
    );
    let err = eval_ast(&mut interp, &bare).expect_err("the gate must still refuse this");
    assert!(
        err.msg.contains("page breaking"),
        "error should still name the during-page-break gate: {}",
        err.msg
    );
}

/// Inside a page-break window the DIRECT path wins: a box built there is drawn
/// straight into `page_graphics`, which `fire_hooks` never re-walks, so a
/// marker minted for it would be silently dropped. `current_page` is `Some`,
/// the registration is real and immediate, and the box carries no marker.
#[test]
fn a_callback_running_during_a_page_break_registers_directly() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    interp.current_page = Some(0);
    let v = eval_ast(&mut interp, &anchor_box_ast("k")).expect("must evaluate");
    match &v {
        Value::InlineBoxes(bs) => match &bs[..] {
            [HorzBox::Pure(PureHorzBox::Graphics { elems, .. })] => {
                assert!(elems.is_empty(), "no deferral inside the window: {elems:?}");
            }
            other => panic!("expected one Graphics box, got {other:?}"),
        },
        other => panic!("expected inline-boxes, got {other:?}"),
    }
    // Both the origin run and the probe run registered directly — the probe is
    // pre-existing behaviour inside the window, and `write_named_dests` dedupes
    // by name. What matters is that the registrations are real.
    assert!(!interp.destinations.is_empty());
    assert!(interp.destinations.iter().all(|d| d.page == 0));
}

// ---------------------------------------------------------------------------
// The marker rides the ordinary graphics transform pipeline.
// ---------------------------------------------------------------------------

/// A marker is an ordinary box-local coordinate and must move with the ink
/// around it under `shift_graphics` — `math_boxes_of_inline_boxes` `dx`-shifts
/// a harvested graphics box's elements through exactly this call. (`fire_hooks`
/// has no `Math` arm today, so such a marker does not yet fire; see
/// `GraphicsElem::Destination`'s doc comment. The arithmetic is pinned here so
/// adding that arm is one arm and nothing else.)
#[test]
fn shift_graphics_moves_a_destination_marker_with_the_ink() {
    let m = GraphicsElem::Destination {
        key: "eq:1".to_string(),
        pt: (Length::pt(3.0), Length::pt(7.0)),
    };
    let shifted = rustyfi_backend::shift_graphics((Length::pt(10.0), Length::pt(20.0)), &m);
    assert_eq!(
        shifted,
        GraphicsElem::Destination {
            key: "eq:1".to_string(),
            pt: (Length::pt(13.0), Length::pt(27.0)),
        }
    );
}

/// A marker carries no ink, so it must not inflate the bbox of the box that
/// holds it — an anchor is normally a `0pt 0pt 0pt` `inline-graphics`.
#[test]
fn a_destination_marker_has_no_bounding_box() {
    let m = GraphicsElem::Destination {
        key: "eq:1".to_string(),
        pt: (Length::pt(3.0), Length::pt(7.0)),
    };
    assert!(rustyfi_backend::graphics_bbox(&m).is_none());
}

//! Reachability of every `#[subast]` edge in the generated box-tree visitor.
//!
//! `syan`'s `visitor!` follows a field only when the field's peeled head type
//! is named in the owning type's `#[subast(..)]` list. When the two fall out
//! of step the field is reclassified a leaf and the generated body for it is
//! **empty** — no error, no warning (`#[derive(Ast)]`'s own "entry matches no
//! field" lint goes through `proc_macro_error::emit_warning!`, which stable
//! rustc discards, and nothing checks the other direction at all). That is the
//! same silent-omission failure the visitor exists to abolish, so it needs its
//! own check.
//!
//! This is that check. For every field of every visited type that holds
//! another node, it plants a sentinel box reachable ONLY through that field
//! and asserts the traversal finds it.
//!
//! Two mechanisms keep it honest as the box types grow:
//!
//! * `classify` below is a **wildcard-free** match over every `PureHorzBox` /
//!   `VertBox` / `GraphicsElem` variant. Adding a variant is a compile error
//!   here, which lands the author in this file.
//! * `covers_every_recursive_variant` asserts that every variant `classify`
//!   calls node-carrying has at least one edge exercised below.

use rustyfi_backend::graphics::{Color, GraphicsElem, Path};
use rustyfi_backend::hbox::{DecoId, GraphicsFnId, HookId, PureHorzBox};
use rustyfi_backend::pagebreak::{Page, PlacedLine};
use rustyfi_backend::tabular::{TabularBox, TabularCellBox};
use rustyfi_backend::vbox::VertBox;
use rustyfi_backend::Length;

/// A leaf box no other part of a fixture ever builds, so finding it proves the
/// traversal reached the exact slot it was planted in.
fn sentinel() -> PureHorzBox {
    PureHorzBox::FixedEmpty {
        width: Length::pt(1234.5),
    }
}

fn empty_path() -> Path {
    Path {
        subpaths: Vec::new(),
    }
}

fn is_sentinel(bx: &PureHorzBox) -> bool {
    *bx == sentinel()
}

/// A `VertBox::Line` whose only content is the sentinel.
fn sentinel_line() -> VertBox {
    VertBox::Line {
        height: Length::ZERO,
        depth: Length::ZERO,
        leading: Length::ZERO,
        contents: vec![(Length::ZERO, sentinel())],
    }
}

/// A `draw-text` graphics element whose only content is the sentinel.
fn sentinel_text() -> GraphicsElem {
    GraphicsElem::Text {
        pt: (Length::ZERO, Length::ZERO),
        contents: vec![(Length::ZERO, sentinel())],
        width: Length::ZERO,
        height: Length::ZERO,
        depth: Length::ZERO,
        transform: None,
    }
}

fn sentinel_cell() -> TabularCellBox {
    TabularCellBox {
        x: Length::ZERO,
        baseline_y: Length::ZERO,
        contents: vec![(Length::ZERO, sentinel())],
    }
}

/// Empty `TabularBox` to hang one edge off at a time.
fn tabular(cells: Vec<TabularCellBox>, rules: Vec<GraphicsElem>) -> TabularBox {
    TabularBox {
        width: Length::ZERO,
        height: Length::ZERO,
        depth: Length::ZERO,
        cells,
        rules,
    }
}

// ── The edge table ──────────────────────────────────────────────────────────
//
// Each entry names one field that holds another node, and builds a value in
// which the sentinel sits behind exactly that field. `check` counts sentinels
// the traversal reaches and requires exactly one.

fn count_in_box(bx: &PureHorzBox) -> usize {
    let mut n = 0;
    bx.visit(|b: &PureHorzBox| {
        if is_sentinel(b) {
            n += 1
        }
    });
    n
}

fn count_in_vert(vb: &VertBox) -> usize {
    let mut n = 0;
    vb.visit(|b: &PureHorzBox| {
        if is_sentinel(b) {
            n += 1
        }
    });
    n
}

fn count_in_graphics(g: &GraphicsElem) -> usize {
    let mut n = 0;
    g.visit(|b: &PureHorzBox| {
        if is_sentinel(b) {
            n += 1
        }
    });
    n
}

fn count_in_tabular(t: &TabularBox) -> usize {
    let mut n = 0;
    t.visit(|b: &PureHorzBox| {
        if is_sentinel(b) {
            n += 1
        }
    });
    n
}

/// Every edge this file exercises, as `"Type::variant.field"`. Kept as data so
/// `covers_every_recursive_variant` can cross-check it against `classify`.
const EDGES: &[&str] = &[
    "PureHorzBox::Discretionary.pre_break",
    "PureHorzBox::Discretionary.post_break",
    "PureHorzBox::Discretionary.no_break",
    "PureHorzBox::Frame.contents",
    "PureHorzBox::EmbeddedBlock.block",
    "PureHorzBox::Footnote.block",
    "PureHorzBox::Tabular.0",
    "PureHorzBox::Graphics.elems",
    "PureHorzBox::Math.rules",
    "VertBox::Line.contents",
    "GraphicsElem::Text.contents",
    "GraphicsElem::Group.0",
    "GraphicsElem::Clip.1",
    "TabularBox.cells",
    "TabularBox.rules",
    "TabularCellBox.contents",
    "PlacedLine.contents",
    "Page.lines",
];

#[test]
fn pure_horz_box_discretionary_slots_are_visited() {
    for (slot, bx) in [
        (
            "pre_break",
            PureHorzBox::Discretionary {
                penalty: 0,
                pre_break: vec![sentinel()],
                post_break: vec![],
                no_break: vec![],
            },
        ),
        (
            "post_break",
            PureHorzBox::Discretionary {
                penalty: 0,
                pre_break: vec![],
                post_break: vec![sentinel()],
                no_break: vec![],
            },
        ),
        (
            "no_break",
            PureHorzBox::Discretionary {
                penalty: 0,
                pre_break: vec![],
                post_break: vec![],
                no_break: vec![sentinel()],
            },
        ),
    ] {
        assert_eq!(
            count_in_box(&bx),
            1,
            "`Discretionary.{slot}` is not reached by the generated traversal"
        );
    }
}

#[test]
fn pure_horz_box_frame_contents_is_visited() {
    let bx = PureHorzBox::Frame {
        width: Length::ZERO,
        height: Length::ZERO,
        depth: Length::ZERO,
        deco: DecoId(0),
        contents: vec![(Length::ZERO, sentinel())],
    };
    assert_eq!(count_in_box(&bx), 1, "`Frame.contents` is not reached");
}

#[test]
fn pure_horz_box_embedded_block_is_visited() {
    let bx = PureHorzBox::EmbeddedBlock {
        width: Length::ZERO,
        height: Length::ZERO,
        depth: Length::ZERO,
        block: vec![sentinel_line()],
        anchor_last: false,
        breakable: false,
    };
    assert_eq!(count_in_box(&bx), 1, "`EmbeddedBlock.block` is not reached");
}

#[test]
fn pure_horz_box_footnote_block_is_visited() {
    let bx = PureHorzBox::Footnote {
        block: vec![sentinel_line()],
    };
    assert_eq!(count_in_box(&bx), 1, "`Footnote.block` is not reached");
}

#[test]
fn pure_horz_box_tabular_payload_is_visited() {
    let bx = PureHorzBox::Tabular(tabular(vec![sentinel_cell()], vec![]));
    assert_eq!(count_in_box(&bx), 1, "`Tabular.0` is not reached");
}

#[test]
fn pure_horz_box_graphics_elems_are_visited() {
    let bx = PureHorzBox::Graphics {
        width: Length::ZERO,
        height: Length::ZERO,
        depth: Length::ZERO,
        elems: vec![sentinel_text()],
        origin_independent: false,
    };
    assert_eq!(count_in_box(&bx), 1, "`Graphics.elems` is not reached");
}

#[test]
fn pure_horz_box_math_rules_are_visited() {
    let bx = PureHorzBox::Math {
        width: Length::ZERO,
        height: Length::ZERO,
        depth: Length::ZERO,
        glyphs: vec![],
        rules: vec![sentinel_text()],
    };
    assert_eq!(count_in_box(&bx), 1, "`Math.rules` is not reached");
}

#[test]
fn vert_box_line_contents_are_visited() {
    assert_eq!(
        count_in_vert(&sentinel_line()),
        1,
        "`VertBox::Line.contents` is not reached"
    );
}

#[test]
fn graphics_elem_edges_are_visited() {
    assert_eq!(
        count_in_graphics(&sentinel_text()),
        1,
        "`GraphicsElem::Text.contents` is not reached"
    );
    assert_eq!(
        count_in_graphics(&GraphicsElem::Group(vec![sentinel_text()])),
        1,
        "`GraphicsElem::Group.0` is not reached"
    );
    assert_eq!(
        count_in_graphics(&GraphicsElem::Clip(empty_path(), vec![sentinel_text()])),
        1,
        "`GraphicsElem::Clip.1` is not reached"
    );
}

#[test]
fn tabular_box_edges_are_visited() {
    assert_eq!(
        count_in_tabular(&tabular(vec![sentinel_cell()], vec![])),
        1,
        "`TabularBox.cells` is not reached"
    );
    assert_eq!(
        count_in_tabular(&tabular(vec![], vec![sentinel_text()])),
        1,
        "`TabularBox.rules` is not reached"
    );
}

#[test]
fn tabular_cell_box_contents_are_visited() {
    let mut n = 0;
    sentinel_cell().visit(|b: &PureHorzBox| {
        if is_sentinel(b) {
            n += 1
        }
    });
    assert_eq!(n, 1, "`TabularCellBox.contents` is not reached");
}

#[test]
fn placed_line_and_page_edges_are_visited() {
    let line = PlacedLine {
        x: Length::ZERO,
        baseline_y: Length::ZERO,
        contents: vec![(Length::ZERO, sentinel())],
    };
    let mut n = 0;
    line.visit(|b: &PureHorzBox| {
        if is_sentinel(b) {
            n += 1
        }
    });
    assert_eq!(n, 1, "`PlacedLine.contents` is not reached");

    let page = Page {
        lines: vec![line],
        body_lines: usize::MAX,
    };
    let mut n = 0;
    page.visit(|b: &PureHorzBox| {
        if is_sentinel(b) {
            n += 1
        }
    });
    assert_eq!(n, 1, "`Page.lines` is not reached");
}

/// Deep nesting through several node types at once — the case a per-edge test
/// cannot see, where one type's descent is generated but the recursion back
/// into it from a third type is not.
#[test]
fn a_sentinel_survives_a_seven_type_descent() {
    // Page -> PlacedLine -> PureHorzBox::Tabular -> TabularBox -> rules ->
    // GraphicsElem::Group -> GraphicsElem::Text -> PureHorzBox::EmbeddedBlock
    // -> VertBox::Line -> PureHorzBox::Discretionary -> sentinel.
    let deep = PureHorzBox::EmbeddedBlock {
        width: Length::ZERO,
        height: Length::ZERO,
        depth: Length::ZERO,
        block: vec![VertBox::Line {
            height: Length::ZERO,
            depth: Length::ZERO,
            leading: Length::ZERO,
            contents: vec![(
                Length::ZERO,
                PureHorzBox::Discretionary {
                    penalty: 0,
                    pre_break: vec![],
                    post_break: vec![],
                    no_break: vec![sentinel()],
                },
            )],
        }],
        anchor_last: false,
        breakable: false,
    };
    let text = GraphicsElem::Text {
        pt: (Length::ZERO, Length::ZERO),
        contents: vec![(Length::ZERO, deep)],
        width: Length::ZERO,
        height: Length::ZERO,
        depth: Length::ZERO,
        transform: None,
    };
    let page = Page {
        lines: vec![PlacedLine {
            x: Length::ZERO,
            baseline_y: Length::ZERO,
            contents: vec![(
                Length::ZERO,
                PureHorzBox::Tabular(tabular(vec![], vec![GraphicsElem::Group(vec![text])])),
            )],
        }],
        body_lines: usize::MAX,
    };
    let mut n = 0;
    page.visit(|b: &PureHorzBox| {
        if is_sentinel(b) {
            n += 1
        }
    });
    assert_eq!(n, 1, "the sentinel did not survive the full descent");
}

/// A visit is inclusive and pre-order: the root is offered to the closure
/// before its children, which is what consumers written against
/// `rustyfi_backend::visit` rely on.
#[test]
fn a_visit_is_inclusive_and_pre_order() {
    let inner = PureHorzBox::FixedEmpty {
        width: Length::pt(2.0),
    };
    let root = PureHorzBox::Frame {
        width: Length::pt(1.0),
        height: Length::ZERO,
        depth: Length::ZERO,
        deco: DecoId(0),
        contents: vec![(Length::ZERO, inner)],
    };
    let mut seen = Vec::new();
    root.visit(|b: &PureHorzBox| {
        seen.push(match b {
            PureHorzBox::Frame { .. } => "frame",
            PureHorzBox::FixedEmpty { .. } => "inner",
            _ => "other",
        })
    });
    assert_eq!(seen, vec!["frame", "inner"]);
}

// ── The variant census ──────────────────────────────────────────────────────

/// Wildcard-free classification: does this variant hold another node?
///
/// The point of the missing `_` arm is that a new `PureHorzBox` / `VertBox` /
/// `GraphicsElem` variant fails to compile HERE, so whoever adds it has to
/// decide whether it carries nodes — and, if it does, extend both the owning
/// type's `#[subast]` list and `EDGES` above.
fn classify(bx: &PureHorzBox) -> bool {
    match bx {
        PureHorzBox::Discretionary { .. }
        | PureHorzBox::Frame { .. }
        | PureHorzBox::EmbeddedBlock { .. }
        | PureHorzBox::Footnote { .. }
        | PureHorzBox::Tabular(_)
        | PureHorzBox::Graphics { .. }
        | PureHorzBox::Math { .. } => true,
        PureHorzBox::InnerString { .. }
        | PureHorzBox::OuterEmpty { .. }
        | PureHorzBox::OuterFil
        | PureHorzBox::FixedEmpty { .. }
        | PureHorzBox::Image { .. }
        | PureHorzBox::GraphicsOuter { .. }
        | PureHorzBox::HookPageBreak { .. }
        | PureHorzBox::FrameMarker { .. }
        | PureHorzBox::InlineFrameMarker { .. }
        | PureHorzBox::InlineMark(_) => false,
    }
}

fn classify_vert(vb: &VertBox) -> bool {
    match vb {
        VertBox::Line { .. } => true,
        VertBox::Skip(_)
        | VertBox::ParagTop(_)
        | VertBox::FramePad(_)
        | VertBox::ClearPage
        | VertBox::HookPageBreak(_)
        | VertBox::FrameStart(_)
        | VertBox::FrameEnd(_)
        | VertBox::ListMark(_) => false,
    }
}

fn classify_graphics(g: &GraphicsElem) -> bool {
    match g {
        GraphicsElem::Text { .. } | GraphicsElem::Group(_) | GraphicsElem::Clip(..) => true,
        GraphicsElem::Fill(..) | GraphicsElem::Stroke(..) | GraphicsElem::DashedStroke(..) => false,
    }
}

/// Every node-carrying variant `classify` knows about has at least one edge in
/// `EDGES`. Together with `classify`'s wildcard-free match, this makes "a new
/// box-carrying variant went untested" a red test rather than a quiet gap.
#[test]
fn covers_every_recursive_variant() {
    let carriers: Vec<(&str, bool)> = vec![
        (
            "PureHorzBox::Discretionary",
            classify(&PureHorzBox::Discretionary {
                penalty: 0,
                pre_break: vec![],
                post_break: vec![],
                no_break: vec![],
            }),
        ),
        (
            "PureHorzBox::Frame",
            classify(&PureHorzBox::Frame {
                width: Length::ZERO,
                height: Length::ZERO,
                depth: Length::ZERO,
                deco: DecoId(0),
                contents: vec![],
            }),
        ),
        (
            "PureHorzBox::EmbeddedBlock",
            classify(&PureHorzBox::EmbeddedBlock {
                width: Length::ZERO,
                height: Length::ZERO,
                depth: Length::ZERO,
                block: vec![],
                anchor_last: false,
                breakable: false,
            }),
        ),
        (
            "PureHorzBox::Footnote",
            classify(&PureHorzBox::Footnote { block: vec![] }),
        ),
        (
            "PureHorzBox::Tabular",
            classify(&PureHorzBox::Tabular(tabular(vec![], vec![]))),
        ),
        (
            "PureHorzBox::Graphics",
            classify(&PureHorzBox::Graphics {
                width: Length::ZERO,
                height: Length::ZERO,
                depth: Length::ZERO,
                elems: vec![],
                origin_independent: false,
            }),
        ),
        (
            "PureHorzBox::Math",
            classify(&PureHorzBox::Math {
                width: Length::ZERO,
                height: Length::ZERO,
                depth: Length::ZERO,
                glyphs: vec![],
                rules: vec![],
            }),
        ),
        ("VertBox::Line", classify_vert(&sentinel_line())),
        ("GraphicsElem::Text", classify_graphics(&sentinel_text())),
        (
            "GraphicsElem::Group",
            classify_graphics(&GraphicsElem::Group(vec![])),
        ),
        (
            "GraphicsElem::Clip",
            classify_graphics(&GraphicsElem::Clip(empty_path(), vec![])),
        ),
    ];
    for (name, carries) in carriers {
        assert!(carries, "`classify` says `{name}` carries no node");
        assert!(
            EDGES.iter().any(|e| e.starts_with(&format!("{name}."))),
            "`{name}` carries nodes but `EDGES` exercises none of its fields"
        );
    }
    // Non-carriers, so the census cannot pass by classifying everything `true`.
    assert!(!classify(&PureHorzBox::OuterFil));
    assert!(!classify(&PureHorzBox::HookPageBreak { id: HookId(0) }));
    assert!(!classify(&PureHorzBox::GraphicsOuter {
        height: Length::ZERO,
        depth: Length::ZERO,
        width: Length::ZERO,
        fn_id: GraphicsFnId(0),
    }));
    assert!(!classify_vert(&VertBox::ClearPage));
    assert!(!classify_graphics(&GraphicsElem::Fill(
        Color::Gray(0.0),
        empty_path()
    )));
    // Struct-shaped nodes have no variants to census; their single edge each is
    // covered by `tabular_*` / `placed_line_and_page_edges_are_visited`.
    for e in [
        "TabularBox.cells",
        "TabularBox.rules",
        "TabularCellBox.contents",
        "PlacedLine.contents",
        "Page.lines",
    ] {
        assert!(EDGES.contains(&e), "`{e}` fell out of `EDGES`");
    }
}

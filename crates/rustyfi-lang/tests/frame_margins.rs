//! Where a `block-frame-breakable`'s vertical MARGINS come from.
//!
//! Upstream normalizes a frame's contents STANDALONE — `aux None
//! TopMarginProhibited Alist.empty vblstsub` (`pageBreak.ml:664`) — so the
//! first inner block's `margin_top` is never appended, and the last inner
//! block's `margin_bottom` goes through `squash_margins _ []`, whose
//! empty-list arm (`:582-585`) emits no skip at all. What surrounds the frame
//! instead is the frame's OWN margins, taken from the OUTER context
//! (`vminstdef.yaml`'s `BackendVertFrame`: `margin_top = ctx.paragraph_top`,
//! `margin_bottom = ctx.paragraph_bottom`).
//!
//! This is not bookkeeping: the body's `ParagTop` carries
//! `min_first_line_ascender` folded in (`lineBreak.ml:855-857`) and the
//! frame's margin does not, so keeping the body's made the advance INTO a
//! frame a CONSTANT — `max(0, 9pt - hgt)` cancels the first line's own height.
//! Measured against real SATySFi 0.0.11 on
//! `scripts/layout_probes/code_line_height.saty`, `+code(`ooo`)` / `lll` /
//! `ggg` advance 29.114 / 31.166 / 29.138pt upstream; this port advanced a
//! flat 32.835pt for all three and now advances 29.115 / 31.167 / 29.139.

use rustyfi_backend::{FontKey, FontMetrics, Length, VertBox};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck};

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

fn eval_str(src: &str) -> Value {
    let file = rustyfi_syntax::parse_file(src).expect("parse");
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let program = elaborate::elaborate_program(&file, &scope).expect("elaborate");
    typecheck::typecheck(&program).expect("typecheck");
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    interp
        .eval(&env, &rustyfi_lang::ast::debrand(&program.body, &store))
        .expect("eval")
}

fn block_boxes(src: &str) -> Vec<VertBox> {
    match eval_str(src) {
        Value::BlockBoxes(vbs) => vbs,
        other => panic!("expected block-boxes, got {other:?}"),
    }
}

/// The OUTER context's paragraph margins bracket the frame; the INNER
/// paragraph's — deliberately absurd here, 100pt/200pt, so nothing else could
/// produce them — are dropped.
#[test]
fn a_frame_carries_the_outer_margins_and_its_body_carries_none() {
    let vbs = block_boxes(
        "let-inline ctx \\math m = inline-nil
         let mydeco pt l1 l2 l3 = []
         let ctx = get-initial-context 100pt (command \\math)
                     |> set-paragraph-margin 7pt 11pt
         in
         block-frame-breakable ctx (0pt, 0pt, 3pt, 5pt)
           (mydeco, mydeco, mydeco, mydeco)
           (fun ctxin -> (
              line-break true true (ctxin |> set-paragraph-margin 100pt 200pt)
                (read-inline ctxin {a})))",
    );

    let shape: Vec<String> = vbs
        .iter()
        .map(|vb| match vb {
            VertBox::Skip(l) => format!("Skip({})", l.0),
            VertBox::ParagTop(l) => format!("ParagTop({})", l.0),
            VertBox::FramePad(l) => format!("FramePad({})", l.0),
            VertBox::FrameStart(_) => "FrameStart".to_string(),
            VertBox::FrameEnd(_) => "FrameEnd".to_string(),
            VertBox::Line { .. } => "Line".to_string(),
            other => format!("{other:?}"),
        })
        .collect();

    assert_eq!(
        shape,
        vec![
            "Skip(7)",       // the frame's own margin_top  = OUTER paragraph_top
            "FrameStart",
            "FramePad(3)",   // paddingT — interior, additive, not a margin
            "Line",
            "FramePad(5)",   // paddingB
            "FrameEnd",
            "Skip(11)",      // the frame's own margin_bottom = OUTER paragraph_bottom
        ],
        "no 100pt/200pt anywhere: the body's own margins do not survive"
    );
}

/// Margins in the MIDDLE of the body are untouched — only the two at the ENDS
/// are the ones upstream never produces.
#[test]
fn a_frame_keeps_the_margins_between_its_body_blocks() {
    let vbs = block_boxes(
        "let-inline ctx \\math m = inline-nil
         let mydeco pt l1 l2 l3 = []
         let ctx = get-initial-context 100pt (command \\math)
                     |> set-paragraph-margin 7pt 11pt
         let para c = line-break true true c (read-inline c {a})
         in
         block-frame-breakable ctx (0pt, 0pt, 0pt, 0pt)
           (mydeco, mydeco, mydeco, mydeco)
           (fun ctxin -> (
              let c = ctxin |> set-paragraph-margin 100pt 200pt in
              para c +++ para c))",
    );
    let inner: Vec<&VertBox> = vbs
        .iter()
        .filter(|vb| !matches!(vb, VertBox::FramePad(_)))
        .collect();
    // Skip(7) FrameStart Line Skip(200) ParagTop(100) Line FrameEnd Skip(11)
    assert!(
        inner
            .iter()
            .any(|vb| matches!(vb, VertBox::Skip(l) if l.0 == 200.0)),
        "the first body block's BOTTOM margin is interior and survives: {inner:?}"
    );
    assert!(
        inner
            .iter()
            .any(|vb| matches!(vb, VertBox::ParagTop(l) if l.0 == 100.0)),
        "the second body block's TOP margin is interior and survives: {inner:?}"
    );
    assert!(
        matches!(inner.first(), Some(VertBox::Skip(l)) if l.0 == 7.0),
        "still bracketed by the outer margins: {inner:?}"
    );
    assert!(
        matches!(inner.last(), Some(VertBox::Skip(l)) if l.0 == 11.0),
        "still bracketed by the outer margins: {inner:?}"
    );
}

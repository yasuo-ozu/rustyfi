//! Page-level primitives blocking `mitou-report.satyh`/`stdjareport.satyh`:
//! `clear-page`, `hook-page-break-block`, `page-break-multicolumn`. Two
//! styles, mirroring existing coverage:
//! - **Typecheck + eval-through-`compile_document`** (`context_box.rs`/
//!   `typecheck.rs`'s style) for `clear-page`/`page-break-multicolumn` —
//!   real source text end to end, checking page counts.
//! - **Raw `Ast` apply chains through `eval::Interp`** (`hooks_crossref.rs`'s
//!   style) for `hook-page-break-block`, since that test needs to inspect
//!   the returned `Value::BlockBoxes`/`interp.hooks` directly, the same way
//!   the inline `hook-page-break` is pinned there.

use satysfi_backend::{
    chop_page, FontKey, FontMetrics, HookId, Length, Page, PageGeometry, PureHorzBox, VertBox,
};
use satysfi_lang::ast::Ast;
use satysfi_lang::eval;
use satysfi_lang::primitives;
use satysfi_lang::value::{DocumentValue, Value};
use satysfi_lang::{elaborate, typecheck, CompileError};
use satysfi_syntax::Span;
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

// ============================================================================
// Typecheck helpers (mirrors context_box.rs/typecheck.rs).
// ============================================================================

fn typecheck_str(src: &str) -> Result<(), CompileError> {
    let file = satysfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let program = elaborate::elaborate_program(&file, &scope)?;
    typecheck::typecheck(&program)?;
    Ok(())
}

fn assert_well_typed(src: &str) {
    if let Err(e) = typecheck_str(src) {
        panic!("expected {src:?} to type-check, got error: {e}");
    }
}

// ---- small Ast-builder helpers (mirrors hooks_crossref.rs) -----------------

fn var(name: &str) -> Ast {
    Ast::Var(name.to_string(), Span::default())
}

fn app1(f: Ast, a: Ast) -> Ast {
    Ast::Apply(Box::new(f), Box::new(a))
}

fn app2(name: &str, a: Ast, b: Ast) -> Ast {
    app1(app1(var(name), a), b)
}

fn str_lit(s: &str) -> Ast {
    Ast::Str(s.to_string())
}

// A shared skeleton: a 200pt-wide context, a generous single-column content
// scheme, and an empty header/footer — just enough page-break plumbing for
// `clear-page`/`page-break-multicolumn` to exercise real page geometry.
const PREAMBLE: &str = "
let-inline ctx \\math m = inline-nil
in
let ctx = get-initial-context 200pt (command \\math) in
let content pbinfo = (| text-origin = (0pt, 0pt); text-height = 400pt |) in
let parts pbinfo =
  (| header-origin = (0pt, 0pt); header-content = block-nil;
     footer-origin = (0pt, 0pt); footer-content = block-nil |)
in
";

// ============================================================================
// `clear-page` (mitou-report.satyh's `document`).
// ============================================================================

#[test]
fn clear_page_typechecks_as_block_boxes() {
    assert_well_typed(
        "let content pbinfo = (| text-origin = (0pt, 0pt); text-height = 100pt |) in
         let parts pbinfo =
           (| header-origin = (0pt, 0pt); header-content = block-nil;
              footer-origin = (0pt, 0pt); footer-content = block-nil |)
         in
         page-break A4Paper content parts (block-nil +++ clear-page)",
    );
}

#[test]
fn clear_page_forces_the_second_body_onto_a_new_page() {
    let mono = Mono;
    let src = format!(
        "{PREAMBLE}
         let body1 = line-break true true ctx (read-inline ctx {{Hello}}) in
         let body2 = line-break true true ctx (read-inline ctx {{World}}) in
         page-break A4Paper content parts (body1 +++ clear-page +++ body2)"
    );
    let doc = satysfi_lang::compile_document(&src, &mono)
        .expect("clear-page document should compile and evaluate");
    assert_eq!(
        doc.pages.len(),
        2,
        "clear-page must split the two bodies onto separate pages"
    );
    assert_eq!(doc.pages[0].lines.len(), 1, "page 1 should hold only body1");
    assert_eq!(doc.pages[1].lines.len(), 1, "page 2 should hold only body2");
}

#[test]
fn without_clear_page_both_bodies_share_one_page() {
    // Same document, minus the `clear-page` marker: control case proving the
    // 2-page split above is really caused by `clear-page`, not by overflow.
    let mono = Mono;
    let src = format!(
        "{PREAMBLE}
         let body1 = line-break true true ctx (read-inline ctx {{Hello}}) in
         let body2 = line-break true true ctx (read-inline ctx {{World}}) in
         page-break A4Paper content parts (body1 +++ body2)"
    );
    let doc = satysfi_lang::compile_document(&src, &mono).expect("should compile and evaluate");
    assert_eq!(doc.pages.len(), 1);
    assert_eq!(doc.pages[0].lines.len(), 2);
}

// ============================================================================
// backend-level: `chop_page`'s `VertBox::ClearPage` handling directly.
// ============================================================================

fn text_line() -> VertBox {
    VertBox::Line {
        height: Length::pt(10.0),
        depth: Length::pt(3.0),
        leading: Length::pt(14.0),
        contents: vec![],
    }
}

#[test]
fn chop_page_ends_the_page_right_after_a_clear_page_marker() {
    let mut vboxes = vec![text_line(), VertBox::ClearPage, text_line()];
    let page1 = chop_page((Length::ZERO, Length::ZERO), Length::pt(400.0), &mut vboxes);
    assert_eq!(page1.len(), 1, "clear-page ends the page after the first line");
    assert_eq!(vboxes.len(), 1, "only the trailing line should remain");
    let page2 = chop_page((Length::ZERO, Length::ZERO), Length::pt(400.0), &mut vboxes);
    assert_eq!(page2.len(), 1);
    assert!(vboxes.is_empty());
}

#[test]
fn chop_page_swallows_a_leading_clear_page_with_nothing_placed_yet() {
    let mut vboxes = vec![VertBox::ClearPage, text_line()];
    let page = chop_page((Length::ZERO, Length::ZERO), Length::pt(400.0), &mut vboxes);
    assert_eq!(
        page.len(),
        1,
        "a redundant leading clear-page must not produce a blank page"
    );
    assert!(vboxes.is_empty());
}

// ============================================================================
// `hook-page-break-block` (stdjareport.satyh's `document`) — mirrors
// hooks_crossref.rs's inline `hook-page-break` coverage.
// ============================================================================

#[test]
fn hook_page_break_block_typechecks_as_block_boxes() {
    assert_well_typed(
        "let content pbinfo = (| text-origin = (0pt, 0pt); text-height = 100pt |) in
         let parts pbinfo =
           (| header-origin = (0pt, 0pt); header-content = block-nil;
              footer-origin = (0pt, 0pt); footer-content = block-nil |)
         in
         page-break A4Paper content parts
           (block-nil +++ hook-page-break-block (fun pbinfo pt -> ()))",
    );
}

#[test]
fn hook_page_break_block_pushes_a_closure_and_returns_a_hookid_marker() {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let closure = Ast::Lambda(
        "pbinfo".to_string(),
        Rc::new(Ast::Lambda("_".to_string(), Rc::new(Ast::Unit))),
    );
    let ast = app1(var("hook-page-break-block"), closure);
    let v = interp.eval(&env, &ast).expect("evaluation should succeed");
    let Value::BlockBoxes(vboxes) = v else {
        panic!("expected block-boxes")
    };
    assert_eq!(vboxes, vec![VertBox::HookPageBreak(HookId(0))]);
    assert_eq!(
        interp.hooks.len(),
        1,
        "the closure must be pushed onto the hook table"
    );
}

/// End to end: evaluate `hook-page-break-block`, place its marker through
/// the real `chop_page` (the same function `page-break`'s per-page loop
/// uses), and confirm `fire_hooks` finds and fires it with the placed
/// page's number — exactly `hooks_crossref.rs`'s
/// `fire_hooks_invokes_the_closure_with_the_correct_page_number`, one box
/// kind up.
#[test]
fn hook_page_break_block_fires_through_chop_page_and_fire_hooks() {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);

    // `fun pbinfo _ -> register-cross-reference "seen" (arabic pbinfo#page-number)`
    let closure_ast = Ast::Lambda(
        "pbinfo".to_string(),
        Rc::new(Ast::Lambda(
            "_".to_string(),
            Rc::new(app2(
                "register-cross-reference",
                str_lit("seen"),
                app1(
                    var("arabic"),
                    Ast::AccessField(
                        Box::new(var("pbinfo")),
                        "page-number".to_string(),
                        Span::default(),
                    ),
                ),
            )),
        )),
    );
    let ast = app1(var("hook-page-break-block"), closure_ast);
    let v = interp.eval(&env, &ast).expect("evaluation should succeed");
    let Value::BlockBoxes(mut vboxes) = v else {
        panic!("expected block-boxes")
    };

    let lines = chop_page((Length::ZERO, Length::ZERO), Length::pt(400.0), &mut vboxes);
    assert!(vboxes.is_empty(), "the hook marker should be fully consumed");
    assert_eq!(
        lines,
        vec![satysfi_backend::PlacedLine {
            x: Length::ZERO,
            baseline_y: Length::ZERO,
            contents: vec![(Length::ZERO, PureHorzBox::HookPageBreak { id: HookId(0) })],
        }],
        "the block-level hook must place through the SAME HookPageBreak wrapper the inline one uses"
    );

    let doc = DocumentValue {
        geometry: PageGeometry::default(),
        pages: vec![Page { lines }],
        images: Vec::new(),
    };
    satysfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks should succeed");
    assert_eq!(
        interp.crossrefs.borrow().probe("seen"),
        Some("1".to_string()),
        "the block-level hook must have seen page-number = 1"
    );
}

// ============================================================================
// `page-break-multicolumn` (stdjareport.satyh's `document`) — STAND-IN:
// falls back to the single-column `page-break` behavior (see
// `primitives.rs`'s `prim_page_break_multicolumn` doc comment).
// ============================================================================

#[test]
fn page_break_multicolumn_typechecks_over_the_full_7_arg_signature() {
    assert_well_typed(
        "let content pbinfo = (| text-origin = (0pt, 0pt); text-height = 100pt |) in
         let parts pbinfo =
           (| header-origin = (0pt, 0pt); header-content = block-nil;
              footer-origin = (0pt, 0pt); footer-content = block-nil |)
         in
         let columnhookf x = block-nil in
         let columnendhookf x = block-nil in
         page-break-multicolumn A4Paper [] columnhookf columnendhookf content parts block-nil",
    );
}

#[test]
fn page_break_multicolumn_falls_back_to_single_column_and_evaluates() {
    let mono = Mono;
    let src = format!(
        "{PREAMBLE}
         let columnhookf x = block-nil in
         let columnendhookf x = block-nil in
         let body = line-break true true ctx (read-inline ctx {{Hello}}) in
         page-break-multicolumn A4Paper [] columnhookf columnendhookf content parts body"
    );
    let doc = satysfi_lang::compile_document(&src, &mono)
        .expect("page-break-multicolumn should compile and evaluate");
    assert_eq!(
        doc.pages.len(),
        1,
        "the STAND-IN renders every column as one column on one page"
    );
    assert_eq!(doc.pages[0].lines.len(), 1);
}

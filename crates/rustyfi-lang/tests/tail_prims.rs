//! Tail-prims sweep: the last few small primitives blocking
//! `footnote-scheme.satyh`/`proof.satyh`/`cd.satyh` (none of the three are
//! ported yet — this file only proves the *primitives* they need now exist,
//! typecheck against their real v0.0.6 signature, and evaluate without
//! panicking).
//!
//! `cd.satyh`'s `length-abs` turns out to need NO new primitive at all: it's
//! plain `pervasives.satyh` code (`if len <' 0pt then 0pt -' len else len`)
//! over length ops already ported — the first test below proves that
//! formula typechecks and evaluates correctly using only pre-existing
//! primitives, with no `@require:` needed. The three *new* primitives this
//! file actually covers are `embed-block-bottom`/`line-stack-bottom`
//! (`proof.satyh`) and `add-footnote` (`footnote-scheme.satyh`), mirroring
//! `context_box.rs`'s two-halves style (source-string typecheck +
//! source-string eval, no loader needed since none of these need
//! `@require:`).

use rustyfi_backend::{FontKey, FontMetrics, Length, PureHorzBox, VertBox};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, CompileError};

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

fn typecheck_str(src: &str) -> Result<(), CompileError> {
    let file = rustyfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let program = elaborate::elaborate_program(&file, &scope)?;
    typecheck::typecheck(&program)?;
    Ok(())
}

fn assert_well_typed(src: &str) {
    if let Err(e) = typecheck_str(src) {
        panic!("expected {src:?} to type-check, got error: {e}");
    }
}

fn assert_type_error(src: &str) {
    match typecheck_str(src) {
        Ok(()) => panic!("expected {src:?} to be rejected by the typechecker, but it passed"),
        Err(CompileError::Type(_)) => {}
        Err(other) => panic!("expected {src:?} to fail with a type error, got: {other}"),
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
    interp.eval(&env, &rustyfi_lang::ast::debrand(&program.body, &store)).expect("eval")
}

// ============================================================================
// `length-abs` (cd.satyh) — already satisfied by existing primitives; no
// Rust-side change needed. Proof: the exact `pervasives.satyh` formula
// typechecks and evaluates.
// ============================================================================

const LENGTH_ABS_SRC: &str =
    "let length-abs len = if len <' 0pt then 0pt -' len else len
     in
     length-abs (0pt -' 5pt)";

#[test]
fn length_abs_formula_typechecks_with_existing_prims() {
    assert_well_typed(LENGTH_ABS_SRC);
}

#[test]
fn length_abs_formula_evaluates_the_absolute_value() {
    match eval_str(LENGTH_ABS_SRC) {
        Value::Length(l) => assert!((l.0 - 5.0).abs() < 1e-9, "expected 5pt, got {}pt", l.0),
        other => panic!("expected a length, got {other:?}"),
    }
}

// ============================================================================
// `embed-block-bottom` (proof.satyh) — `context -> length -> (context ->
// block-boxes) -> inline-boxes`, same STAND-IN shape as `embed-block-top`.
// ============================================================================

#[test]
fn embed_block_bottom_typechecks() {
    assert_well_typed(
        "let-inline ctx \\math m = inline-nil
         in
         embed-block-bottom (get-initial-context 100pt (command \\math)) 100pt (fun ctx -> block-nil)",
    );
}

#[test]
fn embed_block_bottom_rejects_a_non_context_first_argument() {
    assert_type_error(
        "let-inline ctx \\math m = inline-nil
         in
         embed-block-bottom 5 100pt (fun ctx -> block-nil)",
    );
}

#[test]
fn embed_block_bottom_wraps_the_solidified_block_at_the_given_width() {
    let v = eval_str(
        "let-inline ctx \\math m = inline-nil
         in
         embed-block-bottom (get-initial-context 100pt (command \\math)) 50pt (fun ctx -> block-skip 30pt)",
    );
    match v {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1);
            match &boxes[0] {
                rustyfi_backend::HorzBox::Pure(PureHorzBox::EmbeddedBlock {
                    width,
                    height,
                    depth,
                    block,
                    ..
                }) => {
                    assert_eq!(*width, Length::pt(50.0));
                    assert_eq!(*height, Length::pt(30.0));
                    assert_eq!(*depth, Length::pt(0.0));
                    assert_eq!(block.len(), 1);
                }
                other => panic!("expected an EmbeddedBlock, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

// ============================================================================
// `line-stack-bottom` (proof.satyh) — `inline-boxes list -> inline-boxes`.
// ============================================================================

#[test]
fn line_stack_bottom_typechecks() {
    assert_well_typed("line-stack-bottom [inline-skip 10pt; inline-skip 20pt]");
}

#[test]
fn line_stack_bottom_rejects_a_non_inline_boxes_list() {
    assert_type_error("line-stack-bottom [1; 2; 3]");
}

#[test]
fn line_stack_bottom_stacks_each_element_as_one_line_at_the_widest_natural_width() {
    let v = eval_str("line-stack-bottom [inline-skip 10pt; inline-skip 20pt]");
    match v {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1);
            match &boxes[0] {
                rustyfi_backend::HorzBox::Pure(PureHorzBox::EmbeddedBlock { width, block, .. }) => {
                    assert_eq!(*width, Length::pt(20.0), "wid should be the widest line");
                    assert_eq!(block.len(), 2, "each list element becomes its own line");
                    assert!(
                        block.iter().all(|vb| matches!(vb, VertBox::Line { .. })),
                        "every entry should be a stacked line, not a skip"
                    );
                }
                other => panic!("expected an EmbeddedBlock, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

// ============================================================================
// `add-footnote` (footnote-scheme.satyh) — `block-boxes -> inline-boxes`.
// FAITHFUL: wraps the block in a zero-metric `PureHorzBox::Footnote`
// marker that `chop_page` (rustyfi-backend) later extracts and bottom-
// places (docs/plans/document-page-model.md §C).
// ============================================================================

#[test]
fn add_footnote_typechecks() {
    assert_well_typed("add-footnote (block-skip 10pt)");
}

#[test]
fn add_footnote_rejects_a_non_block_boxes_argument() {
    assert_type_error("add-footnote 5");
}

#[test]
fn add_footnote_wraps_the_block_in_a_footnote_marker() {
    match eval_str("add-footnote (block-skip 10pt)") {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1);
            match &boxes[0] {
                rustyfi_backend::HorzBox::Pure(PureHorzBox::Footnote { block }) => {
                    assert_eq!(
                        block,
                        &vec![VertBox::Skip(Length::pt(10.0))],
                        "the footnote marker must carry the block unchanged"
                    );
                }
                other => panic!("expected a Footnote marker, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

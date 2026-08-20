//! Tests for the type-system foundation: `types::{MonoType, Row, Kind,
//! TypeContext, generalize, instantiate}` and `unify::unify`.

use rustyfi_lang::prim_types::{self, arrow, list, product, reff, t_bool, t_int, t_string};
use rustyfi_lang::types::{
    self, resolve_row, BaseType, CmdArgType, Kind, MonoType, Row, TypeContext,
};
use rustyfi_lang::unify::{unify, UnifyError};
use std::collections::BTreeSet;

fn labels(set: &[&str]) -> BTreeSet<String> {
    set.iter().map(|s| s.to_string()).collect()
}

// ============================================================================
// Base types
// ============================================================================

#[test]
fn base_types_unify_when_equal() {
    assert!(unify(&t_int(), &t_int()).is_ok());
}

#[test]
fn base_type_mismatch_is_an_error() {
    let err = unify(&t_int(), &t_bool()).unwrap_err();
    assert!(matches!(err, UnifyError::Mismatch { .. }));
}

// ============================================================================
// Variables
// ============================================================================

#[test]
fn unify_links_a_variable_left_to_right() {
    let mut ctx = TypeContext::new();
    let v = ctx.fresh_var();
    let var_ty = MonoType::Var(v.clone());
    unify(&var_ty, &t_int()).unwrap();
    assert!(matches!(
        &*types::resolve(&var_ty),
        MonoType::Base(BaseType::Int)
    ));
}

#[test]
fn unify_links_a_variable_right_to_left() {
    let mut ctx = TypeContext::new();
    let v = ctx.fresh_var();
    let var_ty = MonoType::Var(v.clone());
    unify(&t_int(), &var_ty).unwrap();
    assert!(matches!(
        &*types::resolve(&var_ty),
        MonoType::Base(BaseType::Int)
    ));
}

#[test]
fn two_variables_unify_to_the_same_representative() {
    let mut ctx = TypeContext::new();
    let v1 = ctx.fresh_var();
    let v2 = ctx.fresh_var();
    unify(&MonoType::Var(v1.clone()), &MonoType::Var(v2.clone())).unwrap();
    // Resolving either one and then unifying with a concrete type pins both.
    unify(&MonoType::Var(v1.clone()), &t_string()).unwrap();
    assert!(matches!(
        &*types::resolve(&MonoType::Var(v2)),
        MonoType::Base(BaseType::String)
    ));
}

// ============================================================================
// Occurs check
// ============================================================================

#[test]
fn occurs_check_rejects_a_directly_self_referential_function_type() {
    let mut ctx = TypeContext::new();
    let v = ctx.fresh_var();
    let var_ty = MonoType::Var(v.clone());
    let self_referential = arrow(t_int(), var_ty.clone());
    let err = unify(&var_ty, &self_referential).unwrap_err();
    assert!(matches!(err, UnifyError::OccursCheck));
}

#[test]
fn occurs_check_looks_through_a_list() {
    let mut ctx = TypeContext::new();
    let v = ctx.fresh_var();
    let var_ty = MonoType::Var(v.clone());
    let self_referential = list(var_ty.clone());
    let err = unify(&var_ty, &self_referential).unwrap_err();
    assert!(matches!(err, UnifyError::OccursCheck));
}

#[test]
fn occurs_check_looks_through_a_record_row() {
    let mut ctx = TypeContext::new();
    let v = ctx.fresh_var();
    let var_ty = MonoType::Var(v.clone());
    let row = Row::Cons(
        "a".to_string(),
        Box::new(var_ty.clone()),
        Box::new(Row::Empty),
    );
    let self_referential = MonoType::Record(row);
    let err = unify(&var_ty, &self_referential).unwrap_err();
    assert!(matches!(err, UnifyError::OccursCheck));
}

// ============================================================================
// Structural constructors: func / product / list / ref
// ============================================================================

#[test]
fn func_types_unify_argument_and_result_wise() {
    let mut ctx = TypeContext::new();
    let a = ctx.fresh_var();
    let b = ctx.fresh_var();
    let concrete = arrow(t_int(), t_bool());
    let generic = arrow(MonoType::Var(a.clone()), MonoType::Var(b.clone()));
    unify(&generic, &concrete).unwrap();
    assert!(matches!(
        &*types::resolve(&MonoType::Var(a)),
        MonoType::Base(BaseType::Int)
    ));
    assert!(matches!(
        &*types::resolve(&MonoType::Var(b)),
        MonoType::Base(BaseType::Bool)
    ));
}

#[test]
fn product_types_unify_elementwise() {
    let p1 = product(vec![t_int(), t_bool()]);
    let p2 = product(vec![t_int(), t_bool()]);
    assert!(unify(&p1, &p2).is_ok());
}

#[test]
fn product_types_of_different_arity_are_an_arity_mismatch() {
    let p1 = product(vec![t_int(), t_bool()]);
    let p2 = product(vec![t_int(), t_bool(), t_int()]);
    let err = unify(&p1, &p2).unwrap_err();
    assert!(matches!(
        err,
        UnifyError::ArityMismatch {
            expected: 2,
            found: 3
        }
    ));
}

#[test]
fn list_types_unify_through_their_element_type() {
    let mut ctx = TypeContext::new();
    let v = ctx.fresh_var();
    unify(&list(MonoType::Var(v.clone())), &list(t_int())).unwrap();
    assert!(matches!(
        &*types::resolve(&MonoType::Var(v)),
        MonoType::Base(BaseType::Int)
    ));
}

#[test]
fn ref_types_unify_through_their_pointee_type() {
    let mut ctx = TypeContext::new();
    let v = ctx.fresh_var();
    unify(&reff(MonoType::Var(v.clone())), &reff(t_int())).unwrap();
    assert!(matches!(
        &*types::resolve(&MonoType::Var(v)),
        MonoType::Base(BaseType::Int)
    ));
}

// ============================================================================
// Records / rows
// ============================================================================

fn closed_row(fields: &[(&str, MonoType)]) -> Row {
    fields.iter().rev().fold(Row::Empty, |acc, (label, ty)| {
        Row::Cons(label.to_string(), Box::new(ty.clone()), Box::new(acc))
    })
}

#[test]
fn closed_records_with_the_same_labels_unify() {
    let r1 = MonoType::Record(closed_row(&[("a", t_int()), ("b", t_string())]));
    let r2 = MonoType::Record(closed_row(&[("a", t_int()), ("b", t_string())]));
    assert!(unify(&r1, &r2).is_ok());
}

#[test]
fn closed_record_missing_a_label_is_an_error() {
    let r1 = MonoType::Record(closed_row(&[("a", t_int()), ("b", t_string())]));
    let r2 = MonoType::Record(closed_row(&[("a", t_int())]));
    let err = unify(&r1, &r2).unwrap_err();
    assert!(matches!(err, UnifyError::MissingLabel { .. }));
}

#[test]
fn closed_record_with_an_extra_label_is_also_an_error() {
    // Symmetric to the previous test: from the smaller record's point of
    // view, the bigger one has an extra label it doesn't have room for.
    let r1 = MonoType::Record(closed_row(&[("a", t_int())]));
    let r2 = MonoType::Record(closed_row(&[("a", t_int()), ("b", t_string())]));
    let err = unify(&r1, &r2).unwrap_err();
    assert!(matches!(err, UnifyError::MissingLabel { .. }));
}

#[test]
fn open_row_var_subsumes_a_bigger_closed_row_leaving_it_fully_closed() {
    let mut ctx = TypeContext::new();
    let rv = ctx.fresh_row_var_with_kind(labels(&["a"]));
    let open = MonoType::Record(Row::Var(rv.clone()));
    let closed = MonoType::Record(closed_row(&[("a", t_int()), ("b", t_string())]));
    unify(&open, &closed).unwrap();

    // The open row got fully resolved by successive single-label
    // extraction (see `unify::row_extract`): it now contains exactly `a`
    // and `b`, with nothing left open.
    let mut seen = Vec::new();
    let mut cur = resolve_row(&Row::Var(rv)).into_owned();
    loop {
        match cur {
            Row::Empty => break,
            Row::Var(_) => panic!("row should have been fully closed by subsumption"),
            Row::Cons(label, ty, rest) => {
                seen.push((label, ty));
                cur = resolve_row(&rest).into_owned();
            }
        }
    }
    seen.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].0, "a");
    assert!(matches!(
        &*types::resolve(&seen[0].1),
        MonoType::Base(BaseType::Int)
    ));
    assert_eq!(seen[1].0, "b");
    assert!(matches!(
        &*types::resolve(&seen[1].1),
        MonoType::Base(BaseType::String)
    ));
}

#[test]
fn two_open_row_vars_unify_by_linking_and_union_their_kinds() {
    let mut ctx = TypeContext::new();
    let rv1 = ctx.fresh_row_var_with_kind(labels(&["a"]));
    let rv2 = ctx.fresh_row_var_with_kind(labels(&["b"]));
    unify(
        &MonoType::Record(Row::Var(rv1.clone())),
        &MonoType::Record(Row::Var(rv2.clone())),
    )
    .unwrap();

    // Whichever one ended up as the representative carries the union of
    // both required-label sets.
    let rep = match resolve_row(&Row::Var(rv1)).into_owned() {
        Row::Var(v) => v,
        other => panic!("expected an unresolved row variable, got {other:?}"),
    };
    assert_eq!(rep.kind(), labels(&["a", "b"]));
}

#[test]
fn kind_bridging_type_variable_requires_labels_of_a_concrete_record() {
    // A bare type variable that field access has already constrained to
    // "must be some record with (at least) label `a`" (`Kind::Record`).
    let mut ctx = TypeContext::new();
    let v = ctx.fresh_var_with_kind(Kind::Record(labels(&["a"])));
    let record = MonoType::Record(closed_row(&[("a", t_int()), ("b", t_string())]));
    assert!(unify(&MonoType::Var(v), &record).is_ok());
}

#[test]
fn kind_bridging_type_variable_rejects_a_record_missing_the_required_label() {
    let mut ctx = TypeContext::new();
    let v = ctx.fresh_var_with_kind(Kind::Record(labels(&["z"])));
    let record = MonoType::Record(closed_row(&[("a", t_int())]));
    let err = unify(&MonoType::Var(v), &record).unwrap_err();
    assert!(matches!(err, UnifyError::MissingLabel { .. }));
}

// ============================================================================
// Command argument types
// ============================================================================

#[test]
fn command_arg_types_unify_elementwise() {
    let c1 = MonoType::InlineCmd(vec![
        CmdArgType {
            optional: false,
            opt_labels: vec![],
            ty: t_int(),
        },
        CmdArgType {
            optional: true,
            opt_labels: vec![],
            ty: t_string(),
        },
    ]);
    let c2 = MonoType::InlineCmd(vec![
        CmdArgType {
            optional: false,
            opt_labels: vec![],
            ty: t_int(),
        },
        CmdArgType {
            optional: true,
            opt_labels: vec![],
            ty: t_string(),
        },
    ]);
    assert!(unify(&c1, &c2).is_ok());
}

#[test]
fn command_arg_optionality_mismatch_is_an_error() {
    let c1 = MonoType::InlineCmd(vec![CmdArgType {
        optional: false,
        opt_labels: vec![],
        ty: t_int(),
    }]);
    let c2 = MonoType::InlineCmd(vec![CmdArgType {
        optional: true,
        opt_labels: vec![],
        ty: t_int(),
    }]);
    let err = unify(&c1, &c2).unwrap_err();
    assert!(matches!(err, UnifyError::OptionalMismatch { .. }));
}

// ============================================================================
// Generalization / instantiation / levels
// ============================================================================

#[test]
fn a_variable_created_at_a_deeper_level_generalizes() {
    let mut ctx = TypeContext::new();
    ctx.enter_level();
    let v = ctx.fresh_var();
    ctx.leave_level();

    let scheme = types::generalize(ctx.level(), &MonoType::Var(v));
    assert!(!scheme.is_monomorphic());
}

#[test]
fn instantiating_a_scheme_twice_gives_independent_variables() {
    let mut ctx = TypeContext::new();
    ctx.enter_level();
    let v = ctx.fresh_var();
    ctx.leave_level();

    let scheme = types::generalize(ctx.level(), &MonoType::Var(v));
    let inst1 = types::instantiate(&scheme, ctx.level());
    let inst2 = types::instantiate(&scheme, ctx.level());

    // If they were the same variable, binding one to `int` and the other
    // to `bool` would conflict.
    unify(&inst1, &t_int()).expect("first instantiation should unify freely");
    unify(&inst2, &t_bool()).expect("second instantiation should unify freely, independently");
}

#[test]
fn a_variable_from_an_outer_level_does_not_generalize() {
    let mut ctx = TypeContext::new();
    let outer = ctx.fresh_var(); // created at the *current* (outer) level
    ctx.enter_level();
    let inner = ctx.fresh_var(); // created one level deeper
    ctx.leave_level();

    let body = arrow(MonoType::Var(outer.clone()), MonoType::Var(inner));
    let scheme = types::generalize(ctx.level(), &body);

    let inst1 = types::instantiate(&scheme, ctx.level());
    let inst2 = types::instantiate(&scheme, ctx.level());
    let (MonoType::Func(_, dom1, cod1), MonoType::Func(_, dom2, cod2)) = (inst1, inst2) else {
        panic!("expected function types");
    };

    // The outer variable is untouched by instantiation: both bodies still
    // share the very same `outer` cell.
    let (MonoType::Var(o1), MonoType::Var(o2)) = (&*types::resolve(&dom1), &*types::resolve(&dom2))
    else {
        panic!("expected variables");
    };
    assert!(o1.same(&o2));
    assert!(o1.same(&outer));

    // The inner (generalized) variable, on the other hand, is fresh each
    // instantiation.
    let (MonoType::Var(i1), MonoType::Var(i2)) = (&*types::resolve(&cod1), &*types::resolve(&cod2))
    else {
        panic!("expected variables");
    };
    assert!(!i1.same(&i2));
}

// ============================================================================
// Display
// ============================================================================

#[test]
fn display_prints_a_function_type_with_a_parenthesized_list_codomain() {
    let ty = arrow(t_int(), list(t_string()));
    assert_eq!(ty.to_string(), "int -> (string list)");
}

#[test]
fn display_prints_a_closed_record_type() {
    let ty = MonoType::Record(closed_row(&[("a", t_int())]));
    assert_eq!(ty.to_string(), "(| a : int |)");
}

#[test]
fn display_prints_products_and_postfix_variants() {
    assert_eq!(product(vec![t_int(), t_bool()]).to_string(), "int * bool");
    assert_eq!(
        MonoType::Variant("option".to_string(), vec![t_int()]).to_string(),
        "int option"
    );
}

// ============================================================================
// Primitive type table coverage
// ============================================================================

#[test]
fn every_registered_primitive_has_a_type() {
    // Mirrors `primitives.rs`'s `prims!` table exactly, plus the
    // separately-defined `inline-fil` constant.
    const NAMES: &[&str] = &[
        "read-inline",
        "read-block",
        "line-break",
        "page-break",
        "page-break-multicolumn",
        "page-break-two-column",
        "+",
        "-",
        "*",
        "/",
        "mod",
        "==",
        "<>",
        "<",
        ">",
        "<=",
        ">=",
        "&&",
        "||",
        "not",
        "+.",
        "-.",
        "*.",
        "/.",
        "float",
        "round",
        "+'",
        "-'",
        "*'",
        "/'",
        "<'",
        ">'",
        "^",
        "arabic",
        "string-same",
        "::",
        "!",
        "string-length",
        "string-sub",
        "string-explode",
        "regexp-of-string",
        "string-match",
        "split-on-regexp",
        "embed-string",
        "inline-fil",
        // ---- phase 4, part 1 additions ----
        "set-font-size",
        "get-font-size",
        "set-leading",
        "set-paragraph-margin",
        "get-text-width",
        "get-initial-context",
        "++",
        "+++",
        "inline-nil",
        "block-nil",
        "inline-skip",
        "inline-glue",
        "block-skip",
        // ---- the reflow
        // marker-box constructors ----
        "list-mark",
        "inline-mark",
        // ---- phase 4, part 2 addition ----
        "set-font-key",
        // ---- frontend-completion.md §Slice 1.A: the ~18 pure primitives ----
        // (`|>` excluded: no primitive of its own — see `elaborate.rs`'s
        // `climb` and this crate's `prims!` table comment.)
        "sin",
        "asin",
        "cos",
        "acos",
        "tan",
        "atan",
        "atan2",
        "log",
        "exp",
        "ceil",
        "floor",
        "show-float",
        "string-byte-length",
        "string-sub-bytes",
        "string-unexplode",
        "display-message",
        "abort-with-message",
        // ---- Slice 1 additions (raster images) ----
        "load-image",
        "load-pdf-image",
        "use-image-by-width",
        // ---- Slice 1 graphics primitives ----
        "start-path",
        "line-to",
        "terminate-path",
        "close-with-line",
        "fill",
        "stroke",
        "inline-graphics",
        // ---- tables, Slice 1 ----
        "tabular",
        // ---- roadmap C2 ----
        "inline-graphics-outer",
        // ---- gr.satyh roadmap prims (§Full roadmap A/B/C/D) ----
        "bezier-to",
        "close-with-bezier",
        "shift-path",
        "linear-transform-path",
        "shift-graphics",
        "linear-transform-graphics",
        "get-graphics-bbox",
        "get-path-bbox",
        "dashed-stroke",
        "draw-text",
        // ---- pervasives.satyh unblockers ----
        "get-natural-metrics",
        "inline-frame-outer",
        "set-manual-rising",
        "script-guard",
        "discretionary",
        // ---- Tier-2 decoration/graphics packages ----
        "get-axis-height",
        // ---- hooks / annotations / cross-references, Slice 1 ----
        "hook-page-break",
        "hook-page-break-block",
        "register-cross-reference",
        "get-cross-reference",
        // ---- group E1: hooks-annotations-crossref.md §A closer ----
        "probe-cross-reference",
        // ---- + §G ----
        "math-char",
        "math-big-char",
        "math-char-with-kern",
        "math-big-char-with-kern",
        "math-concat",
        "math-group",
        "math-sup",
        "math-sub",
        "math-frac",
        "math-radical",
        "math-lower",
        "math-upper",
        "math-pull-in-scripts",
        "math-color",
        "math-char-class",
        "math-variant-char",
        "set-math-variant-char",
        "get-left-math-class",
        "get-right-math-class",
        "math-paren",
        "math-paren-with-middle",
        "text-in-math",
        "convert-string-for-math",
        "embed-math",
        "set-math-command",
        "set-math-font",
        "space-between-maths",
        "raise-inline",
        "embed-block-breakable",
        "unite-path",
        "set-min-gap-of-lines",
        "omit-skip-after",
        // ---- (rows 1-10) ----
        "set-text-color",
        "get-text-color",
        "set-hyphen-penalty",
        "set-hyphen-min",
        "set-space-ratio",
        "set-space-ratio-between-scripts",
        "split-into-lines",
        "block-frame-breakable",
        "embed-block-top",
        "set-font",
        "set-code-text-command",
        "get-natural-length",
        // ---- (annot.satyh) ----
        "get-leftmost-script",
        "get-rightmost-script",
        "inline-frame-breakable",
        "register-destination",
        "register-link-to-uri",
        "register-link-to-location",
        // ---- step 8/9 orphans ----
        "set-dominant-wide-script",
        "set-dominant-narrow-script",
        "set-language",
        "set-every-word-break",
        "register-outline",
        "extract-string",
        // ---- group E2: dominant-script/language getters (context-box- prims.md §C landed) ----
        "get-dominant-wide-script",
        "get-dominant-narrow-script",
        "get-language",
        // ---- group E3: text-mode-context sliver (context-box-prims.md §G) ----
        "get-initial-text-info",
        "deepen-indent",
        "break",
        // ---- proof.satyh/footnote-scheme.satyh unblockers (tail-prims sweep) ----
        "embed-block-bottom",
        "line-stack-bottom",
        "add-footnote",
        // ---- page-level prims blocking mitou-report/stdjareport ----
        "clear-page",
    ];
    assert_eq!(
        NAMES.len(),
        177,
        "keep this list in sync with primitives.rs's prims! table \
         (reflow S4 lists added 2: list-mark, inline-mark; \
         layout-fidelity slydifi added 2: set-space-ratio-between-scripts, and \
         set-hyphen-min which had a prim+type but was missing from the name lists)"
    );
    for name in NAMES {
        assert!(
            prim_types::primitive_type(name).is_some(),
            "primitive `{name}` has no registered type"
        );
    }
}

#[test]
fn unknown_primitive_name_has_no_type() {
    assert!(prim_types::primitive_type("not-a-real-primitive").is_none());
}

// ============================================================================
// math-split spec §2.2: the 8 V0_1-only additions. NOT folded into `NAMES`
// above — that list's assertion checks `primitive_type` (the V0_0-default
// wrapper), and every one of these 8 names is deliberately UNBOUND under
// V0_0 (test 6.3-6 of the math-split spec); they're the hand-sync twin of
// `typecheck::PRIMITIVE_NAMES`'s own "added in 0.1" block instead, verified
// against `primitive_type_with_version` directly.
// ============================================================================

#[test]
fn every_v01_only_primitive_has_a_type_under_v0_1_and_none_under_v0_0() {
    const V01_ONLY_NAMES: &[&str] = &[
        "read-math",
        "stringify-math",
        "set-math-char",
        "set-math-char-class",
        "get-math-char-class",
        "embed-inline-to-math",
        "get-math-axis-height-ratio",
        "%math-attach-scripts",
        // ---- L5a (prim-retype-sweep §2): bitwise ops, Unicode string ops,
        // `read-file`, `register-document-information` — the hand-sync twin
        // of `typecheck::PRIMITIVE_NAMES`'s own "added in 0.1 — L5a" block.
        "<<",
        ">>",
        "band",
        "bor",
        "bxor",
        "bnot",
        "normalize-string-to-nfc",
        "normalize-string-to-nfd",
        "split-grapheme-cluster",
        "read-file",
        "register-document-information",
        // ---- L5b (prim-retype-sweep §3.4/§3.6): the 2 graphics-collection
        // additions. The 3 named + 6 hidden retypes are NOT here — each is
        // one shared name whose type forks per version (`get-graphics-bbox`,
        // `tabular`, `inline-graphics`, `inline-graphics-outer`,
        // `inline-frame-outer/-inner/-breakable`, `block-frame-breakable`),
        // bound under BOTH versions, so they belong in `NAMES` above (they
        // already are), not this V0_1-only twin.
        "unite-graphics",
        "clip-graphics-by-path",
        // ---- language-completeness sweep gap 1: 0.1 float comparisons
        // (`primitives.rs`'s `prims!` table comment on ">."/"<."/">=."/
        // "<=.") — the hand-sync twin of `typecheck::PRIMITIVE_NAMES`'s own
        // trailing block.
        ">.",
        "<.",
        ">=.",
        "<=.",
        // ---- G6 (`…/tmp/g6-g7-standins.md` §1): hyphenation/unidata loader
        // + setter stand-ins, and the `here` lex-time-constant stand-in —
        // the hand-sync twin of `typecheck::PRIMITIVE_NAMES`'s own G6 block.
        "load-hyphenation-dictionary",
        "load-unicode-char-database",
        "set-hyphenation-dictionary",
        "set-unicode-char-database",
        "here",
        // ---- the 0.1 `font` build-out: the LOCAL stand-in for upstream's
        // internal `LoadSingleFont` node, the only way a 0.1 program mints
        // the opaque `font` handle here. V0_1-only for the same reason the
        // TYPE is: 0.0.6 has no `font` type to mint.
        "load-single-font",
    ];
    for name in V01_ONLY_NAMES {
        assert!(
            prim_types::primitive_type_with_version(name, rustyfi_syntax::RustyfiVersion::V0_1)
                .is_some(),
            "V0_1-only primitive `{name}` has no registered type under V0_1"
        );
        assert!(
            prim_types::primitive_type_with_version(name, rustyfi_syntax::RustyfiVersion::V0_0)
                .is_none(),
            "V0_1-only primitive `{name}` must be unbound under V0_0"
        );
    }
}

#[test]
fn poly_primitives_instantiate_independently() {
    // `::` : 'a -> 'a list -> 'a list — two call sites must not share the
    // same `'a`.
    let scheme = prim_types::primitive_type("::").unwrap();
    let ty1 = types::instantiate(&scheme, 0);
    let ty2 = types::instantiate(&scheme, 0);
    // Applying the first instantiation at `int` must not constrain the
    // second instantiation, which we then apply at `bool`.
    let MonoType::Func(_, head1, _) = ty1 else {
        panic!("expected a function type")
    };
    let MonoType::Func(_, head2, _) = ty2 else {
        panic!("expected a function type")
    };
    unify(&head1, &t_int()).unwrap();
    unify(&head2, &t_bool()).unwrap();
}

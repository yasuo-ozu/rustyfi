//! Reachability of every `#[subast]` edge in the generated type-representation
//! visitor (`rustyfi_lang::visit`).
//!
//! `syan`'s `visitor!` follows a field only when the field's peeled head type
//! is named in the owning type's `#[subast(..)]` list. When the two fall out
//! of step the field is reclassified a leaf and the generated body for it is
//! **empty** — no error, no warning (`#[derive(Ast)]`'s own "entry matches no
//! field" lint goes through `proc_macro_error::emit_warning!`, which stable
//! rustc discards, and nothing checks the other direction at all). That is
//! precisely the silent-omission failure this visitor was introduced to
//! abolish — `CmdArgType::opt_labels` was already a live instance of it in
//! the hand-written walks — so it needs its own check.
//!
//! This is that check. For every field of every visited type that holds
//! another node, it plants a sentinel reachable ONLY through that field and
//! asserts the traversal finds it.
//!
//! Two mechanisms keep it honest as the type representation grows:
//!
//! * [`classify`] is a **wildcard-free** match over every `MonoType` and
//!   `Row` variant. Adding a variant is a compile error here, which lands the
//!   author in this file.
//! * [`covers_every_recursive_variant`] asserts that every variant
//!   `classify` calls node-carrying has at least one edge exercised below.

use rustyfi_lang::types::{BaseType, CmdArgType, MonoType, Row};

/// A type no other part of a fixture ever builds, so finding it proves the
/// traversal reached the exact slot it was planted in.
fn sentinel() -> MonoType {
    MonoType::Variant("sentinel#reachability".to_string(), Vec::new())
}

fn is_sentinel(t: &MonoType) -> bool {
    matches!(t, MonoType::Variant(n, args) if n == "sentinel#reachability" && args.is_empty())
}

/// Filler that is never the sentinel.
fn filler() -> MonoType {
    MonoType::Base(BaseType::Int)
}

fn empty_slot(ty: MonoType) -> CmdArgType {
    CmdArgType {
        optional: false,
        opt_labels: Vec::new(),
        ty,
    }
}

// ---------------------------------------------------------------------------
// Counting
// ---------------------------------------------------------------------------

fn count_in_mono(ty: &MonoType) -> usize {
    let mut n = 0;
    ty.visit(|t: &MonoType| {
        if is_sentinel(t) {
            n += 1;
        }
    });
    n
}

fn count_in_row(row: &Row) -> usize {
    let mut n = 0;
    row.visit(|t: &MonoType| {
        if is_sentinel(t) {
            n += 1;
        }
    });
    n
}

fn count_in_slot(slot: &CmdArgType) -> usize {
    let mut n = 0;
    slot.visit(|t: &MonoType| {
        if is_sentinel(t) {
            n += 1;
        }
    });
    n
}

/// Each entry is one `#[subast]` edge: a label, and a `MonoType` whose ONLY
/// sentinel sits behind that edge.
fn mono_edges() -> Vec<(&'static str, MonoType)> {
    vec![
        (
            "MonoType::Func.row",
            MonoType::Func(
                Box::new(Row::Cons(
                    "l".to_string(),
                    Box::new(sentinel()),
                    Box::new(Row::Empty),
                )),
                Box::new(filler()),
                Box::new(filler()),
            ),
        ),
        (
            "MonoType::Func.dom",
            MonoType::Func(
                Box::new(Row::Empty),
                Box::new(sentinel()),
                Box::new(filler()),
            ),
        ),
        (
            "MonoType::Func.cod",
            MonoType::Func(
                Box::new(Row::Empty),
                Box::new(filler()),
                Box::new(sentinel()),
            ),
        ),
        (
            "MonoType::Product.items",
            MonoType::Product(vec![filler(), sentinel()]),
        ),
        ("MonoType::List.elem", MonoType::List(Box::new(sentinel()))),
        ("MonoType::Ref.inner", MonoType::Ref(Box::new(sentinel()))),
        ("MonoType::Code.inner", MonoType::Code(Box::new(sentinel()))),
        (
            "MonoType::Record.row",
            MonoType::Record(Row::Cons(
                "l".to_string(),
                Box::new(sentinel()),
                Box::new(Row::Empty),
            )),
        ),
        (
            "MonoType::Variant.args",
            MonoType::Variant("some-user-variant".to_string(), vec![sentinel()]),
        ),
        (
            "MonoType::InlineCmd.slots",
            MonoType::InlineCmd(vec![empty_slot(sentinel())]),
        ),
        (
            "MonoType::BlockCmd.slots",
            MonoType::BlockCmd(vec![empty_slot(sentinel())]),
        ),
        (
            "MonoType::MathCmd.slots",
            MonoType::MathCmd(vec![empty_slot(sentinel())]),
        ),
    ]
}

fn row_edges() -> Vec<(&'static str, Row)> {
    vec![
        (
            "Row::Cons.ty",
            Row::Cons("l".to_string(), Box::new(sentinel()), Box::new(Row::Empty)),
        ),
        (
            "Row::Cons.rest",
            Row::Cons(
                "l".to_string(),
                Box::new(filler()),
                Box::new(Row::Cons(
                    "m".to_string(),
                    Box::new(sentinel()),
                    Box::new(Row::Empty),
                )),
            ),
        ),
    ]
}

fn slot_edges() -> Vec<(&'static str, CmdArgType)> {
    vec![
        // THE edge four hand-written walks forgot. `synonym_refs` reaching it
        // is what turns `type t = inline [?(l : t) string]` from a stack
        // overflow into a `cyclic type synonym` error.
        (
            "CmdArgType::opt_labels",
            CmdArgType {
                optional: false,
                opt_labels: vec![("l".to_string(), sentinel())],
                ty: filler(),
            },
        ),
        ("CmdArgType::ty", empty_slot(sentinel())),
    ]
}

// ---------------------------------------------------------------------------
// The assertions
// ---------------------------------------------------------------------------

#[test]
fn every_mono_type_edge_is_visited() {
    for (label, fixture) in mono_edges() {
        assert_eq!(
            count_in_mono(&fixture),
            1,
            "`{label}` is declared in a `#[subast]` list but the generated \
             traversal never reached the sentinel planted behind it — the \
             field's peeled head fell out of step with the list, and every \
             consumer of this visitor is silently skipping it"
        );
    }
}

#[test]
fn every_row_edge_is_visited() {
    for (label, fixture) in row_edges() {
        assert_eq!(count_in_row(&fixture), 1, "`{label}` was not reached");
    }
}

#[test]
fn every_cmd_arg_type_edge_is_visited() {
    for (label, fixture) in slot_edges() {
        assert_eq!(count_in_slot(&fixture), 1, "`{label}` was not reached");
    }
}

/// The visit is **inclusive**: a root that is itself a sentinel is seen.
#[test]
fn visit_is_inclusive_of_the_root() {
    assert_eq!(count_in_mono(&sentinel()), 1);
}

/// A leaf variant reaches nothing. `MonoType::Base` carries no node at all,
/// and — deliberately — neither `MonoType::Var` nor `Row::Var` is descended
/// into, because a bound type variable's payload lives behind an
/// `Rc<RefCell<..>>` whose guard cannot outlive the frame (see
/// `rustyfi_lang::visit`). Nothing below can plant a sentinel behind one, so
/// the negative statement is all there is to make.
#[test]
fn leaves_reach_nothing() {
    assert_eq!(count_in_mono(&filler()), 0);
    assert_eq!(count_in_row(&Row::Empty), 0);
}

/// Nesting composes: a sentinel two levels down, behind a `CmdArgType`'s
/// optional-label map inside a `Func` codomain inside a record row, is still
/// found exactly once.
#[test]
fn edges_compose_through_nesting() {
    let deep = MonoType::Record(Row::Cons(
        "f".to_string(),
        Box::new(MonoType::Func(
            Box::new(Row::Empty),
            Box::new(filler()),
            Box::new(MonoType::InlineCmd(vec![CmdArgType {
                optional: false,
                opt_labels: vec![("l".to_string(), sentinel())],
                ty: filler(),
            }])),
        )),
        Box::new(Row::Empty),
    ));
    assert_eq!(count_in_mono(&deep), 1);
}

// ---------------------------------------------------------------------------
// The variant census — a compile error when the representation grows
// ---------------------------------------------------------------------------

/// Whether a variant carries another node. **Wildcard-free on purpose**:
/// adding a `MonoType` or `Row` variant fails to compile here, which is the
/// point — the author then has to say whether it is recursive and, if so, add
/// an edge above.
fn classify(label_of: &MonoType) -> Option<&'static str> {
    match label_of {
        MonoType::Var(_) => None,
        MonoType::Base(_) => None,
        MonoType::Func(..) => Some("MonoType::Func"),
        MonoType::Product(_) => Some("MonoType::Product"),
        MonoType::List(_) => Some("MonoType::List"),
        MonoType::Ref(_) => Some("MonoType::Ref"),
        MonoType::Record(_) => Some("MonoType::Record"),
        MonoType::Variant(..) => Some("MonoType::Variant"),
        MonoType::Code(_) => Some("MonoType::Code"),
        MonoType::InlineCmd(_) => Some("MonoType::InlineCmd"),
        MonoType::BlockCmd(_) => Some("MonoType::BlockCmd"),
        MonoType::MathCmd(_) => Some("MonoType::MathCmd"),
    }
}

/// The `Row` half of [`classify`], likewise wildcard-free.
fn classify_row(row: &Row) -> Option<&'static str> {
    match row {
        Row::Empty => None,
        Row::Var(_) => None,
        Row::Cons(..) => Some("Row::Cons"),
    }
}

/// Every fixture must actually be an instance of the variant its label names
/// — otherwise a copy-paste could leave a variant uncovered while the census
/// below still passed.
#[test]
fn every_fixture_is_an_instance_of_the_variant_it_names() {
    for (label, fixture) in mono_edges() {
        let variant = label.split('.').next().unwrap();
        assert_eq!(
            classify(&fixture),
            Some(variant),
            "the fixture labelled `{label}` is not a `{variant}`"
        );
    }
    for (label, fixture) in row_edges() {
        let variant = label.split('.').next().unwrap();
        assert_eq!(classify_row(&fixture), Some(variant));
    }
}

/// The census. `classify` is wildcard-free, so a new `MonoType` variant is a
/// **compile error** in this file; this test is what then tells the author
/// whether the variant needs an edge. The list is spelled out rather than
/// derived because `MonoType::Var` cannot be built from outside the crate
/// (`TyVarRef::new` is `pub(crate)`), so there is no way to enumerate one
/// instance of every variant here.
#[test]
fn covers_every_recursive_variant() {
    let exercised: Vec<&str> = mono_edges()
        .iter()
        .filter_map(|(label, _)| label.split('.').next())
        .collect();

    for name in [
        "MonoType::Func",
        "MonoType::Product",
        "MonoType::List",
        "MonoType::Ref",
        "MonoType::Record",
        "MonoType::Variant",
        "MonoType::Code",
        "MonoType::InlineCmd",
        "MonoType::BlockCmd",
        "MonoType::MathCmd",
    ] {
        assert!(
            exercised.contains(&name),
            "`{name}` carries another node but no edge above exercises it"
        );
    }

    let row_exercised: Vec<&str> = row_edges()
        .iter()
        .filter_map(|(label, _)| label.split('.').next())
        .collect();
    assert!(row_exercised.contains(&"Row::Cons"));
    assert_eq!(classify_row(&Row::Empty), None);

    // Both `CmdArgType` fields that can hold a node are exercised.
    let slot_exercised: Vec<&str> = slot_edges().iter().map(|(label, _)| *label).collect();
    assert!(slot_exercised.contains(&"CmdArgType::opt_labels"));
    assert!(slot_exercised.contains(&"CmdArgType::ty"));
}

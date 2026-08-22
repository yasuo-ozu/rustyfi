//! Match exhaustiveness and redundancy checking (Slice 1) — a row-major
//! reimplementation of the Maranget "usefulness"
//! matrix algorithm v0.0.6 uses in `src/frontend/exhchecker.ml`. Non-fatal:
//! this module only produces [`MatchWarning`]s, never a [`crate::typecheck::TypeError`]
//! (mirrors `exhchecker.ml`'s own warn-and-continue policy — see that
//! module's `main`, lines 391-424).
//!
//! The core primitive is `U(P, q)`: "is there a value matched by row-vector
//! `q` but by no row of matrix `P`?" (`usefulness`, below). Both diagnostics
//! this module reports fall out of one call shape each:
//!
//! - **Non-exhaustive**: `U(P, [_])` where `P` is the one-column matrix of
//!   every non-guarded arm pattern; a `Some` result is a witness value the
//!   match does not cover.
//! - **Unreachable arm** `i`: `U(P[0..i], row_i)` — arm `i` is redundant iff
//!   this is `None` (it matches nothing the earlier arms don't already).
//!
//! **Port-specific adaptation (read this before touching `specialize`):**
//! v0.0.6 gives every constructor exactly one sub-pattern (unit for
//! nullary). This port's [`Pattern::Ctor`] carries `Option<Box<Pattern>>`,
//! so a constructor's arity is 0 or 1 — a nullary ctor (`None`, `A`) expands
//! into **zero** sub-columns, not one. Get this wrong and `None`/`[]`-style
//! matches miscount.

use crate::ast::branded::{MatchArm, Pattern};
use crate::prim_types::VariantDecl;
use crate::symbol::SymbolStore;
use crate::types::{resolve, BaseType, MonoType};
use rustyfi_syntax::span::Span;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// A non-fatal diagnostic from this pass: either "this match may not be
/// exhaustive" (with a witness pattern baked into `message`) or "this arm is
/// unreachable". `span` is best-effort (see `TypeError`'s doc comment on the
/// same theme) — patterns carry no span of their own, so it falls back to
/// the scrutinee's or the arm body's span, or `None`.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchWarning {
    pub span: Option<Span>,
    pub message: String,
}

/// A pattern's head "constructor", abstracted just enough to drive
/// `specialize`/`default`/completeness — the row-major analogue of
/// `exhchecker.ml`'s per-constructor signature entries. Carries no
/// sub-patterns (those live alongside it wherever a `HeadKey` is produced).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum HeadKey {
    Unit,
    Bool(bool),
    Int(i64),
    Str(String),
    /// The one-and-only tuple "constructor" for an arity-`n` product type.
    Tuple(usize),
    EmptyList,
    Cons,
    Ctor(String),
}

/// One row/column entry of the usefulness matrix: `Vec<Pattern>` (a row),
/// `Vec<Vec<Pattern>>` (a whole matrix `P`). Patterns are cloned freely
/// throughout this module rather than borrowed — matches are small
/// hand-written things, not a performance-critical path (matrix blow-up
/// needs no mitigation at this milestone).
type Matrix<'s> = Vec<Vec<Pattern<'s>>>;

/// Classify a pattern's head shape, normalizing away `Var`/`As` first
/// (`exhchecker.ml`'s `normalize_pat`, lines 151-158: a bare variable is
/// just a wildcard; `pat as name` is its inner pattern). Returns `None` for
/// a wildcard (nothing further constrains the value there), else the head
/// key plus that head's sub-patterns (0, 1, or 2 of them: literals/nullary
/// ctors/`[]` have none, `Ctor(_, Some(p))` has one, `Cons` has two, `Tuple`
/// has as many as its arity).
fn head_of<'s>(p: &Pattern<'s>) -> Option<(HeadKey, Vec<Pattern<'s>>)> {
    match p {
        Pattern::Wild | Pattern::Var(_) => None,
        Pattern::As(inner, _) => head_of(inner),
        Pattern::Unit => Some((HeadKey::Unit, Vec::new())),
        Pattern::Bool(b) => Some((HeadKey::Bool(*b), Vec::new())),
        Pattern::Int(n) => Some((HeadKey::Int(*n), Vec::new())),
        Pattern::Str(s) => Some((HeadKey::Str(s.clone()), Vec::new())),
        Pattern::Tuple(ps) => Some((HeadKey::Tuple(ps.len()), ps.clone())),
        Pattern::EmptyList => Some((HeadKey::EmptyList, Vec::new())),
        Pattern::Cons(head, tail) => {
            Some((HeadKey::Cons, vec![(**head).clone(), (**tail).clone()]))
        }
        Pattern::Ctor(name, payload) => Some((
            HeadKey::Ctor(name.clone()),
            payload.iter().map(|b| (**b).clone()).collect(),
        )),
    }
}

/// Render a witness (or matrix) pattern back to source-ish text, e.g.
/// `Some(_)`, `_ :: _`, `[]`, `(_, true)` (mirrors `pattern_instance` /
/// `string_of_instance`, exhchecker.ml 16-25, 117-148) — only ever called on
/// patterns this module builds itself (or copies from the source), so every
/// variant is worth spelling out reasonably.
///
/// Pattern-bound *variables* are interned, so this takes the store and
/// resolves them back to their source text — the rendered string lands
/// verbatim in a [`MatchWarning`] the golden tests diff, so it must be the
/// name the user wrote, never a symbol index.
fn render_pattern<'s>(store: &'s SymbolStore, p: &Pattern<'s>) -> String {
    let go = |q: &Pattern<'s>| render_pattern(store, q);
    match p {
        Pattern::Wild => "_".to_string(),
        Pattern::Var(name) => store.resolve(*name).to_string(),
        Pattern::Unit => "()".to_string(),
        Pattern::Bool(b) => b.to_string(),
        Pattern::Int(n) => n.to_string(),
        Pattern::Str(s) => format!("{s:?}"),
        Pattern::Tuple(ps) => {
            let inner = ps.iter().map(go).collect::<Vec<_>>().join(", ");
            format!("({inner})")
        }
        Pattern::EmptyList => "[]".to_string(),
        Pattern::Cons(head, tail) => format!("{} :: {}", go(head), go(tail)),
        Pattern::Ctor(name, Some(inner)) => format!("{name}({})", go(inner)),
        Pattern::Ctor(name, None) => name.clone(),
        Pattern::As(inner, name) => {
            format!("{} as {}", go(inner), store.resolve(*name))
        }
    }
}

/// The full constructor signature of a column's type — `complete_sig`
/// (exhchecker.ml 299-310), adapted: rather than a special "always
/// complete" case for products, `Product` is simply modeled as a
/// one-constructor `Finite` signature (arity = element count), which the
/// generic Finite-vs-`Signature` completeness check already handles
/// correctly whether or not any row actually spells out a tuple pattern.
enum Signature {
    /// Every `(head, arity)` this type could ever match on. Completeness of
    /// a matrix column is "does its set of observed heads cover all of
    /// these".
    Finite(Vec<(HeadKey, usize)>),
    /// `int`/`string`/an unresolved type variable/anything else this pass
    /// doesn't model structurally: infinite or unknown domain, so no finite
    /// set of patterns is ever complete without a wildcard/var arm
    /// (`make_int_sig`/`make_string_sig`, exhchecker.ml 286-298).
    Infinite,
}

fn signature(rty: &MonoType, variants: &HashMap<String, Rc<VariantDecl>>) -> Signature {
    match rty {
        MonoType::Base(BaseType::Unit) => Signature::Finite(vec![(HeadKey::Unit, 0)]),
        MonoType::Base(BaseType::Bool) => {
            Signature::Finite(vec![(HeadKey::Bool(true), 0), (HeadKey::Bool(false), 0)])
        }
        MonoType::List(_) => Signature::Finite(vec![(HeadKey::EmptyList, 0), (HeadKey::Cons, 2)]),
        MonoType::Product(tys) => Signature::Finite(vec![(HeadKey::Tuple(tys.len()), tys.len())]),
        MonoType::Variant(name, _) => match variants.get(name) {
            Some(decl) => Signature::Finite(
                decl.ctors
                    .iter()
                    .map(|(cname, payload)| {
                        (
                            HeadKey::Ctor(cname.clone()),
                            if payload.is_some() { 1 } else { 0 },
                        )
                    })
                    .collect(),
            ),
            // Defensive: a variant name not in the table (shouldn't happen
            // after a successful `bind_pattern` pass) — never complete
            // rather than risk under-reporting.
            None => Signature::Infinite,
        },
        // `int`/`string`/`float`/`length`/an unresolved `Var`/any other base
        // or type former this grammar has no pattern syntax for.
        _ => Signature::Infinite,
    }
}

/// The sub-column types a given head introduces when specialized against a
/// column of type `rty` (only meaningful when `head` is actually one of
/// `rty`'s constructors, which is always true for every call site below).
/// Returns 0, 1, or 2 types — the tuple case aside, arity is always 0 or 1
/// per this port's `Ctor` payload shape (see this module's top doc
/// comment), and `Cons` is the sole arity-2 case (head element, tail list).
fn sub_types_for(
    head: &HeadKey,
    rty: &MonoType,
    variants: &HashMap<String, Rc<VariantDecl>>,
) -> Vec<MonoType> {
    match (head, rty) {
        (HeadKey::Tuple(_), MonoType::Product(tys)) => tys.clone(),
        (HeadKey::Cons, MonoType::List(elem)) => {
            vec![(**elem).clone(), MonoType::List(elem.clone())]
        }
        (HeadKey::Ctor(cname), MonoType::Variant(vname, args)) => variants
            .get(vname)
            .and_then(|decl| decl.instantiate_ctor(cname, args))
            .and_then(|(payload, _)| payload)
            .map(|t| vec![t])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Defensively pad or truncate a sub-type list to the arity actually in
/// play (derived from the pattern side, which is always authoritative) —
/// guards `usefulness`'s `split_at` against ever panicking if a type-side
/// lookup above returns an unexpected count (should not happen after a
/// successful typecheck, but this pass must never crash the compiler over a
/// warning). The filler type's identity doesn't matter: it is only ever
/// reached through a defensive mismatch, and even then only constrains a
/// position that a well-typed program can only ever put a wildcard/var
/// against.
fn pad_types(mut tys: Vec<MonoType>, arity: usize) -> Vec<MonoType> {
    while tys.len() < arity {
        tys.push(MonoType::Base(BaseType::Unit));
    }
    tys.truncate(arity);
    tys
}

/// Rebuild a concrete pattern from a head key and its (already-witnessed)
/// sub-patterns — the inverse of `head_of`, used to reassemble a witness as
/// `usefulness` unwinds its recursion.
fn rebuild<'s>(head: &HeadKey, subs: &[Pattern<'s>]) -> Pattern<'s> {
    match head {
        HeadKey::Unit => Pattern::Unit,
        HeadKey::Bool(b) => Pattern::Bool(*b),
        HeadKey::Int(n) => Pattern::Int(*n),
        HeadKey::Str(s) => Pattern::Str(s.clone()),
        HeadKey::Tuple(_) => Pattern::Tuple(subs.to_vec()),
        HeadKey::EmptyList => Pattern::EmptyList,
        HeadKey::Cons => Pattern::Cons(Box::new(subs[0].clone()), Box::new(subs[1].clone())),
        HeadKey::Ctor(name) => Pattern::Ctor(name.clone(), subs.first().cloned().map(Box::new)),
    }
}

fn wildcards<'s>(arity: usize) -> Vec<Pattern<'s>> {
    vec![Pattern::Wild; arity]
}

/// A concrete witness pattern for "some value of an infinite/unknown-domain
/// type not already covered by the literals seen so far" — used only when
/// `signature` says `Infinite`. `int`/`string` get an honest fresh literal
/// (smallest int, or shortest string, not already in `sigma`); anything else
/// (an unresolved type variable, or any base type this grammar has no
/// literal-pattern syntax for) falls back to `_`, which is always
/// acceptable as a "some other value" witness.
fn infinite_witness<'s>(rty: &MonoType, sigma: &HashSet<HeadKey>) -> Pattern<'s> {
    match rty {
        MonoType::Base(BaseType::Int) => {
            let mut n = 0i64;
            loop {
                if !sigma.contains(&HeadKey::Int(n)) {
                    return Pattern::Int(n);
                }
                n += 1;
            }
        }
        MonoType::Base(BaseType::String) => {
            let mut candidate = String::new();
            loop {
                if !sigma.contains(&HeadKey::Str(candidate.clone())) {
                    return Pattern::Str(candidate);
                }
                candidate.push('x');
            }
        }
        _ => Pattern::Wild,
    }
}

// ============================================================================
// The matrix operations (`S(c, P)` / `D(P)`, exhchecker.ml's per-constructor
// filtering and its default-matrix analogue).
// ============================================================================

/// `S(c, P)`: keep rows whose first pattern is `c` (dropping column 0,
/// keeping its `arity` sub-patterns in its place) or a wildcard/var
/// (dropping column 0, filling in `arity` wildcards); drop every other row.
fn specialize<'s>(matrix: &Matrix<'s>, key: &HeadKey, arity: usize) -> Matrix<'s> {
    let mut out = Matrix::new();
    for row in matrix {
        match head_of(&row[0]) {
            None => {
                let mut new_row = wildcards(arity);
                new_row.extend_from_slice(&row[1..]);
                out.push(new_row);
            }
            Some((k, args)) if &k == key => {
                let mut new_row = args;
                new_row.extend_from_slice(&row[1..]);
                out.push(new_row);
            }
            Some(_) => {}
        }
    }
    out
}

/// `D(P)`: keep only rows whose first pattern is a wildcard/var, dropping
/// column 0 outright (no expansion — there is no constructor to expand
/// against).
fn default_matrix<'s>(matrix: &Matrix<'s>) -> Matrix<'s> {
    matrix
        .iter()
        .filter_map(|row| match head_of(&row[0]) {
            None => Some(row[1..].to_vec()),
            Some(_) => None,
        })
        .collect()
}

/// Σ: the distinct head constructors actually present in column 0 of `P`
/// (wildcard/var rows contribute nothing).
fn column_heads<'s>(matrix: &Matrix<'s>) -> HashSet<HeadKey> {
    matrix
        .iter()
        .filter_map(|row| head_of(&row[0]).map(|(k, _)| k))
        .collect()
}

// ============================================================================
// `U(P, q)` — the usefulness function itself.
// ============================================================================

/// Read-only context threaded through the recursion: the variant-type table
/// (`Checker::variants`, keyed by *type* name — see `typecheck.rs`), needed
/// to enumerate a user variant's full constructor set and to instantiate a
/// chosen constructor's payload type.
struct Ctx<'a> {
    variants: &'a HashMap<String, Rc<VariantDecl>>,
}

/// `U(P, q)`: is there a value matched by `q` but by no row of `P`? `P` and
/// `q` always have the same column count as `col_types` (one type per
/// column) — an invariant `pad_types` exists to preserve across every
/// recursive call. Returns the witness row (same length as `q`) when `q` is
/// useful, `None` when `P` already covers it.
fn usefulness<'s>(
    ctx: &Ctx,
    matrix: &Matrix<'s>,
    q: &[Pattern<'s>],
    col_types: &[MonoType],
) -> Option<Vec<Pattern<'s>>> {
    if q.is_empty() {
        // cols = 0: useful iff P has no rows at all.
        return if matrix.is_empty() {
            Some(Vec::new())
        } else {
            None
        };
    }

    if let Some((key, sub_pats)) = head_of(&q[0]) {
        // q[0] is a concrete constructor c(r1..ra): U(P,q) = U(S(c,P), r1..ra ++ q[1..]).
        let arity = sub_pats.len();
        let rty0 = resolve(&col_types[0]);
        let sub_tys = pad_types(sub_types_for(&key, &rty0, ctx.variants), arity);

        let spec = specialize(matrix, &key, arity);
        let mut new_q = sub_pats;
        new_q.extend_from_slice(&q[1..]);
        let mut new_tys = sub_tys;
        new_tys.extend_from_slice(&col_types[1..]);

        let witness = usefulness(ctx, &spec, &new_q, &new_tys)?;
        let (sub_w, rest_w) = witness.split_at(arity);
        let mut result = vec![rebuild(&key, sub_w)];
        result.extend_from_slice(rest_w);
        return Some(result);
    }

    // q[0] is a wildcard/var: branch on whether Σ (column 0's observed
    // heads) is a complete signature for the column's type.
    let rty0 = resolve(&col_types[0]);
    let sigma = column_heads(matrix);
    match signature(&rty0, ctx.variants) {
        Signature::Finite(full) if full.iter().all(|(h, _)| sigma.contains(h)) => {
            // Complete: U(P,q) = OR over c∈Σ of U(S(c,P), wildcards(c)++q[1..]).
            for (key, arity) in &full {
                let sub_tys = pad_types(sub_types_for(key, &rty0, ctx.variants), *arity);
                let spec = specialize(matrix, key, *arity);
                let mut new_q = wildcards(*arity);
                new_q.extend_from_slice(&q[1..]);
                let mut new_tys = sub_tys;
                new_tys.extend_from_slice(&col_types[1..]);

                if let Some(witness) = usefulness(ctx, &spec, &new_q, &new_tys) {
                    let (sub_w, rest_w) = witness.split_at(*arity);
                    let mut result = vec![rebuild(key, sub_w)];
                    result.extend_from_slice(rest_w);
                    return Some(result);
                }
            }
            None
        }
        Signature::Finite(full) => {
            // Incomplete: U(P,q) = U(D(P), q[1..]); the witness's first slot
            // is any constructor not in Σ, applied to wildcards.
            let (mkey, marity) = full
                .iter()
                .find(|(h, _)| !sigma.contains(h))
                .expect("Finite branch guard already established Σ doesn't cover `full`");
            let missing_example = rebuild(mkey, &wildcards(*marity));
            let def = default_matrix(matrix);
            let witness = usefulness(ctx, &def, &q[1..], &col_types[1..])?;
            let mut result = vec![missing_example];
            result.extend_from_slice(&witness);
            Some(result)
        }
        Signature::Infinite => {
            let missing_example = infinite_witness(&rty0, &sigma);
            let def = default_matrix(matrix);
            let witness = usefulness(ctx, &def, &q[1..], &col_types[1..])?;
            let mut result = vec![missing_example];
            result.extend_from_slice(&witness);
            Some(result)
        }
    }
}

// ============================================================================
// Public entry point.
// ============================================================================

/// Check one `match`'s arms for redundancy and exhaustiveness, given the
/// (already as-resolved-as-inference-will-make-it) scrutinee type. Called
/// from `typecheck.rs`'s `Ast::Match` rule, after its arm-typing loop.
///
/// Guarded arms (`MatchArm.guard.is_some()`) may fail at runtime, so they
/// never contribute coverage: they are skipped entirely (never checked for
/// redundancy, never added to the growing matrix), matching
/// `exhchecker.ml`'s separate `nonexh_guard` tracking (lines 358-360). A
/// match whose only catch-all is guarded is therefore correctly reported
/// non-exhaustive.
pub fn check_match<'s>(
    store: &'s SymbolStore,
    scrutinee_ty: &MonoType,
    scrutinee_span: Option<Span>,
    arms: &[MatchArm<'s>],
    variants: &HashMap<String, Rc<VariantDecl>>,
) -> Vec<MatchWarning> {
    let ctx = Ctx { variants };
    let col_types = [scrutinee_ty.clone()];
    let mut rows: Matrix<'s> = Vec::new();
    let mut warnings = Vec::new();

    for (i, arm) in arms.iter().enumerate() {
        if arm.guard.is_some() {
            continue;
        }
        let row = vec![arm.pat.clone()];
        if usefulness(&ctx, &rows, &row, &col_types).is_none() {
            warnings.push(MatchWarning {
                span: crate::typecheck::ast_span(&arm.body),
                message: format!(
                    "match arm {} (`{}`) is unreachable: already covered by an earlier arm",
                    i + 1,
                    render_pattern(store, &arm.pat)
                ),
            });
        }
        rows.push(row);
    }

    let query = [Pattern::Wild];
    if let Some(witness) = usefulness(&ctx, &rows, &query, &col_types) {
        warnings.push(MatchWarning {
            span: scrutinee_span,
            message: format!(
                "this pattern-matching is not exhaustive; example uncovered value: `{}`",
                render_pattern(store, &witness[0])
            ),
        });
    }

    warnings
}

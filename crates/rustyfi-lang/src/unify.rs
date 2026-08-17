//! Structural unification over [`crate::types::MonoType`], mirroring
//! v0.0.6's `unify_sub` (`src/frontend/typechecker.ml:360-522`): occurs
//! check (extended to look through rows, since rows are first-class here —
//! see the module doc comment on `crate::types::Row`), record/row
//! unification, and the `Kind::Record` bridging case.
//!
//! `unify`'s signature takes no `TypeContext`; see `crate::types::FRESH_ID`
//! for how it still mints correctly-leveled fresh variables when it needs
//! to extend an open row.

use crate::types::{self, resolve, resolve_row, CmdArgType, Kind, MonoType, Row, RowVarRef, TyVarRef};
use std::collections::BTreeSet;

#[derive(Debug, thiserror::Error)]
pub enum UnifyError {
    #[error("type mismatch: expected `{expected}`, found `{found}`")]
    Mismatch { expected: MonoType, found: MonoType },

    #[error("occurs check failed: the type would be infinite")]
    OccursCheck,

    #[error("record is missing label `{label}` (required for `{ty}`)")]
    MissingLabel { label: String, ty: MonoType },

    #[error("arity mismatch: expected {expected} element(s), found {found}")]
    ArityMismatch { expected: usize, found: usize },

    #[error("command argument optionality mismatch for `{ty}` (`?` on one side only)")]
    OptionalMismatch { ty: MonoType },

    /// SATySFi 0.1's closed command optional-label map is INVARIANT under
    /// unification (upstream `subtype_label_map_with_equal_domain`,
    /// `signatureSubtyping.ml:482,511-516`): a supplied/declared label set
    /// mismatch — either side has a label the other doesn't — is always
    /// rejected, never widened/narrowed. `expected`/`found` are each a
    /// formatted `?(l1 : τ1, …)` rendering of the two sides' label sets (or
    /// `?()` for an empty one) for a readable message.
    #[error("command optional-argument label set mismatch: expected `{expected}`, found `{found}`")]
    CmdLabelMismatch { expected: String, found: String },
}

/// Unify two monomorphic types in place: free variables get linked
/// (mutating their union-find cell) so that both `a` and `b` — and anything
/// else sharing those variables — subsequently `resolve` to the same type.
pub fn unify(a: &MonoType, b: &MonoType) -> Result<(), UnifyError> {
    let ra = resolve(a);
    let rb = resolve(b);
    match (&ra, &rb) {
        (MonoType::Var(v1), MonoType::Var(v2)) if v1.same(v2) => Ok(()),
        (MonoType::Var(v), _) => bind_var(v, rb.clone()),
        (_, MonoType::Var(v)) => bind_var(v, ra.clone()),

        (MonoType::Base(x), MonoType::Base(y)) => {
            if x == y {
                Ok(())
            } else {
                Err(UnifyError::Mismatch { expected: ra.clone(), found: rb.clone() })
            }
        }

        (MonoType::Func(r1, d1, c1), MonoType::Func(r2, d2, c2)) => {
            unify_row(r1, r2)?;
            unify(d1, d2)?;
            unify(c1, c2)
        }

        (MonoType::Product(ts1), MonoType::Product(ts2)) => {
            if ts1.len() != ts2.len() {
                return Err(UnifyError::ArityMismatch { expected: ts1.len(), found: ts2.len() });
            }
            for (x, y) in ts1.iter().zip(ts2) {
                unify(x, y)?;
            }
            Ok(())
        }

        (MonoType::List(x), MonoType::List(y)) => unify(x, y),
        (MonoType::Ref(x), MonoType::Ref(y)) => unify(x, y),

        (MonoType::Record(r1), MonoType::Record(r2)) => unify_row(r1, r2),

        (MonoType::Variant(n1, a1), MonoType::Variant(n2, a2)) => {
            if n1 != n2 {
                return Err(UnifyError::Mismatch { expected: ra.clone(), found: rb.clone() });
            }
            if a1.len() != a2.len() {
                return Err(UnifyError::ArityMismatch { expected: a1.len(), found: a2.len() });
            }
            for (x, y) in a1.iter().zip(a2) {
                unify(x, y)?;
            }
            Ok(())
        }

        (MonoType::InlineCmd(c1), MonoType::InlineCmd(c2))
        | (MonoType::BlockCmd(c1), MonoType::BlockCmd(c2))
        | (MonoType::MathCmd(c1), MonoType::MathCmd(c2)) => unify_cmd_args(c1, c2),

        _ => Err(UnifyError::Mismatch { expected: ra.clone(), found: rb.clone() }),
    }
}

fn unify_cmd_args(a: &[CmdArgType], b: &[CmdArgType]) -> Result<(), UnifyError> {
    if a.len() != b.len() {
        return Err(UnifyError::ArityMismatch { expected: a.len(), found: b.len() });
    }
    for (x, y) in a.iter().zip(b) {
        if x.optional != y.optional {
            return Err(UnifyError::OptionalMismatch { ty: x.ty.clone() });
        }
        // SATySFi 0.1 closed command optional-label map: EQUAL DOMAIN (both
        // sides kept sorted by label at every producer — `command_scheme`'s
        // harvest, `lower_type_atom`'s sig lowering). A label present on only
        // one side makes the lengths or the pairwise names diverge here,
        // exactly upstream's "invariant label set under a seal"
        // (`signatureSubtyping.ml:511-516`). A no-op when both sides are `[]`
        // (every 0.0.6-reachable `CmdArgType`), so this is byte-identical for
        // the frozen corpus.
        if x.opt_labels.len() != y.opt_labels.len()
            || x.opt_labels.iter().zip(&y.opt_labels).any(|((lx, _), (ly, _))| lx != ly)
        {
            return Err(UnifyError::CmdLabelMismatch {
                expected: fmt_opt_label_set(&x.opt_labels),
                found: fmt_opt_label_set(&y.opt_labels),
            });
        }
        for ((_, tx), (_, ty2)) in x.opt_labels.iter().zip(&y.opt_labels) {
            unify(tx, ty2)?;
        }
        unify(&x.ty, &y.ty)?;
    }
    Ok(())
}

/// Render a closed optional-label set as `?(l1 : τ1, …)` (or `?()` if empty)
/// for a [`UnifyError::CmdLabelMismatch`] message.
fn fmt_opt_label_set(labels: &[(String, MonoType)]) -> String {
    let mut s = String::from("?(");
    for (i, (l, t)) in labels.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&format!("{l} : {t}"));
    }
    s.push(')');
    s
}

// ============================================================================
// Binding a type variable (v0.0.6's `TypeVariable(_)` branches,
// typechecker.ml:436-519).
// ============================================================================

fn bind_var(v: &TyVarRef, ty: MonoType) -> Result<(), UnifyError> {
    if let MonoType::Var(v2) = &ty {
        if v.same(v2) {
            return Ok(());
        }
    }
    if occurs_var(v, &ty) {
        return Err(UnifyError::OccursCheck);
    }
    match v.kind() {
        Kind::Universal => {
            v.bind(ty);
            Ok(())
        }
        Kind::Record(required) => match &ty {
            MonoType::Var(v2) => {
                let merged = match v2.kind() {
                    Kind::Universal => Kind::Record(required.clone()),
                    Kind::Record(r2) => Kind::Record(required.union(&r2).cloned().collect()),
                };
                v2.set_kind(merged);
                v.bind(ty);
                Ok(())
            }
            MonoType::Record(row) => {
                for label in &required {
                    row_require_label(row, label)?;
                }
                v.bind(ty);
                Ok(())
            }
            other => Err(UnifyError::Mismatch {
                expected: MonoType::Var(v.clone()),
                found: other.clone(),
            }),
        },
    }
}

/// Confirm `row` contains `label` (mirrors v0.0.6's
/// `Assoc.domain_included`/`Assoc.intersection` check at
/// typechecker.ml:480-500), extending an open row with a fresh field if
/// necessary.
fn row_require_label(row: &Row, label: &str) -> Result<(), UnifyError> {
    match resolve_row(row) {
        Row::Empty => Err(UnifyError::MissingLabel {
            label: label.to_string(),
            ty: MonoType::Record(Row::Empty),
        }),
        Row::Cons(l, _, rest) => {
            if l == label {
                Ok(())
            } else {
                row_require_label(&rest, label)
            }
        }
        Row::Var(v) => {
            let level = v.level().unwrap_or(0);
            let field = types::new_ty_var(level);
            let remainder = types::new_row_var(level);
            let extended = Row::Cons(
                label.to_string(),
                Box::new(MonoType::Var(field)),
                Box::new(Row::Var(remainder)),
            );
            bind_row_var(&v, extended)
        }
    }
}

// ============================================================================
// Row unification (Rémy-style label extraction/subsumption).
// ============================================================================

fn unify_row(a: &Row, b: &Row) -> Result<(), UnifyError> {
    let ra = resolve_row(a);
    let rb = resolve_row(b);
    match (ra, rb) {
        (Row::Empty, Row::Empty) => Ok(()),
        (Row::Var(v1), Row::Var(v2)) if v1.same(&v2) => Ok(()),

        (Row::Cons(l, t, rest), other) => {
            let (t2, rest2) = row_extract(&other, &l)?;
            unify(&t, &t2)?;
            unify_row(&rest, &rest2)
        }
        (other, Row::Cons(l, t, rest)) => {
            let (t2, rest2) = row_extract(&other, &l)?;
            unify(&t2, &t)?;
            unify_row(&rest2, &rest)
        }

        (Row::Empty, Row::Var(v)) | (Row::Var(v), Row::Empty) => {
            if let Some(label) = v.kind().iter().next().cloned() {
                return Err(UnifyError::MissingLabel { label, ty: MonoType::Record(Row::Empty) });
            }
            bind_row_var(&v, Row::Empty)
        }

        (Row::Var(v1), Row::Var(v2)) => {
            let union: BTreeSet<String> = v1.kind().union(&v2.kind()).cloned().collect();
            v2.set_kind(union);
            bind_row_var(&v1, Row::Var(v2))
        }
    }
}

/// Extract the field labeled `label` out of `row`, returning its type and
/// the row of everything else. If `row` is an open row var that doesn't
/// (yet) mention `label`, this *extends* it in place — binding the
/// variable to `Cons(label, fresh_field, Var(fresh_remainder))` — and
/// returns the fresh field/remainder pair. This is what realizes label
/// subsumption against an open row as "unify one field at a time, leaving
/// a fresh remainder row variable" instead of requiring the whole row to
/// match up front.
fn row_extract(row: &Row, label: &str) -> Result<(MonoType, Row), UnifyError> {
    match resolve_row(row) {
        Row::Empty => Err(UnifyError::MissingLabel {
            label: label.to_string(),
            ty: MonoType::Record(Row::Empty),
        }),
        Row::Cons(l, t, rest) => {
            if l == label {
                Ok((*t, *rest))
            } else {
                let (t2, rest2) = row_extract(&rest, label)?;
                Ok((t2, Row::Cons(l, t, Box::new(rest2))))
            }
        }
        Row::Var(v) => {
            let level = v.level().unwrap_or(0);
            let field = types::new_ty_var(level);
            let remainder = types::new_row_var(level);
            let extended = Row::Cons(
                label.to_string(),
                Box::new(MonoType::Var(field.clone())),
                Box::new(Row::Var(remainder.clone())),
            );
            bind_row_var(&v, extended)?;
            Ok((MonoType::Var(field), Row::Var(remainder)))
        }
    }
}

fn bind_row_var(v: &RowVarRef, row: Row) -> Result<(), UnifyError> {
    if let Row::Var(v2) = &row {
        if v.same(v2) {
            return Ok(());
        }
    }
    if occurs_rowvar_in_row(v, &row) {
        return Err(UnifyError::OccursCheck);
    }
    v.bind(row);
    Ok(())
}

// ============================================================================
// Occurs checks. As in v0.0.6 (typechecker.ml:179-261), these double as a
// level-lowering pass: any free variable found strictly deeper than the
// variable being bound has its level pulled up (lowered numerically) to
// match, which is what keeps `generalize` from over-generalizing a
// variable that unification has linked to something from an outer scope.
// ============================================================================

fn occurs_var(tv: &TyVarRef, ty: &MonoType) -> bool {
    match resolve(ty) {
        MonoType::Var(v) => {
            if v.same(tv) {
                return true;
            }
            if let (Some(tv_level), Some(v_level)) = (tv.level(), v.level()) {
                if tv_level < v_level {
                    v.set_level(tv_level);
                }
            }
            false
        }
        MonoType::Base(_) => false,
        MonoType::Func(row, a, b) => {
            occurs_var_in_row(tv, &row) || occurs_var(tv, &a) || occurs_var(tv, &b)
        }
        MonoType::Product(ts) => ts.iter().any(|t| occurs_var(tv, t)),
        MonoType::List(t) | MonoType::Ref(t) => occurs_var(tv, &t),
        MonoType::Record(row) => occurs_var_in_row(tv, &row),
        MonoType::Variant(_, args) => args.iter().any(|t| occurs_var(tv, t)),
        MonoType::InlineCmd(cs) | MonoType::BlockCmd(cs) | MonoType::MathCmd(cs) => {
            cs.iter().any(|c| occurs_var(tv, &c.ty))
        }
    }
}

fn occurs_var_in_row(tv: &TyVarRef, row: &Row) -> bool {
    match resolve_row(row) {
        Row::Empty => false,
        Row::Var(_) => false,
        Row::Cons(_, t, rest) => occurs_var(tv, &t) || occurs_var_in_row(tv, &rest),
    }
}

fn occurs_rowvar_in_type(rv: &RowVarRef, ty: &MonoType) -> bool {
    match resolve(ty) {
        MonoType::Var(_) => false,
        MonoType::Base(_) => false,
        MonoType::Func(row, a, b) => {
            occurs_rowvar_in_row(rv, &row)
                || occurs_rowvar_in_type(rv, &a)
                || occurs_rowvar_in_type(rv, &b)
        }
        MonoType::Product(ts) => ts.iter().any(|t| occurs_rowvar_in_type(rv, t)),
        MonoType::List(t) | MonoType::Ref(t) => occurs_rowvar_in_type(rv, &t),
        MonoType::Record(row) => occurs_rowvar_in_row(rv, &row),
        MonoType::Variant(_, args) => args.iter().any(|t| occurs_rowvar_in_type(rv, t)),
        MonoType::InlineCmd(cs) | MonoType::BlockCmd(cs) | MonoType::MathCmd(cs) => {
            cs.iter().any(|c| occurs_rowvar_in_type(rv, &c.ty))
        }
    }
}

fn occurs_rowvar_in_row(rv: &RowVarRef, row: &Row) -> bool {
    match resolve_row(row) {
        Row::Empty => false,
        Row::Var(v) => {
            if v.same(rv) {
                return true;
            }
            if let (Some(rv_level), Some(v_level)) = (rv.level(), v.level()) {
                if rv_level < v_level {
                    v.set_level(rv_level);
                }
            }
            false
        }
        Row::Cons(_, t, rest) => occurs_rowvar_in_type(rv, &t) || occurs_rowvar_in_row(rv, &rest),
    }
}

//! One generated, exhaustive traversal of the **type representation** —
//! [`MonoType`](crate::types::MonoType), [`Row`](crate::types::Row) and
//! [`CmdArgType`](crate::types::CmdArgType).
//!
//! # Why this exists
//!
//! The same reason `rustyfi-backend`'s `visit` module exists: a hand-written
//! walk that forgets one container fails **silently**. This family had a live
//! instance of exactly that, and it was not theoretical —
//! [`CmdArgType::opt_labels`](crate::types::CmdArgType::opt_labels), the
//! closed `?(l : τ, …)` map on a 0.1 command-argument slot, is a second
//! `MonoType`-bearing field next to `ty`, and four of the family's walks
//! recursed into `ty` alone:
//!
//! ```text
//!     MonoType::InlineCmd(cs) | .. => cs.iter().any(|c| f(&c.ty))
//!                                                      ^^^^^ and not c.opt_labels
//! ```
//!
//! In `typecheck::synonym_refs` — which builds the graph
//! `check_synonym_cycles` walks — that omission meant a synonym cycle routed
//! through an optional-label slot was never seen, while `expand_synonyms`
//! (whose termination that check is the *only* guarantee of, per its own doc
//! comment) happily expanded into it. `type t = inline [?(l : t) string]`,
//! once referenced, aborted the process with a stack overflow instead of
//! reporting `cyclic type synonym`. `tests/synonym_cycle_opt_labels.rs` is
//! that repro.
//!
//! # What it covers, and what it deliberately does not
//!
//! **A type variable is a LEAF.** Neither `TyVarRef` nor `RowVarRef` appears
//! in any `#[subast]` list, so `MonoType::Var` / `Row::Var` are not descended
//! into and a `TyVarLink::Bound(τ)` payload is never reached. Three separate
//! reasons, and the first alone is decisive:
//!
//! 1. **It cannot be expressed.** A bound payload lives behind
//!    `Rc<RefCell<..>>`; the `Ref<'_, TyVarLink>` guard cannot outlive the
//!    frame that took it, so there is no `&MonoType` *inside the original
//!    tree* to hand a visitor. [`crate::types::resolve`] returns a
//!    [`Cow`](std::borrow::Cow) for precisely this reason, and its doc
//!    comment records the measurement (the alternative — an owned clone —
//!    was 87-89% of all type nodes cloned during typechecking). syan has no
//!    view impl for `Rc` or `RefCell` in any case; a field of that shape is
//!    a hard compile error, not a silent skip.
//! 2. **Termination.** With `Var` a leaf the walk is structurally
//!    well-founded. Following links is only sound because `bind` is
//!    occurs-checked — a property of the *unifier*, not of the type.
//! 3. **It matches the walks that use this.** The family splits cleanly in
//!    two. *Resolving* walks (`unify`, `occurs_*`, `collect_generalizable`,
//!    `substitute`, `fmt_mono`, `mono_mentions_stamp`, …) call `resolve` at
//!    every level and see the post-unification tree. *Structural* walks
//!    (`synonym_refs`, `xver_adapt::check_mono_type`) `match ty` directly and
//!    see the tree as written. This traversal has structural semantics, so
//!    only the second class may use it — which is fine, because the second
//!    class is where the tree is a tree.
//!
//! A resolving walk that adopted this would silently change meaning. Do not.
//!
//! # Using it
//!
//! Every visited type gains inherent `visit` / `visit_mut` methods taking a
//! closure (or a tuple of closures, one per node type). A visit is
//! **pre-order and inclusive**: `t.visit(|x: &MonoType| ..)` sees `t` itself
//! before its children, and descent is unconditional — a consumer that needs
//! to prune must implement the generated `Visit` trait by hand.
//!
//! # The one trap
//!
//! `visitor!` follows a field only if the field's *peeled* head type is named
//! in the owning type's `#[subast(..)]` list. A new `MonoType`-bearing field
//! whose head the list does not mention is reclassified a leaf and its
//! generated body is empty — **zero errors, zero warnings**. That is the same
//! failure this module exists to prevent, one level up.
//! `tests/type_visit_reachability.rs` is the missing check, done at runtime:
//! it plants a uniquely identifiable marker type under every recursive field
//! of every visited type and asserts each one is reached.

syan::visit::visitor!(
    crate::types::MonoType,
    crate::types::Row,
    crate::types::CmdArgType
);

# Survey: tree-shaped types beyond the box tree, and whether `syan`'s `visitor!` helps

Companion to `crates/rustyfi-backend/src/visit.rs` (the box tree, already
converted) and `crates/rustyfi-lang/src/visit.rs` (the type representation,
converted here). This file records what was examined, what was rejected, and
why — including the negative results, which are the more reusable half.

The question is never "is this shorter". It is: **this port's recurring bug is
a walk that fails to descend into a container, where the missing content is
silent.** A candidate is worth converting only if a missed variant or field
would be silent *and* the walk's shape fits what `visitor!` can express.

---

## 0. What `visitor!` can and cannot do (measured, not assumed)

Everything below rests on these, established against `syan 0.2.2` with a
throwaway crate rather than by reading the macro:

| Field shape | Result |
| --- | --- |
| Generic type param (`Ast<I>`) | **Works.** `visitor/params.rs`'s `param_union` lifts each visited type's params onto the generated trait. |
| `Box<T>`, `Vec<T>`, `Option<T>`, nested (`Vec<Vec<T>>`) | **Works** (`OptView`/`SeqView`). |
| `Vec<(String, T)>` — tuple inside a container | **Works.** `peel` returns `Head::Tuple` and lowers each element. |
| `HashMap<K, T>` / `BTreeMap<K, T>` | **Works in 0.2.2** (`MapView` walks the VALUE slot; 0.2.0 silently took the key). |
| **`Rc<T>`** | **Hard compile error.** No view impl exists: `no method named 'view_iter' found for reference '&Rc<Ex<I>>'`. Loud, not silent. |
| **`RefCell<T>`** | Same — no view impl, and a `Ref<'_, T>` guard cannot outlive the frame that took it. |

The `Rc` result is the single most consequential finding, so its escape hatch
was tested too. A downstream `impl<I> OptView<Ex<I>> for Rc<Ex<I>>` **is**
legal (the local type `Ex` appears in the trait's argument list, and `I` is
covered by it). A downstream `impl<I> OptView<Vec<Pat<I>>> for Rc<Vec<Pat<I>>>`
is **E0117** — neither `Rc` nor `Vec` is local, so `Rc<Vec<LocalType>>` cannot
be given a view from outside `syan`:

```
error[E0117]: only traits defined in the current crate can be implemented for
              types defined outside of the crate
   = note: impl doesn't have any local type before any uncovered type parameters
```

**So `Rc<T>` where `T` is a local node is fixable; `Rc<Vec<T>>` is not.** That
one line decides three of the candidates below.

There is no "treat this field as a leaf" attribute. `#[seq]`/`#[opt]` are the
only field markers the derive preserves.

Two shape constraints from the existing box-tree work still apply: a closure
visitor **cannot prune** (that needs a hand-written `Visit` impl), and it
**cannot rebuild** — it hands you `&T`/`&mut T`, never a fresh value of a
different shape.

---

## 1. CONVERTED — `MonoType` / `Row` / `CmdArgType` (`rustyfi-lang/src/types.rs`)

The only candidate where a `visitor!` both **fits** and **fixes a proven bug**.

### Eligibility

Every followed field is `Box`, `Vec`, plain, or `Vec<(String, MonoType)>` —
all supported. The two `Rc<RefCell<..>>` cells (`TyVarRef`, `RowVarRef`) sit
behind structs whose fields are private, and they are deliberately **omitted
from every `#[subast]` list**, so a type variable is a leaf and no `Bound`
link is ever followed.

That omission is the load-bearing design decision, for three reasons, the
first decisive:

1. **It cannot be expressed.** `types::resolve` returns `Cow<'_, MonoType>`
   precisely because "the `Bound` payload lives behind a `RefCell` whose guard
   cannot outlive this frame". There is no `&MonoType` *inside the original
   tree* to hand a visitor.
2. **Termination.** With `Var` a leaf the walk is structurally well-founded.
   Following links is sound only because `bind` is occurs-checked — a property
   of the unifier, not of the type.
3. **It matches the walks that can use it.** Every walker over this family
   (about fifty, counting the `Row` and `CmdArgType` twins) splits cleanly in
   two. *Resolving* walks (`unify`, `occurs_*`, `collect_generalizable`,
   `substitute`, `fmt_mono`, `mono_mentions_stamp`, `mono_alpha_eq`, …) call
   `resolve` at every level. *Structural* walks (`synonym_refs`,
   `xver_adapt::check_mono_type`) `match ty` directly. The generated traversal
   has structural semantics, so only the second class may adopt it.

Most are out of reach for reasons already familiar from the box tree: rebuilders (`substitute`, `instantiate`, `expand_synonyms`, `row_extract`,
`harvest_slot`) return new values; `unify` and `mono_alpha_eq` walk **two**
trees in lockstep; `peel_func_chain` prunes the domain subtree; `fmt_*` thread
a `&mut VarNamer`.

That leaves exactly two structural walks. `xver_adapt::check_mono_type` is
`#[allow(dead_code)]` and already handles every field. The other one had the
bug.

### The bug (found, reproduced, fixed)

`CmdArgType` has **two** `MonoType`-bearing fields:

```rust
pub struct CmdArgType {
    pub optional: bool,
    pub opt_labels: Vec<(String, MonoType)>,   // 0.1's closed `?(l : τ, …)` map
    pub ty: MonoType,
}
```

Four hand-written walks recursed into `ty` and forgot `opt_labels`:

| Site | Consequence |
| --- | --- |
| `typecheck.rs` `synonym_refs` | **Stack overflow / `SIGABRT`** (below) |
| `v1/sig_subtype.rs` `mono_mentions_stamp` | **Silent** — an escaped skolem hiding in a `?(l : τ)` bundle passes the sealing check |
| `unify.rs` `occurs_var` | occurs cycle + level-lowering skipped inside a bundle |
| `unify.rs` `occurs_rowvar_in_type` | same, for row variables |

`synonym_refs` builds the graph `check_synonym_cycles` walks, and
`expand_synonyms`' doc comment states that check is the *only* reason it
terminates. But `expand_synonyms_cmd_args` **does** expand `opt_labels`. So a
cycle routed through an optional-label slot was invisible to the guard and
reachable by the expander. Measured, from ordinary source text:

```
type t = inline [t]                 ->  Err(Type("cyclic type synonym: M.t -> M.t"))   # control
type t = inline [?(l : t) string]   ->  thread ... has overflowed its stack
                                        fatal runtime error: stack overflow, aborting
```

Note a `#[should_panic]` test could not have caught this: a stack overflow is
a `SIGABRT`, not an unwind.

### The fix

`synonym_refs` is now derived — the whole walk is one closure over the
generated traversal, and it cannot omit a field:

```rust
ty.visit(|t: &MonoType| {
    if let MonoType::Variant(name, _) = t {
        if synonyms.contains_key(name) { out.push(name.clone()); }
    }
});
```

Semantics are otherwise identical: structural (no `resolve`), pre-order,
unconditional descent — exactly what the hand-written pair did.

The other three sites **cannot** use the visitor (they resolve links), so they
are one-line hand fixes with a comment saying why they cannot be mechanized.

Tests: `tests/synonym_cycle_opt_labels.rs` (5 cases, including a two-step
cycle, the unreferenced case that used to pass *vacuously*, and a negative
control so a "reject all `opt_labels` mentions" non-fix would fail) and
`tests/type_visit_reachability.rs` (14 `#[subast]` edges).

The reachability test was validated by deliberately deleting
`crate::types::CmdArgType` from `MonoType`'s `#[subast]` list: it fails with
**zero compiler errors and zero warnings**, confirming the trap is live.

---

## 2. REJECTED — `Ast` and friends (`rustyfi-lang/src/ast.rs`)

**Ineligible, structurally.** Three variants hold `Rc<Vec<_>>`:

```rust
InlineText(Rc<Vec<IText<I>>>),
BlockText(Rc<Vec<BText<I>>>),
MathText(Rc<Vec<MathElem<I>>>),
// plus IText::EmbedMath { elems: Rc<Vec<MathElem<I>>> }
```

`Rc` has no view impl, and per §0 an `Rc<Vec<LocalType>>` view **cannot be
supplied downstream** — E0117, because neither `Rc` nor `Vec` is local. Three
further variants (`Lambda`, `LambdaOpt.body`, `LetRecIn`) hold `Rc<Ast<I>>`,
which *could* be fixed with a local `OptView` impl, but the `Rc<Vec<_>>` three
cannot be, short of newtyping the payload and touching elaborate, typecheck,
compile, eval and all 172 primitives. Out of proportion by a wide margin.

(The generic parameter `I` would have been fine — that was checked
separately and works.)

**And it would not help anyway.** The two walks that matter are already
wildcard-free over all 34 variants: `compile::Compiler::compile` and
`typecheck::Checker::infer`. Adding a variant is already a compile error in
both. The six wildcard sites are all *deliberate prunes* — `compile_spine`'s
`other => self.compile(other)` is the "document body reached" signal;
`binding_stage`/`binding_version`/`already_staged`/`ast_span` peel at most two
wrapper levels on purpose. A visitor cannot prune, so it could not replace any
of them.

The one genuinely asymmetric wildcard found — `ast_span` peels `VersionScope`
and `ModuleScope` but **not** `StageScope`, so a `StageScope(_, Var(..))`
reports a spanless error where the other two wrappers report a span — is a
one-arm inconsistency, not a container omission, and is out of scope here.

## 3. REJECTED — `Value` (`rustyfi-lang/src/value.rs`)

**Ineligible twice over.**

- `Value::Ref(Rc<RefCell<Value>>)` — both wrappers unsupported, and
  `Rc<RefCell<Value>>` is again the un-implementable orphan shape.
- **It is a graph, not a tree.** `Env(Rc<Frame>)` with
  `Frame { slots: RefCell<Vec<Value>>, parent: Option<Env> }` is shared by
  `Rc` and back-patched by `let rec`, so a recursive closure's environment
  contains the closure. `visitor!` descends unconditionally and has no visited
  set: it would not terminate.

**And there is nothing to convert.** `Value::type_name` is the only exhaustive
match on `Value` in the workspace, and it is depth-0. There is no `Display`,
no `PartialEq`, no deep clone, no recursive scan. The ~40 `as_*` extractors in
`primitives.rs` are depth ≤ 2 destructurings. `eval::match_pattern` recurses on
the *`Pattern`*, not the value, and its `_ => false` arms encode "shape
mismatch is not an error" by design.

## 4. REJECTED — `Math` / `MathElement` (`rustyfi-lang/src/value.rs`)

**Eligible** — genuinely tree-shaped, 14 variants recursive through
`Vec<Math>`, `Option<Vec<Math>>` and `Vec<Vec<Math>>`, with no `Rc` inside
(`Value::Math`'s `Rc` is at the boundary, above the tree). `MathElement` does
not recurse back into `Math`. `Box<Value>` closure payloads and `Box<Context>`
would be leaves.

**But no walker can use it.** Every `Math` walk in `primitives.rs` threads
position or context sideways, exactly like the box-tree walks that were ruled
out: the layout fold carries `out`, a running `x`, and the previous atom's
`MathKind` (siblings observe each other's trailing class for inter-atom
spacing); `boundary_math_kind` inspects only the boundary atom; `check_subscript`
rebuilds. No collect-shaped walk over `Math` exists, so there is no
silent-omission surface to protect.

Also relevant: math code in `rustyfi-lang`/`rustyfi-backend` is being changed
concurrently by another agent, so a speculative conversion here would collide
for no benefit.

## 5. REJECTED — `ObjRepr` (`rustyfi-backend/src/hbox.rs`)

The imported-PDF object graph — `Array(Vec<ObjRepr>)`,
`Dict(Vec<(Vec<u8>, ObjRepr)>)`, `Stream(Vec<(Vec<u8>, ObjRepr)>, Vec<u8>)`.
**Eligible** (the `Vec<(Vec<u8>, ObjRepr)>` shape peels fine — `Vec<u8>` is a
leaf element of the tuple, `ObjRepr` is followed).

**No benefit.** Both walks are rebuilds a closure visitor cannot express:
`primitives.rs`'s `convert_pdf_obj` maps `lopdf::Object → ObjRepr` (and
matches on the *source* type), and `rustyfi-pdf`'s `write_pdf_obj_value`
emits into a `pdf_writer::Obj` sink whose position is part of the recursion.
The latter is wildcard-free, so a new variant is already a compile error. The
local-id remap is built from the **flat** `resources.0` list, not by a tree
walk, so there is no collect-shaped walk to protect either.

## 6. REJECTED — `Sexp` (`rustyfi-satyrographos/src/satyristes.rs`)

Three variants, one recursive (`List(Vec<Sexp>)`), and **private** (`enum Sexp`,
not `pub`). Every consumer is a shallow, schema-directed destructure
(`let Sexp::List(kv) = form else { continue }`) — there is no generic recursive
walk at all, so there is nothing for a visitor to make exhaustive. A single
container type cannot be "forgotten" the way a 4-container box variant can.

## 7. REJECTED — `rustyfi-loader` dependency structures

`LoadedProgram` / `LoadedFile` / `LoadedCst` / `FileOrigin` are flat records
and a 2-variant CST wrapper. Not recursive; not tree-shaped. `LoadError` is a
27-variant `thiserror` enum whose only self-reference is a boxed `source`.
Nothing here.

## 8. NOT REOPENED — the CST (`rustyfi-syntax`)

Declined twice before, on the grounds that its walks narrow types in
load-bearing ways. Nothing in `syan 0.2.2` changes that argument: the two
0.2.2 deltas relevant here are the `MapView` value-slot fix and normal
maintenance, neither of which touches type narrowing. Re-examined only far
enough to confirm the walkers are still narrowing walks
(`lib.rs`'s `walk_expr`/`walk_atomic` family, `v1/functor.rs`), and both of
those are already documented as **wildcard-free** — `functor.rs`'s module doc
states "No `_` wildcard in any match arm below", so a new variant is already a
compile error and a visitor would buy nothing.

---

## Summary

| Type | Eligible? | Would help? | Verdict |
| --- | --- | --- | --- |
| `MonoType` / `Row` / `CmdArgType` | yes (vars as leaves) | **yes — fixed a `SIGABRT`** | **converted** |
| `Ast` + `Pattern`/`IText`/`BText`/`MathElem` | **no** — `Rc<Vec<_>>`, unfixable (E0117) | no — main walks already exhaustive | rejected |
| `Value` | **no** — `Rc<RefCell<_>>`, and cyclic via `Env` | no — no recursive walker exists | rejected |
| `Math` / `MathElement` | yes | **no** — every walk threads position/context | rejected |
| `ObjRepr` | yes | no — rebuild/emit only; already wildcard-free | rejected |
| `Sexp` | yes (if made `pub`) | no — no recursive walk exists | rejected |
| loader structures | n/a — not recursive | no | rejected |
| CST | (unchanged argument) | no — already wildcard-free | not reopened |

One conversion, one proven crash fixed, three sibling omissions fixed by hand
because they resolve union-find links and structurally cannot be mechanized.

//! Surface CST → `Ast` elaboration. Does scope resolution, operator-
//! precedence/associativity resolution (the CST leaves that flattened, see
//! `cst.rs`'s module doc comment), pattern lowering, the `let-inline`/
//! `let-block` context-argument desugaring, mutable/`while`/`before`
//! desugaring, field access/record-update folding, itemize-tree
//! reconstruction, quoted-math lowering, and (untyped) module name-mangling.
//! This function's signature is the seam where the phase-3 typechecker
//! (typechecker.ml / unification.ml port) slots in.

use crate::ast::{Ast, BText, CmdArg, IText, MatchArm, MathElem, Pattern};
use rustyfi_backend::Length;
use rustyfi_syntax::cst::{self, ast as c};
use rustyfi_syntax::leaf::{AnyHorzCmdTok, AnyMathCmdTok, AnyVertCmdTok, UnopExclamTok, VarTok};
use rustyfi_syntax::span::Span;
use rustyfi_syntax::token::Token;
use rustyfi_syntax::RustyfiVersion;
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;

#[derive(Debug, thiserror::Error)]
#[error("{span}: {msg}")]
pub struct ElabError {
    pub span: Span,
    pub msg: String,
}

fn err<T>(span: Span, msg: impl Into<String>) -> Result<T, ElabError> {
    Err(ElabError {
        span,
        msg: msg.into(),
    })
}

/// The names in scope (primitives plus, progressively, `let`-bound names).
/// A flat name set — there is no real namespacing, so a module's qualified
/// names (`"M.x"`) are just ordinary strings that happen to contain a dot
/// (see the module doc comment on [`qualify_key`]).
///
/// `optional_shape` additionally tracks, for a name bound via a plain
/// (non-`let-rec`) `let`/`let ... in` whose `Param` list contains one or more
/// `Param::Optional` (`?:name`) entries ANYWHERE in the list (`cst.rs`'s
/// `Param` doc comment — `TopLet`/`Expr::LetIn` and the three command-binding
/// forms all admit that marker) — one `bool` per declared parameter position,
/// `true` where that position is a `Param::Optional`. E.g. `let to-math
/// ?:iopt e = ..` records `[true, false]`; `stdja.satyh`'s `let document
/// record ?:configopt inner = ..` (the optional is the *second*, not the
/// first, parameter) records `[false, true, false]`. This is the
/// "marker-less optional-argument defaulting" gap
/// (`docs/plans/frontend-completion.md` Sub-area 2 / `class-signature-lang-
/// gaps.md`): a bare call site (`to-math e1`, no `?:`/`?*` at all) must still
/// supply `None` for `iopt` and match `e1` against `e` — and, symmetrically,
/// `document record body` must supply `None` for `configopt` and match
/// `body` against `inner`, NOT against `configopt`'s `option config` domain
/// (the bug this full per-position shape fixes — a scalar *leading-count-
/// only* encoding can't tell "the optional is at position 1" from "there is
/// no optional at all" once position 0 is mandatory) — see
/// `app_chain_generic`'s use of [`Scope::optional_shape`]. The same map also
/// covers command bindings (`let-inline`/`let-block`/`let-math` — `cst.rs`'s
/// `Param`, shaped by the same `param_optional_shape`): `walk_bindings`'s
/// `LetInline`/`LetBlock`/`LetMath` arms record each command's full param
/// shape the same way; [`Scope::optional_arity`] (a derived *leading-run*
/// count, `take_while` over the shape) still feeds the command-argument
/// paths (`cmd_args`, `math_bot`'s `Cmd` arm), which stay leading-only /
/// unchanged. Absent from the map means "no known optionals" — the
/// overwhelmingly common case, and every existing name's default, so
/// ordinary application is unaffected.
#[derive(Clone, Debug)]
pub struct Scope {
    names: HashSet<String>,
    optional_shape: std::collections::HashMap<String, Vec<bool>>,
    /// A module member's bare (sibling-visible) local name → the ACTUAL Ast
    /// key its value is bound under (module-completion bug fix, see
    /// `push_named_binding`'s doc comment): a member of `module M = struct
    /// .. end` is bound under a MANGLED key (`"$M.atan2"`, never a valid
    /// surface identifier, so it can never collide with anything), not a
    /// bare `"atan2"` `LetIn` — a SIBLING member's body that references it
    /// bare (as literally written, `Ast::Var("atan2")`) must be redirected
    /// to that mangled key at construction time (deliberately NOT the
    /// qualified `"M.atan2"` key — see `push_named_binding`'s doc comment
    /// for why the distinction matters for opaque-type sealing) — this map
    /// is that redirect, consulted by [`scoped_var`] and the inline/block/
    /// math command-key resolution sites. Entries are added ONLY while
    /// processing a module's own `struct` body (`running`, local to that
    /// recursive `walk_bindings` call) and are never propagated to the
    /// caller's outer scope (only `names`/`optional_arity` entries for the
    /// EXPORTED qualified keys are copied back) — so a rename can only ever
    /// affect references
    /// written textually inside that same module, never anything outside
    /// it or after its `end`.
    renames: std::collections::HashMap<String, String>,
    /// The source-language version this scope elaborates under — gates the
    /// SATySFi 0.1-only labeled-optional nodes (`Expr::FunRows`,
    /// `AppArg::Bundled`) so a 0.0.6-compiled file that happens to parse them
    /// (the additive-`cst` accept-surface widening) is rejected with a
    /// version error rather than silently accepted. `V0_0_6` by default so
    /// every existing caller (and the frozen 0.0.6 path) is unaffected.
    version: RustyfiVersion,
}

impl Default for Scope {
    fn default() -> Scope {
        Scope::new(std::iter::empty())
    }
}

impl Scope {
    pub fn new(names: impl IntoIterator<Item = String>) -> Scope {
        Scope::new_with_version(names, RustyfiVersion::V0_0_6)
    }

    /// Like [`Scope::new`] but elaborating under an explicit source version —
    /// the V0_1 compile path (`lib.rs`) uses this so the 0.1 labeled-optional
    /// nodes are accepted.
    pub fn new_with_version(
        names: impl IntoIterator<Item = String>,
        version: RustyfiVersion,
    ) -> Scope {
        Scope {
            names: names.into_iter().collect(),
            optional_shape: std::collections::HashMap::new(),
            renames: std::collections::HashMap::new(),
            version,
        }
    }

    fn with(&self, name: &str) -> Scope {
        let mut s = self.clone();
        s.insert(name);
        s
    }

    /// In-place version of [`Scope::with`], for the folds below that thread
    /// one evolving scope through a sequence of bindings without cloning at
    /// every step. Rebinding a name plainly (no known arity) clears any
    /// stale [`Scope::optional_arity`]/[`Scope::rename`] entry, so a local
    /// parameter/pattern binding can never inherit an outer optional-
    /// leading function's arity — or an outer module member's qualified
    /// redirect — just by sharing its name.
    fn insert(&mut self, name: &str) {
        self.names.insert(name.to_string());
        self.optional_shape.remove(name);
        self.renames.remove(name);
    }

    /// Like [`Scope::insert`], but also records `name`'s full per-position
    /// optional-parameter shape (see the struct doc comment) — used only at
    /// the handful of binding sites that know a def-site `Param` list
    /// (`walk_bindings`'s `TopBinding::Let`/`LetInline`/`LetBlock`/`LetMath`
    /// arms, `Expr::LetIn`/`Expr::LetMathIn`).
    fn insert_with_shape(&mut self, name: &str, shape: Vec<bool>) {
        self.names.insert(name.to_string());
        if shape.iter().any(|&opt| opt) {
            self.optional_shape.insert(name.to_string(), shape);
        } else {
            self.optional_shape.remove(name);
        }
        self.renames.remove(name);
    }

    /// Record that a bare reference to `local` (a module member's own
    /// sibling-visible name) must actually resolve to the Ast key
    /// `actual_key` (its qualified binding — see [`Scope`]'s `renames`
    /// field doc comment and `push_named_binding`). `local` stays `true`
    /// under [`Scope::contains`] (unaffected by this call) — only WHICH KEY
    /// [`Scope::resolve`] returns for it changes.
    fn rename(&mut self, local: &str, actual_key: &str) {
        self.renames.insert(local.to_string(), actual_key.to_string());
    }

    /// The Ast key a bare reference to `name` should actually use: its
    /// [`Scope::rename`] redirect, if one is active, else `name` itself
    /// unchanged (the overwhelmingly common case — every name outside a
    /// module's own body, and every module member before this fix existed).
    fn resolve(&self, name: &str) -> String {
        self.renames.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    /// `name`'s recorded leading-optional-parameter count (the maximal
    /// prefix of `true`s in its [`Scope::optional_shape`] entry), or `0` if
    /// none is known — the command-argument paths (`cmd_args`, `math_bot`'s
    /// `Cmd` arm) only ever auto-omit a *leading* run, so this derived
    /// accessor keeps them byte-identical to before the full-shape widening.
    fn optional_arity(&self, name: &str) -> usize {
        self.optional_shape
            .get(name)
            .map(|shape| shape.iter().take_while(|&&opt| opt).count())
            .unwrap_or(0)
    }

    /// `name`'s recorded full per-position optional-parameter shape (see the
    /// struct doc comment), or `&[]` if none is known — used by
    /// `app_chain_generic`'s marker-less-optional-defaulting, which (unlike
    /// [`Scope::optional_arity`]) must see optionals anywhere in the param
    /// list, not just a leading run.
    fn optional_shape(&self, name: &str) -> &[bool] {
        self.optional_shape.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Every currently-known name starting with `prefix` (used by `open`,
    /// which brings a module's `"M."`-prefixed names into unqualified
    /// scope). Sorted for deterministic alias-binding order.
    fn names_with_prefix(&self, prefix: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .names
            .iter()
            .filter(|n| n.starts_with(prefix))
            .cloned()
            .collect();
        out.sort();
        out
    }
}

/// A `Var` node for a name that must already be in scope (primitive
/// operators and the internal `%context`/`read-inline`/`read-block` wiring
/// are all resolved the same way as user variables). Existence is checked
/// against the BARE `name` (unaffected by any active [`Scope::rename`]
/// redirect); the constructed node's own key goes through
/// [`Scope::resolve`], so a module member's sibling reference compiles
/// directly to that member's qualified key when one is active (module-
/// completion bug fix — see `push_named_binding`'s doc comment).
fn scoped_var(name: &str, span: Span, scope: &Scope) -> Result<Ast, ElabError> {
    if scope.contains(name) {
        Ok(Ast::Var(scope.resolve(name), span))
    } else {
        err(span, format!("unbound variable '{name}'"))
    }
}

/// A user type declaration, surfaced (but not yet lowered into
/// [`crate::types::MonoType`] — that's `typecheck::build_variant_decl`'s job)
/// from a CST [`cst::TypeDecl`]. Ctor payload types are kept as raw CST
/// `TypeExpr`s: cheap to clone, and this untyped elaborator has no use for
/// them beyond passing them through to the typechecker.
#[derive(Clone, Debug)]
pub struct UserTypeDecl {
    pub name: String,
    /// Type-parameter names, in declaration order (e.g. `["a"]` for `'a`).
    pub params: Vec<String>,
    /// `(ctor name, payload type expr)`, in declaration order.
    pub ctors: Vec<(String, Option<c::TypeExpr>)>,
}

/// A user type *synonym* declaration (`type point = length * length`),
/// surfaced in parallel with [`UserTypeDecl`] — see that struct's doc
/// comment; the body is kept as a raw CST `TypeExpr` for the same reason.
/// `typecheck::build_synonym_decl` is where it is actually lowered to a
/// `MonoType` template, and `typecheck::expand_synonyms` is where a
/// reference to `name` elsewhere is transparently replaced by it.
#[derive(Clone, Debug)]
pub struct UserSynonymDecl {
    pub name: String,
    /// Type-parameter names, in declaration order. Only the zero-param case
    /// is reachable through a *use* of the synonym today — see
    /// `cst::ast::TypeAtom`'s doc comment (no applied-type-constructor
    /// syntax exists to instantiate one) — but parsing/storing params keeps
    /// this declaration-side path symmetric with `UserTypeDecl`.
    pub params: Vec<String>,
    pub body: c::TypeExpr,
}

/// One lowered `type` declaration: either shape [`lower_type_decl`] can
/// produce, for `walk_bindings` to sort into `Program`'s two decl lists.
enum LoweredTypeDecl {
    Variant(UserTypeDecl),
    Synonym(UserSynonymDecl),
}

fn lower_type_decl(decl: &cst::TypeDecl) -> LoweredTypeDecl {
    let params: Vec<String> = decl.tyvars.iter().map(|v| v.name.clone()).collect();
    match &decl.body {
        cst::TypeDeclBody::Variant { first, rest, .. } => {
            let mut ctors = Vec::with_capacity(1 + rest.len());
            ctors.push((
                first.ctor.name.clone(),
                first.of_ty.as_ref().map(|o| o.ty.clone()),
            ));
            for bv in rest {
                ctors.push((
                    bv.def.ctor.name.clone(),
                    bv.def.of_ty.as_ref().map(|o| o.ty.clone()),
                ));
            }
            LoweredTypeDecl::Variant(UserTypeDecl {
                name: decl.name.name.clone(),
                params,
                ctors,
            })
        }
        cst::TypeDeclBody::Synonym(ty) => LoweredTypeDecl::Synonym(UserSynonymDecl {
            name: decl.name.name.clone(),
            params,
            body: ty.clone(),
        }),
    }
}

/// The result of elaborating a whole file: every `type` declaration it
/// surfaced (in source order — a later declaration may reference an earlier
/// one, or itself, since variant types are nominal; see
/// `typecheck::build_variant_decl`), every type *synonym* it surfaced (see
/// `typecheck::build_synonym_decl`), plus the elaborated document body.
#[derive(Clone, Debug)]
pub struct Program {
    pub type_decls: Vec<UserTypeDecl>,
    pub synonym_decls: Vec<UserSynonymDecl>,
    pub body: Ast,
}

/// Elaborate a whole file into a [`Program`] (the elaborated body plus any
/// surfaced `type` declarations — see [`elaborate`] for the thin wrapper
/// existing callers that only want the body keep using).
///
/// **Library files.** `File.body` is `None` for a bare `prelude EOI` file (a
/// `.satyh` library with no document expression) — a separate loader crate
/// is responsible for merging a library's `prelude` into a document file's
/// before this function ever sees it, so unlike phase-1/2a there is no
/// "top-level bindings must be followed by `in`" check here at all: by the
/// time `elaborate_program` runs, either `body` is present (an ordinary
/// document, or an already-merged file) or it is a genuine library file,
/// which is a (clean) error to hand to `elaborate_program` directly.
pub fn elaborate_program(file: &cst::File, prelude_scope: &Scope) -> Result<Program, ElabError> {
    elaborate_program_with_versions(file, prelude_scope, &HashSet::new(), None)
}

/// Like [`elaborate_program`], but additionally marking a subset of
/// `file.prelude`'s TOP-LEVEL entries (by index) as originating from a
/// spliced `V0_0_6` dependency (Slice X2a,
/// `docs/plans/design-cross-version-import.md` §"Slice X2 — per-group
/// primitive environment", Option C). Every `Binding` this splices in from
/// one of those entries — and, recursively, every binding inside a `module ..
/// = struct .. end` at such an index — has its elaborated RHS wrapped in
/// [`Ast::VersionScope`]`(V0_0_6, _)`, so `compile.rs`/`eval.rs`/
/// `typecheck.rs` resolve THAT subtree's version-forked primitives (and
/// runtime-version reads) against `V0_0_6` instead of the merged program's
/// ambient `V0_1`.
///
/// `v006_indices` is empty for every existing caller (`elaborate_program`
/// above delegates here with `&HashSet::new()`), which makes `this_v006`
/// (below) `false` for every item and so never constructs a `VersionScope`
/// node — the pure-0.0.6 and pure-0.1 paths are therefore byte-identical to
/// before this slice (same code, an always-false extra branch).
///
/// `wrap_body_version` (Slice X4a, `docs/plans/design-cross-version-import.md`
/// §X4.3 item 3): when `Some(v)`, the file's own document TAIL expression
/// (`file.body`, elaborated into `body_ast` below) is additionally wrapped in
/// `Ast::VersionScope(v, _)` — the ONE new capability X4 needs beyond X2.2's
/// indexed-`prelude`-item wrapping, because a `V0_0_6` ENTRY's tail expression
/// (e.g. a bare `page-break doc` with no intermediate `let`) is itself
/// `V0_0_6`-authored code that may reference forked primitives directly, not
/// just its `prelude` bindings. `None` for every pre-X4a caller
/// (`elaborate_program` above, and `compile_document_v1_with_trials`'s
/// existing call site) — byte-identical, since `match None { .. }` never
/// constructs the extra node.
pub fn elaborate_program_with_versions(
    file: &cst::File,
    prelude_scope: &Scope,
    v006_indices: &HashSet<usize>,
    wrap_body_version: Option<RustyfiVersion>,
) -> Result<Program, ElabError> {
    let Some(body) = &file.body else {
        return err(
            Span::default(),
            "this file has no document expression - it is a library file",
        );
    };
    let items: Vec<&cst::TopBinding> = file.prelude.iter().collect();
    let mut type_decls = Vec::new();
    let mut synonym_decls = Vec::new();
    let (bindings, _exported, final_scope) = walk_bindings(
        &items,
        prelude_scope,
        &[],
        &mut type_decls,
        &mut synonym_decls,
        v006_indices,
    )?;
    // `final_scope` (mod_path `[]`) already IS `prelude_scope` plus every
    // top-level name — including each one's `Scope::optional_arity` entry,
    // which a manual `insert` per `exported` name would have dropped (see
    // `Scope`'s doc comment) — so the file body sees the same
    // marker-less-optional-call defaulting a top-level function's own
    // sibling declarations do.
    let body_ast = expr(body, &final_scope)?;
    let body_ast = match wrap_body_version {
        Some(v) => Ast::VersionScope(v, Box::new(body_ast)),
        None => body_ast,
    };
    Ok(Program {
        type_decls,
        synonym_decls,
        body: nest(bindings, body_ast),
    })
}

/// Wrap `value` in [`Ast::VersionScope`]`(V0_0_6, _)` iff `this_v006` — the
/// one-line helper every `walk_bindings` binding-construction arm below
/// calls right after building its (fully elaborated) RHS. See
/// [`elaborate_program_with_versions`]'s doc comment.
fn maybe_v006_scope(value: Ast, this_v006: bool) -> Ast {
    if this_v006 {
        Ast::VersionScope(RustyfiVersion::V0_0_6, Box::new(value))
    } else {
        value
    }
}

/// Elaborate a whole file into one expression, discarding any `type`
/// declarations it surfaces (existing callers that only need the untyped
/// `Ast`; see [`elaborate_program`] for the version phase 3's typechecker
/// uses).
pub fn elaborate(file: &cst::File, prelude_scope: &Scope) -> Result<Ast, ElabError> {
    Ok(elaborate_program(file, prelude_scope)?.body)
}

// ---- module name-mangling & the top-level/struct-decl fold ---------------

/// The (untyped) module name-mangling scheme: a qualified name's runtime/
/// scope key is simply `mods.join(".") + "." + local`, where `local` is
/// whatever bare key the unqualified form would have used — a plain
/// variable's own name (`"x"` → `"M.x"`), or a command's sigil-inclusive
/// name (`"\cmd"` → `"M.\cmd"`, *not* the surface-syntax `"\M.cmd"` spelling
/// `Token::HorzCmdWithMod`'s `Display` impl renders — this port's `Scope`
/// and `Env` are both flat string-keyed maps with no separate namespace for
/// commands vs. variables, so one uniform "prefix-join" scheme for every
/// kind of name is simplest, and nothing round-trips through source syntax
/// again once elaborated). Nested modules mangle recursively by construction
/// — `mod_path` is the *full* accumulated path (`["M", "N"]`) at the point a
/// name is bound, never re-qualified after the fact, so `module N = struct
/// let x = .. end` inside `module M = struct .. end` yields key `"M.N.x"`
/// directly.
fn qualify_key(mod_path: &[String], local: &str) -> String {
    if mod_path.is_empty() {
        local.to_string()
    } else {
        format!("{}.{}", mod_path.join("."), local)
    }
}

/// If `item` is a `direct \cmd : ty` / `direct +cmd : ty` signature item
/// (`cst::SigItem::DirectHorzCmd`/`DirectVertCmd` — math commands share the
/// `\` sigil with inline ones, see `command_scheme`'s doc comment in
/// `typecheck.rs`, so there is no separate math case here), its bare command
/// name (sigil included — `"\cmd"`/`"+cmd"`, the same key format
/// `push_named_binding` binds locally) and the name token's span, for
/// `typechecker-completion.md` §4's enclosing-scope exposure. `None` for
/// every other `SigItem` (`val`/`type`), which stay module-qualified only.
fn direct_cmd_name(item: &cst::SigItem) -> Option<(String, Span)> {
    match item {
        cst::SigItem::DirectHorzCmd { name, .. } => Some((name.name.clone(), name.span)),
        cst::SigItem::DirectVertCmd { name, .. } => Some((name.name.clone(), name.span)),
        _ => None,
    }
}

/// One step of the top-level/struct-decl fold, deferred (see [`nest`]) so
/// that folding in a `module`'s declarations doesn't require building the
/// "rest of the program" before the module's own bindings are known.
enum Binding {
    Let(String, Ast),
    LetRec(Vec<(String, Rc<Ast>)>),
    LetMutable(String, Ast),
    /// A `let-math` binding (`docs/plans/math-engine.md` §G) — nests as
    /// `Ast::LetMathIn`, not `Ast::LetIn` (see that variant's doc comment).
    LetMath(String, Ast),
}

/// Wrap `tail` in every collected `Binding`, innermost (last-pushed) first —
/// i.e. in the same order `elaborate_prelude`/`elaborate_struct_decls` used
/// to build `Ast::LetIn`/`Ast::LetRecIn` directly, just deferred into data
/// first so a `module`'s bindings can be spliced into the flat sequence
/// before any of it is turned into `Ast`.
fn nest(bindings: Vec<Binding>, tail: Ast) -> Ast {
    let mut ast = tail;
    for b in bindings.into_iter().rev() {
        ast = match b {
            Binding::Let(name, val) => Ast::LetIn(name, Box::new(val), Box::new(ast)),
            Binding::LetRec(bs) => Ast::LetRecIn(bs, Box::new(ast)),
            Binding::LetMutable(name, val) => Ast::LetMutableIn(name, Box::new(val), Box::new(ast)),
            Binding::LetMath(name, val) => Ast::LetMathIn(name, Box::new(val), Box::new(ast)),
        };
    }
    ast
}

/// After binding `local` (inside a `module M = struct .. end`, i.e.
/// `mod_path` non-empty), also bind the qualified alias `M.local` — an
/// `Ast::Var`-referencing `LetIn`, the same alias-binding technique `open`
/// uses below — so later qualified references (and any enclosing `open`)
/// can resolve it. `local` itself is added to `running` (so *sibling*
/// declarations still inside the same `struct .. end` see it unqualified)
/// but never to `exported`: per v0.0.6 semantics, after `end` only the
/// qualified name is visible to what follows.
///
/// Only remaining caller: `TopBinding::LetRec`'s per-name loop, where
/// `local` is ALREADY bound (bare, by construction — a `let rec .. and ..`
/// group's own mutual-recursion scope needs every clause visible to every
/// other one by its bare name, which `rec_bindings` sets up before this
/// runs), so there is no single value this function could re-bind under a
/// mangled key the way `push_named_binding`'s fix does — the bare name
/// genuinely stays physically present in the flat `nest()` chain, and so
/// (pre-existing behavior, not a new regression) can still leak past this
/// module's `end` if a `let rec` binding's bare name happens to collide
/// with something reached later in program order. Narrower in practice
/// than the bug `push_named_binding` fixes (a top-level `let rec .. and
/// ..` group directly inside a `module .. = struct .. end`, rather than
/// any of the far more common plain `val`/`let-inline`/`let-block`/
/// `let-math`/`let mutable` members) — left as a known, separable gap.
fn export_alias(
    mod_path: &[String],
    local: String,
    shape: Vec<bool>,
    bindings: &mut Vec<Binding>,
    running: &mut Scope,
    exported: &mut Vec<String>,
) {
    running.insert_with_shape(&local, shape.clone());
    if mod_path.is_empty() {
        exported.push(local);
    } else {
        let qual = qualify_key(mod_path, &local);
        bindings.push(Binding::Let(qual.clone(), Ast::Var(local, Span::default())));
        running.insert_with_shape(&qual, shape);
        exported.push(qual);
    }
}

/// `shape` is `local`'s recorded per-position optional-parameter shape (see
/// [`Scope`]'s doc comment) — non-empty for `TopBinding::Let` and for
/// `TopBinding::LetInline`/`LetBlock`/`LetMath` (all four have a `Param`
/// list that can carry the marker — `param_optional_shape`); every other
/// binding kind here passes an empty `Vec`.
///
/// Module-member bug fix: for `mod_path` non-empty (inside a `module M =
/// struct .. end`), `value` is bound under a MANGLED key
/// (`"$M.local"` — `$` can never appear in a surface identifier/command
/// name, so this can never collide with anything user-written), not the
/// bare `"local"` name the old two-step (`export_alias`, still used by
/// `TopBinding::LetRec` below) used. The bare `LetIn` used to stay
/// PHYSICALLY PRESENT in the single flat `nest()` chain this whole program
/// compiles to; since that chain has no scope-popping of its own, the bare
/// name stayed bound (and so silently SHADOWED any unrelated same-named
/// binding — a base primitive, or a different package's own member —
/// reached later in program order) for literally the rest of the merged
/// program, not just until this module's `end`. [`Scope::rename`]
/// redirects a SIBLING member's bare reference (still written `local`, as
/// in the source) to the mangled key instead — consulted by [`scoped_var`]
/// and the inline/block/math command-key resolution sites in
/// `inline_elems`/`block_elems`/`math_bot`.
///
/// The qualified alias (`"M.local"` — unchanged in spirit from the old
/// two-step, just now pointing at the mangled key instead of a bare one)
/// is still bound separately, and is load-bearing beyond mere qualified
/// lookup: `v1/module_check.rs`'s sealing pass keys its opaque/stamped
/// type rewrite on this EXACT qualified string (`static_env.seals`), and
/// applies it ONLY to a binding whose OWN key matches — a mangled key
/// never matches, so a member's OWN body (and any SIBLING that reaches it
/// via the mangled-key redirect) keeps seeing its naturally-inferred,
/// TRANSPARENT type, exactly as a bare reference always did; only an
/// EXPLICIT qualified reference (`M.local`, from outside, or written
/// as such from inside) sees the sig's opaque view. Losing this
/// distinction (e.g. by binding the real value directly under the
/// qualified key, or by redirecting sibling references to the qualified
/// key instead of the mangled one) breaks sealing for any member whose
/// body uses ANOTHER sealed sibling's value at a type the sig makes
/// opaque (`v01_sealing.rs`'s `u1_opaque_accept`/`u8_command_decls`/
/// `u9_ctor_hiding`/`t13_escaped_skolem_message` all pin this).
///
/// The redirect and the mangled key both live only in `running`, this
/// recursive `walk_bindings` call's OWN local scope — neither is ever
/// copied back to the caller (only the qualified name's existence/shape
/// is, via `inner_running.optional_shape`), so they can never affect
/// anything outside this module or after its `end`.
fn push_named_binding(
    mod_path: &[String],
    local: String,
    value: Ast,
    shape: Vec<bool>,
    make_binding: impl FnOnce(String, Ast) -> Binding,
    bindings: &mut Vec<Binding>,
    running: &mut Scope,
    exported: &mut Vec<String>,
) {
    if mod_path.is_empty() {
        bindings.push(make_binding(local.clone(), value));
        running.insert_with_shape(&local, shape);
        exported.push(local);
    } else {
        let qual = qualify_key(mod_path, &local);
        let mangled = format!("${qual}");
        bindings.push(make_binding(mangled.clone(), value));
        bindings.push(Binding::Let(qual.clone(), Ast::Var(mangled.clone(), Span::default())));
        running.insert_with_shape(&local, shape.clone());
        running.rename(&local, &mangled);
        running.insert_with_shape(&qual, shape);
        exported.push(qual);
    }
}

/// The per-position optional shape of a `Param` list (see [`Scope`]'s doc
/// comment): one `bool` per parameter, `true` exactly where it's a
/// `Param::Optional`, in declared order — e.g. `stdja.satyh`'s `document
/// record ?:configopt inner` (the optional is the *second*, not the first,
/// parameter) yields `[false, true, false]`. Shared by a plain `let`'s params
/// and a command binding's (`let-inline`/`let-block`/`let-math` —
/// `walk_bindings`'s `LetInline`/`LetBlock`/`LetMath` arms, `Expr::
/// LetMathIn`): both use the same `cst::ast::Param` list now (`cst.rs`'s
/// `Param` doc comment). Recorded into [`Scope::optional_shape`] so a
/// marker-less call can auto-omit any optional slot it reaches, not just a
/// leading one (upstream `typecheck_command_arguments` skips optional slots
/// left unmarked wherever they fall in the parameter list).
fn param_optional_shape(params: &[c::Param]) -> Vec<bool> {
    params
        .iter()
        .map(|p| matches!(p, c::Param::Optional { .. }))
        .collect()
}

/// The optional-parameter shape a *parameter-less alias* binding inherits from
/// its right-hand side. A `let x = y` (or `let x = M.y`) with no parameters of
/// its own is a plain value alias: `x` should carry exactly `y`'s declared
/// optional shape so a marker-less call `x a b` auto-omits `y`'s optionals the
/// same way `y a b` would (`app_chain_generic`). The motivating case is
/// `stdja.satyh`'s top-level `let document = StdJa.document`, re-exporting the
/// module member `document record ?:configopt inner` (shape `[false, true,
/// false]`) under the bare name every real document calls — `document rec
/// '<..>'`, omitting the optional `configopt`; without this the alias records
/// an empty shape and the block-text mis-binds against `configopt`'s domain.
/// Returns `&[]`-equivalent for any RHS that is not a bare (module-qualified)
/// variable reference — a value alias only, never a partial application.
fn alias_optional_shape(value: &c::Expr, scope: &Scope) -> Vec<bool> {
    let c::Expr::Ops(chain) = value else {
        return Vec::new();
    };
    if !chain.tail.is_empty() || chain.before.is_some() {
        return Vec::new();
    }
    let a = &chain.head;
    if a.minus.is_some() || a.excl.is_some() || !a.head_accesses.is_empty() || !a.args.is_empty() {
        return Vec::new();
    }
    head_optional_shape(&a.head, scope).to_vec()
}

/// Fold one sequence of top-level-shaped bindings — the file's own prelude
/// when `mod_path` is empty, or one `module .. = struct .. end` body's decls
/// when it isn't (`nxtoplevel`/`nxstruct` share every alternative but
/// `Module`/`Open` themselves, see `cst.rs`'s doc comment on `StructDecl`) —
/// into an ordered list of [`Binding`]s to [`nest`] around whatever follows,
/// plus the names that become visible to whatever follows *outside* this
/// sequence (identical to every name `running` picked up when `mod_path` is
/// empty; only the qualified aliases when it isn't — see [`export_alias`]).
/// `Module` recurses into this same function with an extended `mod_path`;
/// its returned bindings are spliced directly into the flat list (so its own
/// `Ast::LetIn`s end up nested at exactly the point the `module .. end`
/// appeared, textually), and its exported qualified names are folded into
/// `running` (visible to later siblings) and `exported` (bubbled up to
/// whatever this whole call's caller is folding, so `module N = ..` nested
/// inside `module M = ..` bubbles `"M.N.x"` all the way out to the file
/// level) — along with each bubbled name's [`Scope::optional_arity`] (read
/// back off the recursive call's own final `running`, its third return
/// value), so a qualified call to a leading-`?:`-optional function from
/// *outside* its defining module/`let .. in` still auto-omits, not just an
/// unqualified call from a sibling declaration inside the same one.
///
/// The final `running` (this call's own accumulated scope) is returned as a
/// third element precisely so callers can reuse it wholesale as the scope
/// for whatever comes *after* this sequence, instead of rebuilding one from
/// `exported` name strings alone (which would drop every
/// [`Scope::optional_arity`] entry — see [`elaborate_program`]).
fn walk_bindings(
    items: &[&cst::TopBinding],
    scope: &Scope,
    mod_path: &[String],
    type_decls: &mut Vec<UserTypeDecl>,
    synonym_decls: &mut Vec<UserSynonymDecl>,
    v006_indices: &HashSet<usize>,
) -> Result<(Vec<Binding>, Vec<String>, Scope), ElabError> {
    let mut bindings: Vec<Binding> = Vec::new();
    let mut running = scope.clone();
    let mut exported: Vec<String> = Vec::new();
    for (item_idx, top) in items.iter().enumerate() {
        // Slice X2a: is THIS top-level item (and everything nested inside
        // it, e.g. a `module .. = struct .. end`'s own decls — see the
        // `Module` arm below) part of a spliced V0_0_6 dependency? Always
        // `false` for `elaborate_program`'s empty `v006_indices` (the
        // pure-0.0.6 / pure-0.1 paths), so this is a dead branch there.
        let this_v006 = v006_indices.contains(&item_idx);
        match top {
            cst::TopBinding::Let(top_let) => {
                // Same curry-with-patterns desugaring as a `let-rec` clause
                // (`rec_clause_value`), just with no multi-clause `extra` —
                // top-level non-recursive `let` has no `|`-clause sugar.
                // Its all-var-param fast path reproduces the old direct
                // `Lambda`-chain behavior exactly; the general path handles
                // `gr.satyh`-style tuple-destructuring params.
                let top_let_params = params_to_patbots(&top_let.params);
                let value =
                    rec_clause_value(&top_let_params, &top_let.value, &[], &running)?;
                let value = maybe_v006_scope(value, this_v006);
                // A parameter-less binding may be a plain value alias
                // (`let document = StdJa.document`) — inherit the aliased
                // name's optional shape so a marker-less call auto-omits its
                // optionals (see `alias_optional_shape`).
                let mut shape = param_optional_shape(&top_let.params);
                if shape.is_empty() && top_let.params.is_empty() {
                    shape = alias_optional_shape(&top_let.value, &running);
                }
                push_named_binding(
                    mod_path,
                    top_let.name.name.clone(),
                    value,
                    shape,
                    Binding::Let,
                    &mut bindings,
                    &mut running,
                    &mut exported,
                );
            }
            cst::TopBinding::LetRec { first, ands, .. } => {
                let (recs, rec_scope) = rec_bindings(first, ands, &running)?;
                running = rec_scope;
                let names: Vec<String> = recs.iter().map(|(n, _)| n.clone()).collect();
                // Slice X2a: RHS granularity — wrap EACH recursive clause's
                // own body individually (not the `LetRecIn` node as a
                // whole), matching `elaborate_program_with_versions`'s doc
                // comment.
                let recs = if this_v006 {
                    recs.into_iter()
                        .map(|(n, body)| {
                            (
                                n,
                                Rc::new(Ast::VersionScope(
                                    RustyfiVersion::V0_0_6,
                                    Box::new((*body).clone()),
                                )),
                            )
                        })
                        .collect()
                } else {
                    recs
                };
                bindings.push(Binding::LetRec(recs));
                for n in names {
                    export_alias(
                        mod_path,
                        n,
                        Vec::new(),
                        &mut bindings,
                        &mut running,
                        &mut exported,
                    );
                }
            }
            cst::TopBinding::LetInline {
                ctx, cmd, params, value, ..
            } => {
                let value_ast =
                    elaborate_let_inline(ctx.as_ref(), params, value, &running, "read-inline")?;
                let value_ast = maybe_v006_scope(value_ast, this_v006);
                push_named_binding(
                    mod_path,
                    cmd.name.clone(),
                    value_ast,
                    param_optional_shape(params),
                    Binding::Let,
                    &mut bindings,
                    &mut running,
                    &mut exported,
                );
            }
            cst::TopBinding::LetBlock {
                ctx, cmd, params, value, ..
            } => {
                let value_ast =
                    elaborate_let_inline(ctx.as_ref(), params, value, &running, "read-block")?;
                let value_ast = maybe_v006_scope(value_ast, this_v006);
                push_named_binding(
                    mod_path,
                    cmd.name.clone(),
                    value_ast,
                    param_optional_shape(params),
                    Binding::Let,
                    &mut bindings,
                    &mut running,
                    &mut exported,
                );
            }
            cst::TopBinding::LetMath {
                cmd, params, value, ..
            } => {
                let value_ast = elaborate_let_math(params, value, &running)?;
                let value_ast = maybe_v006_scope(value_ast, this_v006);
                push_named_binding(
                    mod_path,
                    cmd.name.clone(),
                    value_ast,
                    param_optional_shape(params),
                    Binding::LetMath,
                    &mut bindings,
                    &mut running,
                    &mut exported,
                );
            }
            // `type` declarations have no runtime effect in this untyped
            // elaborator: constructors are bare `Ctor` atoms and are never
            // scope-checked, and a synonym is never itself a runtime value,
            // so no scope entry (qualified or not) is needed for either
            // shape. They are still surfaced (unqualified — variant types
            // are nominal by name only, see `UserTypeDecl`; synonyms the
            // same way, see `UserSynonymDecl`) for the typechecker.
            cst::TopBinding::Type(decl) => match lower_type_decl(decl) {
                LoweredTypeDecl::Variant(v) => type_decls.push(v),
                LoweredTypeDecl::Synonym(s) => synonym_decls.push(s),
            },
            cst::TopBinding::LetMutable { name, value, .. } => {
                let value_ast = expr(value, &running)?;
                let value_ast = maybe_v006_scope(value_ast, this_v006);
                push_named_binding(
                    mod_path,
                    name.name.clone(),
                    value_ast,
                    Vec::new(),
                    Binding::LetMutable,
                    &mut bindings,
                    &mut running,
                    &mut exported,
                );
            }
            cst::TopBinding::Module { name, sig, decls, .. } => {
                // Signature annotations (`sig .. end`) are otherwise
                // accepted and ignored: this elaborator does no type
                // checking, so `val`/`type` items have nothing yet to check
                // against (full reconciliation is
                // `typechecker-completion.md` §3, deferred). `direct` items
                // (§4) ARE handled here, though: each exposes its command
                // UNQUALIFIED at the enclosing scope, aliasing the module's
                // own qualified binding — the same `Ast::Var`-alias trick
                // `export_alias`/`Open` already use below, which
                // `typecheck.rs`'s `command_scheme` already threads command
                // types through transparently (an alias site is
                // indistinguishable from `open`'s), so no typecheck.rs
                // change is needed to give the exposed name its command
                // type.
                let mut child_path = mod_path.to_vec();
                child_path.push(name.name.clone());
                let inner_items: Vec<&cst::TopBinding> =
                    decls.iter().map(|d| d.0.as_ref()).collect();
                // Slice X2a: a nested `module .. = struct .. end` has no
                // index correspondence to the OUTER `v006_indices` (that set
                // indexes THIS level's `items`, not `inner_items`) — if the
                // enclosing item is itself v006-marked, every inner item is
                // too (the whole subtree came from the same spliced 0.0.6
                // file); otherwise none are.
                let inner_v006: HashSet<usize> = if this_v006 {
                    (0..inner_items.len()).collect()
                } else {
                    HashSet::new()
                };
                let (inner_bindings, inner_exported, inner_running) = walk_bindings(
                    &inner_items,
                    &running,
                    &child_path,
                    type_decls,
                    synonym_decls,
                    &inner_v006,
                )?;
                // A module's own bare (unqualified) member names never leak
                // past this `end`: `push_named_binding` (used by every
                // ordinary member below) now binds each member DIRECTLY
                // under its qualified key and registers a `Scope::rename`
                // redirect for sibling lookups, instead of the old
                // bare-then-qualified-alias two-step — see that function's
                // doc comment for the fix and why `nest()`'s single flat
                // `LetIn` chain made naive scope-popping unsafe (it silently
                // broke `v1/module_check.rs`'s spine-walking sealing pass,
                // which only recognizes TOP-LEVEL `LetIn`/`LetMathIn`/
                // `LetRecIn`/`LetMutableIn` nodes, not ones nested inside a
                // wrapper sub-expression).
                bindings.extend(inner_bindings);
                for q in &inner_exported {
                    running.insert_with_shape(q, inner_running.optional_shape(q).to_vec());
                }
                if let Some(sig_annot) = sig {
                    for item in &sig_annot.items {
                        if let Some((local, span)) = direct_cmd_name(item) {
                            let qual = qualify_key(&child_path, &local);
                            // Cheap positive-obligation check (a `direct`-only
                            // slice of §3's fuller sig-preservation, which
                            // stays deferred for `val`/`type` items): the
                            // struct must actually define what it declares
                            // `direct`, or the alias below would dangle.
                            if !inner_exported.contains(&qual) {
                                return err(
                                    span,
                                    format!(
                                        "module `{}` signature declares `direct {local} : ..` \
                                         but its `struct .. end` body never defines `{local}`",
                                        name.name
                                    ),
                                );
                            }
                            let shape = running.optional_shape(&qual).to_vec();
                            bindings.push(Binding::Let(
                                local.clone(),
                                Ast::Var(qual, Span::default()),
                            ));
                            running.insert_with_shape(&local, shape);
                            exported.push(local);
                        }
                    }
                }
                exported.extend(inner_exported);
            }
            cst::TopBinding::Open { name, .. } => {
                let prefix = format!("{}.", name.name);
                for q in running.names_with_prefix(&prefix) {
                    let suffix = q[prefix.len()..].to_string();
                    let shape = running.optional_shape(&q).to_vec();
                    bindings.push(Binding::Let(suffix.clone(), Ast::Var(q, Span::default())));
                    running.insert_with_shape(&suffix, shape);
                    // `open` only re-exposes an *existing* qualified name
                    // under its bare suffix locally; it doesn't itself mint
                    // a new qualified name, so nothing goes into `exported`
                    // here.
                }
            }
        }
    }
    Ok((bindings, exported, running))
}

/// Curry a command binding's (`let-inline`/`let-block`/`let-math`) `Param`
/// list — already widened to `PatBot` by `params_to_patbots` — around a
/// value built from the fully-extended scope, mirroring `rec_clause_value`'s
/// two-path shape but per-param rather than clause-tuple (upstream's
/// `curry_lambda_abstract` builds one `UTFunction` per `cmdarglst` element,
/// `parser.mly:50-63`, unlike `let-rec`'s single tupled match — an
/// intentional divergence from `rec_clause_value`'s own arity-N tuple-match
/// shape, kept here because that's what upstream itself does for command
/// arguments).
///
/// The all-variable-parameter fast path (every existing binding in the
/// bundled packages) reproduces the plain `Lambda` chain byte-for-byte: it
/// must, since `elaborate_let_inline`'s "lightweight" (`%context`-wrapping)
/// form builds its `read-inline`/`read-block` application *inside*
/// `build_value`, called with the innermost (fully-extended) scope, so the
/// wrapping ends up nested *inside* every curried parameter exactly like the
/// original direct implementation. The general (genuine-pattern) path lowers
/// each parameter to its own `Lambda(%cmd_argN, Match(%cmd_argN, [pat ->
/// rest]))` — a refutable parameter pattern (e.g. `Some(x)`) can fail to
/// match at *application* time, exactly like a `match` arm would (see
/// `eval.rs`'s `Ast::Match` handling for the resulting runtime error).
fn curry_cmd_params(
    patbots: &[c::PatBot],
    scope: &Scope,
    build_value: impl FnOnce(&Scope) -> Result<Ast, ElabError>,
) -> Result<Ast, ElabError> {
    if patbots.iter().all(is_var_patbot) {
        let mut inner = scope.clone();
        for p in patbots {
            inner = inner.with(patbot_var_name(p));
        }
        let mut value_ast = build_value(&inner)?;
        for p in patbots.iter().rev() {
            value_ast = Ast::Lambda(patbot_var_name(p).to_string(), Rc::new(value_ast));
        }
        return Ok(value_ast);
    }
    let pats: Vec<Pattern> = patbots.iter().map(patbot).collect::<Result<_, _>>()?;
    let mut names = Vec::new();
    for p in &pats {
        collect_pattern_names(p, &mut names);
    }
    let mut inner = scope.clone();
    for n in &names {
        inner = inner.with(n);
    }
    let mut value_ast = build_value(&inner)?;
    let dummy = Span::default();
    for (i, pat) in pats.into_iter().enumerate().rev() {
        let fresh = format!("%cmd_arg{i}");
        value_ast = Ast::Lambda(
            fresh.clone(),
            Rc::new(Ast::Match(
                Box::new(Ast::Var(fresh, dummy)),
                vec![MatchArm {
                    pat,
                    guard: None,
                    body: value_ast,
                }],
            )),
        );
    }
    Ok(value_ast)
}

/// `[ctxvar] let-inline \cmd param* = value` / `[ctxvar] let-block +cmd
/// param* = value` (`nxhorzdec`/`nxvertdec` in `parser.mly`, lines 548-577).
/// Each `param` is upstream's `arg` (`cst.rs`'s `Param` doc comment — a full
/// patbot, a `?:`-marked variable, or — optional-arg-rows increment 3a — a
/// `?(l = x, …)` labeled-optional bundle), curried via the bundle-aware
/// `curry_cmd_params_v1` (which delegates to the original `curry_cmd_params`
/// wholesale when no bundle is present, so this stays byte-identical for
/// every existing binding).
///
/// Two forms, confirmed against v0.0.6 `parser.mly`:
/// * with an explicit leading context variable, the value is elaborated
///   as-is (already inline-boxes/block-boxes typed) under
///   `Lambda(ctxvar, Lambda(p1, .., value))`;
/// * without one (the "lightweight" form), `parser.mly` synthesizes an
///   implicit `%context` variable and wraps the (inline-text/block-text
///   typed) value in a `read-inline`/`read-block` call *inside* the
///   curried parameters but *around* the value itself:
///   `curry_lambda_abstract_pattern params (read-inline %context value)`,
///   all wrapped in `Lambda(%context, ..)`. We reproduce that exactly,
///   using `reader` = `"read-inline"` or `"read-block"`.
fn elaborate_let_inline(
    ctx: Option<&VarTok>,
    params: &[c::Param],
    value: &c::Expr,
    scope: &Scope,
    reader: &str,
) -> Result<Ast, ElabError> {
    match ctx {
        Some(ctxvar) => {
            let ctx_scope = scope.with(&ctxvar.name);
            let value_ast = curry_cmd_params_v1(params, &ctx_scope, |inner| expr(value, inner))?;
            Ok(Ast::Lambda(ctxvar.name.clone(), Rc::new(value_ast)))
        }
        None => {
            const IMPLICIT_CTX: &str = "%context";
            let dummy = Span::default();
            let ctx_scope = scope.with(IMPLICIT_CTX);
            let curried = curry_cmd_params_v1(params, &ctx_scope, |inner| {
                let value_ast = expr(value, inner)?;
                let read_fn = scoped_var(reader, dummy, inner)?;
                let ctx_var = scoped_var(IMPLICIT_CTX, dummy, inner)?;
                Ok(Ast::Apply(
                    Box::new(Ast::Apply(Box::new(read_fn), Box::new(ctx_var))),
                    Box::new(value_ast),
                ))
            })?;
            Ok(Ast::Lambda(IMPLICIT_CTX.to_string(), Rc::new(curried)))
        }
    }
}

/// `let-math \cmd param* = expr` (`docs/plans/math-engine.md` §G; upstream
/// `nxmathdec`, `parser.mly:586-591`): curry `params` (upstream's `arg`,
/// `cst.rs`'s `Param` doc comment) via `curry_cmd_params`, with **no**
/// implicit/explicit context variable at all (contrast `elaborate_let_inline`,
/// which always threads one) — a math command's own type (`math-cmd`)
/// carries no context argument. A zero-param binding (e.g. `let-math \to =
/// rel \`→\``) elaborates to `value` directly, un-wrapped. Shared by
/// `TopBinding::LetMath` (via `walk_bindings`) and the expression-level
/// `Expr::LetMathIn` (`parser.mly:688`, upstream's only command binding with
/// a local `in`-bodied form — see that variant's doc comment).
fn elaborate_let_math(
    params: &[c::Param],
    value: &c::Expr,
    scope: &Scope,
) -> Result<Ast, ElabError> {
    curry_cmd_params_v1(params, scope, |inner| expr(value, inner))
}

/// Elaborate one `let-rec` clause group (shared by the local `Expr::LetRecIn`
/// and the top-level `TopBinding::LetRec`): every name is in scope in every
/// binding's own value (mutual recursion) as well as in the body, and each
/// binding's own parameters curry into a `Lambda` around its elaborated
/// value. Whether the (possibly zero) curried result is actually a function
/// is a *runtime* check (see `eval.rs`'s `Ast::LetRecIn` handling) — nothing
/// here forces `params` to be non-empty, since a paramterless binding whose
/// `value` is itself e.g. a `fun ...` expression is equally valid.
fn rec_bindings(
    first: &c::RecBinding,
    ands: &[c::AndBinding],
    scope: &Scope,
) -> Result<(Vec<(String, Rc<Ast>)>, Scope), ElabError> {
    let all: Vec<&c::RecBinding> = std::iter::once(first)
        .chain(ands.iter().map(|a| &a.binding))
        .collect();
    let mut rec_scope = scope.clone();
    for rb in &all {
        rec_scope = rec_scope.with(&rb.name.name);
    }
    let mut bindings = Vec::with_capacity(all.len());
    for rb in all {
        let value_ast = rec_clause_value(&rb.params, &rb.value, &rb.extra, &rec_scope)?;
        bindings.push((rb.name.name.clone(), Rc::new(value_ast)));
    }
    Ok((bindings, rec_scope))
}

/// Elaborate the (possibly multi-clause) value of one `let-rec` binding
/// (`RecBinding`/`RecClause`: `name [|] patbot* = value (| patbot* =
/// value)*`). Faithfully, a multi-clause function definition desugars into
/// one curried function of `n` fresh parameters (`n` = every clause's shared
/// arity — an `IllegalArgumentLength`-style error if they disagree) that
/// matches a tuple of them against each clause's patterns in turn, first
/// clause first (`option.satyg`'s `let-rec map | f (None) = None | f
/// (Some(v)) = Some(f v)` is exactly this: 2 clauses, arity 2). At arity 1
/// the "tuple" is just the single fresh parameter itself, no `Ast::Tuple`
/// wrapper (matches `list.satyg`'s single-parameter-pattern clauses, e.g.
/// `let-rec append lst1 lst2 = ..` mixed with genuinely-refutable single
/// clauses elsewhere).
///
/// The single-clause, all-variable-parameter case (`let-rec f x y = ..`, no
/// `|` at all — overwhelmingly the common shape) is special-cased to the
/// original direct `Lambda` chain: behaviorally identical to the general
/// path (variable patterns always match and simply bind), it just skips
/// building a throwaway `Match`/fresh-variable indirection for what nearly
/// every binding actually looks like.
fn rec_clause_value(
    params0: &[c::PatBot],
    value0: &c::Expr,
    extra: &[c::RecClause],
    scope: &Scope,
) -> Result<Ast, ElabError> {
    let arity = params0.len();
    for cl in extra {
        if cl.params.len() != arity {
            return err(
                cl.bar.0,
                format!(
                    "every clause of a multi-clause 'let-rec' binding must bind the \
                     same number of parameters (expected {arity}, got {})",
                    cl.params.len()
                ),
            );
        }
    }

    if extra.is_empty() && params0.iter().all(is_var_patbot) {
        let mut inner = scope.clone();
        for p in params0 {
            inner = inner.with(patbot_var_name(p));
        }
        let mut value_ast = expr(value0, &inner)?;
        for p in params0.iter().rev() {
            value_ast = Ast::Lambda(patbot_var_name(p).to_string(), Rc::new(value_ast));
        }
        return Ok(value_ast);
    }

    let fresh: Vec<String> = (0..arity).map(|i| format!("%rec_arg{i}")).collect();
    let mut arms = Vec::with_capacity(1 + extra.len());
    arms.push(rec_clause_arm(params0, value0, scope)?);
    for cl in extra {
        arms.push(rec_clause_arm(&cl.params, &cl.value, scope)?);
    }
    let dummy = Span::default();
    let scrutinee = if arity == 1 {
        Ast::Var(fresh[0].clone(), dummy)
    } else {
        Ast::Tuple(fresh.iter().map(|f| Ast::Var(f.clone(), dummy)).collect())
    };
    let mut body = Ast::Match(Box::new(scrutinee), arms);
    for f in fresh.iter().rev() {
        body = Ast::Lambda(f.clone(), Rc::new(body));
    }
    Ok(body)
}

/// Lower a SATySFi 0.1 `fun ?(l = x, …) p -> body` unit (`Expr::FunRows`) to
/// an [`Ast::LambdaOpt`]. Gated on the V0_1 source version (a 0.0.6-parsed
/// occurrence — reachable only via the additive-`cst` accept surface — is
/// rejected here with a version error). Duplicate labels in one binder list
/// are rejected. Each optional binder and the positional param enter scope
/// as plain names (labeled optionals have no marker-less padding, so NO
/// `optional_arity` entry). A pattern param desugars to a fresh var + `Match`
/// exactly as `rec_clause_value` does for a destructuring parameter.
fn fun_rows_to_ast(
    kw_span: Span,
    opts: &c::CstOptBinders,
    param: &c::PatBot,
    body: &c::Expr,
    scope: &Scope,
) -> Result<Ast, ElabError> {
    if !scope.version.has_row_polymorphism() {
        return err(
            kw_span,
            "labeled optional arguments (`?(l = x)`) are SATySFi 0.1 syntax — \
             this file is compiled as 0.0.6",
        );
    }
    let mut inner = scope.clone();
    for e in &opts.entries {
        inner = inner.with(&e.var.name);
    }
    let body_ast = if is_var_patbot(param) {
        let body_scope = inner.with(patbot_var_name(param));
        expr(body, &body_scope)?
    } else {
        let pat = patbot(param)?;
        let mut names = Vec::new();
        collect_pattern_names(&pat, &mut names);
        let mut body_scope = inner;
        for n in &names {
            body_scope = body_scope.with(n);
        }
        expr(body, &body_scope)?
    };
    lambda_opt_from(opts, param, body_ast)
}

/// Build an `Ast::LambdaOpt` from a `?(l = x, …)` binder bundle, its
/// positional param, and an ALREADY-ELABORATED inner body — the shared core
/// factored out of [`fun_rows_to_ast`] (a value-level `fun ?(l = x) p ->
/// body` unit) so [`curry_cmd_params_v1`]'s bundle arm (a command
/// parameter bundle, optional-arg-rows increment 3a) can reuse the exact
/// same binder logic. Duplicate labels in one binder list are rejected. A
/// `PatBot::Var` param becomes the `LambdaOpt`'s param directly; any other
/// pattern desugars to a fresh `%opt_arg` var + `Match`, exactly like
/// `rec_clause_value`'s destructuring-parameter path.
fn lambda_opt_from(
    opts: &c::CstOptBinders,
    param: &c::PatBot,
    inner_body_ast: Ast,
) -> Result<Ast, ElabError> {
    let mut opt_pairs: Vec<(String, String)> = Vec::with_capacity(opts.entries.len());
    let mut seen = HashSet::new();
    for e in &opts.entries {
        if !seen.insert(e.label.name.clone()) {
            return err(
                e.label.span,
                format!(
                    "duplicate optional label `{}` in one `?(…)` binder list",
                    e.label.name
                ),
            );
        }
        opt_pairs.push((e.label.name.clone(), e.var.name.clone()));
    }
    if is_var_patbot(param) {
        Ok(Ast::LambdaOpt {
            opts: opt_pairs,
            param: patbot_var_name(param).to_string(),
            body: Rc::new(inner_body_ast),
        })
    } else {
        let fresh = "%opt_arg".to_string();
        let pat = patbot(param)?;
        let matched = Ast::Match(
            Box::new(Ast::Var(fresh.clone(), Span::default())),
            vec![MatchArm {
                pat,
                guard: None,
                body: inner_body_ast,
            }],
        );
        Ok(Ast::LambdaOpt {
            opts: opt_pairs,
            param: fresh,
            body: Rc::new(matched),
        })
    }
}

/// The bundle-aware command-parameter currier (optional-arg-rows increment
/// 3a): the same overall shape as [`curry_cmd_params`] — extend the scope
/// with every name this parameter list binds, build the innermost value
/// once against the fully-extended scope, then curry back outward — but
/// additionally handles a `Param::Bundled { opts, body }` entry (`?(l = x,
/// …) pat`) by emitting an `Ast::LambdaOpt` for that slot (via
/// [`lambda_opt_from`]) instead of a plain `Ast::Lambda`/`Match`.
///
/// **Delegates wholesale to [`curry_cmd_params`]** when `params` contains no
/// `Bundled` entry at all — the exact same tested code path as before this
/// increment, so every 0.0.6 command binding and every V0_1 one that
/// doesn't use a bundle (the overwhelming majority) is byte-identical.
///
/// **Version-gated** exactly like `fun_rows_to_ast`: a `Bundled` entry
/// reaching the general fold below under `!scope.version.
/// has_row_polymorphism()` is rejected with the same version error — purely
/// defensive, since only `v1/lower.rs::lower_command_params` ever
/// constructs `Param::Bundled`, and `lower_value_math` additionally rejects
/// it outright for a `val math` binding's own parameter list before it ever
/// reaches elaboration (math command bundles are optional-arg-rows
/// increment 3b).
fn curry_cmd_params_v1(
    params: &[c::Param],
    scope: &Scope,
    build_value: impl FnOnce(&Scope) -> Result<Ast, ElabError>,
) -> Result<Ast, ElabError> {
    if !params.iter().any(|p| matches!(p, c::Param::Bundled { .. })) {
        let patbots = params_to_patbots(params);
        return curry_cmd_params(&patbots, scope, build_value);
    }
    if !scope.version.has_row_polymorphism() {
        let bundle_span = params
            .iter()
            .find_map(|p| match p {
                c::Param::Bundled { opts, .. } => Some(opts.q.0),
                _ => None,
            })
            .expect("just checked a `Param::Bundled` entry exists");
        return err(
            bundle_span,
            "labeled optional arguments (`?(l = x)`) are SATySFi 0.1 syntax — \
             this file is compiled as 0.0.6",
        );
    }
    let mut inner = scope.clone();
    for p in params {
        inner = match p {
            c::Param::Bundled { opts, body } => {
                for e in &opts.entries {
                    inner = inner.with(&e.var.name);
                }
                extend_with_patbot(inner, body)?
            }
            _ => extend_with_patbot(inner, &param_to_patbot(p))?,
        };
    }
    let mut value_ast = build_value(&inner)?;
    let dummy = Span::default();
    for (i, p) in params.iter().enumerate().rev() {
        value_ast = match p {
            c::Param::Bundled { opts, body } => lambda_opt_from(opts, body, value_ast)?,
            c::Param::Optional { name, .. } => Ast::Lambda(name.name.clone(), Rc::new(value_ast)),
            c::Param::Pat(pat) if is_var_patbot(pat) => {
                Ast::Lambda(patbot_var_name(pat).to_string(), Rc::new(value_ast))
            }
            c::Param::Pat(pat) => {
                let pp = patbot(pat)?;
                let fresh = format!("%cmd_arg{i}");
                Ast::Lambda(
                    fresh.clone(),
                    Rc::new(Ast::Match(
                        Box::new(Ast::Var(fresh, dummy)),
                        vec![MatchArm {
                            pat: pp,
                            guard: None,
                            body: value_ast,
                        }],
                    )),
                )
            }
        };
    }
    Ok(value_ast)
}

/// Extend `scope` with every name patbot `p` binds: its single var name if
/// it's a plain `PatBot::Var`, else every name the full pattern binds
/// (`collect_pattern_names`) — the scope half of `curry_cmd_params_v1`'s
/// general fold (mirroring `curry_cmd_params`'s own inline scope-extension,
/// factored out here since the bundle-aware fold interleaves it with a
/// bundle's own binder names).
fn extend_with_patbot(scope: Scope, p: &c::PatBot) -> Result<Scope, ElabError> {
    if is_var_patbot(p) {
        Ok(scope.with(patbot_var_name(p)))
    } else {
        let pat = patbot(p)?;
        let mut names = Vec::new();
        collect_pattern_names(&pat, &mut names);
        let mut s = scope;
        for n in &names {
            s = s.with(n);
        }
        Ok(s)
    }
}

/// Widen a plain (non-`let-rec`) `let`'s `Param` down to a `PatBot`, so
/// `TopLet`/`Expr::LetIn` can share `rec_clause_value`'s pattern-currying
/// machinery with `let-rec` unchanged: the def-site optional marker
/// (`Param::Optional`, `?:name`) carries no elaboration-time semantics of
/// its own in this port (see `cst.rs`'s `Param` doc comment) — it is simply
/// a plain variable binder, `PatBot::Var`.
fn param_to_patbot(p: &c::Param) -> c::PatBot {
    match p {
        c::Param::Optional { name, .. } => c::PatBot::Var(name.clone()),
        c::Param::Pat(pat) => pat.clone(),
        // A `?(l = x, …)` command-parameter bundle (optional-arg-rows
        // increment 3a) never reaches this widener: it is only ever
        // constructed by `v1/lower.rs::lower_command_params` for a command
        // binding's OWN `Param` list, and `curry_cmd_params_v1` — the only
        // caller for that list — checks for a `Bundled` entry itself and
        // routes around `params_to_patbots`/`param_to_patbot` entirely when
        // one is present (see that function's doc comment). A plain `let`/
        // `let-rec`'s `Param` list (the only other caller of this widener)
        // can never contain one either — `lower_param_units` always
        // right-folds a bundled unit into an `Expr::FunRows` chain instead,
        // returning an EMPTY `Param` list when any unit is bundled.
        c::Param::Bundled { .. } => {
            unreachable!("a `?(l = x)` command-parameter bundle cannot reach `param_to_patbot`")
        }
    }
}

fn params_to_patbots(params: &[c::Param]) -> Vec<c::PatBot> {
    params.iter().map(param_to_patbot).collect()
}

fn is_var_patbot(p: &c::PatBot) -> bool {
    matches!(p, c::PatBot::Var(_))
}

/// Panics if `p` isn't `PatBot::Var` — callers must check [`is_var_patbot`] first.
fn patbot_var_name(p: &c::PatBot) -> &str {
    match p {
        c::PatBot::Var(v) => &v.name,
        _ => unreachable!("patbot_var_name called on a non-Var PatBot"),
    }
}

/// One `patbot* = value` clause of a multi-clause `let-rec`, lowered to a
/// [`MatchArm`] over the clause's parameter patterns (see
/// [`rec_clause_value`]'s doc comment for the arity-1-vs-N pattern shape) —
/// the body sees every name the patterns bind, exactly like an ordinary
/// `match` arm ([`match_arm`], below).
fn rec_clause_arm(
    params: &[c::PatBot],
    value: &c::Expr,
    scope: &Scope,
) -> Result<MatchArm, ElabError> {
    let pats: Vec<Pattern> = params.iter().map(patbot).collect::<Result<_, _>>()?;
    let mut names = Vec::new();
    for p in &pats {
        collect_pattern_names(p, &mut names);
    }
    let mut inner = scope.clone();
    for n in &names {
        inner = inner.with(n);
    }
    let body = expr(value, &inner)?;
    let pat = if pats.len() == 1 {
        pats.into_iter().next().unwrap()
    } else {
        Pattern::Tuple(pats)
    };
    Ok(MatchArm {
        pat,
        guard: None,
        body,
    })
}

fn expr(e: &c::Expr, scope: &Scope) -> Result<Ast, ElabError> {
    match e {
        c::Expr::LetRecIn {
            first, ands, body, ..
        } => {
            let (bindings, rec_scope) = rec_bindings(first, ands, scope)?;
            let body_ast = expr(body, &rec_scope)?;
            Ok(Ast::LetRecIn(bindings, Box::new(body_ast)))
        }
        c::Expr::LetIn {
            name,
            params,
            value,
            body,
            ..
        } => {
            // `params` is now a full `param*` (`cst::ast::Expr::LetIn`'s doc
            // comment — widened for `hdecoset.satyh`/`vdecoset.satyh`'s `let
            // deco _ _ _ _ = [] in ..`, and further widened to `Param` for
            // `stdja.satyh`'s `let document record ?:configopt inner = ..`),
            // so this reuses the same single-clause pattern-currying path
            // `let-rec`/`Fun` already share, non-recursively (`scope`, not a
            // scope extended with `name` itself) — `params_to_patbots` first
            // widens any `?:name` marker to a plain `PatBot::Var`.
            let let_in_params = params_to_patbots(params);
            let value_ast = rec_clause_value(&let_in_params, value, &[], scope)?;
            // Record `name`'s leading-`?:`-optional-parameter count (see
            // `Scope`'s doc comment) so a later marker-less bare call in
            // `body` (`app_chain_generic`) auto-omits it, e.g. progsynt.satyh's
            // `let to-math ?:iopt e = .. in .. to-math e1 ..`.
            let mut body_scope = scope.clone();
            body_scope.insert_with_shape(&name.name, param_optional_shape(params));
            let body_ast = expr(body, &body_scope)?;
            Ok(Ast::LetIn(
                name.name.clone(),
                Box::new(value_ast),
                Box::new(body_ast),
            ))
        }
        // `let pat = value in body` (`nxnonrecdec`'s general-pattern case —
        // see `cst::ast::Expr::LetPatternIn`'s doc comment for why this is a
        // separate variant from `LetIn` above). Lowered to the same
        // single-arm-`match` machinery a `match` expression's own arms use
        // (`pattern`/`collect_pattern_names`, below): `value` is elaborated
        // under the OUTER scope (a destructuring let's right-hand side never
        // sees its own bound names, same as `LetIn`), then matched against
        // `pat`, whose bound names are in scope for `body`.
        c::Expr::LetPatternIn { pat, value, body, .. } => {
            let value_ast = expr(value, scope)?;
            let lowered_pat = pattern(pat)?;
            let mut names = Vec::new();
            collect_pattern_names(&lowered_pat, &mut names);
            let mut inner = scope.clone();
            for n in &names {
                inner = inner.with(n);
            }
            let body_ast = expr(body, &inner)?;
            Ok(Ast::Match(
                Box::new(value_ast),
                vec![MatchArm {
                    pat: lowered_pat,
                    guard: None,
                    body: body_ast,
                }],
            ))
        }
        c::Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => Ok(Ast::IfThenElse(
            Box::new(expr(cond, scope)?),
            Box::new(expr(then_branch, scope)?),
            Box::new(expr(else_branch, scope)?),
        )),
        // `fun patbot+ -> body` (`nxlambda`'s `LAMBDA argpats ARROW nxlor`,
        // `argpats = list(patbot)` — see `cst::ast::Expr::Fun`'s doc
        // comment). Delegates to `rec_clause_value` (below), the SAME
        // arity-preserving pattern-currying `let-rec` already needs for its
        // own `patbot*` clause parameters (`fun`'s `extra` clause list is
        // simply empty — a lambda has no `|`-alternation): the common
        // all-plain-variable case still becomes a direct `Lambda` chain,
        // with no `Match`/fresh-variable indirection, and only a genuine
        // destructuring parameter (e.g. the bundled `list.satyg`'s
        // `mapi-adjacent`: `fun (i, acc) x leftopt rightopt -> ..`) pays for
        // the general path.
        c::Expr::Fun { kw, params, body, .. } => {
            if params.is_empty() {
                return err(kw.0, "'fun' needs at least one parameter");
            }
            rec_clause_value(params, body, &[], scope)
        }
        // `fun ?(l = x, …) p -> body` — SATySFi 0.1 labeled-optional lambda
        // unit (one bundle, one positional param).
        c::Expr::FunRows {
            kw,
            opts,
            param,
            body,
            ..
        } => fun_rows_to_ast(kw.0, opts, param, body, scope),
        c::Expr::Match {
            scrutinee,
            first,
            rest,
            ..
        } => {
            let scrut = expr(scrutinee, scope)?;
            let mut arms = Vec::with_capacity(1 + rest.len());
            arms.push(match_arm(first, scope)?);
            for bar in rest {
                arms.push(match_arm(&bar.arm, scope)?);
            }
            Ok(Ast::Match(Box::new(scrut), arms))
        }
        // `let-mutable name <- init in body` (`nxletsub`'s `LETMUTABLE` case).
        c::Expr::LetMutableIn {
            name, init, body, ..
        } => {
            let init_ast = expr(init, scope)?;
            let inner = scope.with(&name.name);
            let body_ast = expr(body, &inner)?;
            Ok(Ast::LetMutableIn(
                name.name.clone(),
                Box::new(init_ast),
                Box::new(body_ast),
            ))
        }
        // `let-math \cmd param* = value in body` (`nxletsub`'s `LETMATH`
        // case, `parser.mly:688`) — upstream's only command binding with a
        // local `in`-bodied form (`LETHORZ`/`LETVERT` stay top-level-only,
        // see `cst.rs`'s `Expr::LetMathIn` doc comment). Structurally
        // identical to the top-level `TopBinding::LetMath` arm of
        // `walk_bindings`: elaborate the (curried) value under the OUTER
        // scope via the shared `elaborate_let_math` helper, then record
        // `cmd`'s leading-`?:`-optional-parameter count for `body`, same as
        // `Expr::LetIn` just above.
        c::Expr::LetMathIn {
            cmd,
            params,
            value,
            body,
            ..
        } => {
            let value_ast = elaborate_let_math(params, value, scope)?;
            let mut body_scope = scope.clone();
            body_scope.insert_with_shape(&cmd.name, param_optional_shape(params));
            let body_ast = expr(body, &body_scope)?;
            Ok(Ast::LetMathIn(
                cmd.name.clone(),
                Box::new(value_ast),
                Box::new(body_ast),
            ))
        }
        // `open Name in body` (`nxletsub`'s `OPEN` case) — same alias-binding
        // technique as the top-level `TopBinding::Open` fold above (see
        // `walk_bindings`), just producing the `LetIn` chain directly since
        // there is no further sequence of sibling top bindings to thread a
        // scope through here.
        c::Expr::OpenIn { name, body, .. } => {
            open_module(&name.name, name.span, scope, |s| expr(body, s))
        }
        // `while cond do body` (`nxwhl`).
        c::Expr::WhileDo { cond, body, .. } => Ok(Ast::WhileDo(
            Box::new(expr(cond, scope)?),
            Box::new(expr(body, scope)?),
        )),
        // `name <- value` (`nxlambda`'s `OVERWRITEEQ` case). Existence is
        // checked against the bare `name.name` (unaffected by any active
        // `Scope::rename` redirect); the constructed node's own key goes
        // through `Scope::resolve`, exactly like `scoped_var` — a mutable
        // module member's sibling overwrite (`first-footnote <- Some m`)
        // must target the SAME mangled key its `LetMutableIn` is bound
        // under, or the typechecker's own `Ast::Overwrite` arm (which looks
        // the name up directly in `env`, independent of `Scope`) reports it
        // unbound (module-completion bug fix — see `push_named_binding`'s
        // doc comment).
        c::Expr::Overwrite { name, value, .. } => {
            if !scope.contains(&name.name) {
                return err(
                    name.span,
                    format!("unbound mutable variable '{}'", name.name),
                );
            }
            Ok(Ast::Overwrite(
                scope.resolve(&name.name),
                name.span,
                Box::new(expr(value, scope)?),
            ))
        }
        c::Expr::Ops(chain) => op_chain(chain, scope),
    }
}

// ---- operator-precedence fold --------------------------------------------

/// Precedence-climbing associativity.
#[derive(Clone, Copy)]
enum Assoc {
    Left,
    Right,
}

/// The v0.0.6 `nxlor`..`nxrtimes` precedence ladder, transcribed from
/// `parser.mly` lines 722-780 (loosest to tightest):
///
/// | level | tokens                                    | assoc |
/// |-------|-------------------------------------------|-------|
/// | 1     | `BinopBar` (`\|>`, ...)                    | left  |
/// | 2     | `BinopAmp`                                 | left  |
/// | 3     | `BinopEq`, `BinopGt`, `BinopLt`             | right |
/// | 4     | `BinopHat` (`^`), `Cons` (`::`)             | right |
/// | 5     | `BinopPlus`, `BinopMinus`, `ExactMinus`     | left  |
/// | 6     | `BinopTimes`, `ExactTimes`, `BinopDivides`, `Mod` | right |
///
/// Deviation note: v0.0.6's plus/minus level is actually a strange
/// left/right mix (`nxlplus`/`nxlminus`/`nxrplus`/`nxrminus`, four mutually
/// referencing nonterminals) that differs from plain left-association only
/// in how chains of `+`/`-` nest — e.g. `nxlminus`'s right operand is
/// `nxrtimes`, not `nxrminus`, so `1 - 2 - 3`'s *tree shape* differs subtly
/// from a naive left fold even though both compute `(1 - 2) - 3`. Since `+`
/// and `*` (this port's only concrete instances at this level) are
/// associative anyway and neither surface syntax nor the tests can observe
/// the tree shape, we implement plain LEFT association for level 5, which
/// matches `-` exactly and is semantically irrelevant for `+`.
///
/// Level 6 (`nxrtimes`) is genuinely right-recursive in the grammar itself
/// (`nxltimes`'s right operand is `nxrtimes`, and `nxrtimes` recurses into
/// itself on the right) — `8 / 4 / 2` really does parse as `8 / (4 / 2)` in
/// v0.0.6, not `(8 / 4) / 2`. We keep this fidelity quirk.
///
/// **`&&`/`||` are NOT short-circuited here.** v0.0.6's `bytecomp/
/// vminstdef.yaml` registers them as ordinary strict primitives
/// (`LogicalAnd`/`LogicalOr`, `is-pdf-mode-primitive: yes`, `code: make_bool
/// (binl && binr)`/`(binl || binr)`) applied like any other binop through
/// `binary_operator` in `parser.mly` (`nxland`/`nxlor`, lines 722-727) — by
/// the time that OCaml `&&`/`||` runs, *both* VM-stack operands are already
/// popped (i.e. both sides were fully evaluated as ordinary call-by-value
/// arguments), so real SATySFi does not short-circuit `&&`/`||` at the
/// source-language level either. This port's `primitives.rs` already
/// registers `"&&"`/`"||"` as strict 2-arg primitives — that already
/// matches v0.0.6 exactly, so no `if`-desugaring is added here.
fn op_prec(tok: &Token) -> (u8, Assoc) {
    match tok {
        Token::BinopBar(_) => (1, Assoc::Left),
        Token::BinopAmp(_) => (2, Assoc::Left),
        Token::BinopEq(_) | Token::BinopGt(_) | Token::BinopLt(_) => (3, Assoc::Right),
        Token::BinopHat(_) | Token::Cons => (4, Assoc::Right),
        Token::BinopPlus(_) | Token::BinopMinus(_) | Token::ExactMinus => (5, Assoc::Left),
        Token::BinopTimes(_) | Token::ExactTimes | Token::BinopDivides(_) | Token::Mod => {
            (6, Assoc::Right)
        }
        _ => unreachable!("BinOpTok::parse only ever matches the operator tokens listed above"),
    }
}

/// `nxbfr`'s postfix `before` (see `OpChain::before`'s doc comment in
/// `cst.rs`): `e1 before e2` → `Ast::Sequential(e1, e2)`, where `e1` is the
/// whole precedence-folded operator chain.
fn op_chain(chain: &c::OpChain, scope: &Scope) -> Result<Ast, ElabError> {
    let head_ast = app_expr(&chain.head, scope)?;
    let folded = if chain.tail.is_empty() {
        head_ast
    } else {
        let mut atoms: VecDeque<Ast> = VecDeque::with_capacity(chain.tail.len() + 1);
        atoms.push_back(head_ast);
        let mut ops: VecDeque<(String, Span, Token)> = VecDeque::with_capacity(chain.tail.len());
        for rhs in &chain.tail {
            let text = rhs.op.op_text();
            // `|>` (frontend-completion.md §Slice1-A / stdlib-port.md Blocker
            // B) is handled entirely by `climb`'s special case below — it is
            // deliberately NOT a `scope`-bound name (no runtime primitive, no
            // `prim_types` entry: `a |> f` lowers straight to `Apply(f, a)`,
            // ordinary application the inferencer/evaluator already handle),
            // so it must skip the "unbound operator" gate every other
            // operator token goes through.
            if text != "|>" && !scope.contains(&text) {
                return err(rhs.op.span, format!("unbound operator '{text}'"));
            }
            // `scope.resolve` redirects a module member's own bare
            // operator (`val (+++) a b = ..` inside `module M = struct ..
            // end`) to its mangled key, exactly like `scoped_var` — `"|>"`
            // is never a `Scope::rename` target (it's deliberately never
            // scope-bound at all, see this arm's own comment above), so
            // resolving it is a no-op.
            ops.push_back((scope.resolve(&text), rhs.op.span, rhs.op.tok.clone()));
            atoms.push_back(app_expr(&rhs.rhs, scope)?);
        }
        climb(&mut atoms, &mut ops, 0)
    };
    match &chain.before {
        Some(bt) => Ok(Ast::Sequential(
            Box::new(folded),
            Box::new(expr(&bt.body, scope)?),
        )),
        None => Ok(folded),
    }
}

/// Standard precedence-climbing fold over an already-elaborated flat
/// `atom (op atom)*` sequence (`atoms.len() == ops.len() + 1`). Every binop
/// elaborates uniformly to `Apply(Apply(Var(op_text), lhs), rhs)` — SATySFi
/// binops (including `::`, see the `primitives.rs` note) are just env-bound
/// primitives, no special-cased AST node needed — **except `|>`**, which is
/// reverse application (`a |> f` ≡ `f a`, upstream `primitives.cppo.ml:552`)
/// special-cased directly to `Apply(rhs, lhs)` rather than
/// `Apply(Apply(Var("|>"), lhs), rhs)`: no primitive named `"|>"` is ever
/// registered (see `op_chain`'s matching skip of the scope-contains gate),
/// since applying a user-supplied closure isn't something any current
/// primitive body does. `|>` sits at level 1 (loosest, left-associative,
/// see `op_prec`), so `a |> f |> g` folds as `(a |> f) |> g` = `g (f a)`,
/// matching the bundled `list.satyg`'s pipe-heavy style (`reverse`,
/// `map-adjacent`, `map-with-ends`).
fn climb(
    atoms: &mut VecDeque<Ast>,
    ops: &mut VecDeque<(String, Span, Token)>,
    min_prec: u8,
) -> Ast {
    let mut lhs = atoms
        .pop_front()
        .expect("one more atom than consumed operators");
    while let Some((_, _, tok)) = ops.front() {
        let (prec, assoc) = op_prec(tok);
        if prec < min_prec {
            break;
        }
        let (text, span, _) = ops.pop_front().unwrap();
        let next_min = match assoc {
            Assoc::Left => prec + 1,
            Assoc::Right => prec,
        };
        let rhs = climb(atoms, ops, next_min);
        lhs = if text == "|>" {
            Ast::Apply(Box::new(rhs), Box::new(lhs))
        } else {
            Ast::Apply(
                Box::new(Ast::Apply(Box::new(Ast::Var(text, span)), Box::new(lhs))),
                Box::new(rhs),
            )
        };
    }
    lhs
}

// ---- application chains --------------------------------------------------

fn app_expr(a: &c::AppExpr, scope: &Scope) -> Result<Ast, ElabError> {
    let ast = if a.excl.is_none() && a.head_accesses.is_empty() {
        if let c::Atomic::Ctor(ctor) = &a.head {
            // A constructor head: the first argument (if any) is its payload
            // (`Some 1`); any further arguments Apply-fold on top of the
            // resulting `Ctor` value, which the evaluator will reject at run
            // time (constructors are not functions).
            let mut args_iter = a.args.iter();
            match args_iter.next() {
                Some(first) => {
                    let payload = app_arg_to_ast(first, scope)?;
                    let mut ast = Ast::Ctor(ctor.name.clone(), Some(Box::new(payload)));
                    for rest in args_iter {
                        ast = apply_one_arg(ast, rest, scope)?;
                    }
                    ast
                }
                None => Ast::Ctor(ctor.name.clone(), None),
            }
        } else {
            app_chain_generic(a, scope)?
        }
    } else {
        // `!Ctor` / `Ctor#field` don't correspond to any valid v0.0.6
        // program (`CONSTRUCTOR` isn't part of `nxbot`, so it can never sit
        // under a `#label`/`UNOP_EXCLAM` prefix there) — fall back to the
        // generic path, which treats the bare constructor as an ordinary
        // (payload-less) atomic value.
        app_chain_generic(a, scope)?
    };
    match &a.minus {
        // Unary minus desugars exactly as v0.0.6's `nxun` does (parser.mly
        // ~line 774): `0 - <the whole application>`.
        Some(m) => {
            let minus = scoped_var("-", m.0, scope)?;
            Ok(Ast::Apply(
                Box::new(Ast::Apply(Box::new(minus), Box::new(Ast::Int(0)))),
                Box::new(ast),
            ))
        }
        None => Ok(ast),
    }
}

fn app_chain_generic(a: &c::AppExpr, scope: &Scope) -> Result<Ast, ElabError> {
    let mut ast = atomic_head_with_excl(&a.head, &a.head_accesses, a.excl.as_ref(), scope)?;
    // Marker-less optional-argument defaulting (`Scope`'s doc comment /
    // `docs/plans/frontend-completion.md` Sub-area 2): if the head is a bare
    // name (no `!`/`#access`) known to have `?:`-optional parameters
    // ANYWHERE in its declared `Param` list (not just leading — see
    // `Scope::optional_shape`'s doc comment for `stdja.satyh`'s `document
    // record ?:configopt inner`, whose optional is the *second* parameter),
    // walk `a.args` position by position against that declared shape:
    //
    //  - at a declared OPTIONAL position, an explicit `?:e`/`?*` marker is
    //    consumed as before (an EXPLICIT call site, e.g. `document r ?:(c)
    //    body`, is elaborated exactly as before — this only ever adds
    //    behavior, never removes it); anything else (a bare/unmarked arg, a
    //    `?(l=e)` bundle, or no argument left at all mid-application) is
    //    NOT consumed — a plain `None` is synthesized for that slot instead,
    //    and the same argument is re-examined against the NEXT declared
    //    position. This is what fixes the "positional-after-omitted-
    //    optional mis-binds against the omitted slot" bug: `document record
    //    body` (no marker at all) becomes `Apply(Apply(Apply(document,
    //    record), None), body)`, matching what an explicit `document record
    //    ?* body` would already elaborate to, instead of unifying `body`
    //    straight against `configopt`'s domain.
    //  - at a declared MANDATORY position, the next argument (of any kind)
    //    is consumed positionally, same as plain application always was.
    //  - once every declared position has been visited (or the head's shape
    //    is unknown/empty — the overwhelmingly common case), any remaining
    //    arguments are applied exactly as before (`apply_one_arg` per
    //    argument), so a call with MORE arguments than the head's own
    //    declared arity (currying into whatever the fully-applied head
    //    itself returns) is unaffected.
    //
    // Guarded on a non-empty `a.args` so a bare function-VALUE reference (no
    // application at all, e.g. `to-math` passed to `List.map`) is left
    // untouched.
    let shape: &[bool] = if a.excl.is_none() && a.head_accesses.is_empty() && !a.args.is_empty() {
        head_optional_shape(&a.head, scope)
    } else {
        &[]
    };
    let mut args_iter = a.args.iter().peekable();
    let mut pos = 0usize;
    while pos < shape.len() {
        if shape[pos] {
            match args_iter.peek() {
                Some(c::AppArg::Optional { .. }) | Some(c::AppArg::Omission(_)) => {
                    let arg = args_iter.next().unwrap();
                    ast = Ast::Apply(Box::new(ast), Box::new(app_arg_to_ast(arg, scope)?));
                }
                Some(_) => {
                    ast = Ast::Apply(Box::new(ast), Box::new(Ast::Ctor("None".to_string(), None)));
                }
                None => break,
            }
        } else {
            match args_iter.next() {
                Some(arg) => ast = apply_one_arg(ast, arg, scope)?,
                None => break,
            }
        }
        pos += 1;
    }
    for arg in args_iter {
        ast = apply_one_arg(ast, arg, scope)?;
    }
    Ok(ast)
}

/// Apply one application-chain argument to the running `func` AST. A SATySFi
/// 0.1 `?(l = e, …)`-bundled argument becomes an [`Ast::ApplyOpt`] (carrying
/// the labeled optionals plus the paired positional argument); every other
/// argument is an ordinary [`Ast::Apply`].
fn apply_one_arg(func: Ast, arg: &c::AppArg, scope: &Scope) -> Result<Ast, ElabError> {
    match arg {
        c::AppArg::Bundled {
            opts,
            excl,
            atom,
            accesses,
        } => {
            let opt_args = elaborate_opt_args(opts, scope)?;
            let arg_ast = atomic_head_with_excl(atom, accesses, excl.as_ref(), scope)?;
            Ok(Ast::ApplyOpt {
                func: Box::new(func),
                opts: opt_args,
                arg: Box::new(arg_ast),
            })
        }
        c::AppArg::BundledCtor { opts, ctor } => {
            let opt_args = elaborate_opt_args(opts, scope)?;
            Ok(Ast::ApplyOpt {
                func: Box::new(func),
                opts: opt_args,
                arg: Box::new(Ast::Ctor(ctor.name.clone(), None)),
            })
        }
        _ => Ok(Ast::Apply(Box::new(func), Box::new(app_arg_to_ast(arg, scope)?))),
    }
}

/// Elaborate a `?(l = e, …)` optional-argument bundle: version-gate it (a
/// 0.0.6-parsed occurrence is rejected here), reject a duplicate label within
/// the one bundle, and elaborate each label's value expression.
fn elaborate_opt_args(
    opts: &c::CstOptArgs,
    scope: &Scope,
) -> Result<Vec<(String, Ast)>, ElabError> {
    if !scope.version.has_row_polymorphism() {
        return err(
            opts.q.0,
            "labeled optional arguments (`?(l = e)`) are SATySFi 0.1 syntax — \
             this file is compiled as 0.0.6",
        );
    }
    let mut out: Vec<(String, Ast)> = Vec::with_capacity(opts.entries.len());
    let mut seen = HashSet::new();
    for e in &opts.entries {
        if !seen.insert(e.label.name.clone()) {
            return err(
                e.label.span,
                format!(
                    "duplicate optional label `{}` in one `?(…)` bundle",
                    e.label.name
                ),
            );
        }
        out.push((e.label.name.clone(), expr(&e.value.0, scope)?));
    }
    Ok(out)
}

/// `a.head`'s recorded full per-position optional-parameter shape (`Scope::
/// optional_shape`), for a bare unqualified or module-qualified variable
/// head only — any other head shape (a parenthesized expression, a
/// dereferenced/accessed value, …) can never name a known `let`/`let ..
/// in` binding directly, so it conservatively reports `&[]` (unknown).
fn head_optional_shape<'a>(head: &c::Atomic, scope: &'a Scope) -> &'a [bool] {
    match head {
        c::Atomic::Var(v) => scope.optional_shape(&v.name),
        c::Atomic::VarWithMod(v) => scope.optional_shape(&qualify_key(&v.mods, &v.name)),
        _ => &[],
    }
}

/// `!x` / `!x#a#b` (`nxunsub`'s `UNOP_EXCLAM nxbot` — parser.mly:795,
/// `let (rng, varnm) = unop in .. UTApply((rng, UTContentOf([], varnm)),
/// utast2)`): the deref operator binds to the atomic head *plus its own
/// `#access` chain* — `nxbot` itself folds `ACCESS` left-recursively
/// (parser.mly:801, `nxbot ACCESS var`), so `nxunsub`'s `utast2` is already
/// the fully-accessed atomic — but never to a *following application
/// argument*: `nxapp`'s only production combining an application head with
/// more arguments is `nxapp nxunsub` (parser.mly:781), so `!x y` parses as
/// `nxapp(nxunsub(!x), y)` = `(!x) y`, not `!(x y)`. This CST's `AppExpr`
/// mirrors that split directly: `excl`+`head_accesses` sit on the *head*
/// only, `args` is the separate, already-folded-in-elaboration application
/// tail — so this helper (used for both an `AppExpr`'s own head and a
/// command-argument-chain's head, see `cmd_arg_chain`) elaborates to
/// `Apply(Var(excl_text), <head+accesses>)` exactly matching v0.0.6's
/// `UTApply` shape above (`varnm` there is always unqualified — this CST has
/// no qualified-`!` form either, so no module-mangling applies to it).
fn atomic_head_with_excl(
    head: &c::Atomic,
    accesses: &[c::AccessSeg],
    excl: Option<&UnopExclamTok>,
    scope: &Scope,
) -> Result<Ast, ElabError> {
    let mut ast = atomic(head, scope)?;
    for acc in accesses {
        ast = Ast::AccessField(Box::new(ast), acc.label.name.clone(), acc.label.span);
    }
    if let Some(e) = excl {
        let deref_fn = scoped_var(&e.text, e.span, scope)?;
        ast = Ast::Apply(Box::new(deref_fn), Box::new(ast));
    }
    Ok(ast)
}

/// Desugar one application-chain argument. `?: value`/`?*` (`AppArg::Optional`/
/// `Omission` — v0.0.6's `UTApplyOptional`/`UTApplyOmission`) desugar
/// *untyped*, straight to the same `option` constructors a program could
/// spell by hand: a supplied `?:(e)` becomes `Some(e)`, an omitted `?*`
/// becomes `None`. This is the one runtime model shared by every optional-arg
/// call site this milestone supports — a plain function's `f ?:(e)`/`f ?*`
/// *and* a command's leading `narg`s (`cst.rs`'s `CmdTail::Args`, whose
/// elements are ALSO `AppArg`s) both go through this same function — so
/// `Some`/`None` are the one encoding a `?->`-typed function's `option`-
/// wrapped domain (`typecheck.rs`'s `lower_type_expr`) must unify against.
/// No type-directed insertion is needed here: every optional slot this
/// grammar can produce carries an explicit `?:`/`?*` marker at the call site
/// (there is no "just omit the argument entirely, no marker at all" form —
/// see `docs/plans/frontend-completion.md` Sub-area 2), so elaboration alone
/// (no typechecker involvement) fully resolves it.
fn app_arg_to_ast(arg: &c::AppArg, scope: &Scope) -> Result<Ast, ElabError> {
    match arg {
        c::AppArg::Optional { value, .. } => {
            let inner = atomic(value, scope)?;
            Ok(Ast::Ctor("Some".to_string(), Some(Box::new(inner))))
        }
        c::AppArg::Omission(_) => Ok(Ast::Ctor("None".to_string(), None)),
        c::AppArg::Atom {
            excl,
            atom,
            accesses,
        } => atomic_head_with_excl(atom, accesses, excl.as_ref(), scope),
        c::AppArg::Ctor(ctor) => Ok(Ast::Ctor(ctor.name.clone(), None)),
        // A `?(l = e, …)` bundle is not a plain argument value — the chain
        // builder (`apply_one_arg`) routes it to `Ast::ApplyOpt` before it
        // ever reaches here. Reaching this arm means a bundle sat where only
        // a value can go (e.g. as a constructor payload).
        c::AppArg::Bundled { opts, .. } | c::AppArg::BundledCtor { opts, .. } => err(
            opts.q.0,
            "a `?(l = e)` labeled-optional bundle cannot be used as a plain \
             argument value here",
        ),
    }
}

/// Shared machinery for `open Name in body` (`Expr::OpenIn`) and `Name.(body)`
/// (`Atomic::OpenModule`, `nxbot`'s `OPENMODULE nxlet RPAREN` production —
/// `Mod.(e)` ≡ `open Mod in e`): bring every `"Name."`-prefixed name
/// currently in scope into unqualified scope, elaborate `body` under that
/// extended scope (via the supplied closure — generic because `Expr::OpenIn`'s
/// body is a plain `Expr` but `Atomic::OpenModule`'s is a `ParenBody`, so
/// `Mod.(e, e, …)` can produce a tuple exactly like `Atomic::Paren`), then
/// wrap the result in one `LetIn` alias per matched name (`x = Name.x`) —
/// there is no separate "module scope" at the `Ast` level, so the aliasing
/// must be visible there too.
fn open_module(
    module_name: &str,
    name_span: Span,
    scope: &Scope,
    body: impl FnOnce(&Scope) -> Result<Ast, ElabError>,
) -> Result<Ast, ElabError> {
    let prefix = format!("{module_name}.");
    let matches = scope.names_with_prefix(&prefix);
    let mut inner = scope.clone();
    for q in &matches {
        let shape = scope.optional_shape(q).to_vec();
        inner.insert_with_shape(&q[prefix.len()..], shape);
    }
    let body_ast = body(&inner)?;
    let mut ast = body_ast;
    for q in matches.into_iter().rev() {
        let suffix = q[prefix.len()..].to_string();
        ast = Ast::LetIn(suffix, Box::new(Ast::Var(q, name_span)), Box::new(ast));
    }
    Ok(ast)
}

fn atomic(a: &c::Atomic, scope: &Scope) -> Result<Ast, ElabError> {
    match a {
        c::Atomic::Length(l) => match Length::from_unit(l.value, &l.unit) {
            Some(len) => Ok(Ast::Length(len)),
            None => err(l.span, format!("unknown length unit '{}'", l.unit)),
        },
        c::Atomic::Float(f) => Ok(Ast::Float(f.value)),
        c::Atomic::Int(i) => Ok(Ast::Int(i.value)),
        c::Atomic::Literal(l) => Ok(Ast::Str(omit_spaces(l.omit_pre, l.omit_post, &l.body))),
        c::Atomic::True(_) => Ok(Ast::Bool(true)),
        c::Atomic::False(_) => Ok(Ast::Bool(false)),
        // A bare nullary constructor reached as a plain atomic argument
        // (the ctor-with-payload case is handled at the `AppExpr` head
        // level in `app_expr`, above; it never reaches here).
        c::Atomic::Ctor(ctor) => Ok(Ast::Ctor(ctor.name.clone(), None)),
        c::Atomic::Var(v) => scoped_var(&v.name, v.span, scope),
        c::Atomic::VarWithMod(tok) => {
            scoped_var(&qualify_key(&tok.mods, &tok.name), tok.span, scope)
        }
        // `(+++)`/`(-->)` — a bare reference to a (possibly user-defined)
        // operator as a first-class value; resolves exactly like `Var`
        // above, under the same name `x +++ y`/`x --> y` (an `OpChain`'s
        // `op_chain`, below) would look up.
        c::Atomic::OpRef(op) => scoped_var(&op.name, op.span, scope),
        // `(command \cmd)` — a first-class reference to an inline command's
        // own binding (`class-signature-lang-gaps.md` gap 1). No new
        // binding machinery: the command's own `let-inline` binding is the
        // referent, so this is just its `Var` under the same sigil'd key
        // `InlineElem::Cmd` resolves — reusing `scoped_var` also gives the
        // usual "unbound command" diagnostic for free.
        c::Atomic::Command { name, .. } => {
            let (key, span) = horz_cmd_key(name);
            scoped_var(&key, span, scope)
        }
        c::Atomic::Unit { .. } => Ok(Ast::Unit),
        c::Atomic::Paren { inner, .. } => paren_body(inner, scope),
        c::Atomic::OpenModule { grp, body } => {
            open_module(&grp.open.name, grp.open.span, scope, |s| paren_body(body, s))
        }
        c::Atomic::Record { body, .. } => record_body_to_ast(body, scope),
        c::Atomic::List { items, .. } => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                out.push(expr(&it.value, scope)?);
            }
            Ok(Ast::List(out))
        }
        c::Atomic::InlineText { elems, .. } => inline_text_ast(elems, scope),
        c::Atomic::BlockText { elems, .. } => {
            Ok(Ast::BlockText(Rc::new(block_elems(elems, scope)?)))
        }
        c::Atomic::MathText { elems, .. } => math_block_ast(elems, scope),
    }
}

/// `( expr )` → itself; `( expr, expr, … )` → `Ast::Tuple`.
fn paren_body(pb: &c::ParenBody, scope: &Scope) -> Result<Ast, ElabError> {
    let first = expr(&pb.first, scope)?;
    if pb.rest.is_empty() {
        Ok(first)
    } else {
        let mut items = Vec::with_capacity(pb.rest.len() + 1);
        items.push(first);
        for r in &pb.rest {
            items.push(expr(&r.value, scope)?);
        }
        Ok(Ast::Tuple(items))
    }
}

/// `(| l = e; … |)` → `Ast::Record`; `(| base with l = e; … |)` → a left
/// fold of `Ast::UpdateField` over `base` (`nxrecordsynt`, parser.mly:
/// 833-840 — `rcd |> List.fold_left (fun utast1 (fldnm, utastF) ->
/// UTUpdateField(utast1, fldnm, utastF)) utast`, i.e. exactly one
/// `UpdateField` per field, left-to-right, threading the accumulator).
fn record_body_to_ast(body: &c::RecordBody, scope: &Scope) -> Result<Ast, ElabError> {
    match body {
        c::RecordBody::Fields(fields) => {
            let mut out = Vec::with_capacity(fields.len());
            for f in fields {
                out.push((f.name.name.clone(), expr(&f.value, scope)?));
            }
            Ok(Ast::Record(out))
        }
        c::RecordBody::Update { base, fields, .. } => {
            let mut ast = expr(base, scope)?;
            for f in fields {
                let v = expr(&f.value, scope)?;
                ast = Ast::UpdateField(Box::new(ast), f.name.name.clone(), Box::new(v));
            }
            Ok(ast)
        }
    }
}

// ---- patterns -------------------------------------------------------------

/// `patas`: a `PatCons`, plus an optional `as name` binding.
fn pattern(p: &c::Pattern) -> Result<Pattern, ElabError> {
    let head = pat_cons(&p.head)?;
    match &p.as_clause {
        Some(ac) => Ok(Pattern::As(Box::new(head), ac.name.name.clone())),
        None => Ok(head),
    }
}

/// `pattr`: `patbot (:: patbot)*`, folded RIGHT (`::` is right-associative):
/// `a :: b :: c` → `Cons(a, Cons(b, c))`.
fn pat_cons(pc: &c::PatCons) -> Result<Pattern, ElabError> {
    let mut segs: Vec<&c::PatBot> = Vec::with_capacity(pc.tail.len() + 1);
    segs.push(&pc.head);
    for seg in &pc.tail {
        segs.push(&seg.tail);
    }
    let mut iter = segs.into_iter().rev();
    let last = iter.next().expect("PatCons always has a head");
    let mut acc = patbot(last)?;
    for pb in iter {
        acc = Pattern::Cons(Box::new(patbot(pb)?), Box::new(acc));
    }
    Ok(acc)
}

fn patbot(pb: &c::PatBot) -> Result<Pattern, ElabError> {
    match pb {
        c::PatBot::CtorApplied { ctor, arg } => {
            Ok(Pattern::Ctor(ctor.name.clone(), Some(Box::new(patbot(arg)?))))
        }
        c::PatBot::Ctor(ctor) => Ok(Pattern::Ctor(ctor.name.clone(), None)),
        c::PatBot::Int(i) => Ok(Pattern::Int(i.value)),
        c::PatBot::True(_) => Ok(Pattern::Bool(true)),
        c::PatBot::False(_) => Ok(Pattern::Bool(false)),
        c::PatBot::Str(l) => Ok(Pattern::Str(l.body.clone())),
        c::PatBot::Wild(_) => Ok(Pattern::Wild),
        c::PatBot::Var(v) => Ok(Pattern::Var(v.name.clone())),
        c::PatBot::Unit { .. } => Ok(Pattern::Unit),
        c::PatBot::Paren { inner, .. } => {
            let first = pattern(&inner.first)?;
            if inner.rest.is_empty() {
                Ok(first)
            } else {
                let mut items = Vec::with_capacity(inner.rest.len() + 1);
                items.push(first);
                for r in &inner.rest {
                    items.push(pattern(&r.value)?);
                }
                Ok(Pattern::Tuple(items))
            }
        }
        c::PatBot::List { items, .. } => {
            let mut acc = Pattern::EmptyList;
            for it in items.iter().rev() {
                acc = Pattern::Cons(Box::new(pattern(&it.value)?), Box::new(acc));
            }
            Ok(acc)
        }
    }
}

/// Collect every name a (lowered) pattern binds — `Var` occurrences plus any
/// `as name` clauses — so the elaborator can extend the scope for a match
/// arm's guard and body.
fn collect_pattern_names(p: &Pattern, out: &mut Vec<String>) {
    match p {
        Pattern::Var(n) => out.push(n.clone()),
        Pattern::As(inner, n) => {
            collect_pattern_names(inner, out);
            out.push(n.clone());
        }
        Pattern::Tuple(ps) => {
            for p in ps {
                collect_pattern_names(p, out);
            }
        }
        Pattern::Cons(head, tail) => {
            collect_pattern_names(head, out);
            collect_pattern_names(tail, out);
        }
        Pattern::Ctor(_, Some(inner)) => collect_pattern_names(inner, out),
        Pattern::Wild
        | Pattern::Unit
        | Pattern::Bool(_)
        | Pattern::Int(_)
        | Pattern::Str(_)
        | Pattern::EmptyList
        | Pattern::Ctor(_, None) => {}
    }
}

fn match_arm(arm: &c::MatchArm, scope: &Scope) -> Result<MatchArm, ElabError> {
    let pat = pattern(&arm.pat)?;
    let mut names = Vec::new();
    collect_pattern_names(&pat, &mut names);
    let mut inner = scope.clone();
    for n in &names {
        inner = inner.with(n);
    }
    let guard = match &arm.guard {
        Some(g) => Some(expr(&g.cond, &inner)?),
        None => None,
    };
    let body = expr(&arm.body, &inner)?;
    Ok(MatchArm { pat, guard, body })
}

// ---- inline/block text ----------------------------------------------------

/// `AnyHorzCmdTok`/`AnyVertCmdTok`'s scope key + span: a plain command uses
/// its own sigil-inclusive name unchanged; a module-qualified one mangles
/// via [`qualify_key`] (see its doc comment on the module name-mangling
/// scheme).
fn horz_cmd_key(name: &AnyHorzCmdTok) -> (String, Span) {
    match name {
        AnyHorzCmdTok::Plain(t) => (t.name.clone(), t.span),
        AnyHorzCmdTok::Mod(t) => (qualify_key(&t.mods, &t.name), t.span),
    }
}

fn vert_cmd_key(name: &AnyVertCmdTok) -> (String, Span) {
    match name {
        AnyVertCmdTok::Plain(t) => (t.name.clone(), t.span),
        AnyVertCmdTok::Mod(t) => (qualify_key(&t.mods, &t.name), t.span),
    }
}

/// [`AnyMathCmdTok`]'s scope key + span — the math-mode analogue of
/// [`horz_cmd_key`]/[`vert_cmd_key`].
fn math_cmd_key(name: &AnyMathCmdTok) -> (String, Span) {
    match name {
        AnyMathCmdTok::Plain(t) => (t.name.clone(), t.span),
        AnyMathCmdTok::Mod(t) => (qualify_key(&t.mods, &t.name), t.span),
    }
}

/// An inline-text group's content (`{ .. }`): itemize-aware entry point.
/// `sxsep`'s two alternatives (parser.mly:1039-1042) are a `nonempty_list`
/// of `*`-headed items (→ `UTItemize`, see [`itemize`]) or plain content;
/// since `InlineElem`'s `ItemBullet` markers are kept flat rather than
/// grouped in-grammar (see `cst.rs`'s doc comment on `InlineElem`), the
/// dispatch happens here instead of in the parser.
fn inline_text_ast(elems: &[c::InlineElem], scope: &Scope) -> Result<Ast, ElabError> {
    // `{| a | b | … |}` horizontal-LIST literal (`sxlist` in parser.mly): a
    // leading `|` (`Sep`) immediately after `{` marks the inline-text-list
    // form — a value of type `inline-text list` — distinct from a plain
    // `{ … }` inline text. The elements arrive flat and `Sep`-delimited (see
    // `InlineElem`'s doc comment); regroup them into one inline-text per cell.
    if matches!(elems.first(), Some(c::InlineElem::Sep(_))) {
        return inline_text_list(elems, scope);
    }
    if elems.iter().any(|e| matches!(e, c::InlineElem::ItemBullet(_))) {
        itemize(elems, scope)
    } else {
        Ok(Ast::InlineText(Rc::new(inline_elems(elems, scope)?)))
    }
}

/// Regroup a `{| a | b | … |}` inline-text-list literal's flat, `Sep`-delimited
/// elements (see [`inline_text_ast`]) into an [`Ast::List`] of one inline-text
/// per cell. The leading/trailing empty groups produced by the framing `{|`
/// and `|}` are structural and dropped; an interior empty group is a real
/// empty cell (`{| a | | b |}`). Each cell is elaborated recursively, so a
/// cell may itself be an itemize (`{* … }`).
fn inline_text_list(elems: &[c::InlineElem], scope: &Scope) -> Result<Ast, ElabError> {
    let mut groups: Vec<&[c::InlineElem]> = Vec::new();
    let mut start = 0usize;
    for (i, e) in elems.iter().enumerate() {
        if matches!(e, c::InlineElem::Sep(_)) {
            groups.push(&elems[start..i]);
            start = i + 1;
        }
    }
    groups.push(&elems[start..]);
    if groups.first().is_some_and(|g| g.is_empty()) {
        groups.remove(0);
    }
    if groups.last().is_some_and(|g| g.is_empty()) {
        groups.pop();
    }
    let mut items = Vec::with_capacity(groups.len());
    for g in groups {
        items.push(inline_text_ast(g, scope)?);
    }
    Ok(Ast::List(items))
}

/// Coalesce chars/spaces/breaks into text runs; commands become
/// `IText::Cmd`; `#var;` embeds become `IText::Embed`; `${..}` embeds become
/// `IText::EmbedMath`. Never sees an `ItemBullet` in a well-formed call (the
/// itemize splitter in [`itemize`] always calls this on a bullet-free
/// slice) — one showing up here is reported as an error rather than
/// panicking, since a defensive diagnostic is friendlier than a panic even
/// though the shape should be unreachable.
fn inline_elems(elems: &[c::InlineElem], scope: &Scope) -> Result<Vec<IText>, ElabError> {
    let mut out = Vec::new();
    let mut text = String::new();
    for el in elems {
        match el {
            c::InlineElem::Char(ch) => text.push_str(&ch.text),
            c::InlineElem::Space(_) => text.push(' '),
            c::InlineElem::Break(_) => text.push('\n'),
            c::InlineElem::Cmd { name, tail } => {
                if !text.is_empty() {
                    out.push(IText::Text(std::mem::take(&mut text)));
                }
                let (key, span) = horz_cmd_key(name);
                if !scope.contains(&key) {
                    return err(span, format!("unbound inline command '{key}'"));
                }
                let leading = scope.optional_arity(&key);
                out.push(IText::Cmd {
                    name: scope.resolve(&key),
                    span,
                    args: cmd_args(tail, scope, leading)?,
                });
            }
            c::InlineElem::Embed { var, .. } => {
                if !text.is_empty() {
                    out.push(IText::Text(std::mem::take(&mut text)));
                }
                let key = qualify_key(&var.mods, &var.name);
                if !scope.contains(&key) {
                    return err(var.span, format!("unbound variable '{key}'"));
                }
                out.push(IText::Embed {
                    expr: Ast::Var(scope.resolve(&key), var.span),
                    span: var.span,
                });
            }
            c::InlineElem::EmbedMath { mgrp, elems } => {
                if !text.is_empty() {
                    out.push(IText::Text(std::mem::take(&mut text)));
                }
                if let Some(first) = elems.first() {
                    if let c::MathBot::Sep(tok) = &first.base {
                        return err(tok.0, "a '|'-separated math list cannot be embedded directly in inline text: `${| … |}` here would be a `math list`, but an embedded formula must be a single `math`");
                    }
                }
                let span = mgrp.open.0.unite(mgrp.close.0);
                out.push(IText::EmbedMath {
                    elems: Rc::new(lower_math_elems(elems, scope)?),
                    span,
                });
            }
            c::InlineElem::ItemBullet(tok) => {
                return err(
                    tok.span,
                    "unexpected itemize bullet '*' outside a bullet list",
                );
            }
            c::InlineElem::Sep(tok) => {
                return err(tok.0, "'|' separator is not supported here yet");
            }
        }
    }
    if !text.is_empty() {
        out.push(IText::Text(text));
    }
    Ok(out)
}

fn block_elems(elems: &[c::BlockElem], scope: &Scope) -> Result<Vec<BText>, ElabError> {
    let mut out = Vec::with_capacity(elems.len());
    for el in elems {
        match el {
            c::BlockElem::Cmd { name, tail } => {
                let (key, span) = vert_cmd_key(name);
                if !scope.contains(&key) {
                    return err(span, format!("unbound block command '{key}'"));
                }
                let leading = scope.optional_arity(&key);
                out.push(BText::Cmd {
                    name: scope.resolve(&key),
                    span,
                    args: cmd_args(tail, scope, leading)?,
                });
            }
            c::BlockElem::Embed { var, .. } => {
                let key = qualify_key(&var.mods, &var.name);
                if !scope.contains(&key) {
                    return err(var.span, format!("unbound variable '{key}'"));
                }
                out.push(BText::Embed {
                    expr: Ast::Var(scope.resolve(&key), var.span),
                    span: var.span,
                });
            }
        }
    }
    Ok(out)
}

/// Flatten a command tail back into its argument list. `CmdTail::Args` is a
/// flat, non-empty `AppArg` sequence (`cst.rs`'s own dedicated grammar, not a
/// reuse of the general application chain — see that type's doc comment), so
/// this is just `cmd_arg_to_ast` per element; a supplied/omitted optional
/// (`?:`/`?*`) desugars to `Some`/`None` exactly like a plain function's
/// optional application (`app_arg_to_ast`'s doc comment) — the one place this
/// port's optional-arg call-site model is shared between commands and plain
/// functions. A `?(l = e, …)`-bundled element (optional-arg-rows increment
/// 3b-β) carries its labels on the returned [`CmdArg`]'s `opts` instead of
/// desugaring to `Some`/`None` — the 0.0.6 leading-padding loop below only
/// ever matches `Optional`/`Omission` (the `?:`/`?*` marker), never
/// `Bundled`/`BundledCtor`, and a V0_1 command's `leading` is always `0`
/// (`param_optional_shape` only marks `Param::Optional` positions, and
/// `Scope::optional_arity` only ever reads the *leading* run of such
/// positions) — so the two mechanisms never interact.
fn cmd_args(tail: &c::CmdTail, scope: &Scope, leading: usize) -> Result<Vec<CmdArg>, ElabError> {
    let args: Vec<&c::AppArg> = match tail {
        c::CmdTail::Semi(_) => Vec::new(),
        c::CmdTail::Args { first, rest, .. } => {
            let mut v: Vec<&c::AppArg> = Vec::with_capacity(1 + rest.len());
            v.push(first);
            for a in rest {
                v.push(a);
            }
            v
        }
    };
    let mut out = Vec::with_capacity(args.len().max(leading));
    let mut args_iter = args.into_iter().peekable();
    let mut supplied = 0;
    while supplied < leading {
        match args_iter.peek() {
            Some(c::AppArg::Optional { .. }) | Some(c::AppArg::Omission(_)) => {
                out.push(CmdArg {
                    opts: Vec::new(),
                    arg: app_arg_to_ast(args_iter.next().unwrap(), scope)?,
                });
                supplied += 1;
            }
            _ => break,
        }
    }
    for _ in supplied..leading {
        out.push(CmdArg {
            opts: Vec::new(),
            arg: Ast::Ctor("None".to_string(), None),
        });
    }
    for a in args_iter {
        out.push(cmd_arg_to_ast(a, scope)?);
    }
    Ok(out)
}

/// One command-application argument, the `?(l = e, …)`-bundle-aware twin of
/// `app_arg_to_ast` (optional-arg-rows increment 3b-β): a `Bundled`/
/// `BundledCtor` arg becomes a [`CmdArg`] whose `opts` carries the labeled
/// optionals — elaborated via `elaborate_opt_args`, exactly like a plain
/// function application's `f ?(l = e) x` (`apply_one_arg`'s `Ast::ApplyOpt`
/// arm); every other `AppArg` shape becomes a `CmdArg` with empty `opts` (the
/// unbundled call — the ONLY shape any pre-3b-β producer, and every
/// 0.0.6-reachable call, ever emits).
fn cmd_arg_to_ast(arg: &c::AppArg, scope: &Scope) -> Result<CmdArg, ElabError> {
    match arg {
        c::AppArg::Bundled {
            opts,
            excl,
            atom,
            accesses,
        } => Ok(CmdArg {
            opts: elaborate_opt_args(opts, scope)?,
            arg: atomic_head_with_excl(atom, accesses, excl.as_ref(), scope)?,
        }),
        c::AppArg::BundledCtor { opts, ctor } => Ok(CmdArg {
            opts: elaborate_opt_args(opts, scope)?,
            arg: Ast::Ctor(ctor.name.clone(), None),
        }),
        _ => Ok(CmdArg {
            opts: Vec::new(),
            arg: app_arg_to_ast(arg, scope)?,
        }),
    }
}

// ---- itemize ---------------------------------------------------------------

/// One node of the itemize tree being built, before it is lowered to the
/// `Ctor("Item", ..)` value shape by [`item_node_to_ast`].
struct ItemNode {
    text: Ast,
    children: Vec<ItemNode>,
}

fn inline_elem_span(el: &c::InlineElem) -> Span {
    match el {
        c::InlineElem::Char(t) => t.span,
        c::InlineElem::Space(t) => t.0,
        c::InlineElem::Break(t) => t.0,
        c::InlineElem::Embed { var, .. } => var.span,
        c::InlineElem::EmbedMath { mgrp, .. } => mgrp.open.0.unite(mgrp.close.0),
        c::InlineElem::Cmd { name, .. } => horz_cmd_key(name).1,
        c::InlineElem::ItemBullet(t) => t.span,
        c::InlineElem::Sep(t) => t.0,
    }
}

/// Consecutive `ItemBullet`-headed runs of an inline-text group elaborate to
/// a single itemize `Ctor("Item", (text, list))` tree instead of plain
/// `InlineText` — transcribed from `parser.mly`'s `make_list_to_itemize`/
/// `insert_last` (lines 331-356) and `typecheck_itemize`/`typecheck_itemize_list`
/// (typechecker.ml:1359-1374, which lower each `UTItem(utast1, utitmzlst)`
/// node to `NonValueConstructor("Item", PrimitiveTuple([e1; e2]))` — the
/// `Item` constructor's `(inline-text * itemize list)` payload shape from
/// `primitives.cppo.ml:159`).
fn itemize(elems: &[c::InlineElem], scope: &Scope) -> Result<Ast, ElabError> {
    // `sxsep`'s itemize alternative is `nonempty_list(sxitem)` (parser.mly:
    // 1042) — i.e. the *whole* group must be bullets-and-their-content, no
    // leading plain text before the first bullet.
    let mut i = 0;
    while i < elems.len() && !matches!(elems[i], c::InlineElem::ItemBullet(_)) {
        i += 1;
    }
    if i != 0 {
        return err(
            inline_elem_span(&elems[0]),
            "content before the first itemize bullet '*' is not supported",
        );
    }
    let mut segments: Vec<(usize, Span, &[c::InlineElem])> = Vec::new();
    while i < elems.len() {
        let (depth, span) = match &elems[i] {
            c::InlineElem::ItemBullet(tok) => (tok.depth, tok.span),
            _ => unreachable!("loop invariant: elems[i] is always an ItemBullet here"),
        };
        let start = i + 1;
        let mut j = start;
        while j < elems.len() && !matches!(elems[j], c::InlineElem::ItemBullet(_)) {
            j += 1;
        }
        segments.push((depth, span, &elems[start..j]));
        i = j;
    }
    // `make_list_to_itemize_sub`'s accumulator starts as a dummy root item
    // with empty inline text and no children (parser.mly:332,
    // `UTItem((.., UTInputHorz([])), [])`).
    let mut root = ItemNode {
        text: Ast::InlineText(Rc::new(Vec::new())),
        children: Vec::new(),
    };
    let mut crrntdp = 0usize;
    for (depth, span, content) in segments {
        if depth > crrntdp + 1 {
            return err(span, format!("illegal item depth {depth} after {crrntdp}"));
        }
        let text_ast = Ast::InlineText(Rc::new(inline_elems(content, scope)?));
        insert_last(&mut root, 1, depth, text_ast);
        crrntdp = depth;
    }
    Ok(item_node_to_ast(root))
}

/// `insert_last` (parser.mly:346-356), simplified: the OCaml version rebuilds
/// an immutable list by peeling `hditmz :: tlitmzlst` heads into an
/// accumulator until exactly one child remains, then either recurses into it
/// (if not yet at the target depth) or appends a new sibling after it —
/// which is equivalent (and much simpler to transcribe with a mutable tree)
/// to just always operating on `node.children`'s *last* element: recurse
/// into it while `i < depth`, otherwise push a new sibling leaf.
fn insert_last(node: &mut ItemNode, i: usize, depth: usize, new_text: Ast) {
    if node.children.is_empty() {
        node.children.push(ItemNode {
            text: new_text,
            children: Vec::new(),
        });
        return;
    }
    if i < depth {
        insert_last(node.children.last_mut().unwrap(), i + 1, depth, new_text);
    } else {
        node.children.push(ItemNode {
            text: new_text,
            children: Vec::new(),
        });
    }
}

fn item_node_to_ast(node: ItemNode) -> Ast {
    let children = Ast::List(node.children.into_iter().map(item_node_to_ast).collect());
    Ast::Ctor(
        "Item".to_string(),
        Some(Box::new(Ast::Tuple(vec![node.text, children]))),
    )
}

// ---- quoted math ------------------------------------------------------------

fn lower_math_elems(elems: &[cst::MathErased], scope: &Scope) -> Result<Vec<MathElem>, ElabError> {
    elems.iter().map(|e| math_elem_cst(e, scope)).collect()
}

/// `mathblock` (parser.mly:1059-1066): a LEADING `|` puts the math area in
/// list mode — `${| m | m |}` is upstream-desugared in-grammar to an
/// ordinary list literal of `math` values (make_cons over UTMath; there is
/// NO matrix/grid node anywhere in the frontend or math backend) —
/// otherwise the area is one plain `math` (today's single-MathText path).
/// Edge cases replicated exactly: list mode triggers only on a LEADING `|`
/// (`${a|b}` is upstream a parse error); the trailing `|` is mandatory
/// (`${|a|b}` rejected); `${|}` = empty list, `${||}` = one empty cell;
/// `|` never carries scripts. Split is over the flat erased stream so the
/// sibling inline `{| … |}` (sxsep) can reuse it later.
fn math_block_ast(elems: &[cst::MathErased], scope: &Scope) -> Result<Ast, ElabError> {
    let leading_sep = matches!(elems.first(), Some(e) if matches!(&e.base, c::MathBot::Sep(_)));
    if !leading_sep {
        return Ok(Ast::MathText(Rc::new(lower_math_elems(elems, scope)?)));
    }
    for e in elems {
        if let c::MathBot::Sep(tok) = &e.base {
            if !e.scripts.is_empty() {
                return err(tok.0, "a '|' math-list separator cannot carry a script ('^'/'_'/primes)");
            }
        }
    }
    if !matches!(elems.last(), Some(e) if matches!(&e.base, c::MathBot::Sep(_))) {
        let c::MathBot::Sep(first) = &elems[0].base else { unreachable!() };
        return err(first.0, "a '|'-separated math list must end with a trailing '|' (write `${| a | b |}`)");
    }
    let mut segments: Vec<Ast> = Vec::new();
    let mut seg_start = 1usize;
    for (i, e) in elems.iter().enumerate().skip(1) {
        if matches!(&e.base, c::MathBot::Sep(_)) {
            let seg = &elems[seg_start..i];
            segments.push(Ast::MathText(Rc::new(lower_math_elems(seg, scope)?)));
            seg_start = i + 1;
        }
    }
    Ok(Ast::List(segments))
}

fn math_elem_cst(m: &c::MathElemCst, scope: &Scope) -> Result<MathElem, ElabError> {
    let base = math_bot(&m.base, scope)?;
    fold_math_scripts(base, &m.scripts, scope)
}

fn math_bot(b: &c::MathBot, scope: &Scope) -> Result<MathElem, ElabError> {
    match b {
        c::MathBot::Cmd { name, args } => {
            let (key, span) = math_cmd_key(name);
            if !scope.contains(&key) {
                return err(span, format!("unbound math command '{key}'"));
            }
            // Marker-less optional defaulting — the command mirror of
            // app_chain_generic (upstream typecheck_command_arguments skips
            // optional slots left unmarked). No non-empty-args guard: a
            // MathElem::Cmd is always an application (a bare command VALUE is
            // `command \cmd`), so `${\cmd}` with `[t?] math-cmd` pads too.
            let leading = scope.optional_arity(&key);
            let mut arg_asts = Vec::with_capacity(args.len().max(leading));
            let mut args_iter = args.iter().peekable();
            let mut supplied = 0;
            while supplied < leading {
                match args_iter.peek() {
                    Some(c::MathArg::Optional { .. }) | Some(c::MathArg::Omission(_)) => {
                        arg_asts.push(math_arg_to_ast(args_iter.next().unwrap(), scope)?);
                        supplied += 1;
                    }
                    _ => break,
                }
            }
            for _ in supplied..leading {
                arg_asts.push(Ast::Ctor("None".to_string(), None));
            }
            for a in args_iter {
                arg_asts.push(math_arg_to_ast(a, scope)?);
            }
            // `CmdArg`-shaped for uniformity with `IText::Cmd`/`BText::Cmd`
            // (optional-arg-rows increment 3b-β; see `MathElem::Cmd`'s doc
            // comment) — `opts` is always empty: the math-mode application
            // grammar (`c::MathArg`) has no `?(l=e)` bundle form at all.
            Ok(MathElem::Cmd {
                name: scope.resolve(&key),
                span,
                args: arg_asts
                    .into_iter()
                    .map(|arg| CmdArg { opts: Vec::new(), arg })
                    .collect(),
            })
        }
        c::MathBot::Chars(tok) => Ok(MathElem::Chars(tok.text.clone())),
        c::MathBot::Embed(tok) => {
            // Math commands are qualified via `math_cmd_key` the same way
            // `horz_cmd_key`/`vert_cmd_key` handle `\Mod.cmd`; `#var`/
            // `#Mod.var` embeds already carry a `mods` list (`VarInMathTok`),
            // so those are mangled the same as everywhere else.
            let key = qualify_key(&tok.mods, &tok.name);
            if !scope.contains(&key) {
                return err(tok.span, format!("unbound variable '{key}'"));
            }
            Ok(MathElem::Embed {
                expr: Ast::Var(scope.resolve(&key), tok.span),
                span: tok.span,
            })
        }
        c::MathBot::Sep(tok) => err(tok.0, "'|' builds a math list and may only be used when the math area starts with '|' (e.g. `${| a | b |}`); it cannot appear mid-formula or inside a `{ … }` math group"),
        c::MathBot::Group { elems, .. } => Ok(MathElem::Group(lower_math_elems(elems, scope)?)),
    }
}

fn math_group_arg(g: &c::MathGroupArg, scope: &Scope) -> Result<Vec<MathElem>, ElabError> {
    match g {
        c::MathGroupArg::Group { elems, .. } => lower_math_elems(elems, scope),
        c::MathGroupArg::Bot(b) => Ok(vec![math_bot(b, scope)?]),
    }
}

/// `mathtop`'s seven script-combo alternatives (parser.mly:1078-1116),
/// folded left over `MathElemCst`'s flat `scripts` vector (see its doc
/// comment in `cst.rs`). Combos with only a subscript, only a superscript,
/// or a subscript+superscript pair (in either written order) are
/// transcribed exactly: whichever token is spelled `SUBSCRIPT` always
/// becomes the inner `Sub` operand and whichever is spelled `SUPERSCRIPT`
/// always becomes the outer `Sup` operand, regardless of which came first in
/// the source (parser.mly's rules 3 and 5 both produce
/// `Sup(Sub(base,subgrp),supgrp)`).
///
/// **Deviation for `PRIMES` combined with an explicit script** (parser.mly's
/// rules 4 and 6): v0.0.6 encodes primes-plus-script by *reusing* the
/// `UTMSubScript`/`UTMSuperScript` nodes as an internal slot-assignment
/// trick so a single later rendering routine can lay out the prime mark and
/// the real script together in one corner-glyph slot (rule 4 makes the
/// primes the `Sub` operand and the explicit `^group` the outer `Sup`; rule
/// 6 makes the explicit `_group` the `Sub` operand and the primes the outer
/// `Sup` — i.e. an explicit script and primes swap which slot they land in
/// depending on which was explicit). This port's `MathElem` has a *distinct*
/// `Primes(base, count)` node with no v0.0.6 counterpart, so there is no
/// equivalent slot to reuse; primes are instead folded in as their own step
/// (`Primes` wraps the running accumulator) and any immediately-following
/// explicit script then applies on top of *that*, in source order. This
/// carries the same information (which script, which count, and that they
/// apply to the same base) without replicating the internal rendering hack,
/// which has no meaning yet anyway (typesetting is deferred to phase 7).
fn fold_math_scripts(
    base: MathElem,
    scripts: &[c::MathScript],
    scope: &Scope,
) -> Result<MathElem, ElabError> {
    let mut acc = base;
    let mut i = 0;
    while i < scripts.len() {
        match &scripts[i] {
            c::MathScript::Sub { group, .. } => {
                if let Some(c::MathScript::Super { group: g2, .. }) = scripts.get(i + 1) {
                    let subg = math_group_arg(group, scope)?;
                    let supg = math_group_arg(g2, scope)?;
                    acc = MathElem::Sup(Box::new(MathElem::Sub(Box::new(acc), subg)), supg);
                    i += 2;
                } else {
                    let subg = math_group_arg(group, scope)?;
                    acc = MathElem::Sub(Box::new(acc), subg);
                    i += 1;
                }
            }
            c::MathScript::Super { group, .. } => {
                if let Some(c::MathScript::Sub { group: g2, .. }) = scripts.get(i + 1) {
                    let supg = math_group_arg(group, scope)?;
                    let subg = math_group_arg(g2, scope)?;
                    acc = MathElem::Sup(Box::new(MathElem::Sub(Box::new(acc), subg)), supg);
                    i += 2;
                } else {
                    let supg = math_group_arg(group, scope)?;
                    acc = MathElem::Sup(Box::new(acc), supg);
                    i += 1;
                }
            }
            c::MathScript::Primes(tok) => {
                acc = MathElem::Primes(Box::new(acc), tok.count);
                i += 1;
            }
        }
    }
    Ok(acc)
}

/// `matharg` (parser.mly:1138-1146 + narg 1201-1210): `?:`-supplied desugars
/// to `Some(<body>)`, `?*` to `None` — the math-command mirror of
/// `app_arg_to_ast`'s `AppArg::Optional`/`Omission` arms. A mandatory
/// (`Plain`) argument elaborates its body directly with no wrapping.
fn math_arg_to_ast(arg: &c::MathArg, scope: &Scope) -> Result<Ast, ElabError> {
    match arg {
        c::MathArg::Plain(body) => math_arg_body_to_ast(body, scope),
        c::MathArg::Optional { body, .. } => Ok(Ast::Ctor(
            "Some".to_string(),
            Some(Box::new(math_arg_body_to_ast(body, scope)?)),
        )),
        c::MathArg::Omission(_) => Ok(Ast::Ctor("None".to_string(), None)),
    }
}

/// The six `matharg` body shapes (`cst.rs`'s `MathArgBody` doc comment):
/// recurse into math (`Math`), program-mode escapes (`!(..)`/`![..]`/
/// `!(|..|)`, elaborated exactly like their `Atomic`/`Expr` counterparts), or
/// inline/block text escapes (`!{..}`/`!<..>`, elaborated to
/// `InlineText`/`BlockText` Asts).
fn math_arg_body_to_ast(body: &c::MathArgBody, scope: &Scope) -> Result<Ast, ElabError> {
    match body {
        c::MathArgBody::Math { elems, .. } => math_block_ast(elems, scope),
        c::MathArgBody::Inline { elems, .. } => inline_text_ast(elems, scope),
        c::MathArgBody::Block { elems, .. } => Ok(Ast::BlockText(Rc::new(block_elems(elems, scope)?))),
        c::MathArgBody::ParenEscape { inner, .. } => paren_body(inner, scope),
        c::MathArgBody::ListEscape { items, .. } => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                out.push(expr(&it.value, scope)?);
            }
            Ok(Ast::List(out))
        }
        c::MathArgBody::RecordEscape { body, .. } => record_body_to_ast(body, scope),
    }
}

// ---- string-literal space omission -----------------------------------------

/// `omit_spaces`/`omit_pre_spaces`/`omit_post_spaces`/`min_indent_space`/
/// `shave_indent` (parser.mly's header section, lines 72-152), transcribed
/// faithfully (byte-for-byte algorithm, but over `char`s rather than bytes so
/// it stays correct on non-ASCII source text — the original indexes
/// `String.length`/`String.sub` byte-wise, which coincides with `char`-wise
/// indexing everywhere the original relies on it, since it only ever tests
/// for `' '`/`'\n'`, both single-byte in UTF-8).
fn omit_spaces(omit_pre: bool, omit_post: bool, raw: &str) -> String {
    let s1 = if omit_pre {
        omit_pre_spaces(raw)
    } else {
        raw.to_string()
    };
    let s2 = if omit_post {
        omit_post_spaces(&s1)
    } else {
        s1
    };
    let min_indent = min_indent_space(&s2);
    let shaved = shave_indent(&s2, min_indent);
    let mut chars: Vec<char> = shaved.chars().collect();
    if chars.last() == Some(&'\n') {
        chars.pop();
    }
    chars.into_iter().collect()
}

/// Strip every leading `' '` (not `'\n'` or other whitespace).
fn omit_pre_spaces(s: &str) -> String {
    s.trim_start_matches(' ').to_string()
}

/// Strip trailing `' '`s; once a `'\n'` is reached, strip that single
/// newline and stop (no further recursion past it).
fn omit_post_spaces(s: &str) -> String {
    let mut chars: Vec<char> = s.chars().collect();
    loop {
        match chars.last() {
            Some(' ') => {
                chars.pop();
            }
            Some('\n') => {
                chars.pop();
                break;
            }
            _ => break,
        }
    }
    chars.into_iter().collect()
}

/// The minimum leading-space count of every line (including the very first,
/// since `min_indent_space_sub`'s initial state is `ReadingSpace`, not
/// `Normal` — so unlike every *subsequent* line, the first line's leading
/// spaces count even without a preceding `'\n'`). A line consisting only of
/// spaces does not update the minimum ("does not take space-only line into
/// account").
fn min_indent_space(s: &str) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut reading_space = true;
    let mut spnum = 0usize;
    let mut minspnum = chars.len();
    for ch in chars {
        if reading_space {
            match ch {
                ' ' => spnum += 1,
                '\n' => spnum = 0,
                _ => {
                    if spnum < minspnum {
                        minspnum = spnum;
                    }
                    reading_space = false;
                }
            }
        } else if ch == '\n' {
            reading_space = true;
            spnum = 0;
        }
    }
    minspnum
}

/// Cut `minspnum` leading spaces off every line.
fn shave_indent(s: &str, minspnum: usize) -> String {
    let mut out = String::new();
    let mut reading_space = false;
    let mut spnum = 0usize;
    for ch in s.chars() {
        if reading_space {
            match ch {
                ' ' => {
                    if spnum >= minspnum {
                        out.push(' ');
                    }
                    spnum += 1;
                }
                '\n' => {
                    out.push('\n');
                    spnum = 0;
                }
                _ => {
                    out.push(ch);
                    reading_space = false;
                }
            }
        } else if ch == '\n' {
            out.push('\n');
            reading_space = true;
            spnum = 0;
        } else {
            out.push(ch);
        }
    }
    out
}

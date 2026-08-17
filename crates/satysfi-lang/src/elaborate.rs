//! Surface CST → `Ast` elaboration. Does scope resolution, operator-
//! precedence/associativity resolution (the CST leaves that flattened, see
//! `cst.rs`'s module doc comment), pattern lowering, the `let-inline`/
//! `let-block` context-argument desugaring, mutable/`while`/`before`
//! desugaring, field access/record-update folding, itemize-tree
//! reconstruction, quoted-math lowering, and (untyped) module name-mangling.
//! This function's signature is the seam where the phase-3 typechecker
//! (typechecker.ml / unification.ml port) slots in.

use crate::ast::{Ast, BText, IText, MatchArm, MathElem, Pattern};
use satysfi_backend::Length;
use satysfi_syntax::cst::{self, ast as c};
use satysfi_syntax::leaf::{AnyHorzCmdTok, AnyVertCmdTok, UnopExclamTok, VarTok};
use satysfi_syntax::span::Span;
use satysfi_syntax::token::Token;
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
#[derive(Clone, Debug, Default)]
pub struct Scope {
    names: HashSet<String>,
}

impl Scope {
    pub fn new(names: impl IntoIterator<Item = String>) -> Scope {
        Scope {
            names: names.into_iter().collect(),
        }
    }

    fn with(&self, name: &str) -> Scope {
        let mut s = self.clone();
        s.names.insert(name.to_string());
        s
    }

    /// In-place version of [`Scope::with`], for the folds below that thread
    /// one evolving scope through a sequence of bindings without cloning at
    /// every step.
    fn insert(&mut self, name: &str) {
        self.names.insert(name.to_string());
    }

    fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
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
/// are all resolved the same way as user variables).
fn scoped_var(name: &str, span: Span, scope: &Scope) -> Result<Ast, ElabError> {
    if scope.contains(name) {
        Ok(Ast::Var(name.to_string(), span))
    } else {
        err(span, format!("unbound variable '{name}'"))
    }
}

/// Elaborate a whole file into one expression.
///
/// **Library files.** `File.body` is `None` for a bare `prelude EOI` file (a
/// `.satyh` library with no document expression) — a separate loader crate
/// is responsible for merging a library's `prelude` into a document file's
/// before this function ever sees it, so unlike phase-1/2a there is no
/// "top-level bindings must be followed by `in`" check here at all: by the
/// time `elaborate` runs, either `body` is present (an ordinary document, or
/// an already-merged file) or it is a genuine library file, which is a
/// (clean) error to hand to `elaborate` directly.
pub fn elaborate(file: &cst::File, prelude_scope: &Scope) -> Result<Ast, ElabError> {
    let Some(body) = &file.body else {
        return err(
            Span::default(),
            "this file has no document expression - it is a library file",
        );
    };
    let items: Vec<&cst::TopBinding> = file.prelude.iter().collect();
    let (bindings, exported) = walk_bindings(&items, prelude_scope, &[])?;
    let mut final_scope = prelude_scope.clone();
    for name in &exported {
        final_scope.insert(name);
    }
    let body_ast = expr(body, &final_scope)?;
    Ok(nest(bindings, body_ast))
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

/// One step of the top-level/struct-decl fold, deferred (see [`nest`]) so
/// that folding in a `module`'s declarations doesn't require building the
/// "rest of the program" before the module's own bindings are known.
enum Binding {
    Let(String, Ast),
    LetRec(Vec<(String, Rc<Ast>)>),
    LetMutable(String, Ast),
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
fn export_alias(
    mod_path: &[String],
    local: String,
    bindings: &mut Vec<Binding>,
    running: &mut Scope,
    exported: &mut Vec<String>,
) {
    running.insert(&local);
    if mod_path.is_empty() {
        exported.push(local);
    } else {
        let qual = qualify_key(mod_path, &local);
        bindings.push(Binding::Let(qual.clone(), Ast::Var(local, Span::default())));
        running.insert(&qual);
        exported.push(qual);
    }
}

fn push_named_binding(
    mod_path: &[String],
    local: String,
    value: Ast,
    make_binding: impl FnOnce(String, Ast) -> Binding,
    bindings: &mut Vec<Binding>,
    running: &mut Scope,
    exported: &mut Vec<String>,
) {
    bindings.push(make_binding(local.clone(), value));
    export_alias(mod_path, local, bindings, running, exported);
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
/// level).
fn walk_bindings(
    items: &[&cst::TopBinding],
    scope: &Scope,
    mod_path: &[String],
) -> Result<(Vec<Binding>, Vec<String>), ElabError> {
    let mut bindings: Vec<Binding> = Vec::new();
    let mut running = scope.clone();
    let mut exported: Vec<String> = Vec::new();
    for top in items {
        match top {
            cst::TopBinding::Let(top_let) => {
                let mut inner = running.clone();
                for p in &top_let.params {
                    inner = inner.with(&p.name);
                }
                let mut value = expr(&top_let.value, &inner)?;
                for p in top_let.params.iter().rev() {
                    value = Ast::Lambda(p.name.clone(), Rc::new(value));
                }
                push_named_binding(
                    mod_path,
                    top_let.name.name.clone(),
                    value,
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
                bindings.push(Binding::LetRec(recs));
                for n in names {
                    export_alias(mod_path, n, &mut bindings, &mut running, &mut exported);
                }
            }
            cst::TopBinding::LetInline {
                ctx, cmd, params, value, ..
            } => {
                let value_ast =
                    elaborate_let_inline(ctx.as_ref(), params, value, &running, "read-inline")?;
                push_named_binding(
                    mod_path,
                    cmd.name.clone(),
                    value_ast,
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
                push_named_binding(
                    mod_path,
                    cmd.name.clone(),
                    value_ast,
                    Binding::Let,
                    &mut bindings,
                    &mut running,
                    &mut exported,
                );
            }
            // `type` declarations have no runtime effect in this untyped
            // elaborator: constructors are bare `Ctor` atoms and are never
            // scope-checked, so no scope entry (qualified or not) is needed.
            cst::TopBinding::Type(_) => {}
            cst::TopBinding::LetMutable { name, value, .. } => {
                let value_ast = expr(value, &running)?;
                push_named_binding(
                    mod_path,
                    name.name.clone(),
                    value_ast,
                    Binding::LetMutable,
                    &mut bindings,
                    &mut running,
                    &mut exported,
                );
            }
            cst::TopBinding::Module { name, decls, .. } => {
                // Signature annotations (`sig .. end`) are accepted and
                // ignored: this elaborator does no type checking, so there
                // is nothing yet to check them against (enforcement is
                // phase 3's job).
                let mut child_path = mod_path.to_vec();
                child_path.push(name.name.clone());
                let inner_items: Vec<&cst::TopBinding> =
                    decls.iter().map(|d| d.0.as_ref()).collect();
                let (inner_bindings, inner_exported) =
                    walk_bindings(&inner_items, &running, &child_path)?;
                bindings.extend(inner_bindings);
                for q in &inner_exported {
                    running.insert(q);
                }
                exported.extend(inner_exported);
            }
            cst::TopBinding::Open { name, .. } => {
                let prefix = format!("{}.", name.name);
                for q in running.names_with_prefix(&prefix) {
                    let suffix = q[prefix.len()..].to_string();
                    bindings.push(Binding::Let(suffix.clone(), Ast::Var(q, Span::default())));
                    running.insert(&suffix);
                    // `open` only re-exposes an *existing* qualified name
                    // under its bare suffix locally; it doesn't itself mint
                    // a new qualified name, so nothing goes into `exported`
                    // here.
                }
            }
        }
    }
    Ok((bindings, exported))
}

/// `[ctxvar] let-inline \cmd param* = value` / `[ctxvar] let-block +cmd
/// param* = value` (`nxhorzdec`/`nxvertdec` in `parser.mly`, lines 548-577).
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
    params: &[VarTok],
    value: &c::Expr,
    scope: &Scope,
    reader: &str,
) -> Result<Ast, ElabError> {
    match ctx {
        Some(ctxvar) => {
            let mut inner = scope.with(&ctxvar.name);
            for p in params {
                inner = inner.with(&p.name);
            }
            let mut value_ast = expr(value, &inner)?;
            for p in params.iter().rev() {
                value_ast = Ast::Lambda(p.name.clone(), Rc::new(value_ast));
            }
            Ok(Ast::Lambda(ctxvar.name.clone(), Rc::new(value_ast)))
        }
        None => {
            const IMPLICIT_CTX: &str = "%context";
            let dummy = Span::default();
            let mut inner = scope.with(IMPLICIT_CTX);
            for p in params {
                inner = inner.with(&p.name);
            }
            let value_ast = expr(value, &inner)?;
            let read_fn = scoped_var(reader, dummy, &inner)?;
            let ctx_var = scoped_var(IMPLICIT_CTX, dummy, &inner)?;
            let mut curried = Ast::Apply(
                Box::new(Ast::Apply(Box::new(read_fn), Box::new(ctx_var))),
                Box::new(value_ast),
            );
            for p in params.iter().rev() {
                curried = Ast::Lambda(p.name.clone(), Rc::new(curried));
            }
            Ok(Ast::Lambda(IMPLICIT_CTX.to_string(), Rc::new(curried)))
        }
    }
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
        let mut inner = rec_scope.clone();
        for p in &rb.params {
            inner = inner.with(&p.name);
        }
        let mut value_ast = expr(&rb.value, &inner)?;
        for p in rb.params.iter().rev() {
            value_ast = Ast::Lambda(p.name.clone(), Rc::new(value_ast));
        }
        bindings.push((rb.name.name.clone(), Rc::new(value_ast)));
    }
    Ok((bindings, rec_scope))
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
            let mut inner = scope.clone();
            for p in params {
                inner = inner.with(&p.name);
            }
            let mut value_ast = expr(value, &inner)?;
            for p in params.iter().rev() {
                value_ast = Ast::Lambda(p.name.clone(), Rc::new(value_ast));
            }
            let body_scope = scope.with(&name.name);
            let body_ast = expr(body, &body_scope)?;
            Ok(Ast::LetIn(
                name.name.clone(),
                Box::new(value_ast),
                Box::new(body_ast),
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
        c::Expr::Fun { kw, params, body, .. } => {
            if params.is_empty() {
                return err(kw.0, "'fun' needs at least one parameter");
            }
            let mut inner = scope.clone();
            for p in params {
                inner = inner.with(&p.name);
            }
            let mut ast = expr(body, &inner)?;
            for p in params.iter().rev() {
                ast = Ast::Lambda(p.name.clone(), Rc::new(ast));
            }
            Ok(ast)
        }
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
        // `open Name in body` (`nxletsub`'s `OPEN` case) — same alias-binding
        // technique as the top-level `TopBinding::Open` fold above (see
        // `walk_bindings`), just producing the `LetIn` chain directly since
        // there is no further sequence of sibling top bindings to thread a
        // scope through here.
        c::Expr::OpenIn { name, body, .. } => {
            let prefix = format!("{}.", name.name);
            let matches = scope.names_with_prefix(&prefix);
            let mut inner = scope.clone();
            for q in &matches {
                inner = inner.with(&q[prefix.len()..]);
            }
            let body_ast = expr(body, &inner)?;
            let mut ast = body_ast;
            for q in matches.into_iter().rev() {
                let suffix = q[prefix.len()..].to_string();
                ast = Ast::LetIn(suffix, Box::new(Ast::Var(q, name.span)), Box::new(ast));
            }
            Ok(ast)
        }
        // `while cond do body` (`nxwhl`).
        c::Expr::WhileDo { cond, body, .. } => Ok(Ast::WhileDo(
            Box::new(expr(cond, scope)?),
            Box::new(expr(body, scope)?),
        )),
        // `name <- value` (`nxlambda`'s `OVERWRITEEQ` case).
        c::Expr::Overwrite { name, value, .. } => {
            if !scope.contains(&name.name) {
                return err(
                    name.span,
                    format!("unbound mutable variable '{}'", name.name),
                );
            }
            Ok(Ast::Overwrite(
                name.name.clone(),
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
            if !scope.contains(&text) {
                return err(rhs.op.span, format!("unbound operator '{text}'"));
            }
            ops.push_back((text, rhs.op.span, rhs.op.tok.clone()));
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
/// primitives, no special-cased AST node needed.
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
        lhs = Ast::Apply(
            Box::new(Ast::Apply(Box::new(Ast::Var(text, span)), Box::new(lhs))),
            Box::new(rhs),
        );
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
                        ast = Ast::Apply(Box::new(ast), Box::new(app_arg_to_ast(rest, scope)?));
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
    for arg in &a.args {
        ast = Ast::Apply(Box::new(ast), Box::new(app_arg_to_ast(arg, scope)?));
    }
    Ok(ast)
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

fn app_arg_to_ast(arg: &c::AppArg, scope: &Scope) -> Result<Ast, ElabError> {
    match arg {
        c::AppArg::Optional { q, .. } => err(
            q.0,
            "optional arguments are not supported yet (phase 3)",
        ),
        c::AppArg::Omission(tok) => err(
            tok.0,
            "omitted arguments are not supported yet (phase 3)",
        ),
        c::AppArg::Atom {
            excl,
            atom,
            accesses,
        } => atomic_head_with_excl(atom, accesses, excl.as_ref(), scope),
        c::AppArg::Ctor(ctor) => Ok(Ast::Ctor(ctor.name.clone(), None)),
    }
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
        c::Atomic::Unit { .. } => Ok(Ast::Unit),
        c::Atomic::Paren { inner, .. } => paren_body(inner, scope),
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
        c::Atomic::MathText { elems, .. } => {
            Ok(Ast::MathText(Rc::new(lower_math_elems(elems, scope)?)))
        }
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

/// An inline-text group's content (`{ .. }`): itemize-aware entry point.
/// `sxsep`'s two alternatives (parser.mly:1039-1042) are a `nonempty_list`
/// of `*`-headed items (→ `UTItemize`, see [`itemize`]) or plain content;
/// since `InlineElem`'s `ItemBullet` markers are kept flat rather than
/// grouped in-grammar (see `cst.rs`'s doc comment on `InlineElem`), the
/// dispatch happens here instead of in the parser.
fn inline_text_ast(elems: &[c::InlineElem], scope: &Scope) -> Result<Ast, ElabError> {
    if elems.iter().any(|e| matches!(e, c::InlineElem::ItemBullet(_))) {
        itemize(elems, scope)
    } else {
        Ok(Ast::InlineText(Rc::new(inline_elems(elems, scope)?)))
    }
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
                out.push(IText::Cmd {
                    name: key,
                    span,
                    args: cmd_args(tail, scope)?,
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
                    expr: Ast::Var(key, var.span),
                    span: var.span,
                });
            }
            c::InlineElem::EmbedMath { mgrp, elems } => {
                if !text.is_empty() {
                    out.push(IText::Text(std::mem::take(&mut text)));
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
                out.push(BText::Cmd {
                    name: key,
                    span,
                    args: cmd_args(tail, scope)?,
                });
            }
            c::BlockElem::Embed { var, .. } => {
                let key = qualify_key(&var.mods, &var.name);
                if !scope.contains(&key) {
                    return err(var.span, format!("unbound variable '{key}'"));
                }
                out.push(BText::Embed {
                    expr: Ast::Var(key, var.span),
                    span: var.span,
                });
            }
        }
    }
    Ok(out)
}

/// Flatten a command tail back into its argument list. `CmdTail::Args`
/// stores the whole tail as one `Expr` (reusing the application-chain
/// grammar rather than a dedicated argument list — see `cst.rs`'s doc
/// comment on `CmdTail`); the lexer's active-mode rules restrict it to a
/// plain (no binops, no unary minus, no `before`, no `let`/`if`/`fun`/
/// `match`/`let-mutable`/`open`/`while`/`<-`) `AppExpr` chain whose head and
/// arguments *are* the command's arguments, so that is the only shape
/// accepted here.
fn cmd_args(tail: &c::CmdTail, scope: &Scope) -> Result<Vec<Ast>, ElabError> {
    match tail {
        c::CmdTail::Semi(_) => Ok(Vec::new()),
        c::CmdTail::Args { args, .. } => cmd_arg_chain(args, scope),
    }
}

fn cmd_arg_chain(e: &c::Expr, scope: &Scope) -> Result<Vec<Ast>, ElabError> {
    match e {
        c::Expr::Ops(chain) if !chain.tail.is_empty() => err(
            chain.tail[0].op.span,
            "unexpected binary operator in a command's argument list",
        ),
        c::Expr::Ops(chain) if chain.head.minus.is_some() => err(
            chain.head.minus.as_ref().unwrap().0,
            "unexpected '-' in a command's argument list",
        ),
        c::Expr::Ops(chain) if chain.before.is_some() => err(
            chain.before.as_ref().unwrap().kw.0,
            "unexpected 'before' in a command's argument list",
        ),
        c::Expr::Ops(chain) => {
            let head_val = atomic_head_with_excl(
                &chain.head.head,
                &chain.head.head_accesses,
                chain.head.excl.as_ref(),
                scope,
            )?;
            let mut out = vec![head_val];
            for a in &chain.head.args {
                out.push(app_arg_to_ast(a, scope)?);
            }
            Ok(out)
        }
        c::Expr::LetRecIn { kw, .. } => err(kw.0, "unexpected 'let-rec' as a command argument"),
        c::Expr::LetIn { kw, .. } => err(kw.0, "unexpected 'let' as a command argument"),
        c::Expr::If { kw, .. } => err(kw.0, "unexpected 'if' as a command argument"),
        c::Expr::Fun { kw, .. } => err(kw.0, "unexpected 'fun' as a command argument"),
        c::Expr::Match { kw, .. } => err(kw.0, "unexpected 'match' as a command argument"),
        c::Expr::LetMutableIn { kw, .. } => {
            err(kw.0, "unexpected 'let-mutable' as a command argument")
        }
        c::Expr::OpenIn { kw, .. } => err(kw.0, "unexpected 'open' as a command argument"),
        c::Expr::WhileDo { kw, .. } => err(kw.0, "unexpected 'while' as a command argument"),
        c::Expr::Overwrite { name, .. } => {
            err(name.span, "unexpected '<-' as a command argument")
        }
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

fn math_elem_cst(m: &c::MathElemCst, scope: &Scope) -> Result<MathElem, ElabError> {
    let base = math_bot(&m.base, scope)?;
    fold_math_scripts(base, &m.scripts, scope)
}

fn math_bot(b: &c::MathBot, scope: &Scope) -> Result<MathElem, ElabError> {
    match b {
        c::MathBot::Cmd { name, args } => {
            let mut arg_asts = Vec::with_capacity(args.len());
            for a in args {
                arg_asts.push(math_arg_to_ast(a, scope)?);
            }
            if !scope.contains(&name.name) {
                return err(name.span, format!("unbound math command '{}'", name.name));
            }
            Ok(MathElem::Cmd {
                name: name.name.clone(),
                span: name.span,
                args: arg_asts,
            })
        }
        c::MathBot::Chars(tok) => Ok(MathElem::Chars(tok.text.clone())),
        c::MathBot::Embed(tok) => {
            // Math mode has no qualified-command CST form yet (no
            // `MathCmdWithModTok`/`AnyMathCmdTok` leaf), but `#var`/`#Mod.var`
            // embeds already carry a `mods` list (`VarInMathTok`), so those
            // are mangled the same as everywhere else.
            let key = qualify_key(&tok.mods, &tok.name);
            if !scope.contains(&key) {
                return err(tok.span, format!("unbound variable '{key}'"));
            }
            Ok(MathElem::Embed {
                expr: Ast::Var(key, tok.span),
                span: tok.span,
            })
        }
        c::MathBot::Sep(tok) => err(tok.0, "math '|' separator is not supported yet (phase 3)"),
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

/// `matharg`: math command arguments either recurse into math (`Math`), are
/// program-mode escapes (`!(..)`/`![..]`/`!(|..|)`, elaborated exactly like
/// their `Atomic`/`Expr` counterparts), or are inline/block text escapes
/// (`!{..}`/`!<..>`, elaborated to `InlineText`/`BlockText` Asts — see
/// `cst.rs`'s doc comment on `MathArg` for why the lexer already makes these
/// token-identical to `Atomic`'s own bracket forms).
fn math_arg_to_ast(arg: &c::MathArg, scope: &Scope) -> Result<Ast, ElabError> {
    match arg {
        c::MathArg::Math { elems, .. } => {
            Ok(Ast::MathText(Rc::new(lower_math_elems(elems, scope)?)))
        }
        c::MathArg::Inline { elems, .. } => inline_text_ast(elems, scope),
        c::MathArg::Block { elems, .. } => Ok(Ast::BlockText(Rc::new(block_elems(elems, scope)?))),
        c::MathArg::ParenEscape { inner, .. } => paren_body(inner, scope),
        c::MathArg::ListEscape { items, .. } => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                out.push(expr(&it.value, scope)?);
            }
            Ok(Ast::List(out))
        }
        c::MathArg::RecordEscape { body, .. } => record_body_to_ast(body, scope),
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

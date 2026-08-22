//! The Hindley–Milner type inferencer: walks an
//! [`crate::elaborate::Program`] and reports the first type error it finds,
//! mirroring v0.0.6's `typecheck`/`typecheck_sub`
//! (`src/frontend/typechecker.ml`) — unification itself lives in
//! `crate::unify`, generalization/instantiation in `crate::types`, this
//! module only walks the AST applying those primitives at each rule, exactly
//! as `typechecker.ml` does over its own `unify`/`Typeenv`.
//!
//! This is validation only: `typecheck` returns `Result<(), TypeError>` and
//! never touches the untyped evaluator.
//!
//! **Deviations from v0.0.6 and permissive corners** are called out inline
//! at each rule with a `PERMISSIVE:` comment. The short version: math-mode
//! command/embed typing, and unbound type-name/type-variable references
//! inside a `type` declaration's payload, are accepted with a fresh/nominal
//! stand-in type rather than rejected.

use crate::ast::branded::{Ast, BText, CmdArg, IText, MathElem, Pattern};
use crate::elaborate::{Program, UserSynonymDecl, UserTypeDecl};
pub use crate::exhaustive::MatchWarning;
use crate::prim_types::{
    self, arrow, builtin_variants_with_version, labeled, list, mandatory, optional, product, reff,
    t_block_boxes, t_block_text, t_bool, t_context, t_deco, t_decoset, t_document, t_float,
    t_font_key, t_graphics, t_image, t_inline_boxes, t_inline_text, t_int, t_length, t_math_boxes,
    t_math_text, t_option, t_paren, t_path, t_prepath, t_string, t_unit, VariantDecl,
};
use crate::symbol::{Symbol, SymbolStore};
use crate::types::{
    self, generalize, instantiate, resolve, resolve_row, BaseType, CmdArgType, Kind, MonoType,
    PolyType, Row, Stage, TypeContext,
};
use crate::unify::{unify, UnifyError};
use rustyfi_syntax::cst::ast::{CmdTypeKind, TypeApp, TypeAtom, TypeExpr, TypeProd};
use rustyfi_syntax::cst::{RecordKind, SigConstraint, SigItem};
use rustyfi_syntax::span::Span;
use rustyfi_syntax::RustyfiVersion;
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::rc::Rc;

// ============================================================================
// Errors
// ============================================================================

/// A type error: a best-effort span (see `ast.rs`'s module doc comment — only
/// `Var`/command/embed nodes carry spans, so most rules fall back to `None`),
/// a "while typing …" context message, and — for anything that actually came
/// from a failed [`unify`] call — the [`UnifyError`] itself, whose `Display`
/// already renders both types involved.
#[derive(Debug)]
pub struct TypeError {
    pub span: Option<Span>,
    pub message: String,
    pub source: Option<UnifyError>,
}

impl TypeError {
    fn from_unify(span: Option<Span>, what: impl Into<String>, source: UnifyError) -> TypeError {
        TypeError {
            span,
            message: format!("while typing {}", what.into()),
            source: Some(source),
        }
    }

    fn simple(span: Option<Span>, message: impl Into<String>) -> TypeError {
        TypeError {
            span,
            message: message.into(),
            source: None,
        }
    }
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.span {
            Some(span) => write!(f, "{span}: {}", self.message)?,
            None => write!(f, "{}", self.message)?,
        }
        if let Some(src) = &self.source {
            write!(f, ": {src}")?;
        }
        Ok(())
    }
}

impl std::error::Error for TypeError {
    fn source<'s>(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

// ============================================================================
// The primitive name table.
//
// `prim_types::primitive_type` is a pure name -> scheme lookup with no way to
// enumerate its own domain, and `primitives.rs`'s `PRIM_DEFS` table (the
// actual source of truth) is private to that module, so this list is
// hand-kept in sync and cross-checked against `primitives.rs`'s source text
// by a test (`tests/typecheck.rs`) rather than derived mechanically. It
// matches `types_unify.rs`'s `every_registered_primitive_has_a_type` test's
// own `NAMES` list.
// ============================================================================

pub const PRIMITIVE_NAMES: &[&str] = &[
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
    // ---- context ops / box combinators ----
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
    // ---- the reflow marker-box
    // constructors (`primitives.rs`'s `prims!` table, `Both` versions) ----
    "list-mark",
    "inline-mark",
    // ---- see primitives.rs's `prims!` table comment on `"set-font-key"` ----
    "set-font-key",
    // ---- the 0.1 `font` build-out: the LOCAL stand-in for upstream's
    // internal `LoadSingleFont` node (see `primitives.rs`'s
    // `prim_load_single_font`). V0_1-only, so
    // `primitive_type_with_version` returns `None` for it under V0_0 and
    // the seeding loops skip it there. ----
    "load-single-font",
    // ---- the ~18 pure primitives ----
    // (`|>` excluded — see primitives.rs's `prims!` table comment; it has
    // no primitive of its own, so it never belongs in this list).
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
    // ---- raster images ----
    "load-image",
    "load-pdf-image",
    "use-image-by-width",
    // ---- graphics primitives ----
    "start-path",
    "line-to",
    "terminate-path",
    "close-with-line",
    "fill",
    "stroke",
    "inline-graphics",
    // ---- tables ----
    "tabular",
    "inline-graphics-outer",
    // ---- gr.satyh primitives ----
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
    // vminst.ml:1807 `BackendInnerFrame` — same signature as
    // `inline-frame-outer` above; the primitive (`primitives.rs`) and its
    // type (`prim_types.rs`) were both already registered for both
    // versions, this list simply omitted the name.
    "inline-frame-inner",
    "set-manual-rising",
    "script-guard",
    "discretionary",
    // ---- Tier-2 decoration/graphics packages ----
    "get-axis-height",
    // ---- hooks / annotations / cross-references ----
    "hook-page-break",
    "hook-page-break-block",
    "register-cross-reference",
    "get-cross-reference",
    // ---- the hooks/annotations/cross-reference closer ----
    "probe-cross-reference",
    // ---- (annot.satyh) ----
    "get-leftmost-script",
    "get-rightmost-script",
    "inline-frame-breakable",
    "register-destination",
    "register-link-to-uri",
    "register-link-to-location",
    // ---- the faithful math primitive layer ----
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
    // vminstdef.yaml:1350 `PrimitiveGetFont` — the reader for the slot
    // `set-font` writes (`ruby` and `quotation` want the CJK face's size
    // ratio); forked at the result head exactly like `set-font`'s second
    // argument.
    "get-font",
    "set-code-text-command",
    "get-natural-length",
    "set-dominant-wide-script",
    "set-dominant-narrow-script",
    "set-language",
    "set-every-word-break",
    "register-outline",
    "extract-string",
    // ---- dominant-script/language getters ----
    "get-dominant-wide-script",
    "get-dominant-narrow-script",
    "get-language",
    // ---- text-mode-context sliver ----
    "get-initial-text-info",
    "deepen-indent",
    "break",
    // ---- proof.satyh/footnote-scheme.satyh unblockers ----
    "embed-block-bottom",
    "line-stack-bottom",
    // vminstdef.yaml:1109 `BackendLineStackTop` — the top-anchored twin
    // (`ruby` stacks its annotation above the base run).
    "line-stack-top",
    "add-footnote",
    // ---- page-level prims blocking mitou-report/stdjareport ----
    "clear-page",
    // ---- added in 0.1 (`math-text`/`math-boxes` split + `read-math`) ----
    "read-math",
    "stringify-math",
    "set-math-char",
    "set-math-char-class",
    "get-math-char-class",
    "embed-inline-to-math",
    "get-math-axis-height-ratio",
    "%math-attach-scripts",
    // ---- hyphenation/unidata loader + setter stand-ins, and the `here`
    // lex-time-constant stand-in. All 5 are V0_1-only
    // (`primitive_type_with_version` returns `None` for them under V0_0,
    // same pattern as the bitwise/Unicode-string comment below documents). ----
    "load-hyphenation-dictionary",
    "load-unicode-char-database",
    "set-hyphenation-dictionary",
    "set-unicode-char-database",
    "here",
    // ---- added in 0.1 — bitwise ops, Unicode string ops,
    // `read-file`, `register-document-information`. All 11
    // unbound under V0_0 (`base_type_env_with_version`'s
    // `primitive_type_with_version` filter skips them there, same as the
    // 8 math-split names just above). `get-initial-text-info` is NOT
    // listed again here — it's already present above as one
    // shared name whose *type* forks per version (`prim_types.rs`). ----
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
    // ---- added in 0.1 — the graphics-
    // collection sweep's 2 added prims, unbound under V0_0. The 3 named + 6
    // hidden retypes (`tabular`, `get-graphics-bbox`, `inline-graphics`,
    // `inline-graphics-outer`, `inline-frame-outer/-inner/-breakable`,
    // `block-frame-breakable`) are NOT listed again here — each is one
    // shared name whose *type* forks per version (`prim_types.rs`),
    // already present above. ----
    "unite-graphics",
    "clip-graphics-by-path",
    // ---- language-completeness sweep: 0.1 float comparisons
    // (`primitives.rs`'s `prims!` table comment on ">."/"<."/">=."/"<=.").
    // Unbound under V0_0 — confirmed genuinely absent from 0.0.6 upstream,
    // unlike "+."/"-."/"*."/"/." which both generations share.
    ">.",
    "<.",
    ">=.",
    "<=.",
];

// `#[allow(dead_code)]`: kept as the back-compat sibling of
// `base_type_env_with_version` (mirrors `primitives::base_env`'s public
// wrapper) even though every current internal caller goes straight to
// `base_type_env_with_version` — `typecheck_verbose`/`typecheck` bypass this
// fn directly via `typecheck_verbose_with_version`/`typecheck_with_version`.
#[allow(dead_code)]
fn base_type_env<'s>(store: &'s SymbolStore) -> TypeEnv<'s> {
    base_type_env_with_version(store, RustyfiVersion::V0_0)
}

pub(crate) fn base_type_env_with_version<'s>(
    store: &'s SymbolStore,
    version: RustyfiVersion,
) -> TypeEnv<'s> {
    let mut env = TypeEnv::default();
    for name in PRIMITIVE_NAMES {
        if let Some(poly) = prim_types::primitive_type_with_version(name, version) {
            // `with_primitive`, not `with`: this is seeding the BASE env,
            // not a user binding, so it must not mark `name` as
            // user-shadowed (see `TypeEnv::with_primitive`'s doc comment).
            env = env.with_primitive(store.intern(name), poly);
        }
    }
    env
}

/// The `Ast::VersionScope(version, _)` typecheck arm's env
/// swap. `TypeEnv` is a flat, persistent-clone name -> scheme map (no
/// separate "local scope stack" the way `compile.rs`'s `Compiler` has), so
/// this clones `env` and OVERWRITES every `PRIMITIVE_NAMES` entry with
/// `version`'s primitive type, leaving every other (user/local) binding
/// untouched — a version-forked primitive used INSIDE the returned env
/// (e.g. a spliced 0.0.6 dependency building a `page` ADT for
/// `page-break`) then checks against `version`'s shapes, while a binder
/// introduced AFTER this call still shadows normally.
///
/// A primitive name already shadowed by a USER binding from BEFORE
/// this `VersionScope` must NOT be re-stomped. Provenance is tracked on
/// `TypeEnv` directly since it has no lexical-scope stack: every REAL
/// binding goes through `TypeEnv::with`/`with_all`, which records the name
/// in `TypeEnv::shadowed`; the primitive-seeding loops use
/// `TypeEnv::with_primitive`, which does not. So `e.shadowed.contains(name)`
/// means "a real user binding rebound this primitive anywhere on the path
/// to here" — skip the overwrite for those.
fn version_scoped_type_env<'s>(
    store: &'s SymbolStore,
    env: &TypeEnv<'s>,
    version: RustyfiVersion,
) -> TypeEnv<'s> {
    let mut e = env.clone();
    for name in PRIMITIVE_NAMES {
        let sym = store.intern(name);
        if e.is_shadowed(sym) {
            // A user binding shadows this primitive name already — respect
            // it instead of re-stomping it with `version`'s builtin scheme.
            continue;
        }
        if let Some(poly) = prim_types::primitive_type_with_version(name, version) {
            e = e.with_primitive(sym, poly);
        }
    }
    e
}

// ============================================================================
// The type environment — the same persistent base/overlay split as
// `elaborate::Scope` (see its doc comment).
// ============================================================================

/// Overlay size at which a `TypeEnv` folds its recent bindings down into a
/// fresh shared base (see [`TypeEnv::maybe_promote`]). Bounds the per-`with`
/// clone cost: every `with` clones only the ≤ this-many-entry overlays plus two
/// cheap `Rc` bumps of the (large, shared) base, so type inference is O(program
/// size) instead of the old O(program × env) — the previous flat-`HashMap`-per-
/// binding clone was ~16M entry-clones on a corpus doc, ~85% of compile time.
const OVERLAY_CAP: usize = 64;

/// One [`TypeEnv`] slot: a scheme plus the STAGE its binder was read at
/// (upstream's `Typeenv.add tyenv varnm (pty, evid, pre.stage)` — every
/// binder upstream registers carries the stage of the expression that
/// introduced it, `typechecker.ml:129/136/731/1509/1574`, and its `val_stage`
/// field in 0.1). Held behind one `Rc` per binding.
struct EnvEntry {
    poly: PolyType,
    /// Where a reference to this name is legal from — see
    /// [`Stage::can_reference`].
    stage: Stage,
}

/// A persistent name → scheme environment split into a large SHARED base
/// (`Rc`, the accumulated prelude/package bindings — cloned by an `Rc` bump)
/// and a small mutable OVERLAY of the most recent bindings (cloned in full per
/// `with`, but capped at [`OVERLAY_CAP`]). Lookups check the overlay first,
/// then the base; later bindings shadow earlier ones, exactly as a flat map
/// would — but `with`/`with_all` never copy the whole environment.
#[derive(Clone, Default)]
pub(crate) struct TypeEnv<'s> {
    // Schemes are held by `Rc`, not by value: a `PolyType` clone is a DEEP
    // copy of its whole type tree — measured at 0.6M-1.8M type nodes copied
    // per corpus document, an order of magnitude more than `instantiate`
    // and `generalize` together, and the largest single cost left in
    // typechecking — so an `Rc` makes every `with`'s overlay clone a
    // refcount bump instead. The stage rides inside that same `Rc` (see
    // [`EnvEntry`]), costing no extra allocation or word per overlay slot.
    //
    // Sharing is sound because a `PolyType`'s structure is immutable once
    // built; its `TyVarRef`s are `Rc<RefCell<..>>`, so the mutable cells
    // unification writes through were already common to every copy.
    base: std::rc::Rc<HashMap<Symbol<'s>, std::rc::Rc<EnvEntry>>>,
    overlay: HashMap<Symbol<'s>, std::rc::Rc<EnvEntry>>,
    /// The set of names bound by a REAL program binding
    /// (`with`/`with_all`, not the `with_primitive` primitive-seeding loops).
    /// `version_scoped_type_env` consults it (via [`TypeEnv::is_shadowed`]) to
    /// tell an untouched builtin scheme apart from a user shadow. Same
    /// base/overlay split as `vars`.
    base_shadowed: std::rc::Rc<std::collections::HashSet<Symbol<'s>>>,
    overlay_shadowed: std::collections::HashSet<Symbol<'s>>,
}

impl<'s> TypeEnv<'s> {
    /// Fold the overlays into fresh shared bases once the vars overlay reaches
    /// [`OVERLAY_CAP`], keeping every `with`'s overlay clone bounded. Amortized
    /// O(1) per binding (each promotion is O(base) but only every `OVERLAY_CAP`
    /// bindings along a path); inference is tree-shaped, so no single env is a
    /// hot branch point where a boundary promotion could recur.
    fn maybe_promote(&mut self) {
        if self.overlay.len() < OVERLAY_CAP {
            return;
        }
        let mut base = (*self.base).clone();
        for (k, v) in self.overlay.drain() {
            base.insert(k, v);
        }
        self.base = std::rc::Rc::new(base);
        if !self.overlay_shadowed.is_empty() {
            let mut sh = (*self.base_shadowed).clone();
            for k in self.overlay_shadowed.drain() {
                sh.insert(k);
            }
            self.base_shadowed = std::rc::Rc::new(sh);
        }
    }

    /// Bind `name` at `stage` — always the stage the BINDER was read at
    /// (`Checker::binding_stage`), never a guess. The parameter is explicit
    /// rather than defaulted precisely because a wrong stage here is silent:
    /// it does not fail to compile, it just lets a reference through that
    /// upstream refuses.
    pub(crate) fn with(&self, name: Symbol<'s>, poly: PolyType, stage: Stage) -> TypeEnv<'s> {
        let mut e = self.clone();
        e.overlay_shadowed.insert(name);
        e.overlay.insert(name, std::rc::Rc::new(EnvEntry { poly, stage }));
        e.maybe_promote();
        e
    }

    /// Install/refresh a BASE PRIMITIVE scheme without recording `name` in
    /// `shadowed` — used ONLY by the two call sites that seed or refresh a
    /// `PRIMITIVE_NAMES` entry directly (`base_type_env_with_version`,
    /// `version_scoped_type_env`), as opposed to a real program binding
    /// (`Ast::LetIn`/lambda param/pattern bind/`LetRecIn`/top-level decl),
    /// which always goes through `with`/`with_all` instead. This is exactly
    /// what lets `version_scoped_type_env` distinguish "still untouched
    /// builtin" from "user-shadowed" (see its doc comment).
    /// A primitive is `Persistent0`, so every stage may name it — upstream
    /// registers the whole primitive table that way (`primitives.cppo.ml:596`,
    /// `Typeenv.add tyenv varnm (pty, evid, Persistent0)`). Anything else
    /// would make `\emph` unreachable from a `@stage: 0` library.
    fn with_primitive(&self, name: Symbol<'s>, poly: PolyType) -> TypeEnv<'s> {
        let mut e = self.clone();
        e.overlay.insert(
            name,
            std::rc::Rc::new(EnvEntry {
                poly,
                stage: Stage::Persistent0,
            }),
        );
        e.maybe_promote();
        e
    }

    /// The raw slot. Every OCCURRENCE goes through `Checker::staged` instead,
    /// which reads this and then enforces the staging matrix — there is
    /// deliberately no stage-blind `get`, so a new reference site cannot
    /// silently skip the check.
    fn entry(&self, name: Symbol<'s>) -> Option<&EnvEntry> {
        self.overlay
            .get(&name)
            .or_else(|| self.base.get(&name))
            .map(|p| &**p)
    }

    /// Whether `name` was bound by a real program binding.
    fn is_shadowed(&self, name: Symbol<'s>) -> bool {
        self.overlay_shadowed.contains(&name) || self.base_shadowed.contains(&name)
    }

    /// Extend with each scheme in order (later shadows earlier) — the
    /// canonical way to commit `infer_binding`'s result.
    pub(crate) fn with_all(
        &self,
        schemes: Vec<(Symbol<'s>, PolyType)>,
        stage: Stage,
    ) -> TypeEnv<'s> {
        let mut e = self.clone();
        for (name, poly) in schemes {
            e.overlay_shadowed.insert(name);
            e.overlay
                .insert(name, std::rc::Rc::new(EnvEntry { poly, stage }));
            e.maybe_promote();
        }
        e
    }

    /// Remove a set of bindings: a parent seal revoking a
    /// nested module's outer-hidden members at the parent's seal point — see
    /// `v1/module_check.rs`'s `member_revoke_triggers`. V0_1-only. Flattens
    /// both layers then removes — cold path, so the one-time flatten is fine.
    #[allow(dead_code)]
    pub(crate) fn without_all(&self, names: &[Symbol<'s>]) -> TypeEnv<'s> {
        let mut vars: HashMap<Symbol<'s>, std::rc::Rc<EnvEntry>> = (*self.base).clone();
        for (k, v) in &self.overlay {
            vars.insert(*k, v.clone());
        }
        let mut sh = (*self.base_shadowed).clone();
        sh.extend(self.overlay_shadowed.iter().copied());
        for n in names {
            vars.remove(n);
            sh.remove(n);
        }
        TypeEnv {
            base: std::rc::Rc::new(vars),
            overlay: HashMap::new(),
            base_shadowed: std::rc::Rc::new(sh),
            overlay_shadowed: std::collections::HashSet::new(),
        }
    }
}

// ============================================================================
// Lowering CST `TypeExpr` (a `type` declaration's ctor payload syntax, a
// synonym's own body, and a `sig .. end`'s `val` annotations — the last
// parsed but not yet consulted, see `elaborate.rs`'s module doc comment) to
// `MonoType`. The grammar
// (`rustyfi_syntax::cst::ast::TypeExpr`/`TypeProd`/`TypeApp`/`TypeAtom`)
// supports function arrows, parens, type variables, bare names, 2+-way
// product types (`*`), and a SINGLE-argument postfix type-constructor
// application (`'a option`, `'a list`) — no record/list-literal/command
// types or N-ary applied constructors — so this lowering is total (never
// fails) and needs no arity checking of its own. `list`/`ref` map to this
// port's dedicated `MonoType::List`/`Ref` formers (mirroring
// `prim_types::list`/`reff`); every other applied name becomes a
// one-argument `MonoType::Variant`. A *synonym* reference is left exactly
// as `name_to_mono` produces it (indistinguishable from an unresolved
// variant name) — transparently replacing it with the synonym's body is
// `expand_synonyms`'s job, below.
// ============================================================================

/// Map a `type` declaration's bare type name to a `MonoType`. Every base
/// type this port's primitives use is recognized by its surface name;
/// anything else becomes a nominal, zero-argument `Variant` reference — the
/// only shape a bare name in this minimal grammar could sensibly mean (no
/// applied-constructor syntax exists to give it arguments), which is exactly
/// what makes mutually-recursive user variant types (`type t = .. of t`),
/// forward references (a later declaration's name used by an earlier one),
/// and type *synonyms* (a synonym reference is resolved the same nominal
/// way — see `expand_synonyms`) "just work": the name is resolved nominally,
/// not by looking anything up at lowering time.
fn name_to_mono(name: &str, version: RustyfiVersion) -> MonoType {
    match name {
        "unit" => t_unit(),
        "bool" => t_bool(),
        "int" => t_int(),
        "float" => t_float(),
        "length" => t_length(),
        "string" => t_string(),
        "inline-text" => t_inline_text(),
        "block-text" => t_block_text(),
        // The surface-name fork lives HERE, and only
        // here. `BaseType::MathText` is reused byte-identically as both
        // 0.0.6's `math` and V0_1's `math-text` (upstream's own rename);
        // `math-boxes` is V0_1-only, new. Under V0_1 the word `math` in a
        // type position is deliberately NOT recognized (0.1 has no `math`
        // type, `types.cppo.ml:148-155`) — it falls through to the nominal-
        // `Variant` default below, matching upstream's unbound-type error;
        // under V0_0, `math-text`/`math-boxes` likewise stay unbound.
        "math" if !version.math_is_split() => MonoType::Base(BaseType::MathText),
        "math-text" if version.math_is_split() => MonoType::Base(BaseType::MathText),
        "math-boxes" if version.math_is_split() => MonoType::Base(BaseType::MathBoxes),
        "inline-boxes" => t_inline_boxes(),
        "block-boxes" => t_block_boxes(),
        "context" => t_context(),
        "document" => t_document(),
        "text-info" => MonoType::Base(BaseType::TextInfo),
        // The graphics- tier base/synonym types upstream's
        // `types.cppo.ml` base_type_map registers (`pre-path`/`path`/
        // `graphics`/`image`, plus `deco`/`deco-set`
        // (`primitives.cppo.ml:275-276`), plus the pragmatic `font`
        // stand-in — see below).
        //
        // `pre-path`/`path`/`graphics`/`image` resolve the SAME WAY under
        // both versions: upstream maps all four to the same base types in
        // both generations (0.0.6 `types.cppo.ml:295-298`, dev-0-1-0
        // `types.cppo.ml:157`, neither cppo-gated), so the port's formers
        // take no version either.
        //
        // The VALUE rep does differ upstream (0.0.6's `BCGraphics` holds one
        // `GraphicD.element`, dev-0-1-0's a list — `graphics_is_collection`,
        // and why 0.0.6's `deco` returns `graphics list` where 0.1's returns
        // one `graphics`), but it's a subset relation, not an
        // incompatibility: the port spells both with one
        // `Value::Graphics(GraphicsElem)`, using `GraphicsElem::Group` for
        // 0.1's collection form. 0.0.6 can only ever produce a leaf
        // (`unite-graphics`, the sole `Group` constructor, is 0.1-only), and
        // every consumer (`shift_graphics`, `graphics_bbox`, the PDF/SVG
        // writers) handles `Group` uniformly, so values cross safely both
        // ways.
        //
        // Do NOT gate these four on `V0_1`: a gate makes `name_to_mono`
        // disagree across versions, which is exactly what
        // `forked_type_names()` (below) diffs — all four would report as
        // forked, and the boundary guard would then reject every 0.1
        // document importing a 0.0.6 dependency that so much as mentions
        // `graphics` in export position.
        //
        // Deliberately NOT ungated: `deco`/`deco-set` (their EXPANSION really
        // does fork; a value-level adapter handles it instead), `font` and `paren`
        // (stand-ins, see below).
        "pre-path" => t_prepath(),
        "path" => t_path(),
        "graphics" => t_graphics(),
        "image" => t_image(),
        "deco" if version == RustyfiVersion::V0_1 => t_deco(version),
        "deco-set" if version == RustyfiVersion::V0_1 => t_decoset(version),
        // `font` — a REAL base type under V0_1. Do NOT make it `t_string()`
        // again: that stand-in made `font * float * float` in a 0.1 sig
        // accidentally coincide with 0.0.6's `string * float * float`,
        // so the cross-version boundary accepted a coincidence. Upstream
        // `saphe-split` registers `("font", FontType)`
        // (`types.cppo.ml`'s `base_type_hash_table:175`, spelled
        // `tFONTKEY` at `primitives.cppo.ml:45`); its values are opaque
        // `BCFontKey of FontKey.t` handles a font envelope mints
        // (`envelopeChecker.ml`'s `check_font_envelope`) — `t_font_key()`/
        // `Value::Font(FontKey)` are that type/value.
        //
        // Under `V0_0` the name keeps falling through to the nominal
        // `Variant("font", [])` default: upstream 0.0.6 has no `font` type
        // (no such row in `types.cppo.ml:280-303`, no `type font` in its
        // bundled `.satyh` packages), so a 0.0.6 program writing `font`
        // means an unrelated opaque user nominal — why it stays in
        // `forked_type_names()` and does not cross
        // (`v1::xver_adapt::forked_note` states the fork).
        "font" if version == RustyfiVersion::V0_1 => t_font_key(),
        // math-package completion: upstream's sig writes `val paren-left
        // : paren` (+18 more) and `val angle-left : length -> paren`.
        // Structural like `deco`/`deco-set` above — the sealed decl's
        // declared type unifies with `paren-left`/`-right`'s inferred
        // `length -> length -> context -> inline-boxes * (length ->
        // length)` by construction. Under V0_0, `paren` keeps falling to
        // the nominal `Variant("paren", [])` default (synonym-expansion via
        // `pervasives.satyh`, unchanged).
        "paren" if version == RustyfiVersion::V0_1 => t_paren(version),
        other => MonoType::Variant(other.to_string(), Vec::new()),
    }
}

// ============================================================================
// Forked-name guard: builtin TYPE
// names that resolve differently — or not at all — between `V0_0` and
// `V0_1`. Builtin TYPE forks have no dedicated table — they live as inline
// version guards in `primitive_type_with_version` (`prim_types.rs`) and
// `name_to_mono` (just above) — so `forked_type_names` derives the set by
// literally diffing their output per name, rather than filtering a table.
// ============================================================================

/// Structural (alpha-equivalence) comparison of two [`MonoType`]s, used by
/// [`forked_type_names`]'s diff below. Plain `MonoType`/`PolyType` derive no
/// `PartialEq` (a hard constraint keeps `types.rs`/`unify.rs`
/// untouched), and even if
/// they did, a naive field compare would be WRONG here: every polymorphic
/// primitive's scheme mints brand-new [`TyVarRef`](crate::types::TyVarRef)s
/// from a process-wide counter (`types.rs`'s `FRESH_ID`) on each call, so two
/// calls to the very SAME (unforked) primitive under two DIFFERENT versions
/// already carry different fresh-variable identities and would falsely
/// "differ" under any identity-sensitive comparison. This instead walks both
/// types in lockstep, building a positional bijection between the two sides'
/// variables (keyed by `ptr_key()`, `types.rs`'s own union-find identity) —
/// two types are equal iff they have the same shape up to a CONSISTENT
/// renaming of variables, exactly "the same type" should mean here.
fn mono_type_alpha_eq(a: &MonoType, b: &MonoType) -> bool {
    let mut vmap = HashMap::new();
    let mut vmap_rev = HashMap::new();
    let mut rmap = HashMap::new();
    let mut rmap_rev = HashMap::new();
    mono_alpha_eq(a, b, &mut vmap, &mut vmap_rev, &mut rmap, &mut rmap_rev)
}

fn mono_alpha_eq(
    a: &MonoType,
    b: &MonoType,
    vmap: &mut HashMap<usize, usize>,
    vmap_rev: &mut HashMap<usize, usize>,
    rmap: &mut HashMap<usize, usize>,
    rmap_rev: &mut HashMap<usize, usize>,
) -> bool {
    match (&*resolve(a), &*resolve(b)) {
        (MonoType::Var(va), MonoType::Var(vb)) => {
            bijective_pair(va.ptr_key(), vb.ptr_key(), vmap, vmap_rev)
        }
        (MonoType::Base(ba), MonoType::Base(bb)) => ba == bb,
        (MonoType::Func(ra, da, ca), MonoType::Func(rb, db, cb)) => {
            row_alpha_eq(&ra, &rb, vmap, vmap_rev, rmap, rmap_rev)
                && mono_alpha_eq(&da, &db, vmap, vmap_rev, rmap, rmap_rev)
                && mono_alpha_eq(&ca, &cb, vmap, vmap_rev, rmap, rmap_rev)
        }
        (MonoType::Product(ta), MonoType::Product(tb)) => {
            ta.len() == tb.len()
                && ta
                    .iter()
                    .zip(tb.iter())
                    .all(|(x, y)| mono_alpha_eq(x, y, vmap, vmap_rev, rmap, rmap_rev))
        }
        (MonoType::List(ta), MonoType::List(tb))
        | (MonoType::Ref(ta), MonoType::Ref(tb))
        | (MonoType::Code(ta), MonoType::Code(tb)) => {
            mono_alpha_eq(&ta, &tb, vmap, vmap_rev, rmap, rmap_rev)
        }
        (MonoType::Record(ra), MonoType::Record(rb)) => {
            row_alpha_eq(&ra, &rb, vmap, vmap_rev, rmap, rmap_rev)
        }
        (MonoType::Variant(na, aa), MonoType::Variant(nb, ab)) => {
            na == nb
                && aa.len() == ab.len()
                && aa
                    .iter()
                    .zip(ab.iter())
                    .all(|(x, y)| mono_alpha_eq(x, y, vmap, vmap_rev, rmap, rmap_rev))
        }
        (MonoType::InlineCmd(ca), MonoType::InlineCmd(cb))
        | (MonoType::BlockCmd(ca), MonoType::BlockCmd(cb))
        | (MonoType::MathCmd(ca), MonoType::MathCmd(cb)) => {
            cmd_args_alpha_eq(&ca, &cb, vmap, vmap_rev, rmap, rmap_rev)
        }
        _ => false,
    }
}

fn row_alpha_eq(
    a: &Row,
    b: &Row,
    vmap: &mut HashMap<usize, usize>,
    vmap_rev: &mut HashMap<usize, usize>,
    rmap: &mut HashMap<usize, usize>,
    rmap_rev: &mut HashMap<usize, usize>,
) -> bool {
    match (&*resolve_row(a), &*resolve_row(b)) {
        (Row::Empty, Row::Empty) => true,
        (Row::Var(va), Row::Var(vb)) => bijective_pair(va.ptr_key(), vb.ptr_key(), rmap, rmap_rev),
        (Row::Cons(la, ta, ra), Row::Cons(lb, tb, rb)) => {
            la == lb
                && mono_alpha_eq(&ta, &tb, vmap, vmap_rev, rmap, rmap_rev)
                && row_alpha_eq(&ra, &rb, vmap, vmap_rev, rmap, rmap_rev)
        }
        _ => false,
    }
}

fn cmd_args_alpha_eq(
    a: &[CmdArgType],
    b: &[CmdArgType],
    vmap: &mut HashMap<usize, usize>,
    vmap_rev: &mut HashMap<usize, usize>,
    rmap: &mut HashMap<usize, usize>,
    rmap_rev: &mut HashMap<usize, usize>,
) -> bool {
    a.len() == b.len()
        && a.iter().zip(b.iter()).all(|(ca, cb)| {
            ca.optional == cb.optional
                && ca.opt_labels.len() == cb.opt_labels.len()
                && ca
                    .opt_labels
                    .iter()
                    .zip(cb.opt_labels.iter())
                    .all(|((la, tya), (lb, tyb))| {
                        la == lb && mono_alpha_eq(tya, tyb, vmap, vmap_rev, rmap, rmap_rev)
                    })
                && mono_alpha_eq(&ca.ty, &cb.ty, vmap, vmap_rev, rmap, rmap_rev)
        })
}

/// Record (or check) that pointer-key `ka` (side A) and `kb` (side B)
/// correspond under the bijection being built; `false` if either side is
/// already mapped to something else (a genuine structural mismatch).
fn bijective_pair(
    ka: usize,
    kb: usize,
    map: &mut HashMap<usize, usize>,
    map_rev: &mut HashMap<usize, usize>,
) -> bool {
    match (map.get(&ka).copied(), map_rev.get(&kb).copied()) {
        (Some(mapped_b), Some(mapped_a)) => mapped_b == kb && mapped_a == ka,
        (None, None) => {
            map.insert(ka, kb);
            map_rev.insert(kb, ka);
            true
        }
        _ => false,
    }
}

/// Every name that means a DIFFERENT TYPE under `V0_0` than under `V0_1`,
/// i.e. whose `name_to_mono` lowering differs. Its sole consumer is the
/// cross-version type-boundary guard (`v1::xver_adapt::reject_type_names`,
/// via `lib.rs`'s `free.types` check): if a 0.0.6 dependency writes this
/// name in a type, does the 0.1 consumer read it as the same type?
///
/// Do NOT also admit names whose PRIMITIVE scheme forks
/// (`primitive_type_with_version`). `PRIMITIVE_NAMES` is a list of VALUE
/// names, none of which is a type, but there is a real collision:
/// `math-char-class` is both a builtin type (nominal, version-blind) and a
/// primitive whose scheme mentions the genuinely-forked `math`. Pulling the
/// TYPE name in anyway makes the guard reject `math.satyh`'s perfectly safe
/// sig mention (`\math-style : [math-char-class; math] math-cmd`) and,
/// through it, every 0.1 document reaching the 0.0.6 math package: only
/// its constructor set forks, so it stays out of this guard's reject set.
///
/// A forked primitive VALUE is not this guard's problem: each
/// 0.0.6 dependency's bindings are wrapped in `Ast::VersionScope(V0_0, _)`, so a forked
/// primitive referenced inside one resolves against 0.0.6's own `PrimDef`
/// and runs under `Interp::version = V0_0` — why `lib.rs` has no value half
/// of the guard. The names below are just the builtin TYPE names, filtered
/// to those that really do lower differently.
pub(crate) fn forked_type_names() -> BTreeSet<String> {
    [
        "math",
        "math-text",
        "math-boxes",
        "pre-path",
        "path",
        "graphics",
        "image",
        "deco",
        "deco-set",
        "font",
        "paren",
        "math-char-class",
    ]
    .into_iter()
    .filter(|&n| {
        !mono_type_alpha_eq(
            &name_to_mono(n, RustyfiVersion::V0_0),
            &name_to_mono(n, RustyfiVersion::V0_1),
        )
    })
    .map(str::to_string)
    .collect()
}

fn lower_type_atom(
    atom: &TypeAtom,
    tyvars: &HashMap<String, MonoType>,
    version: RustyfiVersion,
) -> MonoType {
    match atom {
        // `[ty; ty?; ..] inline-cmd`/`block-cmd`/`math-cmd` — the direct
        // wire-up to the existing `CmdArgType.optional` field:
        // each bracketed element lowers to one `CmdArgType`,
        // `optional` set exactly when the element carried a trailing `?`.
        TypeAtom::Cmd { args, kind, .. } => {
            let cmd_args: Vec<CmdArgType> = args
                .iter()
                .map(|a| {
                    let ty = lower_type_expr(&a.ty, tyvars, version);
                    // SATySFi 0.1 closed command optional-label map:
                    // `?(l:τ,…)` prefixing
                    // this slot's mandatory `ty`. Sorted by label so
                    // `unify_cmd_args`'s zip-equal equal-domain test is
                    // order-insensitive against whatever surface order the
                    // sig was written in (`command_scheme`'s harvest sorts
                    // the same way). Every 0.0.6-reachable item has
                    // `opt_labels == []` (that grammar has no such prefix at
                    // all), so it takes the `optional`/`mandatory` split on
                    // the 0.0.6 positional `?` suffix marker instead.
                    if a.opt_labels.is_empty() {
                        if a.opt.is_some() {
                            optional(ty)
                        } else {
                            mandatory(ty)
                        }
                    } else {
                        let mut labels: Vec<(String, MonoType)> = a
                            .opt_labels
                            .iter()
                            .map(|f| {
                                (
                                    f.label.name.clone(),
                                    lower_type_expr(&f.ty, tyvars, version),
                                )
                            })
                            .collect();
                        labels.sort_by(|x, y| x.0.cmp(&y.0));
                        labeled(labels, ty)
                    }
                })
                .collect();
            match kind {
                CmdTypeKind::Inline(_) => MonoType::InlineCmd(cmd_args),
                CmdTypeKind::Block(_) => MonoType::BlockCmd(cmd_args),
                CmdTypeKind::Math(_) => MonoType::MathCmd(cmd_args),
            }
        }
        TypeAtom::Paren { inner, .. } => lower_type_expr(inner, tyvars, version),
        // `(| l1 : ty1; l2 : ty2; … |)` — a CLOSED record type: fold the
        // fields into a `Row::Cons` chain (in source order) ending in
        // `Row::Empty`, matching `MonoType::Record`'s row representation
        // (`types.rs`'s module doc comment) — distinct from `RecordKind`'s
        // label-only `Kind::Record` bound (`lower_record_kind`, below),
        // which drops field types entirely; a type-position record keeps
        // them, since it's a concrete type, not a lower-bound obligation.
        TypeAtom::Record { fields, .. } => {
            let row = fields.iter().rev().fold(Row::Empty, |rest, f| {
                Row::Cons(
                    f.name.name.clone(),
                    Box::new(lower_type_expr(&f.ty, tyvars, version)),
                    Box::new(rest),
                )
            });
            MonoType::Record(row)
        }
        // `(| l1 : ty1, … | ?'r |)` — a SATySFi 0.1 OPEN record type:
        // same fold as the closed form
        // above, but the row's TAIL is a fresh `Row::Var` rather than
        // `Row::Empty` — reusing the existing generic `Row`/`RowVarRef`/
        // `unify_row` machinery (no new type machinery). The row variable's
        // *name* (`?'r`) is not itself tracked anywhere past this point (the
        // same permissive-fallback philosophy `TypeAtom::Var`'s "should not
        // happen" case above already uses for an untracked name) — this
        // models one open record type at a time, not
        // cross-signature shared-row polymorphism (that needs the `rowquant`
        // grammar this track deliberately defers, see `cst.rs`'s
        // `TypeAtom::RecordOpen` doc comment). No `V0_0` version gate is
        // needed: `TypeAtom::RecordOpen` is unreachable from a `V0_0` token
        // stream by construction (see that variant's doc comment).
        TypeAtom::RecordOpen { inner, .. } => {
            let row = inner
                .fields
                .iter()
                .rev()
                .fold(Row::Var(types::new_row_var(0)), |rest, f| {
                    Row::Cons(
                        f.name.name.clone(),
                        Box::new(lower_type_expr(&f.ty, tyvars, version)),
                        Box::new(rest),
                    )
                });
            MonoType::Record(row)
        }
        TypeAtom::Var(tv) => match tyvars.get(&tv.name) {
            Some(v) => v.clone(),
            // PERMISSIVE: a type variable not among the declaration's own
            // `tyvars` (should not happen for anything the parser accepts,
            // since `TypeAtom::Var` can only ever spell one of them here —
            // there is no scoping construct in this grammar that could
            // introduce any other free type variable) — treat it as its own
            // fresh, ungeneralized variable rather than rejecting the whole
            // declaration.
            None => MonoType::Var(types::new_ty_var(0)),
        },
        TypeAtom::Name(name) => name_to_mono(&name.name, version),
        // `Mod.t` (bare, 0-ary): full module-signature-aware resolution is
        // out of scope for this port's parser-level fix (`cst.rs`'s
        // `TypeAtom::NameMod` doc comment) — permissively treat it as its
        // own nominal type keyed by the qualified spelling, exactly like
        // `name_to_mono`'s own fallback for an unregistered unqualified
        // name (`MonoType::Variant(name, [])`).
        TypeAtom::NameMod(qn) => {
            MonoType::Variant(format!("{}.{}", qn.mods.join("."), qn.name), Vec::new())
        }
    }
}

/// `txprod`: a [`TypeProd`] is either a single [`TypeApp`] (returned as-is)
/// or a genuine `*`-separated product (`MonoType::Product`, always 2+ items
/// by construction — see [`prim_types::product`]).
fn lower_type_prod(
    prod: &TypeProd,
    tyvars: &HashMap<String, MonoType>,
    version: RustyfiVersion,
) -> MonoType {
    if prod.rest.is_empty() {
        lower_type_app(&prod.first, tyvars, version)
    } else {
        let mut items = Vec::with_capacity(1 + prod.rest.len());
        items.push(lower_type_app(&prod.first, tyvars, version));
        for st in &prod.rest {
            items.push(lower_type_app(&st.ty, tyvars, version));
        }
        product(items)
    }
}

/// `txapppre`/`txapp` (restricted to a single argument — see [`TypeApp`]'s
/// doc comment): either a bare atom, or one atom applied to a single postfix
/// type-constructor name (`'a option`, `('a list) list`).
fn lower_type_app(
    app: &TypeApp,
    tyvars: &HashMap<String, MonoType>,
    version: RustyfiVersion,
) -> MonoType {
    // A bare atom (no postfix constructor).
    if app.rest.is_empty() {
        return lower_type_atom(&app.head, tyvars, version);
    }
    // `arg1 … argN ctor`: the last atom is the constructor, the rest are its
    // arguments (see `TypeApp`'s doc comment).
    let (ctor, args): (&TypeAtom, Vec<MonoType>) = {
        let n = app.rest.len();
        let arg_tys = std::iter::once(&app.head)
            .chain(app.rest[..n - 1].iter())
            .map(|a| lower_type_atom(a, tyvars, version))
            .collect();
        (&app.rest[n - 1], arg_tys)
    };
    match ctor {
        TypeAtom::Name(name) => {
            let single = if args.len() == 1 {
                Some(args[0].clone())
            } else {
                None
            };
            match name.name.as_str() {
                "list" if single.is_some() => list(single.unwrap()),
                "ref" if single.is_some() => reff(single.unwrap()),
                // `code τ` — the staged-value type, spelled in SOURCE only by
                // 0.1 (`dev-0-1-0 src/frontend/manualTypeDecoder.ml:31-36`,
                // decoded as a one-argument application right beside `list`
                // at `:37-42` and `ref` at `:44-49`). 0.0.6's own manual-type
                // decoder (`v0.0.6 src/frontend/typeenv.ml:527-530`) knows
                // only `list` and `ref`, so under `V0_0` an `int code`
                // annotation stays what it has always been here: an unknown
                // nominal `Variant("code", [int])`, which unifies with
                // nothing and is refused just as upstream's
                // `UndefinedTypeName` refuses it.
                //
                // 0.1 writes types PREFIX (`code int`); `v1/lower.rs` has
                // already flipped that into this cst's postfix
                // `TypeApp { head, rest }` shape by the time it arrives.
                "code" if single.is_some() && version.has_code_type_syntax() => {
                    MonoType::Code(Box::new(single.unwrap()))
                }
                // `T implicit` — `satysfi-base`'s typeclass-dictionary marker —
                // is transparent: an implicit `T` argument just has type `T`.
                "implicit" if single.is_some() => single.unwrap(),
                other => MonoType::Variant(other.to_string(), args),
            }
        }
        // `… Mod.t` — permissive nominal-type fallback, qualified-name-keyed
        // (see `lower_type_atom`'s `NameMod` arm).
        TypeAtom::NameMod(qn) => {
            MonoType::Variant(format!("{}.{}", qn.mods.join("."), qn.name), args)
        }
        // A non-name final atom is not a real constructor (unreachable in valid
        // source); lower it alone to keep this total.
        _ => lower_type_atom(ctor, tyvars, version),
    }
}

/// `dom -> cod`, with `?->`'s optional-argument prefix (`opts`) folded in as
/// leading `option`-wrapped mandatory domains — a stand-in:
/// `config ?-> block-text -> document` lowers to `Func(option(config),
/// Func(block-text, document))`, exactly the shape the call-site model
/// produces (`Some`/`None` applied to a plain,
/// `option`-typed domain — see `elaborate.rs`'s `app_arg_to_ast`) — one
/// consistent optional-arg model. Not upstream's real
/// `option_row`/arity-changing encoding; this only needs the two encodings
/// to *unify*, which a plain `option` domain already does. `pub(crate)`:
/// `v1/module_check.rs`
/// reuses this exact lowering for a sig `val`'s declared type, twice per decl
/// (skolemize-by-lowering — a flexible-var map for the committed scheme, a
/// rigid-stamp map for the subsumption check).
pub(crate) fn lower_type_expr(
    ty: &TypeExpr,
    tyvars: &HashMap<String, MonoType>,
    version: RustyfiVersion,
) -> MonoType {
    match ty {
        TypeExpr::Fun { opts, dom, cod, .. } => {
            let result = arrow(
                lower_type_prod(dom, tyvars, version),
                lower_type_expr(cod, tyvars, version),
            );
            opts.iter().rev().fold(result, |acc, opt| {
                arrow(t_option(lower_type_prod(&opt.ty, tyvars, version)), acc)
            })
        }
        TypeExpr::Atom(prod) => lower_type_prod(prod, tyvars, version),
        // `?(l1 : ty1, …) dom -> cod` — a
        // CLOSED row (`Row::Cons(l1, ty1, … Row::Empty)`), matching what
        // `Ast::LambdaOpt` infers — see this fn's own callers
        // (`declare_synonym`/`build_variant_decl`), which reject this node
        // under `V0_0` via `check_type_expr_v0_1_only` BEFORE ever
        // reaching here, so by the time this arm runs the version is always
        // `V0_1` for any input that could legally have parsed this node from
        // real 0.1 source (a 0.0.6 source hitting this arm is caught by that
        // earlier gate, with a clear version-error message rather than
        // silently building a nonsense type here).
        TypeExpr::OptRowFun {
            opt_dom, dom, cod, ..
        } => {
            let row = opt_dom.entries.iter().rev().fold(Row::Empty, |acc, e| {
                Row::Cons(
                    e.label.name.clone(),
                    Box::new(lower_type_expr(&e.ty, tyvars, version)),
                    Box::new(acc),
                )
            });
            MonoType::Func(
                Box::new(row),
                Box::new(lower_type_prod(dom, tyvars, version)),
                Box::new(lower_type_expr(cod, tyvars, version)),
            )
        }
    }
}

/// Reject a `?(l : ty) -> ...`
/// labeled-optional-argument type domain under `V0_0` with a clear version
/// error, mirroring `elaborate.rs`'s `Expr::FunRows`/`AppArg::Bundled`
/// value-level gates — the TYPE-level analogue. It lives
/// here (not `elaborate.rs`) because a `type`/ctor-payload `TypeExpr` is
/// never routed through the elaborator at all: `UserTypeDecl`/
/// `UserSynonymDecl` (`elaborate.rs`) carry a raw CST `TypeExpr` fragment
/// straight through to [`Checker::declare_variant`]/[`Checker::
/// declare_synonym`], the first (and only) place `Checker.version` is in
/// scope for it. A cheap existence walk, not a lowering pass —
/// [`lower_type_expr`] itself stays total/infallible for every other caller
/// (its own doc comment) — this is called BEFORE it, at each dual-version
/// entry point.
fn check_type_expr_v0_1_only(ty: &TypeExpr, version: RustyfiVersion) -> Result<(), TypeError> {
    if version.has_row_polymorphism() {
        return Ok(());
    }
    if let Some(span) = find_opt_row_fun_in_expr(ty) {
        return Err(TypeError::simple(
            Some(span),
            "`?(l : ty) -> ...` labeled-optional-argument type domains are SATySFi \
             0.1 syntax — this file is compiled as 0.0.6",
        ));
    }
    Ok(())
}

fn find_opt_row_fun_in_expr(ty: &TypeExpr) -> Option<Span> {
    match ty {
        TypeExpr::OptRowFun { opt_dom, .. } => Some(opt_dom.q.0),
        TypeExpr::Fun { dom, cod, .. } => {
            find_opt_row_fun_in_prod(dom).or_else(|| find_opt_row_fun_in_expr(cod))
        }
        TypeExpr::Atom(p) => find_opt_row_fun_in_prod(p),
    }
}

fn find_opt_row_fun_in_prod(p: &TypeProd) -> Option<Span> {
    find_opt_row_fun_in_app(&p.first)
        .or_else(|| p.rest.iter().find_map(|s| find_opt_row_fun_in_app(&s.ty)))
}

fn find_opt_row_fun_in_app(a: &TypeApp) -> Option<Span> {
    std::iter::once(&a.head)
        .chain(a.rest.iter())
        .find_map(find_opt_row_fun_in_atom)
}

fn find_opt_row_fun_in_atom(a: &TypeAtom) -> Option<Span> {
    match a {
        TypeAtom::Cmd { args, .. } => args.iter().find_map(|it| find_opt_row_fun_in_expr(&it.ty)),
        TypeAtom::Paren { inner, .. } => find_opt_row_fun_in_expr(inner),
        TypeAtom::Record { fields, .. } => {
            fields.iter().find_map(|f| find_opt_row_fun_in_expr(&f.ty))
        }
        TypeAtom::RecordOpen { inner, .. } => inner
            .fields
            .iter()
            .find_map(|f| find_opt_row_fun_in_expr(&f.ty)),
        TypeAtom::Var(_) | TypeAtom::Name(_) | TypeAtom::NameMod(_) => None,
    }
}

// ============================================================================
// Signature items: the `constraint 'a :: (| l1; l2; … |)`
// per-item suffix.
// ============================================================================

/// Every distinct type-variable name occurring in `ty`, in first-occurrence
/// order. Unlike a `type` declaration, a [`SigItem`] has no upfront `tyvars`
/// list (`val document : 'a -> …` names `'a` inline, mid-type), so building
/// its lowering's `tyvars` map requires walking the type first — every
/// occurrence of the same name must resolve to the *same* fresh variable,
/// which is also the one a matching `constraint` suffix (naming that same
/// `'a`) attaches its `Kind::Record` bound to.
///
/// `#[allow(dead_code)]`: only exercised by this module's own tests today —
/// no sig-enforcement pass calls [`lower_sig_item`] yet.
#[allow(dead_code)]
fn collect_type_vars(ty: &TypeExpr, out: &mut Vec<String>) {
    fn push(name: &str, out: &mut Vec<String>) {
        if !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    fn walk_atom(atom: &TypeAtom, out: &mut Vec<String>) {
        match atom {
            TypeAtom::Cmd { args, .. } => {
                for a in args {
                    walk_expr(&a.ty, out);
                }
            }
            TypeAtom::Paren { inner, .. } => walk_expr(inner, out),
            TypeAtom::Record { fields, .. } => {
                for f in fields {
                    walk_expr(&f.ty, out);
                }
            }
            TypeAtom::Var(tv) => push(&tv.name, out),
            TypeAtom::Name(_) => {}
            TypeAtom::NameMod(_) => {}
            // An open record's fields carry
            // type vars same as a closed record's; its row-variable tail
            // (`?'r`) is a ROW var, a different namespace this fn doesn't
            // track (it collects `TypeAtom::Var`/`'a`-style tyvars only).
            TypeAtom::RecordOpen { inner, .. } => {
                for f in &inner.fields {
                    walk_expr(&f.ty, out);
                }
            }
        }
    }
    fn walk_app(app: &TypeApp, out: &mut Vec<String>) {
        walk_atom(&app.head, out);
        for a in &app.rest {
            walk_atom(a, out);
        }
    }
    fn walk_prod(prod: &TypeProd, out: &mut Vec<String>) {
        walk_app(&prod.first, out);
        for st in &prod.rest {
            walk_app(&st.ty, out);
        }
    }
    fn walk_expr(ty: &TypeExpr, out: &mut Vec<String>) {
        match ty {
            TypeExpr::Fun { opts, dom, cod, .. } => {
                for opt in opts {
                    walk_prod(&opt.ty, out);
                }
                walk_prod(dom, out);
                walk_expr(cod, out);
            }
            TypeExpr::Atom(prod) => walk_prod(prod, out),
            // A labeled-optional domain's
            // entry types can mention tyvars same as any other domain.
            TypeExpr::OptRowFun {
                opt_dom, dom, cod, ..
            } => {
                for e in &opt_dom.entries {
                    walk_expr(&e.ty, out);
                }
                walk_prod(dom, out);
                walk_expr(cod, out);
            }
        }
    }
    walk_expr(ty, out);
}

/// Lower a [`RecordKind`]'s field list to its label set, dropping field
/// *types* — `Kind::Record` (`types.rs`) stores labels only, so
/// `constraint 'a :: (| title : inline-text; … |)` checks label
/// *presence*, not the field's declared type. A documented
/// limitation, not a grammar gap: the
/// impl's row still gets its own field types from ordinary usage, so this
/// is unlikely to admit a wrong program in practice.
#[allow(dead_code)]
fn lower_record_kind(rk: &RecordKind) -> BTreeSet<String> {
    rk.fields.iter().map(|f| f.name.name.clone()).collect()
}

/// Lower one value/direct [`SigItem`] to its name and [`MonoType`],
/// attaching each `constraint 'a :: (| … |)` suffix as a `Kind::Record`
/// bound on `'a`'s freshly-minted variable — "this variable must be a
/// record containing at least these labels", built on the existing
/// Rémy-style row machinery. Returns `None` for a bare `SigItem::Type`
/// item (a type *name* declaration, not a value with a `MonoType` of its
/// own).
///
/// **The obligation check itself rides on existing code, for free.** Once
/// this variable is ever unified against a concrete `MonoType::Record`
/// (which is exactly what enforcing the signature against a real `struct`
/// implementation would do), `unify::bind_var`'s `Kind::Record` branch
/// already rejects a row missing any declared label via
/// `row_require_label`.
#[allow(dead_code)]
pub(crate) fn lower_sig_item(
    item: &SigItem,
    ctx: &mut TypeContext,
    version: RustyfiVersion,
) -> Option<(String, MonoType)> {
    let (name, ty, constraints): (&str, &TypeExpr, &[SigConstraint]) = match item {
        SigItem::ValHorzCmd {
            name,
            ty,
            constraints,
            ..
        } => (&name.name, ty, constraints),
        SigItem::ValVertCmd {
            name,
            ty,
            constraints,
            ..
        } => (&name.name, ty, constraints),
        SigItem::Val {
            name,
            ty,
            constraints,
            ..
        } => (&name.name, ty, constraints),
        SigItem::DirectHorzCmd {
            name,
            ty,
            constraints,
            ..
        } => (&name.name, ty, constraints),
        SigItem::DirectVertCmd {
            name,
            ty,
            constraints,
            ..
        } => (&name.name, ty, constraints),
        SigItem::Type { .. } => return None,
    };
    let mut names = Vec::new();
    collect_type_vars(ty, &mut names);
    let mut tyvars = HashMap::new();
    for n in names {
        let found = constraints.iter().find(|c| c.tyvar.name == n);
        let v = match found {
            Some(c) => ctx.fresh_var_with_kind(Kind::Record(lower_record_kind(&c.kind))),
            None => ctx.fresh_var_with_kind(Kind::Universal),
        };
        tyvars.insert(n, MonoType::Var(v));
    }
    Some((name.to_string(), lower_type_expr(ty, &tyvars, version)))
}

/// Lower one [`UserTypeDecl`] (surfaced by `elaborate::elaborate_program`)
/// into a [`VariantDecl`], the same shape `prim_types::builtin_variants_with_version`
/// produces for `option`/`itemize` — see that struct's doc comment for how
/// `param_vars` and `instantiate_ctor` fit together. Each ctor's payload is
/// passed through [`expand_synonyms`] so a payload that names a synonym
/// (`type wrap = | W of point`) is stored already-transparent — `unify`
/// never has to know synonyms exist.
fn build_variant_decl(
    decl: &UserTypeDecl,
    synonyms: &HashMap<String, SynonymDecl>,
    version: RustyfiVersion,
) -> Result<VariantDecl, TypeError> {
    let param_vars: Vec<types::TyVarRef> =
        decl.params.iter().map(|_| types::new_ty_var(0)).collect();
    let tyvar_map: HashMap<String, MonoType> = decl
        .params
        .iter()
        .cloned()
        .zip(param_vars.iter().cloned().map(MonoType::Var))
        .collect();
    let mut ctors = Vec::with_capacity(decl.ctors.len());
    for (name, ty) in &decl.ctors {
        let payload = match ty {
            None => None,
            Some(t) => {
                check_type_expr_v0_1_only(t, version)?;
                Some(expand_synonyms(
                    &lower_type_expr(t, &tyvar_map, version),
                    synonyms,
                )?)
            }
        };
        ctors.push((name.clone(), payload));
    }
    Ok(VariantDecl {
        name: decl.name.clone(),
        params: decl.params.len(),
        ctors,
        param_vars,
    })
}

// ============================================================================
// Type synonyms (`type point = length * length`): registration, plus the
// transparent expansion that keeps a synonym's name from ever reaching
// `unify` — mirrors upstream's `SynonymType`/`add_synonym`
// (`typechecker.ml`), just against this port's `MonoType`/`substitute`
// machinery instead of a substitution-on-the-fly unifier case.
// ============================================================================

/// A user type-synonym declaration, lowered from [`UserSynonymDecl`] — the
/// transparent-expansion counterpart of [`VariantDecl`] (`prim_types.rs`).
/// Unlike a variant, a synonym never gets a runtime tag or a
/// `Checker::ctors` entry; the only thing that ever looks at one is
/// [`expand_synonyms`].
struct SynonymDecl {
    /// The declaration's own type-parameter placeholders — the same
    /// technique as `VariantDecl::param_vars` (matched by pointer identity
    /// via `types::substitute`). Realistically always empty: this grammar
    /// has no applied-type-constructor syntax to *reference* a synonym with
    /// arguments (`TypeAtom`'s doc comment), so a real reference site always
    /// supplies zero args — kept general anyway, so a nonzero-param synonym
    /// fails with a clear arity error rather than silently misbehaving if
    /// that ever changes.
    param_vars: Vec<types::TyVarRef>,
    /// The synonym's body, lowered exactly once via `lower_type_expr`. Any
    /// *other* synonym name it mentions is still an opaque
    /// `MonoType::Variant(name, [])` at this point (`lower_type_expr` has no
    /// notion of the synonym table) — [`expand_synonyms`] resolves those
    /// lazily, on demand, at each reference site.
    body: MonoType,
}

fn build_synonym_decl(decl: &UserSynonymDecl, version: RustyfiVersion) -> SynonymDecl {
    let param_vars: Vec<types::TyVarRef> =
        decl.params.iter().map(|_| types::new_ty_var(0)).collect();
    let tyvar_map: HashMap<String, MonoType> = decl
        .params
        .iter()
        .cloned()
        .zip(param_vars.iter().cloned().map(MonoType::Var))
        .collect();
    SynonymDecl {
        param_vars,
        body: lower_type_expr(&decl.body, &tyvar_map, version),
    }
}

/// Collect the name of every *synonym* (i.e. present in `synonyms`) directly
/// mentioned inside `ty`, ignoring argument count — used only to build the
/// "synonym references synonym" graph for [`check_synonym_cycles`], where
/// arity is irrelevant (a cycle is a cycle no matter how many arguments each
/// step is nominally applied to).
fn synonym_refs(ty: &MonoType, synonyms: &HashMap<String, SynonymDecl>, out: &mut Vec<String>) {
    match ty {
        MonoType::Var(_) | MonoType::Base(_) => {}
        MonoType::Func(row, dom, cod) => {
            synonym_refs_row(row, synonyms, out);
            synonym_refs(dom, synonyms, out);
            synonym_refs(cod, synonyms, out);
        }
        MonoType::Product(ts) => ts.iter().for_each(|t| synonym_refs(t, synonyms, out)),
        MonoType::List(t) | MonoType::Ref(t) | MonoType::Code(t) => synonym_refs(t, synonyms, out),
        MonoType::Record(row) => synonym_refs_row(row, synonyms, out),
        MonoType::Variant(name, args) => {
            if synonyms.contains_key(name) {
                out.push(name.clone());
            }
            args.iter().for_each(|t| synonym_refs(t, synonyms, out));
        }
        MonoType::InlineCmd(cs) | MonoType::BlockCmd(cs) | MonoType::MathCmd(cs) => {
            cs.iter().for_each(|c| synonym_refs(&c.ty, synonyms, out));
        }
    }
}

fn synonym_refs_row(row: &Row, synonyms: &HashMap<String, SynonymDecl>, out: &mut Vec<String>) {
    match row {
        Row::Empty | Row::Var(_) => {}
        Row::Cons(_, t, rest) => {
            synonym_refs(t, synonyms, out);
            synonym_refs_row(rest, synonyms, out);
        }
    }
}

/// Reject a cyclic synonym (`type a = b` / `type b = a`, or a directly
/// self-referential `type a = a * a`) with a clear error instead of letting
/// [`expand_synonyms`] recurse forever. Run unconditionally over every
/// registered synonym at [`Checker::new`] time, so a cyclic pair is caught
/// even if nothing in the program actually references it.
fn check_synonym_cycles(synonyms: &HashMap<String, SynonymDecl>) -> Result<(), TypeError> {
    for start in synonyms.keys() {
        let mut stack = vec![start.clone()];
        check_synonym_cycles_from(start, synonyms, &mut stack)?;
    }
    Ok(())
}

fn check_synonym_cycles_from(
    name: &str,
    synonyms: &HashMap<String, SynonymDecl>,
    stack: &mut Vec<String>,
) -> Result<(), TypeError> {
    let mut refs = Vec::new();
    synonym_refs(&synonyms[name].body, synonyms, &mut refs);
    for r in refs {
        if stack.contains(&r) {
            let mut cycle = stack.clone();
            cycle.push(r);
            return Err(TypeError::simple(
                None,
                format!("cyclic type synonym: {}", cycle.join(" -> ")),
            ));
        }
        stack.push(r.clone());
        check_synonym_cycles_from(&r, synonyms, stack)?;
        stack.pop();
    }
    Ok(())
}

/// Recursively and transparently expand every synonym reference inside
/// `ty`, so `unify` never sees a synonym's name — only real base/product/
/// function/variant types. `synonyms` was already validated acyclic by
/// [`check_synonym_cycles`] (at `Checker::new` time), so the recursion here
/// is guaranteed to terminate; arity (a reference's argument count against
/// the synonym's own parameter count) is checked here, the one place that
/// actually has concrete arguments to check it against.
fn expand_synonyms(
    ty: &MonoType,
    synonyms: &HashMap<String, SynonymDecl>,
) -> Result<MonoType, TypeError> {
    match ty {
        MonoType::Var(_) | MonoType::Base(_) => Ok(ty.clone()),
        MonoType::Func(row, dom, cod) => Ok(MonoType::Func(
            Box::new(expand_synonyms_row(row, synonyms)?),
            Box::new(expand_synonyms(dom, synonyms)?),
            Box::new(expand_synonyms(cod, synonyms)?),
        )),
        MonoType::Product(ts) => Ok(MonoType::Product(
            ts.iter()
                .map(|t| expand_synonyms(t, synonyms))
                .collect::<Result<_, _>>()?,
        )),
        MonoType::List(t) => Ok(MonoType::List(Box::new(expand_synonyms(t, synonyms)?))),
        MonoType::Ref(t) => Ok(MonoType::Ref(Box::new(expand_synonyms(t, synonyms)?))),
        MonoType::Code(t) => Ok(MonoType::Code(Box::new(expand_synonyms(t, synonyms)?))),
        MonoType::Record(row) => Ok(MonoType::Record(expand_synonyms_row(row, synonyms)?)),
        MonoType::Variant(name, args) => {
            let args: Vec<MonoType> = args
                .iter()
                .map(|t| expand_synonyms(t, synonyms))
                .collect::<Result<_, _>>()?;
            let Some(syn) = synonyms.get(name) else {
                return Ok(MonoType::Variant(name.clone(), args));
            };
            if args.len() != syn.param_vars.len() {
                return Err(TypeError::simple(
                    None,
                    format!(
                        "type synonym '{name}' expects {} argument{}, got {}",
                        syn.param_vars.len(),
                        if syn.param_vars.len() == 1 { "" } else { "s" },
                        args.len()
                    ),
                ));
            }
            let mut var_map: HashMap<usize, MonoType> = HashMap::new();
            for (pv, arg) in syn.param_vars.iter().zip(args.iter()) {
                var_map.insert(types::ptr_key(pv), arg.clone());
            }
            let substituted = types::substitute(&syn.body, &var_map, &HashMap::new());
            expand_synonyms(&substituted, synonyms)
        }
        MonoType::InlineCmd(cs) => Ok(MonoType::InlineCmd(expand_synonyms_cmd_args(cs, synonyms)?)),
        MonoType::BlockCmd(cs) => Ok(MonoType::BlockCmd(expand_synonyms_cmd_args(cs, synonyms)?)),
        MonoType::MathCmd(cs) => Ok(MonoType::MathCmd(expand_synonyms_cmd_args(cs, synonyms)?)),
    }
}

fn expand_synonyms_row(
    row: &Row,
    synonyms: &HashMap<String, SynonymDecl>,
) -> Result<Row, TypeError> {
    match row {
        Row::Empty => Ok(Row::Empty),
        Row::Var(v) => Ok(Row::Var(v.clone())),
        Row::Cons(label, t, rest) => Ok(Row::Cons(
            label.clone(),
            Box::new(expand_synonyms(t, synonyms)?),
            Box::new(expand_synonyms_row(rest, synonyms)?),
        )),
    }
}

fn expand_synonyms_cmd_args(
    cs: &[CmdArgType],
    synonyms: &HashMap<String, SynonymDecl>,
) -> Result<Vec<CmdArgType>, TypeError> {
    cs.iter()
        .map(|c| {
            Ok(CmdArgType {
                optional: c.optional,
                opt_labels: c
                    .opt_labels
                    .iter()
                    .map(|(l, t)| Ok((l.clone(), expand_synonyms(t, synonyms)?)))
                    .collect::<Result<_, TypeError>>()?,
                ty: expand_synonyms(&c.ty, synonyms)?,
            })
        })
        .collect()
}

// ============================================================================
// The checker.
// ============================================================================

pub(crate) struct Checker<'s> {
    /// The interner every identifier in the tree being checked came from.
    /// Needed both to mint derived lookup keys (`"{modpfx}.{ctor}"`) and to
    /// resolve a `Symbol` back to text for an error message — see
    /// [`Checker::text`].
    store: &'s SymbolStore,
    ctx: TypeContext,
    /// Constructor name -> the (`Rc`-shared) declaration it belongs to.
    /// Later declarations shadow earlier ones of the same ctor name, mirroring
    /// ordinary name shadowing elsewhere in this port.
    ctors: HashMap<String, Rc<VariantDecl>>,
    /// The same declarations, keyed by *type* name instead — needed by the
    /// exhaustiveness pass (`exhaustive::check_match`) to enumerate a
    /// variant's full constructor set given the scrutinee's resolved
    /// `MonoType::Variant(name, _)`.
    variants: HashMap<String, Rc<VariantDecl>>,
    /// The synonym table. A field rather than a local of `new_with_version`
    /// so `declare_variant` can expand ctor payloads through it after
    /// construction time (per-binding registration).
    synonyms: HashMap<String, SynonymDecl>,
    /// Non-fatal diagnostics accumulated by the exhaustiveness/redundancy
    /// pass (see `typecheck_verbose`); v0.0.6's `exhchecker.ml` warns and
    /// continues rather than rejecting the program.
    warnings: Vec<MatchWarning>,
    /// The target version this session is checking against — set by
    /// [`Checker::install_builtin_variants`] (every real construction path's
    /// call; `Checker::empty` alone never does, defaulting to `V0_0`).
    /// Threaded into `name_to_mono`'s surface-type-name fork and
    /// `Ast::LetMathIn`'s scheme rule (`math_command_scheme` vs
    /// `math_command_scheme_v01`).
    version: RustyfiVersion,
    /// The generation of the code currently being inferred, when that is
    /// NOT [`Checker::version`] — set (save/restore) by the
    /// `Ast::VersionScope` infer arm, `None` outside every such scope.
    /// Deliberately a SEPARATE field rather than a temporary overwrite of
    /// `version`, since that one must stay pinned to `V0_1` for a merged
    /// whole-program session (`v1::module_check::check_program_inner`). See
    /// [`Checker::binding_version`], its only consumer.
    scoped_version: Option<RustyfiVersion>,
    /// The module path of the member body currently being inferred (pushed by
    /// the `Ast::ModuleScope` arm; empty at top level). A BARE constructor
    /// reference is looked up under `<path>.Ctor` (innermost prefix first)
    /// before the bare fallback — so two modules' same-named constructors
    /// (`Term.Paren` vs `Type.Paren`) no longer collide. See [`Ast::ModuleScope`].
    ctor_scope: Vec<String>,
    /// The stage this expression is being read at. Starts at whatever the
    /// file declared (`@stage:`, default [`Stage::Stage1`]) and is shifted by
    /// `&`/`~`: a quote reads its body one stage LATER, a splice one stage
    /// EARLIER. Only `Next`/`Prev` consult it.
    stage: Stage,
}

/// One elaborated top-level-shaped binding, viewed by reference — the
/// typecheck-side mirror of `elaborate::Binding` and of the four Let-shaped
/// `Ast` spine variants (`ast.rs`). The module checker constructs these
/// directly from its own per-`val` walk; the whole-program path never
/// constructs one explicitly (its `infer` arms pass the same references
/// through).
pub(crate) enum BindingView<'a, 's> {
    /// `Ast::LetIn` — plain value OR `\`/`+`-sigiled command binding; the
    /// sigil dispatch (`command_scheme`) stays inside the checker.
    Let {
        name: Symbol<'s>,
        value: &'a Ast<'s>,
    },
    /// `Ast::LetMathIn` — a math-command binding (distinct variant by
    /// construction).
    LetMath {
        name: Symbol<'s>,
        value: &'a Ast<'s>,
    },
    /// `Ast::LetRecIn`'s binding group (all names in scope in all bodies).
    LetRec(&'a [(Symbol<'s>, Rc<Ast<'s>>)]),
    /// `Ast::LetMutableIn` — value restriction, never generalized.
    LetMutable { name: Symbol<'s>, init: &'a Ast<'s> },
}

impl<'s> Checker<'s> {
    // See `base_type_env`'s `#[allow(dead_code)]` note above — same shape,
    // same reason.
    #[allow(dead_code)]
    fn new(program: &Program<'s>) -> Result<Checker<'s>, TypeError> {
        Self::new_with_version(program, RustyfiVersion::V0_0)
    }

    /// Bare session: empty tables, fresh `TypeContext`. Registers NOTHING —
    /// not even builtins — so `new_with_version` can compose the exact
    /// statement order of the original monolithic constructor.
    pub(crate) fn empty(store: &'s SymbolStore) -> Checker<'s> {
        Checker {
            store,
            ctx: TypeContext::new(),
            ctors: HashMap::new(),
            variants: HashMap::new(),
            synonyms: HashMap::new(),
            warnings: Vec::new(),
            version: RustyfiVersion::V0_0,
            scoped_version: None,
            ctor_scope: Vec::new(),
            // A document is stage 1; a library overrides this from its
            // `@stage:` header before checking begins.
            stage: Stage::default(),
        }
    }

    /// Set the session's target version with NO other side effect — a pure
    /// field write, safe to call before anything else.
    /// `new_with_version`/`v1::module_check::check_program`'s session-setup
    /// sequences both call this FIRST, ahead of their order-critical
    /// `declare_synonym` loop, so a V0_1 synonym body that names
    /// `math-text`/`math-boxes` resolves correctly even though
    /// `install_builtin_variants` (which also sets this field) does not run
    /// until afterward. A no-op for every 0.0.6 path: `empty()` already
    /// defaults to `V0_0`.
    pub(crate) fn set_version(&mut self, version: RustyfiVersion) {
        self.version = version;
    }

    /// A symbol's source text. Every diagnostic this module formats goes
    /// through here: `Symbol`'s own `Debug` is index-only by design, and the
    /// golden tests diff the resolved strings.
    fn text(&self, sym: Symbol<'s>) -> &'s str {
        self.store.resolve(sym)
    }

    /// Register the builtin variant decls for `version`, and record
    /// `version` on `self` — redundant with `set_version` for a path that
    /// calls both, kept so this method alone suffices for a caller that
    /// skips `set_version` (e.g. the bare-builtins test construction).
    pub(crate) fn install_builtin_variants(&mut self, version: RustyfiVersion) {
        self.version = version;
        self.install_additional_builtin_variants(version);
    }

    /// Register `version`'s builtin variant/ctor set WITHOUT
    /// touching `self.version` (unlike [`Checker::install_builtin_
    /// variants`], which also sets the whole-session version tag) — called
    /// from the `Ast::VersionScope` `infer` arm below, lazily, the first
    /// time a version-scoped subtree is reached, so a `V0_0`-only ADT like
    /// `page` (`A0Paper`/…/`A4Paper`/`UserDefinedPaper`, gated on
    /// `has_page_adt()`, `prim_types.rs`'s `builtin_variants_with_version`)
    /// is constructible/matchable inside a spliced dependency's internal
    /// `page-break A4Paper …` call even though the whole-program `Checker`
    /// was built under `V0_1` (where `page` isn't registered at all).
    /// `self.ctors`/`self.variants` are flat, last-writer-wins, program-
    /// global tables already (`hide_ctors`'s doc comment); every
    /// `builtin_variants_with_version` entry OTHER than `page` is identical
    /// between the two versions (only `page` is gated by `has_page_adt()`,
    /// `prim_types.rs:2119`), so this is a safe additive merge, not a
    /// replace — idempotent across repeated `VersionScope` nodes of the same
    /// version.
    pub(crate) fn install_additional_builtin_variants(&mut self, version: RustyfiVersion) {
        for decl in builtin_variants_with_version(version) {
            let decl = Rc::new(decl);
            self.variants.insert(decl.name.clone(), decl.clone());
            for (cname, _) in &decl.ctors {
                self.ctors.insert(cname.clone(), decl.clone());
            }
        }
    }

    /// One synonym registration. Does NOT cycle-check — register all, then
    /// call `check_cycles`. Fallible:
    /// `check_type_expr_v0_1_only` rejects a `?(l:ty)->` domain in the
    /// synonym's body under `V0_0` before it is ever lowered.
    pub(crate) fn declare_synonym(&mut self, decl: &UserSynonymDecl) -> Result<(), TypeError> {
        check_type_expr_v0_1_only(&decl.body, self.version)?;
        self.synonyms
            .insert(decl.name.clone(), build_synonym_decl(decl, self.version));
        Ok(())
    }

    /// Cycle-check the accumulated synonym table — a thin wrapper over the
    /// existing free fn `check_synonym_cycles`.
    pub(crate) fn check_cycles(&self) -> Result<(), TypeError> {
        check_synonym_cycles(&self.synonyms)
    }

    /// One variant-decl registration.
    pub(crate) fn declare_variant(&mut self, decl: &UserTypeDecl) -> Result<(), TypeError> {
        let decl = Rc::new(build_variant_decl(decl, &self.synonyms, self.version)?);
        self.variants.insert(decl.name.clone(), decl.clone());
        for (cname, _) in &decl.ctors {
            self.ctors.insert(cname.clone(), decl.clone());
        }
        // If the variant's own type name is module-qualified (`M.t`), also
        // register each constructor under a qualified key (`M.Ctor`) so a
        // within-module bare reference (via `Checker::lookup_ctor`, driven by
        // `Ast::ModuleScope`) resolves to THIS module's ctor even when another
        // module declares the same bare ctor name. Builtins have undotted type
        // names, so this adds nothing for them.
        if let Some((modpfx, _)) = decl.name.rsplit_once('.') {
            for (cname, _) in &decl.ctors {
                self.ctors.insert(format!("{modpfx}.{cname}"), decl.clone());
            }
        }
        Ok(())
    }

    /// The original whole-program constructor, re-expressed as the exact
    /// same statement sequence through the methods above: synonyms →
    /// cycle-check → builtins → user variant decls. This order-preservation
    /// is load-bearing for session-incrementality.
    fn new_with_version(
        program: &Program<'s>,
        version: RustyfiVersion,
    ) -> Result<Checker<'s>, TypeError> {
        // Synonyms are registered (and checked for cycles) before any
        // variant decl is lowered, since a variant's ctor payload may name a
        // synonym (`build_variant_decl` expands through `synonyms`).
        let mut c = Checker::empty(program.store);
        c.set_version(version);
        for usd in &program.synonym_decls {
            c.declare_synonym(usd)?;
        }
        c.check_cycles()?;
        c.install_builtin_variants(version);
        for utd in &program.type_decls {
            c.declare_variant(utd)?;
        }
        Ok(c)
    }

    fn fresh(&mut self) -> MonoType {
        MonoType::Var(self.ctx.fresh_var())
    }

    fn unify_ctx(
        &mut self,
        expected: &MonoType,
        found: &MonoType,
        span: Option<Span>,
        what: &str,
    ) -> Result<(), TypeError> {
        unify(expected, found).map_err(|e| TypeError::from_unify(span, what, e))
    }

    /// Turn a `\`/`+`-named `LetIn` binding's ordinarily-inferred value type
    /// `tv` into the genuine command type (`MonoType::InlineCmd`/`BlockCmd`)
    /// it gets bound under: a user-defined command is typed as
    /// `[τ1; ..; τn] inline-cmd` (resp. `block-cmd`), matching v0.0.6's real
    /// `HorzCommandType`/`VertCommandType` (`typechecker.ml`'s
    /// `UTLetHorzIn`/`UTLetVertIn` rules), not a plain "context-curried"
    /// function.
    ///
    /// Two shapes reach this function, per [`command_sigil`]'s call site:
    ///
    /// * a genuine `let-inline`/`let-block` definition, whose value is the
    ///   `Lambda(ctxvar, Lambda(p1, .., Lambda(pn, body)))` chain
    ///   `elaborate::elaborate_let_inline` builds — [`peel_func_chain`]
    ///   recovers that `Func` chain; the leading domain must unify with
    ///   `context`, the final codomain with `inline-boxes`/`block-boxes`,
    ///   and the domains between become the command's `CmdArgType` list.
    /// * a qualified-name *alias* of an already-command-typed binding (a
    ///   module's own `M.\cmd` re-export, or an `open` re-binding — both
    ///   build `LetIn(name, Ast::Var(qualified), body)`): the aliased name
    ///   was already run through this function at its own definition site,
    ///   so `tv` here already *is* the command type — this branch passes it
    ///   through unchanged (re-generalized) instead of peeling a `Func`
    ///   chain out of something that isn't one.
    fn command_scheme(
        &mut self,
        name: &str,
        sigil: char,
        tv: MonoType,
        span: Option<Span>,
    ) -> Result<PolyType, TypeError> {
        debug_assert!(sigil == '\\' || sigil == '+');
        let is_inline = sigil == '\\';
        let (want_result, kind, other_kind) = if is_inline {
            (t_inline_boxes(), "inline", "block")
        } else {
            (t_block_boxes(), "block", "inline")
        };

        match &*resolve(&tv) {
            MonoType::InlineCmd(_) if is_inline => {
                return Ok(generalize(self.ctx.level(), &tv));
            }
            MonoType::BlockCmd(_) if !is_inline => {
                return Ok(generalize(self.ctx.level(), &tv));
            }
            MonoType::InlineCmd(_) | MonoType::BlockCmd(_) => {
                return Err(TypeError::simple(
                    span,
                    format!(
                        "'{name}' is bound to a {other_kind} command, but its \
                         name marks it as {article} {kind} command",
                        article = if kind == "inline" { "an" } else { "a" },
                    ),
                ));
            }
            // A qualified-name alias (`M.\cmd` re-export, or an `open`) of a
            // GENUINE `let-math` binding: math commands share the `\` sigil
            // with inline commands (there is no separate math-command token),
            // so an alias site only ever reaches this generic `Ast::LetIn`
            // path (never `Ast::LetMathIn`, which is produced only at a math
            // command's OWN definition site — top-level `let-math` via
            // `walk_bindings`, or the expression-level `let-math .. in ..`
            // form, `elaborate.rs`'s `Expr::LetMathIn` arm — never at an
            // alias site; see that variant's doc comment). Pass a already-
            // `MathCmd`-typed alias through unchanged, exactly like the
            // `InlineCmd`/`BlockCmd` arms above do for their own kind.
            MonoType::MathCmd(_) if is_inline => {
                return Ok(generalize(self.ctx.level(), &tv));
            }
            _ => {}
        }

        // V0_1 harvests each param's closed
        // `?(l:τ,…)` label map from the `Row` that `Ast::LambdaOpt` leaves on
        // that param's own arrow (`peel_func_chain_rows`), instead of the
        // 0.0.6 "`_ option` domain ⇒ optional slot" heuristic below (which
        // stays untouched, byte-identical, under V0_0 — it never sees a
        // non-`Row::Empty` row at all, since V0_0 code never builds
        // `Ast::LambdaOpt`).
        let params: Vec<CmdArgType> = if self.version.has_row_polymorphism() {
            let (mut slots, result) = peel_func_chain_rows(tv);
            if slots.is_empty() {
                return Err(TypeError::simple(
                    span,
                    format!(
                        "the binding for '{name}' must be a function taking a \
                         context as its first argument (e.g. via `val inline ctx \
                         {name} .. = ..`)"
                    ),
                ));
            }
            let (ctx_row, ctx_ty) = slots.remove(0);
            // A labeled bundle can never legally land on the ctx binder
            // (`elaborate_let_inline` always wraps it in a plain
            // `Ast::Lambda`, which infers `Row::Empty` — `prim_types::arrow`)
            // — guard it defensively rather than silently dropping/mis-
            // attributing a label.
            if !matches!(&*resolve_row(&ctx_row), Row::Empty) {
                return Err(TypeError::simple(
                    span,
                    format!(
                        "the context argument of '{name}' cannot carry a labeled \
                         optional bundle"
                    ),
                ));
            }
            self.unify_ctx(
                &t_context(),
                &ctx_ty,
                span,
                &format!("the context argument of '{name}'"),
            )?;
            self.unify_ctx(
                &want_result,
                &result,
                span,
                &format!("the result of '{name}'"),
            )?;
            slots
                .into_iter()
                .map(|(row, dom)| harvest_slot(row, dom))
                .collect()
        } else {
            let (mut doms, result) = peel_func_chain(tv);
            if doms.is_empty() {
                return Err(TypeError::simple(
                    span,
                    format!(
                        "the binding for '{name}' must be a function taking a \
                         context as its first argument (e.g. via `let-inline ctx \
                         {name} .. = ..`)"
                    ),
                ));
            }
            let ctx_ty = doms.remove(0);
            self.unify_ctx(
                &t_context(),
                &ctx_ty,
                span,
                &format!("the context argument of '{name}'"),
            )?;
            self.unify_ctx(
                &want_result,
                &result,
                span,
                &format!("the result of '{name}'"),
            )?;
            // Optional command params, simplified (Sub-area 2): this grammar
            // has no def-site `?:param` marker, so a param counts as optional
            // exactly when its INFERRED domain resolves to `_ option` — i.e.
            // the body actually uses it as an `option` (`match p with Some ..
            // | None -> ..`). `CmdArgType.ty` then stores the option's INNER
            // type, matching the `[ty?; ..]` signature-lowering shape 1:1
            // (`lower_type_atom`'s `TypeAtom::Cmd` arm); `check_cmd_args`
            // re-wraps it in `option(..)` per call, since call-site args
            // always arrive pre-wrapped as `Some`/`None`
            // (`elaborate.rs`'s `app_arg_to_ast`).
            doms.into_iter()
                .map(|d| match resolve(&d).into_owned() {
                    MonoType::Variant(vname, mut vargs)
                        if vname == "option" && vargs.len() == 1 =>
                    {
                        optional(vargs.pop().unwrap())
                    }
                    _ => mandatory(d),
                })
                .collect()
        };
        let cmd_ty = if is_inline {
            MonoType::InlineCmd(params)
        } else {
            MonoType::BlockCmd(params)
        };
        Ok(generalize(self.ctx.level(), &cmd_ty))
    }

    /// `Ast::LetMathIn`'s scheme-building rule — the math-command analog of
    /// `command_scheme` above, but simpler: a math command has **no**
    /// implicit context argument (see `elaborate.rs`'s
    /// `elaborate_let_math`), so every domain of `tv`'s function-chain
    /// becomes a `CmdArgType` (the same optional-param heuristic as
    /// `command_scheme`), and the bare result — not a peeled first argument
    /// — must be `math`. A zero-arity binding (`tv` not a `Func` at all,
    /// e.g. `let-math \to = rel \`→\``) falls out naturally:
    /// `peel_func_chain` returns no domains and `tv` itself as the result.
    fn math_command_scheme(
        &mut self,
        name: &str,
        tv: MonoType,
        span: Option<Span>,
    ) -> Result<PolyType, TypeError> {
        let (doms, result) = peel_func_chain(tv);
        self.unify_ctx(
            &t_math_text(),
            &result,
            span,
            &format!("the result of math command '{name}'"),
        )?;
        let params: Vec<CmdArgType> = doms
            .into_iter()
            .map(|d| match resolve(&d).into_owned() {
                MonoType::Variant(vname, mut vargs) if vname == "option" && vargs.len() == 1 => {
                    optional(vargs.pop().unwrap())
                }
                _ => mandatory(d),
            })
            .collect();
        Ok(generalize(self.ctx.level(), &MonoType::MathCmd(params)))
    }

    /// `Ast::LetMathIn`'s V0_1 scheme-building rule — the `val math` analog
    /// of `math_command_scheme` above. The lowering
    /// (`v1/lower.rs::lower_bind_v1`) ALWAYS synthesizes exactly three
    /// trailing lambdas around a `val math` body — `fun ctx -> fun sub ->
    /// fun sup -> …` — so `tv`'s function chain always has at least 3
    /// domains; the LAST three are peeled off as `(d_ctx, d_sub, d_sup)`
    /// (`context`, `option math-text`, `option math-text`), the bare result
    /// must be `math-boxes`, and the REMAINING leading domains become the
    /// command's ordinary `CmdArgType` params.
    ///
    /// Like `command_scheme`'s V0_1 branch,
    /// a leading user parameter may be a `?(l = x, …)` bundle, so this uses
    /// the row-carrying `peel_func_chain_rows` + `harvest_slot` instead of
    /// the plain `_ option` heuristic. The synthesized ctx/sub/sup trailing
    /// trio can never legally carry a bundle (`lower_value_math` always
    /// wraps them in plain `fun`s, inferring `Row::Empty`) — guarded
    /// defensively, since an off-by-one here would silently turn `sub`/`sup`
    /// into a labeled slot or eat the last user param (the trio is at the
    /// TAIL of the domain chain, opposite inline/block where ctx is FIRST).
    fn math_command_scheme_v01(
        &mut self,
        name: &str,
        tv: MonoType,
        span: Option<Span>,
    ) -> Result<PolyType, TypeError> {
        let (mut slots, result) = peel_func_chain_rows(tv);
        if slots.len() < 3 {
            return Err(TypeError::simple(
                span,
                format!(
                    "'val math' command '{name}' must take a context and (via the \
                     synthesized `with sub sup`/`%math-attach-scripts` wrapper) two \
                     optional scripts as its trailing arguments — see the math-split spec"
                ),
            ));
        }
        let (row_sup, d_sup) = slots.pop().unwrap();
        let (row_sub, d_sub) = slots.pop().unwrap();
        let (row_ctx, d_ctx) = slots.pop().unwrap();
        for (which, row) in [
            ("context", &row_ctx),
            ("'sub'", &row_sub),
            ("'sup'", &row_sup),
        ] {
            if !matches!(&*resolve_row(row), Row::Empty) {
                return Err(TypeError::simple(
                    span,
                    format!(
                        "the {which} argument of 'val math' command '{name}' cannot \
                         carry a labeled optional bundle"
                    ),
                ));
            }
        }
        self.unify_ctx(
            &t_context(),
            &d_ctx,
            span,
            &format!("the context argument of 'val math' command '{name}'"),
        )?;
        self.unify_ctx(
            &t_option(t_math_text()),
            &d_sub,
            span,
            &format!("the 'sub' argument of 'val math' command '{name}'"),
        )?;
        self.unify_ctx(
            &t_option(t_math_text()),
            &d_sup,
            span,
            &format!("the 'sup' argument of 'val math' command '{name}'"),
        )?;
        self.unify_ctx(
            &t_math_boxes(),
            &result,
            span,
            &format!(
                "the result of 0.1 math command '{name}' — a `math-boxes`, \
                 usually via `read-math`"
            ),
        )?;
        let params: Vec<CmdArgType> = slots
            .into_iter()
            .map(|(row, dom)| harvest_slot(row, dom))
            .collect();
        Ok(generalize(self.ctx.level(), &MonoType::MathCmd(params)))
    }

    /// Shared by `check_itext`'s `IText::Cmd`, `check_btext`'s `BText::Cmd`,
    /// and `check_math_elem`'s `MathElem::Cmd`: check a command application's
    /// argument count (exact — every optional slot is either explicitly
    /// marked at the call site or `None`-padded by elaboration, so it is
    /// never actually *absent* from `args`) and each argument's type against
    /// `params`. An `optional` param's `args[i].arg` is always a
    /// `Some(..)`/`None` value (`app_arg_to_ast`'s desugaring), so it's
    /// checked against `option(param.ty)`, not `param.ty` directly.
    ///
    /// Each `args[i]` additionally carries
    /// a (possibly empty) supplied `?(l = e, …)` bundle (`args[i].opts`) —
    /// every label must be declared in that slot's own closed
    /// `param.opt_labels` map (upstream's `UnexpectedOptionalLabel`,
    /// `typechecker.ml:900-901`); a declared label this call omits simply
    /// defaults to `None` at runtime, nothing to check here for it. `opts`
    /// is `[]` for every 0.0.6-reachable call, so this loop is a no-op
    /// there.
    fn check_cmd_args(
        &mut self,
        env: &TypeEnv<'s>,
        name: &str,
        span: Span,
        params: &[CmdArgType],
        args: &[CmdArg<'s>],
    ) -> Result<(), TypeError> {
        if params.len() != args.len() {
            return Err(TypeError::simple(
                Some(span),
                format!(
                    "command '{name}' expects {} argument{}, got {}",
                    params.len(),
                    if params.len() == 1 { "" } else { "s" },
                    args.len()
                ),
            ));
        }
        for (i, (param, arg)) in params.iter().zip(args.iter()).enumerate() {
            for (label, val) in &arg.opts {
                match param.opt_labels.iter().find(|(l, _)| l == label) {
                    Some((_, lty)) => {
                        let tval = self.infer(env, val)?;
                        self.unify_ctx(
                            lty,
                            &tval,
                            ast_span(val).or(Some(span)),
                            &format!("optional argument `{label}` of '{name}'"),
                        )?;
                    }
                    None => {
                        return Err(TypeError::simple(
                            ast_span(val).or(Some(span)),
                            format!(
                                "command '{name}' has no optional label `{label}` \
                                 on argument {}",
                                i + 1
                            ),
                        ));
                    }
                }
            }
            let targ = self.infer(env, &arg.arg)?;
            let expected = if param.optional {
                t_option(param.ty.clone())
            } else {
                param.ty.clone()
            };
            self.unify_ctx(
                &expected,
                &targ,
                ast_span(&arg.arg).or(Some(span)),
                &format!("argument {} of '{name}'", i + 1),
            )?;
        }
        Ok(())
    }

    // ---- expressions -------------------------------------------------------

    /// Infer the scheme(s) of ONE binding against a static `env`,
    /// WITHOUT extending anything. Returns the
    /// `(name, PolyType)` pairs in binding order (singleton for
    /// Let/LetMath/LetMutable; group order for LetRec). The caller decides
    /// what to put in the environment — the whole-program path commits them
    /// verbatim (`env.with_all`).
    ///
    /// Level discipline, sigil dispatch, generalization, and the
    /// value-restriction asymmetry all live INSIDE this method, so callers
    /// cannot get them wrong.
    pub(crate) fn infer_binding(
        &mut self,
        env: &TypeEnv<'s>,
        binding: BindingView<'_, 's>,
    ) -> Result<Vec<(Symbol<'s>, PolyType)>, TypeError> {
        match binding {
            BindingView::Let { name, value } => {
                self.ctx.enter_level();
                let tv = self.infer(env, value)?;
                self.ctx.leave_level();
                let scheme = match command_sigil(self.text(name)) {
                    // A `\`/`+`-named binding: either a genuine `let-inline`/
                    // `let-block` definition (`value` is the
                    // `Lambda(ctxvar, Lambda(p1, .., body))` chain
                    // `elaborate_let_inline` builds) or a qualified-name
                    // alias of one (`value` is a bare `Ast::Var`, from a
                    // module's own `M.\cmd` re-export or an `open`) — see
                    // `command_scheme`.
                    Some(sigil) => {
                        self.command_scheme(self.text(name), sigil, tv, ast_span(value))?
                    }
                    None => generalize(self.ctx.level(), &tv),
                };
                Ok(vec![(name, scheme)])
            }

            // `let-math \cmd param* = expr in body` — structurally
            // identical to the `Let` command- binding rule above, but for a
            // binding that is ALREADY known (by construction, via the
            // dedicated Ast variant) to be a math command, so there is no
            // sigil to dispatch on and no "which kind of `\`-binding is
            // this" ambiguity to resolve.
            BindingView::LetMath { name, value } => {
                self.ctx.enter_level();
                let tv = self.infer(env, value)?;
                self.ctx.leave_level();
                // V0_0's `let-math` and V0_1's `val math` both lower to the
                // SAME `Ast::LetMathIn` — only the SCHEME RULE forks, since
                // a `val math` binding's lowering always synthesizes exactly
                // three trailing ctx/sub/sup lambdas that
                // `math_command_scheme`'s v0.0.6 rule knows nothing about.
                //
                // It forks on the BINDING's generation, not the session's
                // (`binding_version`, not `self.version`): in a merged
                // cross-version program the session is always `V0_1` while a
                // spliced 0.0.6 package's `let-math` RHS carries its own
                // `Ast::VersionScope(V0_0, _)`. On every single-version
                // program the two agree by construction (no `VersionScope`
                // node exists there at all).
                let scheme = if self.binding_version(value).math_is_split() {
                    self.math_command_scheme_v01(self.text(name), tv, ast_span(value))?
                } else {
                    self.math_command_scheme(self.text(name), tv, ast_span(value))?
                };
                Ok(vec![(name, scheme)])
            }

            BindingView::LetRec(bindings) => {
                self.ctx.enter_level();
                // The group's own stage, so a `@stage: 0` library's mutually
                // recursive clauses can still see each other (they are read at
                // stage 0, and a stage-0 read of a stage-1 binder is refused).
                let group_stage = self.binding_stage_rec(bindings);
                let mut rec_env = env.clone();
                let mut vars = Vec::with_capacity(bindings.len());
                for (name, _) in bindings {
                    let v = self.fresh();
                    vars.push(v.clone());
                    rec_env = rec_env.with(*name, PolyType::mono(v), group_stage);
                }
                for ((name, val), v) in bindings.iter().zip(vars.iter()) {
                    let tv = self.infer(&rec_env, val)?;
                    self.unify_ctx(
                        v,
                        &tv,
                        ast_span(val),
                        &format!("let-rec binding '{}'", self.text(*name)),
                    )?;
                }
                self.ctx.leave_level();
                let mut schemes = Vec::with_capacity(bindings.len());
                for ((name, _), v) in bindings.iter().zip(vars.iter()) {
                    let scheme = generalize(self.ctx.level(), v);
                    schemes.push((*name, scheme));
                }
                Ok(schemes)
            }

            BindingView::LetMutable { name, init } => {
                // NO generalization: `let-mutable`'s binding is the
                // classic ML "value restriction" case — a mutable reference
                // must stay monomorphic, or `let-mutable r <- [] in ((r <-
                // 1 :: !r); (r <- true :: !r); !r)`-style code could smuggle
                // an `int` and a `bool` through the very same cell. Binding
                // it via `PolyType::mono` (not `generalize`) enforces this
                // directly: every use of `name` in `body` shares the exact
                // same `Ref` type, not a fresh instantiation.
                let tinit = self.infer(env, init)?;
                Ok(vec![(name, PolyType::mono(reff(tinit)))])
            }
        }
    }

    /// Infer one expression against a static env — a `pub(crate)` wrapper
    /// over the private `infer` below, which stays private so the ~40
    /// internal `self.infer(` call sites need not change. `v1::module_check`
    /// uses this for the document body and any non-binding expression.
    pub(crate) fn infer_expr(
        &mut self,
        env: &TypeEnv<'s>,
        ast: &Ast<'s>,
    ) -> Result<MonoType, TypeError> {
        self.infer(env, ast)
    }

    /// Drain accumulated non-fatal match warnings (exhaustiveness /
    /// redundancy) — for per-binding callers; the whole-program path reads
    /// the `warnings` field directly.
    pub(crate) fn take_warnings(&mut self) -> Vec<MatchWarning> {
        std::mem::take(&mut self.warnings)
    }

    /// Mutable access to the inference context — sig lowering needs it
    /// (`lower_sig_item(item, &mut ctx)`), and its subsumption check mints
    /// fresh vars through it.
    pub(crate) fn ctx_mut(&mut self) -> &mut TypeContext {
        &mut self.ctx
    }

    /// Expand every synonym reference inside `ty` against this session's
    /// synonym table — a `pub(crate)` accessor over the private free fn
    /// [`expand_synonyms`], added for `v1/module_check.rs`:
    /// a sig `val`'s declared type may mention the
    /// module's own `type t = ..` synonym (by its pre-qualified `"M.t"`
    /// name, `v1/lower.rs`'s `TypeNameEnv`), and must expand through the
    /// SAME table the impl side's `build_variant_decl`/ordinary inference
    /// already does, so e.g. `val f : t -> t` over `type t = int` checks
    /// against the impl's expanded `int -> int`.
    pub(crate) fn expand_synonyms_in(&self, ty: &MonoType) -> Result<MonoType, TypeError> {
        expand_synonyms(ty, &self.synonyms)
    }

    /// Deregister constructors hidden by a signature seal (
    /// `v1/module_check.rs`'s ctor-hide trigger — see that module's doc
    /// comment). Each entry is removed only if the currently-registered
    /// decl's type name matches, guarding against bare-name ctor collisions
    /// (`Checker.ctors` is last-writer-wins program-globally, 0.0.6-
    /// inherited): if a LATER unsealed variant re-registered the same ctor
    /// name after this one, its entry survives the hide untouched.
    pub(crate) fn hide_ctors(&mut self, entries: &[(String, String)]) {
        for (ctor, tyname) in entries {
            if self.ctors.get(ctor).is_some_and(|d| &d.name == tyname) {
                self.ctors.remove(ctor);
            }
            // Also drop the module-qualified key registered by
            // `declare_variant` (guarded by the same decl-identity check).
            if let Some((modpfx, _)) = tyname.rsplit_once('.') {
                let q = format!("{modpfx}.{ctor}");
                if self.ctors.get(&q).is_some_and(|d| &d.name == tyname) {
                    self.ctors.remove(&q);
                }
            }
        }
    }

    /// `f ?(l = e, …) arg` — SATySFi 0.1 labeled-optional application
    /// inference. Kept OUT of the hot [`Checker::infer`] match (via
    /// `#[inline(never)]`) so its labeled-optional locals (`Vec`, `Row`) do
    /// not enlarge the deeply-recursed `infer` stack frame — see
    /// `infer_lambda_opt`.
    #[inline(never)]
    fn infer_apply_opt(
        &mut self,
        env: &TypeEnv<'s>,
        func: &Ast<'s>,
        opts: &[(String, Ast<'s>)],
        arg: &Ast<'s>,
    ) -> Result<MonoType, TypeError> {
        let tf = self.infer(env, func)?;
        let ta = self.infer(env, arg)?;
        let tr = self.fresh();
        let mut opt_tys = Vec::with_capacity(opts.len());
        for (label, e) in opts {
            opt_tys.push((label.clone(), self.infer(env, e)?));
        }
        let mut row = Row::Var(self.ctx.fresh_row_var());
        for (label, ty) in opt_tys.into_iter().rev() {
            row = Row::Cons(label, Box::new(ty), Box::new(row));
        }
        self.unify_ctx(
            &tf,
            &MonoType::Func(Box::new(row), Box::new(ta), Box::new(tr.clone())),
            ast_span(func),
            "function application",
        )?;
        Ok(tr)
    }

    /// `fun ?(l = x, …) p -> body` — SATySFi 0.1 labeled-optional lambda
    /// inference. Kept OUT of the hot [`Checker::infer`] match
    /// (`#[inline(never)]`): `infer` recurses to the AST depth of the program
    /// under check, and Rust sizes a function's stack frame to its LARGEST
    /// match arm's locals; inlining this arm's `env.clone()` + `Vec`/`Row`
    /// locals into every `infer` frame measurably enlarged the deep recursion
    /// (enough to overflow the test harness's small default thread stack on a
    /// big merged program). Extracting it restores `infer`'s frame to its
    /// pre-optional-arg size.
    #[inline(never)]
    fn infer_lambda_opt(
        &mut self,
        env: &TypeEnv<'s>,
        opts: &[(String, Symbol<'s>)],
        param: Symbol<'s>,
        body: &Ast<'s>,
    ) -> Result<MonoType, TypeError> {
        let mut inner = env.clone();
        let mut opt_tys = Vec::with_capacity(opts.len());
        for (label, binder) in opts {
            let tl = self.fresh();
            inner = inner.with(*binder, PolyType::mono(t_option(tl.clone())), self.stage);
            opt_tys.push((label.clone(), tl));
        }
        let tp = self.fresh();
        inner = inner.with(param, PolyType::mono(tp.clone()), self.stage);
        let tb = self.infer(&inner, body)?;
        let mut row = Row::Empty;
        for (label, tl) in opt_tys.into_iter().rev() {
            row = Row::Cons(label, Box::new(tl), Box::new(row));
        }
        Ok(MonoType::Func(Box::new(row), Box::new(tp), Box::new(tb)))
    }

    /// The stage a binding whose right-hand side is `value` is introduced at
    /// — upstream's `pre.stage` at the point of `Typeenv.add`.
    ///
    /// Upstream reads a whole FILE at one stage, so `pre.stage` is simply the
    /// ambient stage there. This port flattens every library into one
    /// `let`-chain and instead marks each spliced binding's RHS with
    /// [`Ast::StageScope`] (`elaborate.rs`'s per-item wrap), so the ambient
    /// stage is only right for a binding the current file wrote itself. The
    /// peeling through `ModuleScope`/`VersionScope` is because those wrappers
    /// are applied INSIDE `push_named_binding`/the `LetRec` arm, i.e. after
    /// the stage wrap, for a module member.
    ///
    /// Only the `ModuleScope` half of that peel actually fires: `elaborate.rs`
    /// applies `maybe_v006_scope` to a binding's RHS and `stage_wrap_item`
    /// outside it, so a cross-version staged binding is always
    /// `StageScope(_, VersionScope(_, ..))`. The `VersionScope` arm is kept
    /// anyway, because `elaborate::already_staged` peels the same two and the
    /// two must not disagree. Deleting the `ModuleScope` arm breaks
    /// `xver_staging.rs`'s
    /// `the_file_stage_and_the_version_scope_compose_on_a_module_member`.
    pub(crate) fn binding_stage(&self, value: &Ast<'s>) -> Stage {
        fn declared<'s>(a: &Ast<'s>) -> Option<Stage> {
            match a {
                Ast::StageScope(st, _) => Some(*st),
                Ast::ModuleScope(_, b) | Ast::VersionScope(_, b) => declared(b),
                _ => None,
            }
        }
        declared(value).unwrap_or(self.stage)
    }

    /// The GENERATION one binding was authored in — the version analogue of
    /// [`Checker::binding_stage`], and needed for exactly the same reason: a
    /// merged cross-version program has ONE `Checker::version`, hard-coded
    /// to `V0_1` (`v1::module_check::check_program_inner`), while each
    /// spliced 0.0.6 dependency's bindings carry their own
    /// `Ast::VersionScope(V0_0, _)` on the RHS (`elaborate::
    /// maybe_v006_scope`). Any scheme rule that FORKS on the version must
    /// ask the binding, not the session, or a 0.0.6 package's binding is
    /// read under 0.1's rule.
    ///
    /// Concretely: `let-math`. `math_command_scheme` (0.0.6) and
    /// `math_command_scheme_v01` are two different rules for the same
    /// `Ast::LetMathIn`, and 0.1's demands three synthesized trailing
    /// `ctx`/`sub`/`sup` lambdas a 0.0.6 `let-math \frac = math-frac` does
    /// not have — dispatching on `self.version` refused EVERY `let-math` in
    /// a crossed 0.0.6 package, including the bundled `math.satyh` that
    /// `@require:` reaches transitively (`texlogo`, `latexcmds`, `siunitx`,
    /// …).
    ///
    /// The wrapper peel is `binding_stage`'s, minus its terminating arm:
    /// `elaborate::walk_bindings` puts `VersionScope` INSIDE `StageScope`
    /// (`already_staged`'s doc comment), so a staged spliced binding is
    /// `StageScope(_, VersionScope(_, ..))` and this must look through
    /// `StageScope` too.
    ///
    /// The peel alone is not enough, hence the `scoped_version` fallback:
    /// `maybe_v006_scope` wraps a TOP-LEVEL binding's RHS, but an
    /// EXPRESSION-level `let-math \c = e in body` (`elaborate.rs`'s
    /// `Expr::LetMathIn` arm, e.g. `siunitx`'s `let-math \C = ord \`C\` in
    /// ${\math-sup{}{\circ}\C}`) is a node inside another binding's
    /// already-wrapped RHS and carries no wrapper of its own — so the
    /// ambient generation, recorded by the `Ast::VersionScope` infer arm as
    /// it descends, answers for it instead. Outside every scope this is
    /// `None` and the session's own version answers.
    pub(crate) fn binding_version(&self, value: &Ast<'s>) -> RustyfiVersion {
        fn declared<'s>(a: &Ast<'s>) -> Option<RustyfiVersion> {
            match a {
                Ast::VersionScope(v, _) => Some(*v),
                Ast::StageScope(_, b) | Ast::ModuleScope(_, b) => declared(b),
                _ => None,
            }
        }
        declared(value)
            .or(self.scoped_version)
            .unwrap_or(self.version)
    }

    /// [`Checker::binding_stage`] for a `let-rec` GROUP. Every clause of one
    /// group comes from one item of one file, so they share a stage; the
    /// elaborator wraps them identically and the first is representative.
    pub(crate) fn binding_stage_rec(&self, bindings: &[(Symbol<'s>, Rc<Ast<'s>>)]) -> Stage {
        match bindings.first() {
            Some((_, v)) => self.binding_stage(v),
            None => self.stage,
        }
    }

    /// Look `name` up for an OCCURRENCE, enforcing the staging matrix
    /// ([`Stage::can_reference`]) — upstream's `UTContentOf` arm, which is
    /// where a stage-0 name used from stage 1 (or the reverse) is refused.
    ///
    /// `Ok(None)` means unbound, left to each caller because each has its own
    /// "should not happen post-elaboration" wording. `what` names the kind of
    /// occurrence for the diagnostic (`"variable"`, `"inline command"`, …).
    fn staged<'e>(
        &self,
        env: &'e TypeEnv<'s>,
        name: Symbol<'s>,
        span: Option<Span>,
        what: &str,
    ) -> Result<Option<&'e PolyType>, TypeError> {
        let Some(entry) = env.entry(name) else {
            return Ok(None);
        };
        if !self.stage.can_reference(entry.stage) {
            return Err(TypeError::simple(
                span,
                format!(
                    "invalid occurrence of {what} '{}' as to stage: it is bound at {}, \
                     but this is {}",
                    self.text(name),
                    entry.stage.as_str(),
                    self.stage.as_str()
                ),
            ));
        }
        Ok(Some(&entry.poly))
    }

    /// `infer`, reading `ast` at a different stage and restoring afterwards --
    /// the type-side twin of the `ctor_scope` push/pop below it.
    fn infer_at(
        &mut self,
        stage: Stage,
        env: &TypeEnv<'s>,
        ast: &Ast<'s>,
    ) -> Result<MonoType, TypeError> {
        let saved = std::mem::replace(&mut self.stage, stage);
        let result = self.infer(env, ast);
        self.stage = saved;
        result
    }

    fn infer(&mut self, env: &TypeEnv<'s>, ast: &Ast<'s>) -> Result<MonoType, TypeError> {
        match ast {
            // A binding spliced in from a file whose `@stage:` was not the
            // default: read it at that stage, so its quotes are legal.
            Ast::StageScope(stage, body) => self.infer_at(*stage, env, body),
            // `&e` — quote. Legal only at stage 0; its body is read one
            // stage later, and the result is that body's type wrapped in
            // `code` (upstream `typechecker.ml`'s `UTNext` arm).
            Ast::Next(inner) => {
                if self.stage != Stage::Stage0 {
                    return Err(TypeError::simple(
                        None,
                        format!(
                            "`&` (next-stage quote) is only valid at stage 0, but this is {}",
                            self.stage.as_str()
                        ),
                    ));
                }
                let ty = self.infer_at(Stage::Stage1, env, inner)?;
                Ok(MonoType::Code(Box::new(ty)))
            }
            // `~e` — splice. Legal only at stage 1; its body is read one stage
            // earlier and must produce `code b`, which this expression then
            // stands for (upstream's `UTPrev` arm).
            Ast::Prev(inner) => {
                if self.stage != Stage::Stage1 {
                    return Err(TypeError::simple(
                        None,
                        format!(
                            "`~` (previous-stage splice) is only valid at stage 1, but this is {}",
                            self.stage.as_str()
                        ),
                    ));
                }
                let ty = self.infer_at(Stage::Stage0, env, inner)?;
                let beta = MonoType::Var(self.ctx.fresh_var());
                unify(&ty, &MonoType::Code(Box::new(beta.clone())))
                    .map_err(|e| TypeError::from_unify(None, "a `~` splice", e))?;
                Ok(beta)
            }
            Ast::Unit => Ok(t_unit()),
            Ast::Bool(_) => Ok(t_bool()),
            Ast::Int(_) => Ok(t_int()),
            Ast::Float(_) => Ok(t_float()),
            Ast::Length(_) => Ok(t_length()),
            Ast::Str(_) => Ok(t_string()),

            Ast::Var(name, span) => match self.staged(env, *name, Some(*span), "variable")? {
                Some(poly) => Ok(instantiate(poly, self.ctx.level())),
                // Should not happen post-elaboration: `elaborate.rs`'s
                // `scoped_var` already rejects any unbound name before this
                // ever runs. Surfaced as a (spanned) error rather than a
                // panic anyway, since "should not happen" isn't "cannot".
                None => Err(TypeError::simple(
                    Some(*span),
                    format!(
                        "internal error: unbound variable '{}' reached the typechecker",
                        self.text(*name)
                    ),
                )),
            },

            Ast::Apply(f, a) => {
                let tf = self.infer(env, f)?;
                let ta = self.infer(env, a)?;
                let tr = self.fresh();
                // Version-split the optional-argument row: 0.0.6 functions
                // provably carry no labeled optionals (`Row::Empty`, matching
                // `arrow()` and every prim). Under 0.1 a fresh open row var
                // absorbs the callee's
                // declared optional row, letting a *plain* call of an
                // opt-taking function typecheck (defaulting every optional to
                // `None` at run time) and making higher-order code
                // row-polymorphic after generalization.
                let opts_row = if self.version.has_row_polymorphism() {
                    Row::Var(self.ctx.fresh_row_var())
                } else {
                    Row::Empty
                };
                self.unify_ctx(
                    &tf,
                    &MonoType::Func(Box::new(opts_row), Box::new(ta), Box::new(tr.clone())),
                    ast_span(f),
                    "function application",
                )?;
                Ok(tr)
            }

            Ast::Lambda(param, body) => {
                let tp = self.fresh();
                let inner = env.with(*param, PolyType::mono(tp.clone()), self.stage);
                let tb = self.infer(&inner, body)?;
                Ok(arrow(tp, tb))
            }

            // `f ?(l = e, …) arg` (SATySFi 0.1; upstream typechecker.ml's
            // `Apply(labmap, …)`). The callee must unify with a function
            // whose optional row carries at least each supplied `l : τ_l`,
            // with an open tail (a fresh row var) for any further optionals
            // the callee declares that this call omits.
            Ast::ApplyOpt { func, opts, arg } => self.infer_apply_opt(env, func, opts, arg),

            // `fun ?(l = x, …) p -> body` (SATySFi 0.1; upstream
            // `Function(evid_labmap, …)`). Each labeled optional binder `x`
            // is bound at `option τ_l` inside the body; the resulting
            // function type carries a CLOSED row `?(l : τ_l, …)` — the very
            // same fresh `τ_l` shared between the binder's `option τ_l` and
            // the row's `Cons(l, τ_l)`.
            Ast::LambdaOpt { opts, param, body } => self.infer_lambda_opt(env, opts, *param, body),

            Ast::LetIn(name, value, body) => {
                let schemes = self.infer_binding(env, BindingView::Let { name: *name, value })?;
                let inner = env.with_all(schemes, self.binding_stage(value));
                self.infer(&inner, body)
            }

            // `let-math \cmd param* = expr in body` — see
            // `infer_binding`'s `BindingView::LetMath` arm.
            Ast::LetMathIn(name, value, body) => {
                let schemes =
                    self.infer_binding(env, BindingView::LetMath { name: *name, value })?;
                let inner = env.with_all(schemes, self.binding_stage(value));
                self.infer(&inner, body)
            }

            Ast::LetRecIn(bindings, body) => {
                let schemes = self.infer_binding(env, BindingView::LetRec(bindings))?;
                let inner = env.with_all(schemes, self.binding_stage_rec(bindings));
                self.infer(&inner, body)
            }

            Ast::IfThenElse(cond, then_b, else_b) => {
                let tc = self.infer(env, cond)?;
                self.unify_ctx(&t_bool(), &tc, ast_span(cond), "the condition of 'if'")?;
                let tt = self.infer(env, then_b)?;
                let te = self.infer(env, else_b)?;
                self.unify_ctx(&tt, &te, ast_span(else_b), "the branches of 'if'")?;
                Ok(tt)
            }

            Ast::Match(scrutinee, arms) => {
                let tscrut = self.infer(env, scrutinee)?;
                let mut result: Option<MonoType> = None;
                for arm in arms {
                    let arm_env = self.bind_pattern(env.clone(), &arm.pat, &tscrut)?;
                    if let Some(guard) = &arm.guard {
                        let tg = self.infer(&arm_env, guard)?;
                        self.unify_ctx(&t_bool(), &tg, ast_span(guard), "a match guard")?;
                    }
                    let tbody = self.infer(&arm_env, &arm.body)?;
                    match &result {
                        None => result = Some(tbody),
                        Some(r) => {
                            self.unify_ctx(r, &tbody, ast_span(&arm.body), "the arms of 'match'")?
                        }
                    }
                }
                // Exhaustiveness/redundancy: non-fatal, so it runs
                // only after every arm has
                // typechecked, against `tscrut` as resolved as inference will
                // ever make it. See `exhaustive::check_match`'s doc comment.
                let resolved_scrut = resolve(&tscrut);
                let new_warnings = crate::exhaustive::check_match(
                    self.store,
                    &resolved_scrut,
                    ast_span(scrutinee),
                    arms,
                    &self.variants,
                );
                self.warnings.extend(new_warnings);
                // `Match`'s `arms` is always non-empty (`c::Expr::Match`
                // requires a `first` arm plus zero or more `rest`), so
                // `result` is always `Some` in practice; the fallback fresh
                // variable is defensive only.
                Ok(result.unwrap_or_else(|| self.fresh()))
            }

            Ast::Tuple(items) => {
                let tys = items
                    .iter()
                    .map(|it| self.infer(env, it))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(product(tys))
            }

            Ast::Ctor(name, payload) => self.infer_ctor(env, name, payload.as_deref(), None),

            Ast::Record(fields) => {
                let mut typed = Vec::with_capacity(fields.len());
                for (label, e) in fields {
                    typed.push((label.clone(), self.infer(env, e)?));
                }
                let mut row = Row::Empty;
                for (label, ty) in typed.into_iter().rev() {
                    row = Row::Cons(label, Box::new(ty), Box::new(row));
                }
                Ok(MonoType::Record(row))
            }

            Ast::List(items) => {
                let elem = self.fresh();
                for it in items {
                    let t = self.infer(env, it)?;
                    self.unify_ctx(&elem, &t, ast_span(it), "a list element")?;
                }
                Ok(list(elem))
            }

            Ast::InlineText(elems) => {
                for e in elems.iter() {
                    self.check_itext(env, e)?;
                }
                Ok(t_inline_text())
            }

            Ast::BlockText(elems) => {
                for e in elems.iter() {
                    self.check_btext(env, e)?;
                }
                Ok(t_block_text())
            }

            Ast::MathText(elems) => {
                for e in elems.iter() {
                    self.check_math_elem(env, e)?;
                }
                Ok(MonoType::Base(BaseType::MathText))
            }

            Ast::LetMutableIn(name, init, body) => {
                // NO generalization: `let-mutable`'s binding is the
                // classic ML "value restriction" case — see
                // `infer_binding`'s `BindingView::LetMutable` arm.
                let schemes =
                    self.infer_binding(env, BindingView::LetMutable { name: *name, init })?;
                let inner = env.with_all(schemes, self.binding_stage(init));
                self.infer(&inner, body)
            }

            Ast::Overwrite(name, span, value) => {
                let t_ref = match self.staged(env, *name, Some(*span), "mutable variable")? {
                    Some(poly) => instantiate(poly, self.ctx.level()),
                    None => {
                        return Err(TypeError::simple(
                            Some(*span),
                            format!(
                            "internal error: unbound mutable variable '{}' reached the typechecker",
                            self.text(*name)
                        ),
                        ))
                    }
                };
                let inner = self.fresh();
                self.unify_ctx(
                    &t_ref,
                    &reff(inner.clone()),
                    Some(*span),
                    &format!("the overwrite target '{}'", self.text(*name)),
                )?;
                let tvalue = self.infer(env, value)?;
                // Prefer the overwrite's own (always-present) span over
                // `ast_span(value)`, which is `None` for most value shapes
                // (literals carry no span at all — see `ast.rs`'s module
                // doc comment) and would otherwise leave this common error
                // unlocated.
                self.unify_ctx(
                    &inner,
                    &tvalue,
                    ast_span(value).or(Some(*span)),
                    &format!("the overwrite value for '{}'", self.text(*name)),
                )?;
                Ok(t_unit())
            }

            Ast::WhileDo(cond, body) => {
                let tc = self.infer(env, cond)?;
                self.unify_ctx(&t_bool(), &tc, ast_span(cond), "the condition of 'while'")?;
                let tb = self.infer(env, body)?;
                self.unify_ctx(&t_unit(), &tb, ast_span(body), "the body of 'while'")?;
                Ok(t_unit())
            }

            Ast::Sequential(a, b) => {
                let ta = self.infer(env, a)?;
                // v0.0.6 requires the left-hand side of `before`/`;` to be
                // `unit` (`typechecker.ml`'s `UTSequential` case): not just
                // "evaluated and discarded" but type-checked as `unit`
                // specifically, so e.g. a stray non-unit expression used
                // only for effect (but returning, say, an `int`) is rejected
                // rather than silently ignored.
                self.unify_ctx(
                    &t_unit(),
                    &ta,
                    ast_span(a),
                    "the left-hand side of 'before'",
                )?;
                self.infer(env, b)
            }

            Ast::AccessField(e, label, span) => {
                let te = self.infer(env, e)?;
                let field = self.fresh();
                let rv = self.ctx.fresh_row_var();
                let open_row = MonoType::Record(Row::Cons(
                    label.clone(),
                    Box::new(field.clone()),
                    Box::new(Row::Var(rv)),
                ));
                self.unify_ctx(
                    &open_row,
                    &te,
                    Some(*span),
                    &format!("the field access '#{label}'"),
                )?;
                Ok(field)
            }

            Ast::UpdateField(base, label, value) => {
                let tbase = self.infer(env, base)?;
                let tvalue = self.infer(env, value)?;
                let rv = self.ctx.fresh_row_var();
                let open_row = MonoType::Record(Row::Cons(
                    label.clone(),
                    Box::new(tvalue),
                    Box::new(Row::Var(rv)),
                ));
                self.unify_ctx(
                    &open_row,
                    &tbase,
                    ast_span(base),
                    &format!("the record update of '{label}'"),
                )?;
                Ok(tbase)
            }

            // Swap the active primitive-type env to `version`'s
            // for `body` — see `version_scoped_type_env`'s doc comment —
            // and make sure `version`'s builtin ADTs (e.g. `page`,
            // `V0_0`-only) are registered in the (otherwise
            // whole-program-tagged) ctor table — see
            // `install_additional_builtin_variants`'s doc comment. Never
            // reached on a pure single-version program (no
            // `Ast::VersionScope` node is ever produced there).
            Ast::VersionScope(version, body) => {
                self.install_additional_builtin_variants(*version);
                let scoped = version_scoped_type_env(self.store, env, *version);
                // Also make the generation available to any version-forking
                // rule reached INSIDE the body — an expression-level
                // `let-math .. in ..` is the one that needs it, since its
                // own `Ast::LetMathIn` carries no wrapper of its own. See
                // `Checker::binding_version`.
                let saved = self.scoped_version.replace(*version);
                let r = self.infer(&scoped, body);
                self.scoped_version = saved;
                r
            }
            // A module member's body: resolve its bare constructor references
            // against `path`'s constructors first. `path` is the full absolute
            // module path (nested modules wrap with `["M","N"]`), so replace
            // rather than push.
            Ast::ModuleScope(path, body) => {
                let saved = std::mem::replace(&mut self.ctor_scope, path.clone());
                let r = self.infer(env, body);
                self.ctor_scope = saved;
                r
            }
        }
    }

    /// Look up a constructor honoring the current [`Checker::ctor_scope`]: try
    /// the innermost-out module-qualified keys (`M.N.Ctor`, `M.Ctor`) before
    /// the bare fallback (`Ctor`). Keeps the returned decl's ctor NAME strings
    /// bare — only the table KEY is qualified — so eval/exhaustiveness/error
    /// text are untouched.
    fn lookup_ctor(&self, name: &str) -> Option<Rc<VariantDecl>> {
        for k in (1..=self.ctor_scope.len()).rev() {
            let key = format!("{}.{}", self.ctor_scope[..k].join("."), name);
            if let Some(d) = self.ctors.get(&key) {
                return Some(d.clone());
            }
        }
        self.ctors.get(name).cloned()
    }

    /// Shared by `Ast::Ctor` and pattern-matching's `Pattern::Ctor`: look up
    /// `name`'s declaration, mint fresh type arguments for its (possibly
    /// zero) parameters, and check the payload — either an already-inferred
    /// expression type to unify against (`Ast::Ctor`'s case, via `infer`
    /// directly) or nothing (patterns bind their own payload separately, in
    /// `bind_pattern`). `expected_result`, if given, is unified against the
    /// application's result type — used by nothing yet in this port's
    /// rules but kept general for symmetry; always `None` from `infer`,
    /// which just returns the result type instead.
    fn infer_ctor(
        &mut self,
        env: &TypeEnv<'s>,
        name: &str,
        payload: Option<&Ast<'s>>,
        expected_result: Option<&MonoType>,
    ) -> Result<MonoType, TypeError> {
        let decl = self
            .lookup_ctor(name)
            .ok_or_else(|| TypeError::simple(None, format!("unknown constructor '{name}'")))?;
        let args: Vec<MonoType> = (0..decl.params).map(|_| self.fresh()).collect();
        let (payload_ty, result_ty) = decl.instantiate_ctor(name, &args).ok_or_else(|| {
            TypeError::simple(
                None,
                format!("constructor '{name}' applied with the wrong number of type arguments"),
            )
        })?;
        if let Some(expected) = expected_result {
            self.unify_ctx(expected, &result_ty, None, &format!("constructor '{name}'"))?;
        }
        match (payload_ty, payload) {
            (Some(expected), Some(actual)) => {
                let actual_ty = self.infer(env, actual)?;
                self.unify_ctx(
                    &expected,
                    &actual_ty,
                    ast_span(actual),
                    &format!("the payload of constructor '{name}'"),
                )?;
            }
            (None, None) => {}
            (Some(_), None) => {
                return Err(TypeError::simple(
                    None,
                    format!("constructor '{name}' expects a payload but none was given"),
                ))
            }
            (None, Some(_)) => {
                return Err(TypeError::simple(
                    None,
                    format!("constructor '{name}' takes no payload but one was given"),
                ))
            }
        }
        Ok(result_ty)
    }

    // ---- patterns ------------------------------------------------------

    /// Type-check `pat` against `ty`, extending (a clone of) `env` with
    /// every name it binds. Mirrors `typechecker.ml`'s `typecheck_pattern`.
    fn bind_pattern(
        &mut self,
        env: TypeEnv<'s>,
        pat: &Pattern<'s>,
        ty: &MonoType,
    ) -> Result<TypeEnv<'s>, TypeError> {
        match pat {
            Pattern::Wild => Ok(env),
            Pattern::Var(name) => Ok(env.with(*name, PolyType::mono(ty.clone()), self.stage)),
            Pattern::Unit => {
                self.unify_ctx(&t_unit(), ty, None, "a unit pattern")?;
                Ok(env)
            }
            Pattern::Bool(_) => {
                self.unify_ctx(&t_bool(), ty, None, "a boolean pattern")?;
                Ok(env)
            }
            Pattern::Int(_) => {
                self.unify_ctx(&t_int(), ty, None, "an integer pattern")?;
                Ok(env)
            }
            Pattern::Str(_) => {
                self.unify_ctx(&t_string(), ty, None, "a string pattern")?;
                Ok(env)
            }
            Pattern::Tuple(pats) => {
                let elem_tys: Vec<MonoType> = pats.iter().map(|_| self.fresh()).collect();
                self.unify_ctx(&product(elem_tys.clone()), ty, None, "a tuple pattern")?;
                let mut env = env;
                for (p, t) in pats.iter().zip(elem_tys.iter()) {
                    env = self.bind_pattern(env, p, t)?;
                }
                Ok(env)
            }
            Pattern::EmptyList => {
                let elem = self.fresh();
                self.unify_ctx(&list(elem), ty, None, "an empty-list pattern")?;
                Ok(env)
            }
            Pattern::Cons(head, tail) => {
                let elem = self.fresh();
                self.unify_ctx(&list(elem.clone()), ty, None, "a cons pattern")?;
                let env = self.bind_pattern(env, head, &elem)?;
                self.bind_pattern(env, tail, &list(elem))
            }
            Pattern::Ctor(name, payload) => {
                let decl = self.lookup_ctor(name).ok_or_else(|| {
                    TypeError::simple(None, format!("unknown constructor '{name}' in a pattern"))
                })?;
                let args: Vec<MonoType> = (0..decl.params).map(|_| self.fresh()).collect();
                let (payload_ty, result_ty) = decl.instantiate_ctor(name, &args).ok_or_else(|| {
                    TypeError::simple(
                        None,
                        format!(
                            "constructor '{name}' applied with the wrong number of type arguments in a pattern"
                        ),
                    )
                })?;
                self.unify_ctx(
                    &result_ty,
                    ty,
                    None,
                    &format!("the constructor pattern '{name}'"),
                )?;
                match (payload_ty, payload) {
                    (Some(expected), Some(p)) => self.bind_pattern(env, p, &expected),
                    (None, None) => Ok(env),
                    (Some(_), None) => Err(TypeError::simple(
                        None,
                        format!(
                            "constructor pattern '{name}' expects a payload but none was given"
                        ),
                    )),
                    (None, Some(_)) => Err(TypeError::simple(
                        None,
                        format!("constructor pattern '{name}' takes no payload but one was given"),
                    )),
                }
            }
            Pattern::As(inner, name) => {
                let env = self.bind_pattern(env, inner, ty)?;
                Ok(env.with(*name, PolyType::mono(ty.clone()), self.stage))
            }
        }
    }

    // ---- inline / block / math text -------------------------------------

    /// Check one inline-text element. A command's own type is a genuine
    /// `MonoType::InlineCmd(params)` (`[...] inline-cmd`, mirroring v0.0.6's
    /// `HorzCommandType`) — bound either by `Ast::LetIn`'s command-binding
    /// rule (`Checker::command_scheme`) or, for the port's built-in
    /// commands, directly by `prim_types::primitive_type`'s `\emph` entry.
    /// Checking an application here is exact-arity plus one unification per
    /// argument against `params`, via `check_cmd_args` — there is no
    /// `context -> arg1 -> .. -> inline-boxes` function shape to unify the
    /// whole command type against.
    fn check_itext(&mut self, env: &TypeEnv<'s>, it: &IText<'s>) -> Result<(), TypeError> {
        match it {
            IText::Text(_) | IText::CodeText(_) => Ok(()),
            IText::Cmd { name, span, args } => {
                let tcmd = match self.staged(env, *name, Some(*span), "inline command")? {
                    Some(poly) => instantiate(poly, self.ctx.level()),
                    None => {
                        return Err(TypeError::simple(
                            Some(*span),
                            format!(
                            "internal error: unbound inline command '{}' reached the typechecker",
                            self.text(*name)
                        ),
                        ))
                    }
                };
                match &*resolve(&tcmd) {
                    MonoType::InlineCmd(params) => {
                        self.check_cmd_args(env, self.text(*name), *span, &params, args)
                    }
                    other => Err(TypeError::simple(
                        Some(*span),
                        format!(
                            "internal error: inline command '{}' does not have an \
                             inline-cmd type (found `{other}`)",
                            self.text(*name)
                        ),
                    )),
                }
            }
            IText::Embed { expr, span } => {
                let te = self.infer(env, expr)?;
                self.unify_ctx(
                    &t_inline_text(),
                    &te,
                    Some(*span),
                    "an inline-text '#…;' embed",
                )?;
                Ok(())
            }
            IText::EmbedMath { elems, span: _ } => {
                // PERMISSIVE: there is no real type to check a quoted-math
                // embed's expressions against. Type each against its own
                // fresh variable, purely so unbound-name mistakes inside it
                // are still caught, without asserting anything about the
                // result.
                for me in elems.iter() {
                    self.check_math_elem(env, me)?;
                }
                Ok(())
            }
        }
    }

    /// Block-text analogue of `check_itext`'s `IText::Cmd` case — see its
    /// doc comment; a `BText::Cmd`'s type is `MonoType::BlockCmd(params)`.
    fn check_btext(&mut self, env: &TypeEnv<'s>, bt: &BText<'s>) -> Result<(), TypeError> {
        match bt {
            BText::Cmd { name, span, args } => {
                let tcmd = match self.staged(env, *name, Some(*span), "block command")? {
                    Some(poly) => instantiate(poly, self.ctx.level()),
                    None => {
                        return Err(TypeError::simple(
                            Some(*span),
                            format!(
                            "internal error: unbound block command '{}' reached the typechecker",
                            self.text(*name)
                        ),
                        ))
                    }
                };
                match &*resolve(&tcmd) {
                    MonoType::BlockCmd(params) => {
                        self.check_cmd_args(env, self.text(*name), *span, &params, args)
                    }
                    other => Err(TypeError::simple(
                        Some(*span),
                        format!(
                            "internal error: block command '{}' does not have a \
                             block-cmd type (found `{other}`)",
                            self.text(*name)
                        ),
                    )),
                }
            }
            BText::Embed { expr, span } => {
                let te = self.infer(env, expr)?;
                self.unify_ctx(
                    &t_block_text(),
                    &te,
                    Some(*span),
                    "a block-text '#…;' embed",
                )?;
                Ok(())
            }
        }
    }

    /// Walk one quoted math element. `Chars`/`Group`/`Sub`/`Sup`/`Primes`
    /// carry no program-mode content of their own (nothing to check beyond
    /// recursing). `Cmd`/`Embed` are where math meets the ordinary
    /// expression language: a `Cmd`'s `name` must resolve to a genuine
    /// `MathCmd` type (checked exactly like `check_itext`'s `IText::Cmd`,
    /// via `check_cmd_args` — a math command's optional `?:`/`?*`-marked
    /// or marker-less-padded arguments are handled by `check_cmd_args` the
    /// same generic way), and an `Embed`'s (`#expr`) type must unify with
    /// `math` (a math command parameter, or another program-mode value
    /// that itself produces math — `Value::Math`/ `Value::MathText` are
    /// the two runtime shapes this unifies against, see `value.rs`).
    fn check_math_elem(&mut self, env: &TypeEnv<'s>, m: &MathElem<'s>) -> Result<(), TypeError> {
        match m {
            MathElem::Chars(_) => Ok(()),
            MathElem::Group(elems) => {
                for e in elems {
                    self.check_math_elem(env, e)?;
                }
                Ok(())
            }
            MathElem::Sub(base, script) | MathElem::Sup(base, script) => {
                self.check_math_elem(env, base)?;
                for e in script {
                    self.check_math_elem(env, e)?;
                }
                Ok(())
            }
            MathElem::Primes(base, _) => self.check_math_elem(env, base),
            MathElem::Cmd { name, span, args } => {
                let tcmd = match self.staged(env, *name, Some(*span), "math command")? {
                    Some(poly) => instantiate(poly, self.ctx.level()),
                    None => {
                        return Err(TypeError::simple(
                            Some(*span),
                            format!(
                                "internal error: unbound math command '{}' reached the typechecker",
                                self.text(*name)
                            ),
                        ))
                    }
                };
                match &*resolve(&tcmd) {
                    MonoType::MathCmd(params) => {
                        self.check_cmd_args(env, self.text(*name), *span, &params, args)
                    }
                    other => Err(TypeError::simple(
                        Some(*span),
                        format!(
                            "internal error: math command '{}' does not have a \
                             math-cmd type (found `{other}`)",
                            self.text(*name)
                        ),
                    )),
                }
            }
            MathElem::Embed { expr, span } => {
                let te = self.infer(env, expr)?;
                self.unify_ctx(&t_math_text(), &te, Some(*span), "a math '#…' embed")?;
                Ok(())
            }
        }
    }
}

/// If `name` (an `Ast::LetIn` binding's name) is command-shaped, the sigil
/// that says which kind — `'\\'` for an inline command, `'+'` for a block
/// command — else `None` for an ordinary variable binding.
///
/// Looks only at the *local* segment (after the last `.`): a
/// module-qualified command is spelled e.g. `"M.\cmd"`, sigil on the local
/// part only (module names can never start with `\`/`+` — `qualify_key`'s
/// doc comment). A bare name has no `.` at all, so `rsplit('.').next()`
/// degrades to the whole string.
///
/// **Must also check the second character.** A genuine command sigil is
/// always immediately followed by an identifier, but a parenthesized
/// operator NAME (`cst.rs`'s `BindName`) can merely *start* with the same
/// character, e.g. `let (+++>) = ..` (`itemize.satyh`) or `let (+.) = ..`.
/// Requiring an alphabetic second character mirrors the lexer's own split
/// and keeps such an operator name an ordinary variable binding rather than
/// a false-positive command.
fn command_sigil(name: &str) -> Option<char> {
    let local = name.rsplit('.').next().unwrap_or(name);
    let mut chars = local.chars();
    match chars.next() {
        Some(c @ ('\\' | '+')) if chars.next().is_some_and(|c2| c2.is_ascii_alphabetic()) => {
            Some(c)
        }
        _ => None,
    }
}

/// Greedily unwrap a (resolved) `Func` chain into its list of domains and
/// final codomain: `dom1 -> dom2 -> .. -> domN -> result` becomes
/// `(vec![dom1, .., domN], result)`. Only ever follows the *codomain* at
/// each step (never recurses into a domain, even one that is itself a
/// `Func`) — used by `Checker::command_scheme` to recover a `let-inline`/
/// `let-block` binding's `context -> arg1 -> .. -> argN -> result` shape
/// from its ordinarily-inferred function type.
fn peel_func_chain(ty: MonoType) -> (Vec<MonoType>, MonoType) {
    let mut doms = Vec::new();
    let mut cur = ty;
    loop {
        // Owned: this walk moves each arrow's domain/codomain out.
        match resolve(&cur).into_owned() {
            MonoType::Func(_row, dom, cod) => {
                doms.push(*dom);
                cur = *cod;
            }
            other => return (doms, other),
        }
    }
}

/// [`peel_func_chain`]'s row-carrying twin:
/// same greedy unwrap, but keeps each arrow's own (resolved) optional-
/// argument [`Row`] alongside its domain, since a V0_1 command's `LambdaOpt`-
/// produced arrows carry each parameter's `?(l:τ,…)` bundle on that
/// PARAMETER's own arrow (the arrow whose *domain* is the labeled argument —
/// see `Checker::command_scheme`'s V0_1 harvest). Used only by
/// `command_scheme`; `check_cmd_args`/`math_command_scheme*` still use the
/// row-blind `peel_func_chain` (they never harvest labels).
fn peel_func_chain_rows(ty: MonoType) -> (Vec<(Row, MonoType)>, MonoType) {
    let mut slots = Vec::new();
    let mut cur = ty;
    loop {
        // Owned, as in `peel_func_chain`.
        match resolve(&cur).into_owned() {
            MonoType::Func(row, dom, cod) => {
                slots.push((*row, *dom));
                cur = *cod;
            }
            other => return (slots, other),
        }
    }
}

/// Turn one V0_1 command parameter's [`Row`] (the row `Ast::LambdaOpt`'s
/// inference leaves on the `Func` arrow whose *domain* is that parameter —
/// see [`peel_func_chain_rows`]) into a closed-label-map [`CmdArgType`]: walk
/// the (resolved) row's `Cons` chain into a `Vec<(String, MonoType)>`, sorted
/// by label so `unify_cmd_args`'s equal-domain zip is order-insensitive.
/// A leftover `Row::Var` (an
/// under-constrained/free row — the ordinary case for a slot with no `?(…)`
/// bundle at all) defaults to no labels, same as `Row::Empty`. Shared by
/// `Checker::command_scheme`'s V0_1 branch and
/// `Checker::math_command_scheme_v01` so both harvest
/// identically.
fn harvest_slot(row: Row, dom: MonoType) -> CmdArgType {
    let mut opt_labels: Vec<(String, MonoType)> = Vec::new();
    let mut cur = resolve_row(&row).into_owned();
    loop {
        match cur {
            Row::Empty => break,
            Row::Var(_) => break,
            Row::Cons(label, lty, rest) => {
                opt_labels.push((label, *lty));
                cur = resolve_row(&rest).into_owned();
            }
        }
    }
    opt_labels.sort_by(|a, b| a.0.cmp(&b.0));
    labeled(opt_labels, dom)
}

/// A best-effort span for an `Ast` node: only `Var`/`Overwrite`/
/// `AccessField` carry one directly (see `ast.rs`'s module doc comment);
/// everything else falls back to `None`; the resulting `TypeError` then just
/// prints without a location prefix.
pub(crate) fn ast_span<'s>(ast: &Ast<'s>) -> Option<Span> {
    match ast {
        Ast::Var(_, span) => Some(*span),
        Ast::Overwrite(_, span, _) => Some(*span),
        Ast::AccessField(_, _, span) => Some(*span),
        Ast::VersionScope(_, inner) => ast_span(inner),
        Ast::ModuleScope(_, inner) => ast_span(inner),
        _ => None,
    }
}

/// Type-check a whole elaborated [`Program`], additionally returning every
/// non-fatal [`MatchWarning`] the exhaustiveness/redundancy pass collected
/// — v0.0.6's `exhchecker.ml` warns
/// on a non-exhaustive or redundant `match` rather than rejecting the
/// program, so these never turn a would-have-passed program into a
/// `TypeError`.
pub fn typecheck_verbose<'s>(program: &Program<'s>) -> Result<Vec<MatchWarning>, TypeError> {
    typecheck_verbose_with_version(program, RustyfiVersion::V0_0)
}

/// Same as [`typecheck_verbose`], for a given target `version` — threads
/// through to `Checker::new_with_version`/`base_type_env_with_version` so a
/// `V0_1` program's `page-break` resolves against the `length * length`
/// tuple type (`prim_types::t_page_or_geometry`) and never sees `page`'s
/// `VariantDecl` (gated out of `builtin_variants_with_version(V0_1)`) — a
/// `V0_1` program that writes `A4Paper` gets the SAME "unbound constructor"
/// error upstream's own 0.1 compiler would give it, which is the faithful
/// behavior (the ADT is genuinely gone, not merely discouraged).
pub fn typecheck_verbose_with_version<'s>(
    program: &Program<'s>,
    version: RustyfiVersion,
) -> Result<Vec<MatchWarning>, TypeError> {
    let mut checker = Checker::new_with_version(program, version)?;
    let env = base_type_env_with_version(checker.store, version);
    checker.infer(&env, &program.body)?;
    Ok(checker.warnings)
}

/// Type-check a whole elaborated [`Program`]. Validation only: on success the
/// caller proceeds to evaluate `program.body`; the evaluator is untouched by
/// this phase. A thin wrapper over [`typecheck_verbose`] that discards its
/// warnings.
pub fn typecheck<'s>(program: &Program<'s>) -> Result<(), TypeError> {
    typecheck_with_version(program, RustyfiVersion::V0_0)
}

/// Same as [`typecheck`], for a given target `version`. See
/// `typecheck_verbose_with_version`'s doc comment.
pub fn typecheck_with_version<'s>(
    program: &Program<'s>,
    version: RustyfiVersion,
) -> Result<(), TypeError> {
    typecheck_verbose_with_version(program, version).map(|_warnings| ())
}

// ============================================================================
// Per-binding ≡ whole-program equivalence, and session-incrementality.
// A `#[cfg(test)]` unit module since `BindingView`/`Checker`/`infer_binding`
// etc. are `pub(crate)` — an integration test can't reach them.
// ============================================================================
#[cfg(test)]
mod l3_per_binding_tests {
    use super::*;
    use crate::{elaborate, primitives};

    fn elaborate_src<'s>(store: &'s SymbolStore, src: &str) -> Program<'s> {
        let file = rustyfi_syntax::parse_file(src).expect("parse failed");
        let env = primitives::base_env();
        let scope = elaborate::Scope::new(store, env.names());
        elaborate::elaborate_program(&file, &scope).expect("elaborate failed")
    }

    /// Manually drive the checker per binding: walk `program.body`'s Let
    /// chain constructing `BindingView`s by hand, `infer_binding` +
    /// `with_all` at each step, `infer_expr` on the non-Let tail. This is
    /// exactly what `infer`'s own recursion does internally — driven here
    /// from outside the engine through the `pub(crate)` per-binding API, the
    /// same way `v1/module_check.rs` does.
    fn drive_manually<'s>(
        program: &Program<'s>,
        version: RustyfiVersion,
    ) -> Result<Vec<MatchWarning>, TypeError> {
        let mut checker = Checker::new_with_version(program, version)?;
        let mut env = base_type_env_with_version(program.store, version);
        let mut ast: &Ast<'s> = &program.body;
        loop {
            ast = match ast {
                Ast::LetIn(name, value, body) => {
                    let schemes =
                        checker.infer_binding(&env, BindingView::Let { name: *name, value })?;
                    env = env.with_all(schemes, checker.binding_stage(value));
                    body
                }
                Ast::LetMathIn(name, value, body) => {
                    let schemes =
                        checker.infer_binding(&env, BindingView::LetMath { name: *name, value })?;
                    env = env.with_all(schemes, checker.binding_stage(value));
                    body
                }
                Ast::LetRecIn(bindings, body) => {
                    let schemes = checker.infer_binding(&env, BindingView::LetRec(bindings))?;
                    env = env.with_all(schemes, checker.binding_stage_rec(bindings));
                    body
                }
                Ast::LetMutableIn(name, init, body) => {
                    let schemes = checker
                        .infer_binding(&env, BindingView::LetMutable { name: *name, init })?;
                    env = env.with_all(schemes, checker.binding_stage(init));
                    body
                }
                other => {
                    checker.infer_expr(&env, other)?;
                    break;
                }
            };
        }
        Ok(checker.take_warnings())
    }

    /// Elaborate `src` once, then compare `typecheck_verbose_with_version`
    /// against the manual per-binding drive: identical verdict, identical
    /// `TypeError` `Display` on error, identical `MatchWarning` list (incl.
    /// order — `MatchWarning` derives `PartialEq`) on success.
    fn assert_equivalent(src: &str) {
        let version = RustyfiVersion::V0_0;
        let store = SymbolStore::new();
        let program = elaborate_src(&store, src);
        let whole = typecheck_verbose_with_version(&program, version);
        let manual = drive_manually(&program, version);
        match (whole, manual) {
            (Ok(w1), Ok(w2)) => {
                assert_eq!(w1, w2, "warnings differ for {src:?}");
            }
            (Err(e1), Err(e2)) => {
                assert_eq!(
                    format!("{e1}"),
                    format!("{e2}"),
                    "error strings differ for {src:?}"
                );
            }
            (Ok(w), Err(e)) => panic!(
                "{src:?}: whole-program accepted (warnings={w:?}), manual drive rejected: {e}"
            ),
            (Err(e), Ok(w)) => panic!(
                "{src:?}: whole-program rejected ({e}), manual drive accepted (warnings={w:?})"
            ),
        }
    }

    #[test]
    fn per_binding_drive_matches_whole_program_across_binding_kinds() {
        let cases: &[&str] = &[
            // ---- plain `let` ----
            "let x = 1 in x + 1",
            "let x = 1 in x + true", // failing: type mismatch
            // ---- polymorphic `let` ----
            "let id = fun x -> x in (id 1, id true)",
            // ---- `let-inline` command binding (+ its application) ----
            "let-inline ctx \\emph it = read-inline ctx it
             in
             { \\emph{ ok } }",
            "let-inline ctx \\bad = ctx + 1
             in
             ()", // failing: not context-headed
            // ---- `let-block` command binding (+ its application) ----
            "let-block ctx +p it = line-break true true ctx (read-inline ctx it)
             in
             '< +p{ ok } >",
            "let-block ctx +duo a b = read-block ctx a
             in
             '< +duo{x} >", // failing: wrong arity
            // ---- `let-math` command binding ----
            "let-math \\g m = ${#m#m} in 0",
            "let-math \\f = 3 in 0", // failing: value isn't `math`
            // ---- `let-rec` group ----
            "let-rec is-even n = if n == 0 then true else is-odd (n - 1)
             and is-odd n = if n == 0 then false else is-even (n - 1)
             in
             is-even 4",
            "let-rec f n = if n == 0 then 0 else (f true)
             in
             f 1", // failing: recursive use at a mismatched type
            // ---- `let-mutable` (value restriction) ----
            "let-mutable x <- 0
             in
             (x <- 5)",
            "let-mutable r <- []
             in
             ((r <- (1 :: !r)) before (r <- (true :: !r)))", // failing: value restriction
            // ---- a `match` to also exercise warning accumulation ----
            "match Some 1 with
             | Some n -> n
             | None -> 0",
        ];
        for src in cases {
            assert_equivalent(src);
        }
    }

    /// Session-incrementality. `declare_variant` after
    /// `infer_binding` affects only *later* checking against the session —
    /// a ctor referenced before its `declare_variant` fails with the same
    /// "unknown constructor" error the whole-program path gives for a
    /// genuinely-undeclared one; after `declare_variant`, it typechecks.
    #[test]
    fn session_incrementality_declare_variant_affects_only_later_bindings() {
        let store = SymbolStore::new();
        let program = elaborate_src(&store, "type t = | A of int in 0");
        assert_eq!(program.type_decls.len(), 1);
        let decl = &program.type_decls[0];

        let mut checker = Checker::empty(&store);
        checker.install_builtin_variants(RustyfiVersion::V0_0);
        let env = base_type_env_with_version(&store, RustyfiVersion::V0_0);

        // Before `declare_variant`, `A` is unknown — same message shape a
        // genuinely-undeclared constructor gets.
        let a_payload = Ast::Ctor("A".to_string(), Some(Box::new(Ast::Int(1))));
        let before = checker
            .infer_binding(
                &env,
                BindingView::Let {
                    name: store.intern("before"),
                    value: &a_payload,
                },
            )
            .expect_err("`A` should be unknown before declare_variant");
        assert_eq!(format!("{before}"), "unknown constructor 'A'");

        let nosuch_payload = Ast::Ctor("NoSuchCtor".to_string(), None);
        let genuinely_unknown = checker
            .infer_binding(
                &env,
                BindingView::Let {
                    name: store.intern("n"),
                    value: &nosuch_payload,
                },
            )
            .expect_err("a genuinely undeclared ctor should also fail");
        assert_eq!(
            format!("{genuinely_unknown}"),
            "unknown constructor 'NoSuchCtor'"
        );

        // After `declare_variant`, `A` becomes visible and typechecks —
        // this later binding sees it; the earlier `before` call above is
        // unaffected (it already returned its error).
        checker
            .declare_variant(decl)
            .expect("declare_variant should succeed");
        let after = checker.infer_binding(
            &env,
            BindingView::Let {
                name: store.intern("after"),
                value: &a_payload,
            },
        );
        assert!(
            after.is_ok(),
            "A(1) should typecheck after declare_variant: {after:?}"
        );
    }
}

// ============================================================================
// The shadowing-fix follow-up to the version-scope env swap above:
// `version_scoped_type_env`'s `Ast::VersionScope` overwrite must not
// re-stomp a `PRIMITIVE_NAMES` entry the user already shadowed BEFORE the
// `VersionScope` is reached. A `#[cfg(test)]` unit module (mirroring
// `l3_per_binding_tests` above) since `Checker`/`TypeEnv`/`Ast` are all
// `pub(crate)` or crate-private-shaped enough that a hand-built synthetic
// `Ast` fixture (no parser/elaborator round-trip needed to pin this one
// shape) is the most direct way to exercise the exact env-swap path.
// ============================================================================
#[cfg(test)]
mod x2b_shadow_tests {
    use super::*;

    /// `let page-break = 42 in <VersionScope V0_0> page-break` — infer
    /// the whole tree under a `V0_1`-ambient `Checker`. `page-break` is a
    /// `PRIMITIVE_NAMES` member (a version-forked one, no less: its `V0_0`
    /// scheme takes the `page` ADT, its `V0_1` scheme a `length * length`
    /// tuple — see `page_prims.rs`). An overwrite loop that replaced every
    /// `PRIMITIVE_NAMES` entry unconditionally on entering the
    /// `VersionScope` would re-stomp the user's `page-break = 42` with
    /// `V0_0`'s builtin `page-break` (a curried function type), inferring the
    /// inner `Var` as a function, not `int`. Instead
    /// `version_scoped_type_env` sees `page-break` recorded in
    /// `env.shadowed` (set by the `LetIn` arm's `env.with_all`, which goes
    /// through `TypeEnv::with`) and skips the overwrite for that one name, so
    /// the `Var` resolves through the untouched user binding and the whole
    /// expression types as `int`.
    #[test]
    fn version_scope_does_not_clobber_a_user_shadowed_primitive() {
        assert!(
            PRIMITIVE_NAMES.contains(&"page-break"),
            "fixture assumption: page-break must be a PRIMITIVE_NAMES member"
        );

        let span = Span::default();
        let store = SymbolStore::new();
        let page_break = store.intern("page-break");
        let ast = Ast::LetIn(
            page_break,
            Box::new(Ast::Int(42)),
            Box::new(Ast::VersionScope(
                RustyfiVersion::V0_0,
                Box::new(Ast::Var(page_break, span)),
            )),
        );
        let program = Program {
            type_decls: Vec::new(),
            synonym_decls: Vec::new(),
            body: ast,
            store: &store,
        };

        let mut checker = Checker::new_with_version(&program, RustyfiVersion::V0_1)
            .expect("checker construction over an empty-decls program should succeed");
        let env = base_type_env_with_version(&store, RustyfiVersion::V0_1);
        let ty = checker.infer(&env, &program.body).unwrap_or_else(|e| {
            panic!(
                "inferring the version-scoped `Var` over the user's shadowed \
                 `page-break = 42` binding should type-check as `int`, not error: {e}"
            )
        });
        assert!(
            matches!(ty, MonoType::Base(BaseType::Int)),
            "the VersionScope env swap must respect the user's `page-break` shadow \
             (expected MonoType::Base(BaseType::Int), got {ty:?} instead) — a \
             MonoType::Func here would mean version_scoped_type_env re-stomped the \
             user binding with V0_0's builtin page-break scheme"
        );
    }

    /// Companion positive control (same shape as the test above, MINUS the
    /// enclosing user shadow): a version-forked primitive referenced inside
    /// a `VersionScope` still resolves to `version`'s own (function-typed)
    /// scheme. Contrasts directly with
    /// `version_scope_does_not_clobber_a_user_shadowed_primitive`'s `int`
    /// result: same `page-break` name, same `VersionScope(V0_0, _)`, the only
    /// difference being the absence of a prior `let page-break = …` shadow.
    #[test]
    fn version_scope_still_resolves_unshadowed_forked_primitive() {
        let span = Span::default();
        let store = SymbolStore::new();
        let ast = Ast::VersionScope(
            RustyfiVersion::V0_0,
            Box::new(Ast::Var(store.intern("page-break"), span)),
        );
        let program = Program {
            type_decls: Vec::new(),
            synonym_decls: Vec::new(),
            body: ast,
            store: &store,
        };
        let mut checker = Checker::new_with_version(&program, RustyfiVersion::V0_1)
            .expect("checker construction over an empty-decls program should succeed");
        let env = base_type_env_with_version(&store, RustyfiVersion::V0_1);
        let ty = checker.infer(&env, &program.body).unwrap_or_else(|e| {
            panic!(
                "an unshadowed page-break reference inside a VersionScope should still \
                 type-check (X2a's original capability, unaffected by X2b): {e}"
            )
        });
        assert!(
            matches!(ty, MonoType::Func(..)),
            "page-break (unshadowed) inside a VersionScope should still resolve to its \
             builtin (function-typed) scheme, got {ty:?} instead — the X2b shadow guard \
             must not have blocked this NON-shadowed overwrite"
        );
    }
}

// ============================================================================
// Acceptance test against the real `stdja.satyh` `sig … end` block (
// command values are covered by `crates/rustyfi-lang/tests/typecheck.rs`'s
// end-to-end fixtures; this module covers the `SigItem`/`constraint` lowering
// directly, since `lower_sig_item` is a crate-private entry point no
// sig-enforcement pass calls yet).
// ============================================================================
#[cfg(test)]
mod sig_constraint_tests {
    use super::*;
    use rustyfi_syntax::cst::{SigAnnot, TopBinding};

    fn parse_module_sig(src: &str) -> SigAnnot {
        let file = rustyfi_syntax::parse_file(src).expect("parse failed");
        for b in &file.prelude {
            if let TopBinding::Module { sig: Some(sig), .. } = b {
                return sig.clone();
            }
        }
        panic!("no `module .. : sig .. end` found in {src:?}");
    }

    #[test]
    fn constraint_suffix_lowers_to_a_kind_record_bound_on_its_tyvar() {
        let sig = parse_module_sig(
            "module M : sig\n\
             val document : 'a -> config ?-> block-text -> document\n\
             constraint 'a :: (| title : inline-text; author : inline-text |)\n\
             end = struct\n\
             let document x c bt = bt\n\
             end",
        );
        let mut ctx = TypeContext::new();
        let mut saw_record_kind = false;
        for item in &sig.items {
            let (name, ty) =
                lower_sig_item(item, &mut ctx, RustyfiVersion::V0_0).expect("a value item");
            assert_eq!(name, "document");
            // Walk the lowered `Func` chain: `'a`'s fresh variable is the
            // very first domain (`Func(Var('a), Func(option(config),
            // Func(block-text, document)))` — see `lower_type_expr`'s doc
            // comment for the `?->` shape).
            if let MonoType::Func(_row, dom, _) = &ty {
                if let MonoType::Var(v) = &**dom {
                    if let Kind::Record(labels) = v.kind() {
                        saw_record_kind = true;
                        let expected: BTreeSet<String> =
                            ["title", "author"].iter().map(|s| s.to_string()).collect();
                        assert_eq!(labels, expected);
                    }
                }
            }
        }
        assert!(
            saw_record_kind,
            "expected 'a's fresh variable to carry a Kind::Record bound"
        );
    }

    #[test]
    fn kind_record_bound_accepts_a_row_with_every_required_label() {
        // Direct demonstration that the constraint's lowered `Kind::Record`
        // bound rides on *existing* `unify`/`bind_var` machinery for free —
        // no sig-enforcement pass exists yet to drive this against a real
        // `struct` implementation, but
        // the positive-presence check itself already works once something
        // does.
        let mut ctx = TypeContext::new();
        let labels: BTreeSet<String> = ["title", "author"].iter().map(|s| s.to_string()).collect();
        let v = ctx.fresh_var_with_kind(Kind::Record(labels));
        let constrained = MonoType::Var(v);
        let full = MonoType::Record(Row::Cons(
            "title".to_string(),
            Box::new(t_inline_text()),
            Box::new(Row::Cons(
                "author".to_string(),
                Box::new(t_inline_text()),
                Box::new(Row::Empty),
            )),
        ));
        unify(&constrained, &full).expect("row has both required labels");
    }

    #[test]
    fn kind_record_bound_rejects_a_row_missing_a_required_label() {
        let mut ctx = TypeContext::new();
        let labels: BTreeSet<String> = ["title", "author"].iter().map(|s| s.to_string()).collect();
        let v = ctx.fresh_var_with_kind(Kind::Record(labels));
        let constrained = MonoType::Var(v);
        let missing_author = MonoType::Record(Row::Cons(
            "title".to_string(),
            Box::new(t_inline_text()),
            Box::new(Row::Empty),
        ));
        let err = unify(&constrained, &missing_author)
            .expect_err("row is missing the required 'author' label");
        assert!(
            format!("{err:?}").contains("author"),
            "error should name the missing label: {err:?}"
        );
    }

    #[test]
    fn real_stdja_sig_block_lowers_every_item_to_a_monotype() {
        // Mirrors the whole `sig … end` block of the real upstream
        // `stdja.satyh:24-51` (v0.0.6 checkout) — command values, command
        // types, `?->`, and the `constraint` suffix all together. Every item
        // parses and lowers without error (an empty `struct end` body is
        // enough — sig enforcement against a real implementation is not
        // this test's job).
        let sig = parse_module_sig(
            "module StdJa : sig\n\
             val default-config : config\n\
             val document : 'a -> config ?-> block-text -> document\n\
             constraint 'a :: (|\n\
             title : inline-text;\n\
             author : inline-text;\n\
             show-toc : bool;\n\
             show-title : bool;\n\
             |)\n\
             val font-latin-roman : string * float * float\n\
             direct \\ref : [string] inline-cmd\n\
             direct \\ref-page : [string] inline-cmd\n\
             direct \\figure : [inline-text; block-text] inline-cmd\n\
             direct +p : [inline-text] block-cmd\n\
             direct +pn : [inline-text] block-cmd\n\
             direct +section : [string?; string?; inline-text; block-text] block-cmd\n\
             direct +subsection : [string?; string?; inline-text; block-text] block-cmd\n\
             direct \\emph : [inline-text] inline-cmd\n\
             end = struct\n\
             end",
        );
        let mut ctx = TypeContext::new();
        let mut names = Vec::new();
        for item in &sig.items {
            let (name, _ty) =
                lower_sig_item(item, &mut ctx, RustyfiVersion::V0_0).expect("a value item");
            names.push(name);
        }
        assert_eq!(
            names,
            vec![
                "default-config",
                "document",
                "font-latin-roman",
                "\\ref",
                "\\ref-page",
                "\\figure",
                "+p",
                "+pn",
                "+section",
                "+subsection",
                "\\emph",
            ]
        );
    }
}

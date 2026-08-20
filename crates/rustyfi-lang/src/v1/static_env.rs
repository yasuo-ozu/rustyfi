//! Sub-slice 2d-1 (`…/tmp/slice2d-sealing.md` §2.1): the semantic-signature
//! data model — the port-shaped analogue of upstream `staticEnv.ml`'s
//! `struct_signature`/`type_environment` (`staticEnv.ml:106-141`), radically
//! first-order: no `abstracted` quantifier wrapper (2d-2's stamps are
//! globally fresh strings, so the `quantifier = kind OpaqueIDMap.t`
//! bookkeeping collapses — see §3.1 of the spec), no macros/stages, functor
//! signatures deferred to 2f.
//!
//! 2d-1 populates and consumes only [`DeclaredVal`]/[`StaticEnv::seals`]/
//! [`StaticEnv::hidden`]/[`StampMint`]; [`TypeOpacity`]/[`DeclaredType`] and
//! the non-`vals` fields of [`ElabSig`] are shape-only placeholders that pin
//! the §3.1 representation decision (stamped nominal names, e.g. `"M.t#3"`)
//! ahead of 2d-2, which is their first writer — per the spec's §6 item 10
//! scope guard, do not widen these further until that sub-slice lands.

use crate::types::{MonoType, PolyType, Stage};
use rustyfi_syntax::cst_v1::ast as ast_v1;
use rustyfi_syntax::span::Span;
use std::collections::HashMap;

/// A member's declared value entry (upstream `value_entry`,
/// staticEnv.ml:82-86). `scheme` is what sealing COMMITS;
/// `rigid` is the skolemized body the subsumption check ([`crate::v1::
/// sig_subtype::val_subsumes`]) unifies against (§3.4 — both are lowered
/// from the same source `TypeExpr`, so they are alpha-equivalent by
/// construction).
#[derive(Clone, Debug)]
pub(crate) struct DeclaredVal {
    /// The bare member name (`"x"`, `"\emph"`-style, or a `( + )`-style
    /// operator `BindName` string) — NOT qualified; the qualified key
    /// (`"M.x"`) is [`StaticEnv::seals`]'s map key, not stored redundantly
    /// here.
    pub(crate) name: String,
    /// Quantified over the decl's `quant` tyvars — the scheme sealing
    /// COMMITS at the alias binding (§4.2 step 4).
    pub(crate) scheme: PolyType,
    /// The same body with `quant` tyvars lowered to rigid nominal stamps
    /// (`"'a#7"`, §3.4) instead — the fixed pattern the depth check unifies
    /// the module's inferred (instantiated) scheme against.
    pub(crate) rigid: MonoType,
    /// The decl's `val` keyword span (diagnostics).
    pub(crate) span: Span,
    /// The stage the SIGNATURE declares this member at — `val ~x` is
    /// [`Stage::Stage0`], `val persistent ~x` is [`Stage::Persistent0`], a
    /// plain `val x` is [`Stage::Stage1`] (upstream `value_entry.val_stage`,
    /// `dev-0-1-0 staticEnv.ml`; the decoder is `moduleTypechecker.ml:505-523`).
    /// Checked against the stage of the binding that IMPLEMENTS the member by
    /// `module_check`'s phase-D spine walk, and — when a PARENT signature
    /// re-declares this member of a nested child — against that parent's
    /// declared stage by `module_check::process_link_member`. Both are the
    /// port's stand-in for upstream's `signatureSubtyping.ml:279-298` stage
    /// arm. Command decls (`val \cmd`, `val +cmd`) have no stage slot in the
    /// grammar at all (`parser_v1.mly:604-607`), so they record `Stage1`.
    pub(crate) stage: Stage,
    /// This decl's own `"#N"` stamp suffix (shared by every rigid tyvar
    /// `rigid` mentions — one [`StampMint`] draw per `Decl::Val`, not per
    /// tyvar; see `module_check.rs`'s seal-table walk). Not part of the
    /// spec's §2.1 struct sketch verbatim, but required by [`crate::v1::
    /// sig_subtype::val_subsumes`]'s escaped-skolem check (§4.2 step 5) to
    /// know which stamps belong to THIS check — keeping it alongside
    /// `rigid` (rather than a parallel map) avoids any risk of the two
    /// drifting apart.
    pub(crate) stamp_marker: String,
}

/// One resolved type entry (`types → (arity, opacity)`, upstream `lookup_
/// type_entry`). First constructed for real by Sub-slice 2d-2's
/// `module_check::prescan_seal_types` (opaque decls) and
/// `module_check::check_transparent_type` (transparent decls); the shape
/// was fixed a sub-slice early because the abstract-type representation
/// decision (spec §3.1) pinned it, and [`crate::v1::sig_subtype`]'s unit
/// tests exercised it directly ahead of the sub-slice that first constructs
/// one for real.
#[derive(Clone, Debug)]
pub(crate) enum TypeOpacity {
    /// `type t = τ` in a sig: must equal the impl's type (2d-2). The
    /// resolved body is stored (rather than just a bare marker) for 2d-3's
    /// `ElabSig.types` writer to consume — 2d-2 itself never re-reads it
    /// (transparent resolution flows through the ordinary synonym table
    /// instead, §2.1's θ-for-free argument), hence the `dead_code` allow.
    #[allow(dead_code)]
    Transparent(MonoType),
    /// `type t :: kind` in a sig: the stamped abstract nominal key that
    /// escapes the seal, e.g. `"M.t#3"` (§3.1).
    Abstract { stamped: String },
}

#[derive(Clone, Debug)]
pub(crate) struct DeclaredType {
    /// From `KindV1`: `o -> o` ⇒ 1 (kind_base count − 1).
    pub(crate) arity: usize,
    pub(crate) opacity: TypeOpacity,
    /// The declaring `type`/`type ::` keyword's span — kept for parity with
    /// [`DeclaredVal::span`] and future diagnostics; not yet re-read by
    /// 2d-2 itself (every 2d-2 error already carries its own precise span
    /// at the reference site).
    #[allow(dead_code)]
    pub(crate) span: Span,
}

/// One constructor deregistered by a signature seal (Sub-slice 2d-2, spec
/// §2.2): `T` → the sealing module and the concrete type name it belonged
/// to (the same string `typecheck::Checker::hide_ctors`'s identity guard
/// compares against `VariantDecl.name`). Populated by `module_check`'s
/// ctor-hide bookkeeping, consulted by `rewrite_hidden_error` to turn a raw
/// "unknown constructor" error into a precise sealing diagnostic.
#[derive(Clone, Debug)]
pub(crate) struct HiddenCtor {
    pub(crate) module: String,
    pub(crate) type_name: String,
}

/// One elaborated signature body (upstream `struct_signature`,
/// staticEnv.ml:106-121). A `Vec`, not a map: decl order drives
/// deterministic error order (upstream folds the sig in order too). 2d-1
/// never constructs one of these (`module_check.rs`'s seal-table walk
/// writes `vals` straight into [`StaticEnv::seals`]/`hidden` without an
/// intermediate `ElabSig`); the type exists now so the representation is
/// pinned before 2d-2/2d-3 need it.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // first constructed (as a whole) by 2d-2/2d-3
pub(crate) struct ElabSig {
    pub(crate) vals: Vec<DeclaredVal>,
    /// 2d-2.
    pub(crate) types: Vec<(String, DeclaredType)>,
    /// 2d-3 (`Decl::Module`).
    pub(crate) modules: Vec<(String, ElabSig)>,
    /// 2d-3 (`Decl::Signature`).
    pub(crate) signatures: Vec<(String, ElabSig)>,
}

/// The whole-program static environment the module checker threads
/// (upstream `type_environment`, staticEnv.ml:134-141, first-order). 2d-1
/// populates `seals`/`hidden`; 2d-2 adds every `types`-shaped map below (the
/// other maps arrive with their owning sub-slice — NOT stubbed with dead
/// fields — each field arrives with its writer, per the spec's §6 item 10
/// scope guard):
///
/// - 2d-3: `pub(crate) modules: HashMap<String, ElabSig>` (module aliases/
///   decls); `pub(crate) signatures: HashMap<String, ElabSig>` (`signature S
///   = …`).
#[derive(Default)]
pub(crate) struct StaticEnv {
    /// Sealed-module surface: qualified member name (`"M.x"`, `"M.N.y"`) →
    /// its declared entry. Consulted by the driver at each alias commit
    /// (`module_check.rs`'s `Ast::LetIn` interception, §4.2 steps 3-4).
    pub(crate) seals: HashMap<String, DeclaredVal>,
    /// Members hidden by a seal: qualified name → owning module path
    /// (`"M.N"`). Consulted by the unbound-use rewriter (§4.2 step 6).
    pub(crate) hidden: HashMap<String, String>,
    /// Every sealed module's declared type surface: qualified type name
    /// (`"M.t"`) → arity/opacity (Sub-slice 2d-2, spec §2.1/§3 phase A/C1).
    /// The resolution source for `LONG_LOWER` references (`v1/module_
    /// check.rs`'s `rename_type_name`) and the opaque-stamping/transparent-
    /// equality machinery.
    pub(crate) types: HashMap<String, DeclaredType>,
    /// Struct types NOT named by any sig `type` decl: qualified type name
    /// (`"M.u"`) → owning module path (`"M"`). An external `M.u` reference
    /// (or a self-reference from M's own sig without a declaring `type`
    /// decl) is precisely this map's mirror of `hidden`, for types instead
    /// of values (Sub-slice 2d-2, spec §3 phase A step 4).
    pub(crate) hidden_types: HashMap<String, String>,
    /// Constructors deregistered by a seal: bare ctor name → the owning
    /// module/type it belonged to (Sub-slice 2d-2, spec §2.2). Consulted by
    /// `rewrite_hidden_error`'s ctor-format matchers.
    pub(crate) hidden_ctors: HashMap<String, HiddenCtor>,
    /// Deferred ctor-deregistration triggers: the qualified alias name of a
    /// sealed module's LAST value member (elaboration emits every member's
    /// alias contiguously and in source order, spec §2.2) → the list of
    /// `(ctor, concrete type name)` pairs to deregister once that alias
    /// commits. Modules with zero value members hide immediately instead
    /// (`module_check::check_program`'s `immediate_hides`, never routed
    /// through this map).
    pub(crate) ctor_hide_triggers: HashMap<String, Vec<(String, String)>>,
    /// Sub-slice 2d-3b (`…/tmp/slice2d3b-2f2-sigmembers.md` §3.4): a
    /// PARENT seal's deferred member revocation — a nested `Decl::Module`
    /// member whose sub-signature omits values the child itself exports.
    /// Trigger alias (the qualified name of the PARENT's subtree-last value
    /// member, `last_value_alias_in_subtree`) → (the revoking parent's
    /// module name, for diagnostics; the list of qualified member names to
    /// remove from the `TypeEnv`/mark [`StaticEnv::hidden`]). Fires in phase
    /// D, mirroring [`StaticEnv::ctor_hide_triggers`]'s deferred-trigger
    /// shape, via `TypeEnv::without_all` (`typecheck.rs`).
    pub(crate) member_revoke_triggers: HashMap<String, (String, Vec<String>)>,
    /// Sub-slice 2d-3b §3.5: a struct's own `signature S = ..` bind hidden
    /// by a parent seal that never declares `signature S`: qualified name
    /// (`"M.S"`) → owning module path. Consulted by `resolve_sig_decls`'s
    /// miss diagnostics for the precise "not exported by its signature"
    /// wording (mirrors [`StaticEnv::hidden`]/[`StaticEnv::hidden_types`]).
    pub(crate) hidden_sigs: HashMap<String, String>,
    /// Sub-slice 2f-2b (spec §5.1): functor members constrained by an
    /// enclosing seal — the functor's own qualified path (`"Map.Make"`) →
    /// its DECLARED interface. Applications of this functor are checked
    /// against `dom` and their results sealed with `cod[param := arg]`
    /// (`module_check.rs`'s phase-A0 instantiation store). Owned (cloned
    /// out of the `deps`-borrowed `cst_v1` tree at registration time) rather
    /// than lifetime-tied, so `StaticEnv` itself stays a single, non-generic
    /// type — the simpler of the spec's two implementation choices (§5.1
    /// step 3's parenthetical).
    pub(crate) sealed_functors: HashMap<String, SealedFunctorSig>,
    /// Sub-slice 2f-2b §5.1 step 5: a struct functor an umbrella seal omits
    /// — full functor path → owning module path. An application of a
    /// hidden functor is a precise "not exported by its signature" error
    /// (`check_functor_applications`), mirroring [`StaticEnv::hidden`].
    pub(crate) hidden_functors: HashMap<String, String>,
}

/// One functor member's declared interface, registered by a `Decl::Module {
/// Make : (Key : S_dom) -> S_cod }` sig member (Sub-slice 2f-2b spec §5.1).
#[derive(Clone, Debug)]
pub(crate) struct SealedFunctorSig {
    /// The declared parameter's bare name (`"Key"`).
    pub(crate) param: String,
    /// The declared domain signature `S_dom` — identity-checked against the
    /// implementation's own parameter signature at REGISTRATION time
    /// (`module_check.rs::handle_functor_sig_member`); not re-read
    /// afterward (every per-application domain check reuses the impl's own
    /// `FunctorDef::param_sig`, guaranteed identical by that same check).
    #[allow(dead_code)]
    pub(crate) dom: ast_v1::SigExpr,
    /// The declared codomain signature `S_cod` — substituted per application
    /// (`subst_sig_expr(cod, param, arg_path)`, `v1/functor.rs`) before
    /// being used to seal that application's result (§5.2).
    pub(crate) cod: ast_v1::SigExpr,
    /// Where names inside `dom`/`cod` resolve outward from (the sig
    /// member's own enclosing module path, e.g. `["Map"]`).
    pub(crate) def_site: Vec<String>,
    /// The `Decl::Module`'s own keyword span (diagnostics).
    pub(crate) span: Span,
}

/// Globally unique `#n` suffixes. `#` is not a lexable identifier char in
/// either SATySFi version, so a stamped name can never collide with (or be
/// spoofed by) a user-written nominal type name — the invariant both the
/// skolem trick (§3.4) and abstract stamping (§3.1, 2d-2) rest on.
#[derive(Default)]
pub(crate) struct StampMint(u64);

impl StampMint {
    pub(crate) fn next(&mut self) -> u64 {
        let n = self.0;
        self.0 += 1;
        n
    }
}

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

use crate::types::{MonoType, PolyType};
use satysfi_syntax::span::Span;
use std::collections::HashMap;

/// A member's declared value entry (upstream `value_entry`,
/// staticEnv.ml:82-86, minus stage). `scheme` is what sealing COMMITS;
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

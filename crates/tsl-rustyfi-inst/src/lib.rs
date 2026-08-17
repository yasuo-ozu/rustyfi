//! rustyfi instantiation of `catasys-infer` and `catasys-exhaust`.
//!
//! ADOPTION: rustyfi's own type/unification engine is absorbed as
//! `catasys-infer`'s seed (a one-time flow rustyfi -> library, see
//! `catasys-infer`'s module doc), and this crate is where rustyfi flows back
//! to depend on the now-generic library engine. Command types (`InlineCmd`
//! / `BlockCmd` / `MathCmd`) carry their argument optionality flags
//! directly in the `RustyfiFormer` tag (`Vec<bool>`, one entry per
//! parameter, `true` = optional) rather than in a separate side table.
#![forbid(unsafe_code)]

/// A rustyfi type-constructor former.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustyfiFormer {
    /// A named base type.
    Base(String),
    /// The function type constructor.
    Func,
    /// An n-ary product (tuple) type constructor.
    Product(usize),
    /// The list type constructor.
    List,
    /// The reference type constructor.
    Ref,
    /// A variant constructor, named, with its argument arity.
    Variant(String, usize),
    /// An inline command type; one optionality flag per parameter.
    InlineCmd(Vec<bool>),
    /// A block command type; one optionality flag per parameter.
    BlockCmd(Vec<bool>),
    /// A math command type; one optionality flag per parameter.
    MathCmd(Vec<bool>),
}

/// The kind annotation carried by a rustyfi unification variable. Left
/// unit-shaped for now: rustyfi's kind system (if any beyond simple types)
/// is not yet modeled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RustyfiVarKind;

/// The rustyfi instantiation of `catasys_infer::TypeLang`.
pub struct RustyfiLang;

impl catasys_infer::TypeLang for RustyfiLang {
    type Former = RustyfiFormer;
    type VarKind = RustyfiVarKind;

    fn unify_former(a: &Self::Former, b: &Self::Former) -> Result<(), catasys_infer::FormerClash> {
        use RustyfiFormer::*;
        match (a, b) {
            (Base(x), Base(y)) if x == y => Ok(()),
            (Func, Func) => Ok(()),
            (Product(x), Product(y)) if x == y => Ok(()),
            (List, List) => Ok(()),
            (Ref, Ref) => Ok(()),
            (Variant(x, ax), Variant(y, ay)) if x == y && ax == ay => Ok(()),
            (InlineCmd(x), InlineCmd(y)) if x == y => Ok(()),
            (BlockCmd(x), BlockCmd(y)) if x == y => Ok(()),
            (MathCmd(x), MathCmd(y)) if x == y => Ok(()),
            _ => Err(catasys_infer::FormerClash),
        }
    }

    fn meet_kinds(
        a: &Self::VarKind,
        b: &Self::VarKind,
    ) -> Result<Self::VarKind, catasys_infer::KindClash> {
        let _ = (a, b);
        // Only one kind exists today, so the meet is trivial; this is the
        // hook where a real kind lattice would be consulted.
        Ok(RustyfiVarKind)
    }
}

/// The rustyfi instantiation of `catasys_exhaust::PatSig`, used to check
/// exhaustiveness/usefulness of rustyfi's variant pattern matches.
pub struct RustyfiPatSig;

impl catasys_exhaust::PatSig for RustyfiPatSig {
    /// A variant constructor name (or a literal tag).
    type Ctor = String;
    /// A type head name, used to look up its complete constructor set.
    type TyTag = String;

    fn arity(&self, c: &Self::Ctor) -> usize {
        let _ = c;
        todo!("look up variant arity from the rustyfi type environment")
    }

    fn complete(&self, head: &Self::TyTag) -> Option<Vec<Self::Ctor>> {
        let _ = head;
        todo!("look up the complete variant set for a rustyfi sum type")
    }
}

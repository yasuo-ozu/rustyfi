//! Interned identifiers: [`SymbolStore`] (an append-only unique-string
//! registry) and [`Symbol`] (a `Copy`, `u32`-sized handle into one).
//!
//! Phase 0 of `docs/plans/design-symbol-debruijn-slots.md`. This module is
//! deliberately self-contained: nothing in the pipeline uses it yet. Phase 1
//! brands the *compile side* (`Ast` → elaborate → typecheck) with
//! `Symbol<'s>`; the runtime side never sees one, because names are resolved
//! away at the compile membrane (`compile.rs`).
//!
//! # Why a lifetime brand
//!
//! A `Symbol<'s>` is just an index, so on its own it would be meaninglessly
//! interchangeable between stores and would outlive the store it names. The
//! `PhantomData<&'s SymbolStore>` ties every symbol to the borrow of the store
//! that minted it, which is what lets the AST hold bare `u32`s while the
//! compiler still guarantees a live store is available wherever a symbol is
//! read back as text. The brand costs nothing at run time — and it is
//! *deliberately* confined to the front half of the pipeline: letting it reach
//! `Value` would cascade a lifetime through all 172 `prim_*` functions for
//! zero speed (see the design doc §1).
//!
//! # Interning is not enough — `resolve` must be cheap
//!
//! The port models namespacing with flat mangled string keys (`"M.x"`,
//! `"\cmd"`, `"$M.atan2"`, `"%cmd_arg0"`), and several consumers *inspect that
//! text*: the command-sigil test, `Scope::names_with_prefix`, `open_module`'s
//! prefix scan, and — critically for byte-identity — the **lexicographic
//! sorting** of optional-argument label rows and record kinds. A `Symbol`'s
//! index order is *insertion* order, never lexicographic, so all of those must
//! keep working on resolved text. [`SymbolStore::resolve`] therefore returns a
//! borrowed `&str` (no allocation, no refcount bump) rather than an owned or
//! reference-counted string.

use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;

/// An interned identifier: an index into the [`SymbolStore`] that minted it,
/// branded with that store's borrow lifetime.
///
/// `Copy` + `Eq` + `Hash` in one `u32`, so scope stacks, type environments and
/// the compiler's lexical frames compare and hash identifiers by integer
/// instead of by string content.
///
/// Equality is index equality, which is *exactly* string equality for symbols
/// from the same store (the store deduplicates on intern). Comparing symbols
/// from two different stores is meaningless but not unsound — see
/// [`SymbolStore::resolve`].
///
/// [`Ord`] is **index order (insertion order), not lexicographic order.** It
/// exists only so symbols can key a `BTreeMap`/`BTreeSet` for deterministic
/// iteration within one run. Anywhere the *output* depends on ordering (type
/// error text, record kinds, optional-label rows), sort by
/// [`SymbolStore::resolve`] text instead — see this module's header.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Symbol<'s>(u32, PhantomData<&'s SymbolStore>);

impl Symbol<'_> {
    /// The raw index. Useful for dense side-tables keyed by symbol; carries no
    /// meaning without the store that minted it.
    #[inline]
    pub fn index(self) -> u32 {
        self.0
    }
}

/// Prints the index, not the text — a `Symbol` is a bare `u32` plus a
/// zero-sized brand, so it has no way to reach its store from here.
///
/// This is a deliberate choice (design doc §7): golden tests must diff
/// *resolved* strings produced at their format site, never `Debug`-of-AST.
impl std::fmt::Debug for Symbol<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Symbol({})", self.0)
    }
}

/// An append-only, deduplicating registry of identifier strings.
///
/// Interning takes `&self`, not `&mut self`: elaboration and typechecking both
/// *mint new derived names mid-pass* (`qualify_key`'s `"M.x"`, `$`-mangled
/// module keys, `"%patbind…"`, fresh desugar names like `"%cmd_arg0"`,
/// qualified constructor lookup keys), so the store must stay interned-into
/// while the tree it brands is being walked. Hence the interior mutability.
///
/// Entries are never removed or mutated, which is what makes [`resolve`]
/// able to hand out `&str`s that live as long as the store borrow.
///
/// [`resolve`]: SymbolStore::resolve
#[derive(Default)]
pub struct SymbolStore {
    inner: RefCell<StoreInner>,
}

#[derive(Default)]
struct StoreInner {
    /// Index → text. `Box<str>` (not `String`) because the pointed-to bytes
    /// must never move once pushed: growing this `Vec` relocates the *boxes*,
    /// not the string data they own, which is the invariant `resolve`'s
    /// lifetime extension rests on.
    texts: Vec<Box<str>>,
    /// Text → index, for dedup. Keys alias `texts`' contents conceptually but
    /// are stored separately (a second `Box<str>`) to keep this safe code.
    index: HashMap<Box<str>, u32>,
}

impl SymbolStore {
    /// A fresh, empty store.
    pub fn new() -> SymbolStore {
        SymbolStore::default()
    }

    /// Intern `text`, returning its symbol. Interning the same text twice
    /// returns the same symbol, so `Symbol` equality is string equality.
    pub fn intern<'s>(&'s self, text: &str) -> Symbol<'s> {
        let mut inner = self.inner.borrow_mut();
        if let Some(&i) = inner.index.get(text) {
            return Symbol(i, PhantomData);
        }
        let i = u32::try_from(inner.texts.len())
            .expect("SymbolStore overflow: more than u32::MAX distinct identifiers");
        let boxed: Box<str> = text.into();
        inner.index.insert(boxed.clone(), i);
        inner.texts.push(boxed);
        Symbol(i, PhantomData)
    }

    /// The text `sym` was interned from.
    ///
    /// # Panics
    ///
    /// If `sym` was minted by a *different* store that happened to share this
    /// one's borrow lifetime and holds more entries than this one. The brand
    /// makes that hard to write by accident and it is not unsound — the bounds
    /// check below turns it into a panic rather than a bogus read — but a
    /// program should still keep one store per pipeline run.
    pub fn resolve<'s>(&'s self, sym: Symbol<'s>) -> &'s str {
        let inner = self.inner.borrow();
        let s: &str = inner
            .texts
            .get(sym.0 as usize)
            .unwrap_or_else(|| panic!("Symbol({}) does not belong to this SymbolStore", sym.0));
        // SAFETY: `s` points into the heap allocation owned by a `Box<str>`
        // that lives in `inner.texts`. That allocation's address is fixed for
        // as long as the box exists, and:
        //   * entries are only ever *pushed* — never removed, replaced, or
        //     mutated (the only writer is `intern`, which pushes);
        //   * growing `texts` relocates the `Box` pointers, not the `str`
        //     bytes they point at;
        //   * the data outlives the `RefCell` borrow guard and is dropped only
        //     with `self`,
        // so widening the guard-scoped borrow to `&'s self`'s lifetime does
        // not create a dangling or aliasing-mutable reference. This is the
        // standard interner pattern; it is needed because a `RefCell` cannot
        // otherwise lend out data past the guard.
        unsafe { &*(s as *const str) }
    }

    /// How many distinct strings have been interned. (Also the index the next
    /// new symbol will get — useful for sizing dense side-tables.)
    pub fn len(&self) -> usize {
        self.inner.borrow().texts.len()
    }

    /// Whether nothing has been interned yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl std::fmt::Debug for SymbolStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SymbolStore({} symbols)", self.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn intern_dedups_and_resolves() {
        let store = SymbolStore::new();
        let a = store.intern("foo");
        let b = store.intern("bar");
        let a2 = store.intern("foo");
        assert_eq!(a, a2);
        assert_ne!(a, b);
        assert_eq!(store.resolve(a), "foo");
        assert_eq!(store.resolve(b), "bar");
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn indices_are_dense_and_insertion_ordered() {
        let store = SymbolStore::new();
        assert!(store.is_empty());
        let syms: Vec<_> = ["z", "y", "x"].iter().map(|s| store.intern(s)).collect();
        assert_eq!(
            syms.iter().map(|s| s.index()).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        // Ord is insertion order, NOT lexicographic — the whole reason
        // byte-identical sorting has to go through `resolve`.
        let mut sorted = syms.clone();
        sorted.sort();
        assert_eq!(sorted, syms);
        let mut by_text: Vec<&str> = syms.iter().map(|&s| store.resolve(s)).collect();
        by_text.sort_unstable();
        assert_eq!(by_text, vec!["x", "y", "z"]);
    }

    #[test]
    fn resolved_borrows_survive_further_interning() {
        // The load-bearing property: a `&str` handed out by `resolve` must stay
        // valid while the store keeps growing (elaborate/typecheck mint derived
        // names mid-walk, while earlier resolved text is still in use).
        let store = SymbolStore::new();
        let first = store.resolve(store.intern("first"));
        for i in 0..10_000 {
            store.intern(&format!("derived%{i}"));
        }
        assert_eq!(first, "first");
        assert_eq!(store.len(), 10_001);
    }

    #[test]
    fn mangled_keys_round_trip_verbatim() {
        // The port's identifiers are mangled composites; nothing about them is
        // normalized on the way through.
        let store = SymbolStore::new();
        for key in ["M.x", "\\cmd", "+p", "M.\\cmd", "$M.atan2", "%context"] {
            assert_eq!(store.resolve(store.intern(key)), key);
        }
        assert_eq!(store.len(), 6);
    }

    #[test]
    fn symbols_are_hashable_keys() {
        let store = SymbolStore::new();
        let set: HashSet<Symbol<'_>> = ["a", "b", "a", "c"].iter().map(|s| store.intern(s)).collect();
        assert_eq!(set.len(), 3);
        assert_eq!(std::mem::size_of::<Symbol<'_>>(), 4);
    }
}

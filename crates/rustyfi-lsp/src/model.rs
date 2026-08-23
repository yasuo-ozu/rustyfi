//! The shared foundation under hover, go-to-definition and completion: a
//! **cursor → syntax** mapping over one buffer.
//!
//! # What it is
//!
//! A [`Model`] is what a file's parse tree looks like once everything the
//! three interactive features need — and nothing else — has been read out of
//! it:
//!
//! - a [`Def`] per name the file **binds**, carrying the byte range of the
//!   name itself, the byte range over which that name is **visible**, how it
//!   was written (`let-inline`, `val`, a parameter, …) and, when the author
//!   wrote one, the range of its declared type;
//! - a [`Ref`] per name the file **mentions**, carrying its namespace and any
//!   `Module.` qualification;
//! - the ranges over which an `open` (or an `include`) that this file cannot
//!   see into is in effect, because those are exactly the places where a
//!   confident answer is not available;
//! - the `@require:`/`@import:`/`use` headers, which are the file's own edges
//!   to other files.
//!
//! Everything is a plain byte range, so the parse trees are dropped as soon as
//! the walk finishes and nothing downstream has to name a CST type.
//!
//! # Why one mapping and not three
//!
//! Hover asks "what is under the cursor and where did it come from",
//! definition asks "where did it come from", completion asks "what could go
//! here" — these are one question about scope asked three ways. Written per
//! feature they would each need their own idea of what a name is, and the
//! three would disagree; written once, [`Model::hit_at`] and
//! [`Model::resolve`] answer all three and a bug is fixed in one place.
//!
//! # Namespaces, not one big table
//!
//! SATySFi has five identifier namespaces that can hold the same spelling at
//! once, and conflating them is how a language server ends up jumping from
//! `\emph` to a value called `emph`. [`Ns`] keeps them apart. Note in
//! particular that `let-math \frac` binds a *math* command with the same token
//! type an *inline* command uses (`cst.rs` says so explicitly), so the token
//! kind alone cannot tell them apart — only the binding form can, which is
//! what the walk records.
//!
//! # Both generations
//!
//! [`walk006`](crate::walk006) and [`walk01`](crate::walk01) fill the same
//! [`Builder`] from the two grammars. They are separate walks because the
//! grammars genuinely differ (0.1 has modules, signatures, `val` declarations
//! and stage qualifiers where 0.0.6 has `let-inline`/`let-block`/`let-math`
//! and `direct` sig items), but they produce one vocabulary, so everything
//! above this module is version-blind.
//!
//! # Partial buffers are the normal case
//!
//! Hover and completion run while the user is typing, so a file that does not
//! parse is not an edge case. When the whole-file parse fails, [`build_model`]
//! re-parses the file's *fields* — headers, then the top-level binding list,
//! then the body — instead of the file rule. A syan `Vec<T>` stops at the
//! first element that does not parse and succeeds with what it already has, so
//! this recovers every complete top-level binding before the one being typed.
//! The half-typed binding itself contributes nothing, which is the honest
//! outcome: its own parameters are not knowable yet.

use rustyfi_syntax::span::Span;
use rustyfi_syntax::{cst, cst_v1, RustyfiVersion};
use syan::parse::Parse;

use rustyfi_syntax::stream::AtomStream;

// ---------------------------------------------------------------------------
// Ranges
// ---------------------------------------------------------------------------

/// A half-open byte range `[start, end)` into the analysed buffer.
///
/// Byte offsets rather than [`Span`]s throughout: a `Span` additionally
/// carries a 1-based line and a `char` column, neither of which is what LSP
/// wants (see [`crate::LineIndex`]), and carrying them here would invite
/// someone to use them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}

impl ByteRange {
    /// A range, with `end` clamped up to `start` so it can never be inverted.
    pub fn new(start: usize, end: usize) -> Self {
        ByteRange {
            start,
            end: end.max(start),
        }
    }

    /// The range a parsed token or node occupies.
    pub fn of(span: Span) -> Self {
        ByteRange::new(span.start.byte, span.end.byte)
    }

    /// Half-open containment: `start <= byte < end`.
    pub fn contains(self, byte: usize) -> bool {
        self.start <= byte && byte < self.end
    }

    /// Closed containment: `start <= byte <= end`.
    ///
    /// The one a *cursor* wants. An editor reports the caret position, and a
    /// caret sitting immediately after the last character of `foo` is still
    /// "on" `foo` as far as the user is concerned — every other language
    /// server behaves this way, and hovering the end of a word returning
    /// nothing reads as a broken feature.
    pub fn touches(self, byte: usize) -> bool {
        self.start <= byte && byte <= self.end
    }

    /// How many bytes the range covers — the tie-break for "innermost",
    /// both in [`Model::hit_at`] and in [`Model::resolve_in_scope`].
    pub fn len(self) -> usize {
        self.end - self.start
    }

    /// Whether the range covers nothing. A `sig` item's scope is empty by
    /// construction — it declares a name rather than binding one — so this is
    /// a real state and not merely `len() == 0`'s spelling.
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

/// Which of SATySFi's identifier namespaces a name lives in.
///
/// Two names in different namespaces never resolve to each other, however
/// identical their spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ns {
    /// An ordinary value: `let x`, a parameter, a pattern binding.
    Value,
    /// An inline command, `\cmd`.
    InlineCmd,
    /// A block command, `+cmd`.
    BlockCmd,
    /// A math command, `\cmd` inside `${…}`. Spelled with the same token as
    /// an inline command, which is why the namespace has to carry the
    /// distinction.
    MathCmd,
    /// A type name.
    Type,
    /// A type variable, `'a`.
    TypeVar,
    /// A variant constructor, `Some`.
    Ctor,
    /// A module name.
    Module,
    /// A signature name (0.1 only).
    Signature,
    /// A record label or an optional-argument label. Structural: it is not
    /// bound anywhere, so it never resolves. Recorded so that hover can still
    /// say what it is, and so the coverage test below has something to match.
    Field,
}

impl Ns {
    /// How to name this namespace to a human, in hover text.
    pub fn noun(self) -> &'static str {
        match self {
            Ns::Value => "value",
            Ns::InlineCmd => "inline command",
            Ns::BlockCmd => "block command",
            Ns::MathCmd => "math command",
            Ns::Type => "type",
            Ns::TypeVar => "type variable",
            Ns::Ctor => "constructor",
            Ns::Module => "module",
            Ns::Signature => "signature",
            Ns::Field => "record label",
        }
    }
}

/// One name the file binds.
#[derive(Clone, Debug)]
pub struct Def {
    pub ns: Ns,
    /// The name as written — command names keep their `\`/`+` sigil, exactly
    /// as the token carries it, so a [`Ref`]'s name compares equal without any
    /// normalisation step to get wrong.
    pub name: String,
    /// The identifier token itself: what go-to-definition jumps to, and what
    /// a rename would rewrite.
    pub name_span: ByteRange,
    /// Where this name is visible. A later binding of the same name in the
    /// same namespace has a *narrower* scope (it starts later and ends no
    /// later), which is what makes "innermost wins" the whole of shadowing —
    /// see [`Model::resolve_in_scope`].
    pub scope: ByteRange,
    /// How the binding was written: `"let"`, `"let-rec"`, `"let-inline"`,
    /// `"val"`, `"parameter"`, … Rendered verbatim into hover.
    pub form: &'static str,
    /// The declared type, as the range of the source text the author wrote.
    ///
    /// Deliberately a *range* and not an inferred type: nothing in this crate
    /// runs inference, and a type quoted from the buffer is true by
    /// construction. `None` means the author wrote none, and then hover says
    /// nothing about types rather than guessing.
    pub ty: Option<ByteRange>,
    /// Index into [`Model::defs`] of the module this name is a member of.
    pub container: Option<usize>,
    /// A `sig` item: this *declares* a name rather than binding one. Both a
    /// signature's `val foo : τ` and the `struct`'s `let foo = …` produce a
    /// `Def`; the declaration contributes the type, the binding contributes
    /// the location, and [`Model::member`] prefers the binding.
    pub declaration: bool,
}

/// One name the file mentions.
#[derive(Clone, Debug)]
pub struct Ref {
    pub ns: Ns,
    /// The `A.B` of an `A.B.foo`, outermost first; empty for a bare name.
    pub quals: Vec<String>,
    pub name: String,
    /// The whole token, qualification included — an editor highlights and
    /// jumps from `A.B.foo` as one word.
    pub span: ByteRange,
}

/// A `@require:` / `@import:` / `use` header — the file's edge to another
/// file.
#[derive(Clone, Debug)]
pub struct HeaderRef {
    pub kind: HeaderKind,
    /// The package or path name as written, without quotes.
    pub name: String,
    /// The whole header, which is what an editor underlines.
    pub span: ByteRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderKind {
    /// `@require: name` — resolved against the library root.
    Require,
    /// `@import: name` — resolved relative to this file's directory.
    Import,
    /// 0.1's `use package M` / `use M` / `use M of "path"`. Not resolvable
    /// without an envelope graph, so it is recorded for hover and never
    /// followed.
    Use,
}

/// A region where a name this file cannot see into may be in scope.
///
/// `open M` where `M` is not a module defined in this buffer, an `include`
/// inside a `struct`, and 0.1's `use … open` all have the same consequence: a
/// set of names the file does not contain is spliced in at that point, and
/// those names shadow anything bound *before* it. A resolution that would
/// otherwise be confident stops being confident there, and this is how
/// [`Model::resolve`] knows to say nothing rather than to guess.
#[derive(Clone, Debug)]
pub struct Opaque {
    /// From where the spliced names take effect to the end of the enclosing
    /// scope.
    pub scope: ByteRange,
}

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

/// Everything the interactive features know about one buffer.
pub struct Model<'s> {
    /// The buffer, and its line table. One table per model rather than one
    /// per question: hover reports the line a name was bound on, and building
    /// a fresh index for each answer makes a cursor sweep quadratic in the
    /// file size for no reason.
    index: crate::LineIndex<'s>,
    version: RustyfiVersion,
    complete: bool,
    pub defs: Vec<Def>,
    pub refs: Vec<Ref>,
    pub headers: Vec<HeaderRef>,
    pub opaques: Vec<Opaque>,
}

/// What sits under the cursor.
#[derive(Clone, Copy, Debug)]
pub enum Hit<'m> {
    /// The cursor is on a binding occurrence — the `x` of `let x = …`.
    Def(&'m Def),
    /// The cursor is on a mention.
    Ref(&'m Ref),
    /// The cursor is on a `@require:`/`@import:`/`use` header.
    Header(&'m HeaderRef),
}

impl<'s> Model<'s> {
    /// The buffer this model describes.
    pub fn source(&self) -> &'s str {
        self.index.source()
    }

    /// The zero-based line `byte` sits on. O(log lines).
    pub fn line_of(&self, byte: usize) -> u32 {
        self.index.position(byte).line
    }

    /// The generation the buffer was read under — after the ambiguity
    /// re-check, so this is what it actually parses as.
    pub fn version(&self) -> RustyfiVersion {
        self.version
    }

    /// Whether the whole file parsed, as opposed to being recovered
    /// binding-by-binding. Callers use it to decide how much to claim: a
    /// completion list from a partial parse is missing whatever the
    /// half-typed binding would have bound.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// The innermost thing containing `byte`.
    ///
    /// "Innermost" is by width: a `Ref` and a `Def` never overlap (a name is
    /// written once and is either a binding or a mention), but a header spans
    /// a whole line, so the narrower of two candidates is always the more
    /// specific answer.
    pub fn hit_at(&self, byte: usize) -> Option<Hit<'_>> {
        // A candidate only displaces the incumbent on a strictly smaller
        // width, and the three passes run most-specific-first, so a name
        // written inside a header beats the header itself.
        let mut best: Option<(usize, Hit<'_>)> = None;
        for d in self.defs.iter().filter(|d| d.name_span.touches(byte)) {
            if best.is_none_or(|(w, _)| d.name_span.len() < w) {
                best = Some((d.name_span.len(), Hit::Def(d)));
            }
        }
        for r in self.refs.iter().filter(|r| r.span.touches(byte)) {
            if best.is_none_or(|(w, _)| r.span.len() < w) {
                best = Some((r.span.len(), Hit::Ref(r)));
            }
        }
        for h in self.headers.iter().filter(|h| h.span.touches(byte)) {
            if best.is_none_or(|(w, _)| h.span.len() < w) {
                best = Some((h.span.len(), Hit::Header(h)));
            }
        }
        best.map(|(_, hit)| hit)
    }

    /// The module a `A.B.` prefix names at `byte`, as an index into
    /// [`Self::defs`] — completion's entry point for a qualified prefix.
    pub fn module_at(&self, quals: &[String], byte: usize) -> Option<usize> {
        let mut module = self.scope_index(Ns::Module, quals.first()?, byte)?;
        for step in &quals[1..] {
            module = self.member_index(module, Ns::Module, step)?;
        }
        Some(module)
    }

    /// The definition a mention points at, or `None` when the file cannot say
    /// confidently.
    ///
    /// Three ways to end up with `None`, and all three are deliberate:
    /// the name is bound in another file (the overwhelmingly common case for
    /// `document`, `\emph`, `+p` and everything a package exports); the name
    /// is qualified by a module this file does not define; or an opaque `open`
    /// stands between the candidate binding and the mention, so a name from
    /// outside the file may be shadowing it (see [`Opaque`]).
    pub fn resolve(&self, r: &Ref) -> Option<&Def> {
        self.resolve_index(r).map(|i| &self.defs[i])
    }

    fn resolve_index(&self, r: &Ref) -> Option<usize> {
        if r.quals.is_empty() {
            return self.scope_index(r.ns, &r.name, r.span.start);
        }
        // A qualified name: walk the module chain from the innermost module
        // visible at the mention, then take the member.
        let mut module = self.scope_index(Ns::Module, &r.quals[0], r.span.start)?;
        for step in &r.quals[1..] {
            module = self.member_index(module, Ns::Module, step)?;
        }
        self.member_index(module, r.ns, &r.name)
    }

    /// A bare name, resolved by scope at `byte`.
    pub fn resolve_in_scope(&self, ns: Ns, name: &str, byte: usize) -> Option<&Def> {
        self.scope_index(ns, name, byte).map(|i| &self.defs[i])
    }

    fn scope_index(&self, ns: Ns, name: &str, byte: usize) -> Option<usize> {
        // A `Field` is structural — a record label is not bound anywhere, and
        // matching one against a value of the same spelling would be exactly
        // the wrong answer this crate is trying not to give.
        if ns == Ns::Field {
            return None;
        }
        let best = self
            .defs
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                d.ns == ns && d.name == name && !d.declaration && d.scope.contains(byte)
            })
            // Narrowest scope wins. In a lexically scoped language the scopes
            // containing one point are nested, so "narrowest" IS "innermost",
            // and a rebinding at the same level counts too: `let x = 1 … let
            // x = 2 …` gives the second a scope that starts later and ends at
            // the same place, hence a strictly narrower one.
            .min_by_key(|(_, d)| (d.scope.len(), usize::MAX - d.scope.start))?;
        match self.shadowed_by_opaque(best.1, byte) {
            true => None,
            false => Some(best.0),
        }
    }

    /// Whether an `open` this file cannot see into sits between `def` and the
    /// mention at `byte`.
    ///
    /// Only an open that takes effect **after** the binding can shadow it: a
    /// binding written after the open shadows the open's names instead, which
    /// is the ordinary case (a file's own `let`s follow its headers and its
    /// `open`s), so this costs almost nothing in practice.
    fn shadowed_by_opaque(&self, def: &Def, byte: usize) -> bool {
        self.opaques
            .iter()
            .any(|o| o.scope.contains(byte) && o.scope.start > def.name_span.start)
    }

    /// A member of the module at `defs[module]`.
    ///
    /// Prefers the `struct`'s own binding over the signature's declaration of
    /// the same name: both are real, but only the binding is a place to jump
    /// to. Later bindings win over earlier ones, matching evaluation order.
    pub fn member(&self, module: usize, ns: Ns, name: &str) -> Option<&Def> {
        self.member_index(module, ns, name).map(|i| &self.defs[i])
    }

    fn member_index(&self, module: usize, ns: Ns, name: &str) -> Option<usize> {
        let matching = || {
            self.defs
                .iter()
                .enumerate()
                .filter(move |(_, d)| d.container == Some(module) && d.ns == ns && d.name == name)
        };
        matching()
            .filter(|(_, d)| !d.declaration)
            .next_back()
            .or_else(|| matching().next_back())
            .map(|(i, _)| i)
    }

    /// Every member of a module, one per (namespace, name), preferring the
    /// binding over the declaration exactly as [`Self::member`] does.
    pub fn members(&self, module: usize) -> Vec<&Def> {
        let mut out: Vec<&Def> = Vec::new();
        for d in self.defs.iter().filter(|d| d.container == Some(module)) {
            match out.iter().position(|k| k.ns == d.ns && k.name == d.name) {
                Some(i) if out[i].declaration || !d.declaration => out[i] = d,
                Some(_) => {}
                None => out.push(d),
            }
        }
        out
    }

    /// Every name of namespace `ns` visible at `byte`, one per name, the
    /// innermost binding of each.
    ///
    /// This is completion's whole candidate set. Names bound in *other* files
    /// are absent, which is a real limitation and a deliberate one: a list
    /// padded out with names that may or may not be in scope is worse than a
    /// short list that is right.
    pub fn in_scope(&self, ns: Ns, byte: usize) -> Vec<&Def> {
        let mut out: Vec<&Def> = Vec::new();
        // `touches`, not `contains`: completion is asked about a CURSOR, and
        // the cursor at the very end of a file is exactly where a user is
        // typing the next binding. Resolution keeps the half-open rule — a
        // reference is a token, and a token never starts at a scope's end.
        for d in self
            .defs
            .iter()
            .filter(|d| d.ns == ns && !d.declaration && d.scope.touches(byte))
        {
            match out.iter().position(|k| k.name == d.name) {
                Some(i) if d.scope.len() < out[i].scope.len() => out[i] = d,
                Some(_) => {}
                None => out.push(d),
            }
        }
        out
    }

    /// The source text of a range, whitespace-collapsed onto one line —
    /// hover renders a declared type by quoting the buffer, and a type
    /// spanning three lines must not paste three lines into a popup.
    pub fn text(&self, range: ByteRange) -> String {
        let source = self.source();
        let start = crate::line_index::floor_boundary(source, range.start);
        let end = crate::line_index::floor_boundary(source, range.end.max(start));
        source[start..end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

// ---------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------

/// Accumulates a [`Model`] while a walk runs. The two grammar walks
/// ([`crate::walk006`], [`crate::walk01`]) are `impl` blocks on this type.
pub(crate) struct Builder<'s> {
    pub(crate) source: &'s str,
    pub(crate) defs: Vec<Def>,
    pub(crate) refs: Vec<Ref>,
    pub(crate) headers: Vec<HeaderRef>,
    pub(crate) opaques: Vec<Opaque>,
}

impl<'s> Builder<'s> {
    fn new(source: &'s str) -> Self {
        Builder {
            source,
            defs: Vec::new(),
            refs: Vec::new(),
            headers: Vec::new(),
            opaques: Vec::new(),
        }
    }

    /// Record a binding; returns its index, which a module needs so its
    /// members can name it as their container.
    pub(crate) fn def(&mut self, def: Def) -> usize {
        self.defs.push(def);
        self.defs.len() - 1
    }

    pub(crate) fn reference(&mut self, ns: Ns, name: &str, span: Span) {
        self.refs.push(Ref {
            ns,
            quals: Vec::new(),
            name: name.to_string(),
            span: ByteRange::of(span),
        });
    }

    pub(crate) fn qualified(&mut self, ns: Ns, quals: &[String], name: &str, span: Span) {
        self.refs.push(Ref {
            ns,
            quals: quals.to_vec(),
            name: name.to_string(),
            span: ByteRange::of(span),
        });
    }

    /// The common case: a binding with no container, no signature and no
    /// written type.
    pub(crate) fn bind(
        &mut self,
        ns: Ns,
        name: &str,
        span: Span,
        scope: ByteRange,
        form: &'static str,
    ) -> usize {
        self.def(Def {
            ns,
            name: name.to_string(),
            name_span: ByteRange::of(span),
            scope,
            form,
            ty: None,
            container: None,
            declaration: false,
        })
    }

    pub(crate) fn header(&mut self, kind: HeaderKind, name: &str, span: Span) {
        self.headers.push(HeaderRef {
            kind,
            name: name.to_string(),
            span: ByteRange::of(span),
        });
    }

    pub(crate) fn opaque(&mut self, scope: ByteRange) {
        self.opaques.push(Opaque { scope });
    }

    /// Bring a module's names into `scope`, the way `open M` does.
    ///
    /// When `M` is a module this file defines, its exports are re-bound over
    /// `scope` as ordinary [`Def`]s pointing back at their real locations —
    /// so "narrowest scope wins" handles the shadowing without a second rule,
    /// and go-to-definition still lands on the member itself.
    ///
    /// When it is not — the usual case, since most `open`s name a `@require:`d
    /// package — the names it would bring are unknowable here, so the region
    /// is marked [`Opaque`] and every resolution that an unseen name could
    /// have shadowed declines instead of guessing.
    ///
    /// A module carrying a signature exports only what the signature declares:
    /// re-binding a private helper would let it shadow a same-named
    /// file-level binding that the real program leaves alone.
    pub(crate) fn open_module(&mut self, quals: &[String], name: &str, scope: ByteRange) {
        let Some(module) = self.find_module(quals, name, scope.start) else {
            self.opaque(scope);
            return;
        };
        let sealed = self
            .defs
            .iter()
            .any(|d| d.container == Some(module) && d.declaration);
        let exported: Vec<Def> = self
            .defs
            .iter()
            .filter(|d| d.container == Some(module) && !d.declaration)
            .filter(|d| !sealed || self.declares(module, d.ns, &d.name))
            .map(|d| Def {
                scope,
                container: None,
                ..d.clone()
            })
            .collect();
        self.defs.extend(exported);
    }

    fn declares(&self, module: usize, ns: Ns, name: &str) -> bool {
        self.defs
            .iter()
            .any(|d| d.container == Some(module) && d.declaration && d.ns == ns && d.name == name)
    }

    /// The module a path names, as an index, using only what the walk has
    /// recorded so far. Sound because a module must be written before it can
    /// be opened.
    fn find_module(&self, quals: &[String], name: &str, byte: usize) -> Option<usize> {
        let mut current = match quals.first() {
            None => self.module_in_scope(name, byte)?,
            Some(head) => self.module_in_scope(head, byte)?,
        };
        if quals.is_empty() {
            return Some(current);
        }
        for step in &quals[1..] {
            current = self.module_member(current, step)?;
        }
        self.module_member(current, name)
    }

    fn module_in_scope(&self, name: &str, byte: usize) -> Option<usize> {
        self.defs
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                d.ns == Ns::Module && d.name == name && !d.declaration && d.scope.contains(byte)
            })
            .min_by_key(|(_, d)| d.scope.len())
            .map(|(i, _)| i)
    }

    fn module_member(&self, module: usize, name: &str) -> Option<usize> {
        self.defs
            .iter()
            .enumerate()
            .filter(|(_, d)| d.container == Some(module) && d.ns == Ns::Module && d.name == name)
            .next_back()
            .map(|(i, _)| i)
    }

    /// Copy a signature's declared types onto the bindings that implement
    /// them, so hovering `M.foo` at a use site shows the type the author
    /// wrote in the `sig` even though the `let` itself carries none.
    ///
    /// Only fills a hole: a binding with its own ascription keeps it.
    fn adopt_declared_types(&mut self) {
        let declared: Vec<(Option<usize>, Ns, String, ByteRange)> = self
            .defs
            .iter()
            .filter(|d| d.declaration && d.ty.is_some())
            .map(|d| (d.container, d.ns, d.name.clone(), d.ty.unwrap()))
            .collect();
        for d in &mut self.defs {
            if d.declaration || d.ty.is_some() {
                continue;
            }
            if let Some((_, _, _, ty)) = declared
                .iter()
                .find(|(c, ns, n, _)| *c == d.container && *ns == d.ns && *n == d.name)
            {
                d.ty = Some(*ty);
            }
        }
    }

    fn finish(mut self, version: RustyfiVersion, complete: bool) -> Model<'s> {
        self.adopt_declared_types();
        Model {
            index: crate::LineIndex::new(self.source),
            version,
            complete,
            defs: self.defs,
            refs: self.refs,
            headers: self.headers,
            opaques: self.opaques,
        }
    }
}

/// Build the model for one buffer.
///
/// `lang` pins the generation the way `rustyfi lsp --lang` does; `None`
/// detects it, by the same ladder [`crate::analyze_detected`] uses and for the
/// same reason — a library file of either generation typically opens `module M
/// = struct`, which is no signal at all, and reading a 0.1 package as 0.0.6
/// would leave the model empty on 32 of this port's own 34 bundled 0.1
/// packages.
///
/// Where that ladder compares how far each generation's *parse* got, this one
/// compares how much of the file each generation could *recover*, which is the
/// same question asked of the thing being built here.
pub fn build_model(source: &str, lang: Option<RustyfiVersion>) -> Model<'_> {
    if let Some(v) = lang {
        return build_as(source, v);
    }
    let sniffed = rustyfi_syntax::sniff_version(source);
    let primary = sniffed.unwrap_or(RustyfiVersion::DEFAULT);
    let first = build_as(source, primary);
    // A decisive `use`/`val` or `@stage:`/`let-*` head is obeyed even when the
    // file does not parse: reading it as the other generation would describe a
    // file the author did not write.
    if first.complete || sniffed.is_some() {
        return first;
    }
    let other = match primary {
        RustyfiVersion::V0_1 => RustyfiVersion::V0_0,
        _ => RustyfiVersion::V0_1,
    };
    let second = build_as(source, other);
    match second.complete || second.defs.len() > first.defs.len() {
        true => second,
        false => first,
    }
}

fn build_as(source: &str, version: RustyfiVersion) -> Model<'_> {
    let mut b = Builder::new(source);
    let complete = match version {
        RustyfiVersion::V0_1 => b.parse_and_walk_v01(source),
        _ => b.parse_and_walk_v006(source),
    };
    b.finish(version, complete)
}

impl<'s> Builder<'s> {
    /// Read the buffer under the 0.0.6 grammar, walking either the whole file
    /// or as much of it as recovers. Returns whether the whole file parsed.
    fn parse_and_walk_v006(&mut self, source: &str) -> bool {
        let (atoms, lex_failed) = lex(source, RustyfiVersion::V0_0);
        let mut stream = crate::budget::stream(atoms.clone());
        if !lex_failed {
            if let Ok(file) = <cst::File as Parse<_>>::parse(&mut stream) {
                self.v006_file(&file);
                return true;
            }
        }
        // Recovery: `File`'s own fields, one at a time, without its `eoi`.
        // `Vec<T>` is total (syan's collection impl stops at the first element
        // that fails and returns what it has), so this yields every complete
        // top-level binding before the one being typed.
        let mut stream = crate::budget::stream(atoms);
        let headers = <Vec<cst::Header> as Parse<_>>::parse(&mut stream).unwrap_or_default();
        let prelude = <Vec<cst::TopBinding> as Parse<_>>::parse(&mut stream).unwrap_or_default();
        let text_end = remainder_start(&mut stream, source);
        let in_kw = <Option<rustyfi_syntax::leaf::KwIn> as Parse<_>>::parse(&mut stream)
            .ok()
            .flatten();
        let body = match in_kw.is_some() {
            true => <Option<cst::ast::Expr> as Parse<_>>::parse(&mut stream)
                .ok()
                .flatten(),
            false => None,
        };
        self.v006_parts(&headers, &prelude, body.as_ref(), text_end);
        false
    }

    /// The 0.1 counterpart. The library shape recovers the same way; a 0.1
    /// *document* is headers plus one expression, so a half-typed one recovers
    /// its headers and, when the expression itself still parses, its body.
    fn parse_and_walk_v01(&mut self, source: &str) -> bool {
        let (atoms, lex_failed) = lex(source, RustyfiVersion::V0_1);
        let mut stream = crate::budget::stream(atoms.clone());
        if !lex_failed {
            if let Ok(file) = <cst_v1::FileV1 as Parse<_>>::parse(&mut stream) {
                self.v01_file(&file);
                return true;
            }
        }
        let mut stream = crate::budget::stream(atoms);
        let headers = <Vec<cst_v1::HeaderV1> as Parse<_>>::parse(&mut stream).unwrap_or_default();
        let head = <Option<V01LibraryHead> as Parse<_>>::parse(&mut stream)
            .ok()
            .flatten();
        match head {
            Some(head) => {
                let binds = <Vec<cst_v1::Bind> as Parse<_>>::parse(&mut stream).unwrap_or_default();
                let text_end = remainder_start(&mut stream, source);
                self.v01_library_parts(&headers, &head, &binds, source.len(), text_end);
            }
            None => {
                let body = <Option<cst_v1::ast::Expr> as Parse<_>>::parse(&mut stream)
                    .ok()
                    .flatten();
                self.v01_document_parts(&headers, body.as_ref());
            }
        }
        false
    }
}

/// Where the text the recovery path could **not** read begins.
///
/// The boundary between the last binding that parsed and the one being typed,
/// which is what that binding's own text ends at — and, crucially, where the
/// name it bound starts being visible. Getting it wrong by taking the end of
/// the file instead gives every recovered binding an empty scope, so the
/// half-typed buffer that motivated the recovery in the first place answers
/// nothing.
///
/// Read from the stream rather than from the high-water mark: a `Vec<T>` rolls
/// back the element that failed, so the next atom to be served is exactly the
/// first token of the remainder, whereas the mark remembers how far the failed
/// attempt reached before it was undone.
fn remainder_start(stream: &mut AtomStream, source: &str) -> usize {
    use syan::parse::ParseStream;
    stream
        .peek()
        .map(|a| a.span.start.byte)
        .unwrap_or(source.len())
}

/// The `module M :> S = struct` head of a 0.1 library, as its own rule so the
/// recovery path can try it and roll back without hand-written backtracking.
///
/// A field-for-field prefix of [`cst_v1::FileV1::Library`], stopping at
/// `struct`; parsing it separately is what lets the binding list that follows
/// be recovered with `Vec<Bind>` when the closing `end` has not been typed
/// yet.
// `module_kw` and `eq` are read by the generated parser and by nothing else;
// dropping them would change what the rule accepts.
#[allow(dead_code)]
#[derive(Parse, Debug)]
pub(crate) struct V01LibraryHead {
    pub(crate) module_kw: rustyfi_syntax::leaf::KwModule,
    pub(crate) name: rustyfi_syntax::leaf::CtorTok,
    pub(crate) sig_annot: Option<cst_v1::SigAnnotV1>,
    pub(crate) eq: rustyfi_syntax::leaf::DefEqTok,
    pub(crate) struct_kw: rustyfi_syntax::leaf::KwStruct,
}

/// Lex, keeping whatever came out before a failure.
///
/// A half-typed buffer commonly does not lex at all — `{\emp` is "unexpected
/// token in an active area" and `'<+p` is an unterminated group — and
/// discarding the tokens before the failure would leave the model empty on
/// exactly the buffers the editor asks about most. Every token that *did* come
/// out is well-formed and correctly positioned, so the recovery path below
/// parses them as usual and recovers every complete binding written before the
/// break. Returns whether the lex failed, which is one of the two ways
/// [`Model::is_complete`] can be false.
fn lex(source: &str, version: RustyfiVersion) -> (Vec<rustyfi_syntax::Atom>, bool) {
    let (atoms, err) = rustyfi_syntax::lex_partial(source, version);
    (atoms, err.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_touches_its_own_end_but_does_not_contain_it() {
        let r = ByteRange::new(2, 5);
        assert!(r.contains(4) && !r.contains(5));
        assert!(r.touches(5) && !r.touches(6));
    }

    #[test]
    fn an_inverted_range_is_clamped_rather_than_wrapping() {
        let r = ByteRange::new(9, 3);
        assert_eq!((r.start, r.end), (9, 9));
        assert_eq!(r.len(), 0);
    }
}

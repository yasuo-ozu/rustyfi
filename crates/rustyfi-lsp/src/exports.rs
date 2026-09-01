//! What a buffer's DEPENDENCIES offer to completion.
//!
//! [`crate::build_model`] takes a single `&str` and knows nothing outside it,
//! which is right for hover and go-to-definition -- both start from a token
//! the user is pointing at -- and wrong for completion, which has to answer
//! "what could go here?" from the whole environment. The consequence was that
//! omni-completion worked and looked broken: in a real document every command
//! comes from a package, so `\La` offered nothing while `\code`, defined
//! twenty lines up, offered itself. Reported exactly that way.
//!
//! An [`Export`] is one name a dependency binds, flattened to the three things
//! a completion item needs. Deliberately NOT a [`crate::Def`]: a `Def` carries
//! spans into a source string it borrows, and these outlive the text they came
//! from -- they are cached across edits to the buffer that consumes them.
//!
//! # Which names count
//!
//! Two kinds, and the second is the one that matters most in practice.
//!
//! **File-level bindings**: a name whose scope reaches the end of its file.
//! Anything narrower is inside a function body or a local `let ... in` that no
//! other file can name. Parameters fall out by the same rule with no special
//! case, since a parameter's scope ends with its body.
//!
//! **`direct` declarations in a 0.0.6 module signature.** A file-level test
//! alone was WRONG, and wrong for the commands a user reaches for first: the
//! bundled document classes wrap everything in `module StdJaBook : sig … end
//! = struct … end`, so `\emph` and `+subsection` have a scope that ends at
//! `end`, not at EOF. They were excluded twice over -- once by the scope
//! test, and again because a `sig` item is recorded with `declaration: true`
//! and `scope: 0..0` (`walk006.rs`, the `declare` closure: "a declaration
//! binds nothing, so it is visible nowhere").
//!
//! `direct` is exactly the right admission rule rather than a workaround for
//! one package. In 0.0.6 a `direct` member is the one kind reachable UNQUALIFIED
//! from a consumer -- that is what the keyword means -- so the set of `direct`
//! declarations in a dependency's signature IS its unqualified surface, which
//! is precisely the set completion should offer for a bare `\`/`+` word.
//! An ordinary `val` in the same signature needs `M.name` and is deliberately
//! not offered here; see the qualified-word case in `features::completions`,
//! which does not consult this list at all.
//!
//! Reported as "\emph, \subsection is not complemented".

use crate::model::{build_model, Ns};

/// One name a dependency binds, as completion needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    /// As written, sigil included -- `\emph`, `+p`, `frac`. The same spelling
    /// [`crate::Def::name`] uses, so a needle matches against both without
    /// normalisation.
    pub name: String,
    pub ns: Ns,
    /// How it was bound (`"let-inline"`, `"let-block"`, …), for the detail
    /// line, so a reader can tell a command from a value at a glance.
    pub form: &'static str,
    /// The file it came from, as a display name -- the stem, not the path,
    /// because a completion popup has one line and `stdjabook` says more in
    /// it than forty characters of directory do.
    pub origin: String,
}

/// Collect one file's file-level bindings.
///
/// `origin` is what to show in the popup; callers pass the file stem.
pub fn from_source(source: &str, lang: Option<crate::RustyfiVersion>, origin: &str) -> Vec<Export> {
    let model = build_model(source, lang);
    let end = source.len();
    let mut out: Vec<Export> = Vec::new();
    for d in &model.defs {
        // A `direct` sig item is the module's UNQUALIFIED surface -- see the
        // module doc. Admitted before the two filters below, because it fails
        // both: it is a declaration, and its scope is the empty 0..0 a
        // declaration carries.
        let direct = d.declaration && d.form == "direct";
        // Any other declaration names something without binding it here; the
        // binding is elsewhere in the same file and is the one worth
        // offering, so taking both would duplicate.
        if d.declaration && !direct {
            continue;
        }
        // A parameter is never an export, and the scope test alone does not
        // say so: at the END of a file every open scope reaches EOF, so the
        // parameters of the last binding pass it. Excluded by form.
        if d.form == "parameter" {
            continue;
        }
        // File-level, per the module doc: scope reaches the end of the file.
        //
        // KNOWN IMPRECISION, and it is one-directional. A local `let … in`
        // in the LAST binding of a file has a scope that also reaches EOF,
        // and nothing here distinguishes it from a top-level one -- the two
        // are the same shape, and 0.0.6's top level IS a `let … in` chain.
        // So a trailing local can be offered by another file. That is noise
        // in a popup, never a wrong answer about a name that exists; the
        // opposite error, dropping a real export, is the bug this whole
        // module exists to fix. Pinned as a known case rather than left to
        // be rediscovered.
        if !direct && d.scope.end < end {
            continue;
        }
        // A type variable is scoped to one signature and means nothing to a
        // consumer; a module's own name does travel, so it stays.
        if d.ns == Ns::TypeVar {
            continue;
        }
        if out.iter().any(|e| e.name == d.name && e.ns == d.ns) {
            continue;
        }
        out.push(Export {
            name: d.name.clone(),
            ns: d.ns,
            form: d.form,
            origin: origin.to_string(),
        });
    }
    out
}

/// The display name for a path: its stem, or the whole thing if it has none.
pub fn origin_of(path: &std::path::Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

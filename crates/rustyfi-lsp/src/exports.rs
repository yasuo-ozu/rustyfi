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
//! Only FILE-LEVEL bindings: a name whose scope reaches the end of its file is
//! one a consumer can see, and anything narrower is inside a function body or
//! a local `let ... in` that no other file can name. That test is one
//! comparison and it is exactly right for `.satyh` libraries, which is what
//! dependencies are.
//!
//! Parameters are excluded by the same rule without a special case, since a
//! parameter's scope ends with its body.

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
        // A declaration (a `val` in a signature, say) names something without
        // binding it here; the binding itself is elsewhere in the same file
        // and is the one worth offering, so taking both would duplicate.
        if d.declaration {
            continue;
        }
        // File-level, per the module doc: scope reaches the end of the file.
        if d.scope.end < end {
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

//! `search <keyword>...`: acquire the registry index and list every
//! package whose name or description contains the keywords (case-insensitive
//! substring matches), reporting `name`, the highest available version, and the
//! description.
//!
//! Several keywords narrow the match: a hit must match every one of them.
//!
//! ## `name` is what `install` accepts
//!
//! The bundled default registry is Satyrographos' own, an OPAM repository, so
//! its packages enumerate as OPAM package ids (`satysfi-fonts-theano`) while
//! `install NAME` — via `registry::lookup`'s conventional "try `<name>`, else
//! `satysfi-<name>`" resolution — takes the bare library name
//! (`fonts-theano`). Printing the raw opam id would give a name that does not
//! round-trip to `@require:`/a `Satyristes` dependency entry, so
//! [`SearchHit::name`] is always the installable form; the registry's own id
//! is kept in [`SearchHit::registry_name`] when it differs.

use crate::error::Error;
use crate::registry::{self, RegistryOptions};

/// One matched package.
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// The name to pass to `install` — see the module doc for why this is not
    /// always the registry's own raw package id.
    pub name: String,
    /// The registry's own package id, when it differs from [`Self::name`]
    /// (an OPAM-backed index's `satysfi-`-prefixed id). `None` when the two
    /// are identical.
    pub registry_name: Option<String>,
    /// The highest available version (empty if the entry lists none).
    pub version: String,
    pub description: Option<String>,
}

/// The inverse of `registry::lookup`'s "try `<name>`, else `satysfi-<name>`"
/// resolution: given a package id `search` enumerated, the name `install`
/// would actually accept for it.
fn installable_name(raw: &str) -> &str {
    raw.strip_prefix("satysfi-")
        .filter(|rest| !rest.is_empty())
        .unwrap_or(raw)
}

/// Search the registry index for every keyword in `terms` (all must match).
/// Returns hits sorted by name; an empty `terms` lists everything.
pub fn search(
    terms: &[&str],
    reg_opts: &RegistryOptions,
    registry_url_fallback: Option<&str>,
) -> Result<Vec<SearchHit>, Error> {
    let url = reg_opts.resolve_url(registry_url_fallback)?;
    let reg = registry::acquire(&url, reg_opts)?;
    let needles: Vec<String> = terms
        .iter()
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();

    let mut hits = Vec::new();
    for name in registry::all_package_names(&reg)? {
        // An index holds entries this port cannot install — an OPAM `conf-*`
        // package has no source archive at all — and one of them must not
        // sink the whole search.
        let Ok(idx) = registry::lookup(&reg, &name) else {
            continue;
        };
        let desc = idx.description.clone();
        let haystack = format!("{} {}", name.to_lowercase(), desc.as_deref().unwrap_or("").to_lowercase());
        if needles.iter().all(|n| haystack.contains(n.as_str())) {
            let version = registry::select_version(&idx, &name, None)
                .map(|(v, _)| v)
                .unwrap_or_default();
            let install_name = installable_name(&name).to_string();
            let registry_name = if install_name != name { Some(name.clone()) } else { None };
            hits.push(SearchHit {
                name: install_name,
                registry_name,
                version,
                description: desc,
            });
        }
    }
    // Enumeration order (`all_package_names`) is by raw registry id, which
    // does not match `name` one-for-one once an opam prefix is stripped —
    // re-sort by the name actually shown/installable.
    hits.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(hits)
}

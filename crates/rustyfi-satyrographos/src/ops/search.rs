//! `search <keyword>...` (plan §8): acquire the registry index and list every
//! package whose name or description contains the keywords (case-insensitive
//! substring matches), reporting `name`, the highest available version, and the
//! description.
//!
//! Several keywords NARROW: a hit has to match every one of them. Searching a
//! repository of a few thousand packages for one common word returns more than
//! anyone reads, and the useful next move is to add a word, not to get the
//! union of two floods.

use crate::error::Error;
use crate::registry::{self, RegistryOptions};

/// One matched package.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub name: String,
    /// The highest available version (empty if the entry lists none).
    pub version: String,
    pub description: Option<String>,
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
        // Each keyword may land in the name or in the description; what has to
        // hold for every one of them is that it landed SOMEWHERE.
        if needles.iter().all(|n| haystack.contains(n.as_str())) {
            let version = registry::select_version(&idx, &name, None)
                .map(|(v, _)| v)
                .unwrap_or_default();
            hits.push(SearchHit {
                name,
                version,
                description: desc,
            });
        }
    }
    // `all_package_names` already returns sorted names.
    Ok(hits)
}

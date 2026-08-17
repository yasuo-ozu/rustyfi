//! `satyrographos search <term>` (plan §8): acquire the registry index and
//! list every package whose name or description contains `term` (a
//! case-insensitive substring match), reporting `name`, the highest available
//! version, and the description.

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

/// Search the registry index for `term`. Returns hits sorted by name.
pub fn search(
    term: &str,
    reg_opts: &RegistryOptions,
    registry_url_fallback: Option<&str>,
) -> Result<Vec<SearchHit>, Error> {
    let url = reg_opts.resolve_url(registry_url_fallback)?;
    let reg = registry::acquire(&url, reg_opts)?;
    let needle = term.to_lowercase();

    let mut hits = Vec::new();
    for name in registry::all_package_names(&reg)? {
        let idx = registry::lookup(&reg, &name)?;
        let desc = idx.description.clone();
        let name_matches = name.to_lowercase().contains(&needle);
        let desc_matches = desc
            .as_deref()
            .map(|d| d.to_lowercase().contains(&needle))
            .unwrap_or(false);
        if needle.is_empty() || name_matches || desc_matches {
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

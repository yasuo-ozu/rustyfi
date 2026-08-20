//! Library-root resolution (plan §3/§4) and the managed-root marker
//! (plan §6).
//!
//! A "root" here is a SATySFi library root: the directory that holds
//! `dist/packages/…` (where the loader resolves `@require:` from, see
//! `rustyfi-loader`) and, for this port, a `.satyrographos/` bookkeeping
//! subtree (receipts + staging). `install` materialises into `<root>/dist/…`
//! and records `<root>/.satyrographos/receipts/<name>.toml`.

use std::path::{Path, PathBuf};

use crate::error::Error;

/// The marker/bookkeeping directory under a managed root.
pub const MANAGED_DIR: &str = ".satyrographos";

/// A checked-out source tree's own root — the development case.
pub const DEV_DIR: &str = "lib-rustyfi";

/// A project-local root, beside the `Satyristes` that describes the project.
pub const LOCAL_DIR: &str = ".rustyfi";

/// Under a user or system prefix, the layout the release archive unpacks to
/// (`<prefix>/{bin,lib,share}`), so untarring one into `~/.local` or
/// `/usr/local` puts a root exactly where this looks.
const PREFIX_SUFFIX: &str = "lib/rustyfi";

/// Search a directory and every ancestor for a root, then the user and system
/// prefixes. Returns the first candidate that exists.
///
/// The order, from `start` upward and then outward:
///
/// 1. `<dir>/lib-rustyfi/` — a checked-out source tree, so working inside one
///    always uses that tree's own packages rather than anything installed.
/// 2. `<dir>/.rustyfi/`, but only where a `Satyristes` sits beside it — the
///    project-local install. The manifest is what marks the directory as a
///    project; without it, a stray `.rustyfi/` further up the filesystem
///    would capture unrelated documents.
/// 3. `~/.local/lib/rustyfi` — user-wide.
/// 4. `/usr/local/lib/rustyfi`, then `/usr/lib/rustyfi` — system-wide.
///
/// 1 and 2 are checked *per directory* on the way up, so the nearest project
/// wins outright: a `lib-rustyfi/` three levels above never beats a
/// `.rustyfi/` in the directory you are standing in.
///
/// Existence is the whole test for 1 and 2 — a directory of either name is a
/// deliberate marker. The prefixes in 3 and 4 are ordinary locations that
/// merely might exist, so they must actually hold a `dist/` to count.
pub fn discover(start: &Path) -> Option<PathBuf> {
    discover_all(start).into_iter().next()
}

/// Every root found from `start`, nearest first — the search PATH, not just
/// its head.
///
/// A project-local root does not replace the wider ones, it precedes them: a
/// project that installs one package into `.rustyfi/` still resolves the rest
/// of its `@require:`s from the development tree or the system install, the
/// same way upstream SATySFi searches `$CWD/.satysfi`, `$HOME/.satysfi` and
/// `/usr/share/satysfi` in turn and takes the first root that has the file.
/// Only the FIRST entry is a write target (see [`resolve_root`]); the rest are
/// there to read from.
///
/// Duplicates are dropped, so a root reachable two ways (say the walk reaches
/// `~/.local/lib/rustyfi` and the user prefix names it again) is searched once.
pub fn discover_all(start: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let push = |p: PathBuf, found: &mut Vec<PathBuf>| {
        let key = p.canonicalize().unwrap_or_else(|_| p.clone());
        if !found
            .iter()
            .any(|q| q.canonicalize().unwrap_or_else(|_| q.clone()) == key)
        {
            found.push(p);
        }
    };

    // NORMALISE first, or the walk is nonsense. Two traps, both real:
    //
    // - `parent()` of a relative `doc.saty` is `""`, whose own parent is
    //   `None`, so a document named without a directory would see only the
    //   working directory. `absolute("")` errors rather than returning the
    //   working directory, so spell it `.`.
    // - `absolute()` merely prefixes the working directory; it does NOT
    //   resolve `..`. Walking `repo/../other/doc` upward therefore passes
    //   through `repo/..`, then `repo` — finding a root in the very directory
    //   the path was leading AWAY from. `canonicalize` resolves the traversal
    //   (and symlinks); it needs the path to exist, which a start directory
    //   does, and falls back to `absolute` when it does not.
    let start = if start.as_os_str().is_empty() {
        Path::new(".")
    } else {
        start
    };
    let start = start
        .canonicalize()
        .or_else(|_| std::path::absolute(start))
        .unwrap_or_else(|_| start.to_path_buf());
    let mut dir = Some(start.as_path());
    while let Some(d) = dir {
        let dev = d.join(DEV_DIR);
        if dev.is_dir() {
            push(dev, &mut found);
        }
        let local = d.join(LOCAL_DIR);
        if d.join(crate::satyristes::SATYRISTES_NAME).is_file() && local.is_dir() {
            push(local, &mut found);
        }
        dir = d.parent();
    }
    for prefix in prefix_roots() {
        if prefix.join("dist").is_dir() {
            push(prefix, &mut found);
        }
    }
    found
}

/// The user-wide root, then the system-wide ones, in search order.
pub fn prefix_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    // `$HOME` on unix, `%USERPROFILE%` on Windows, where the same archive
    // unpacks to the same relative layout.
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        roots.push(PathBuf::from(home).join(".local").join(PREFIX_SUFFIX));
    }
    if !cfg!(windows) {
        roots.push(PathBuf::from("/usr/local").join(PREFIX_SUFFIX));
        roots.push(PathBuf::from("/usr").join(PREFIX_SUFFIX));
    }
    roots
}

/// Resolve the root to operate on, following the plan §4 precedence for the
/// *library-management* commands (the compile-mode version→path fallback of
/// §3 step 4 lives in the CLI layer, not here — this crate never sees a
/// `RustyfiVersion`):
///
/// 1. `dest` (the raw `--dest DIR` override) — used verbatim, bypassing
///    discovery entirely.
/// 2. `lib_root` (the `--lib-root DIR` flag).
/// 3. `$RUSTYFI_LIB_ROOT`.
/// 4. [`discover`] from the current directory.
///
/// If none is available, [`Error::RootResolution`] (CLI exit `3`). `dest`
/// and `lib_root` are mutually exclusive at the CLI (an `ArgGroup`); if both
/// somehow arrive here, `dest` wins.
pub fn resolve_root(lib_root: Option<&Path>, dest: Option<&Path>) -> Result<PathBuf, Error> {
    if let Some(dest) = dest {
        return Ok(dest.to_path_buf());
    }
    if let Some(lib_root) = lib_root {
        return Ok(lib_root.to_path_buf());
    }
    if let Some(env) = std::env::var_os("RUSTYFI_LIB_ROOT") {
        return Ok(PathBuf::from(env));
    }
    // Package operations start from where the user is standing; the compiler
    // starts from the document instead, but runs the same search.
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(found) = discover(&cwd) {
            return Ok(found);
        }
    }
    Err(Error::RootResolution)
}

/// The shared `--lib-root`/`--dest` root selection carried by both
/// [`InstallOptions`](crate::ops::install::InstallOptions) and
/// [`RootOptions`](crate::ops::uninstall::RootOptions). Implementing it in one
/// place keeps every operation's root resolution identical (plan §4): the
/// install/reconcile/registry paths go through [`resolve_managed_root`] (which
/// also lays down the `.satyrographos/` skeleton), and the read-only
/// list/status/uninstall paths through [`resolve_root`].
///
/// [`resolve_managed_root`]: RootSelection::resolve_managed_root
/// [`resolve_root`]: RootSelection::resolve_root
pub(crate) trait RootSelection {
    fn lib_root(&self) -> Option<&Path>;
    fn dest(&self) -> Option<&Path>;

    /// Resolve the target root by the plan §4 precedence (see the free
    /// [`resolve_root`] function).
    fn resolve_root(&self) -> Result<PathBuf, Error> {
        resolve_root(self.lib_root(), self.dest())
    }

    /// [`resolve_root`](Self::resolve_root), then ensure the root's
    /// `.satyrographos/` bookkeeping skeleton exists — the prelude every
    /// materialising operation (`install`, registry install, reconcile) runs
    /// before staging.
    fn resolve_managed_root(&self) -> Result<PathBuf, Error> {
        let root = self.resolve_root()?;
        ensure_managed(&root)?;
        Ok(root)
    }
}

/// The `<root>/.satyrographos/` bookkeeping directory.
pub fn managed_dir(root: &Path) -> PathBuf {
    root.join(MANAGED_DIR)
}

/// Whether `root` is already managed by this tool (its `.satyrographos/`
/// marker exists, even if empty) — plan §6's managed-root check.
pub fn is_managed(root: &Path) -> bool {
    managed_dir(root).is_dir()
}

/// Ensure `root` is a managed root, creating the `.satyrographos/`,
/// `.satyrographos/receipts/`, and `.satyrographos/tmp/` skeleton on first
/// use (plan §6: "`install`/`uninstall` create it on first use"). Idempotent.
pub fn ensure_managed(root: &Path) -> Result<(), Error> {
    for sub in [managed_dir(root), receipts_dir(root), tmp_dir(root)] {
        std::fs::create_dir_all(&sub).map_err(|e| Error::io(&sub, e))?;
    }
    Ok(())
}

/// `<root>/.satyrographos/receipts/`.
pub fn receipts_dir(root: &Path) -> PathBuf {
    managed_dir(root).join("receipts")
}

/// `<root>/.satyrographos/tmp/` — staging and archive extraction, kept under
/// `<root>` so the final rename into `dist/` is same-filesystem/atomic
/// (plan §6).
pub fn tmp_dir(root: &Path) -> PathBuf {
    managed_dir(root).join("tmp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmp(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "rustyfi-roots-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).expect("temp dir");
        p
    }

    #[test]
    fn finds_a_development_tree_from_a_nested_directory() {
        let root = tmp("dev");
        fs::create_dir_all(root.join(DEV_DIR).join("dist/packages")).unwrap();
        let deep = root.join("doc/chapters");
        fs::create_dir_all(&deep).unwrap();
        assert_eq!(discover(&deep), Some(root.join(DEV_DIR)));
    }

    #[test]
    fn finds_a_project_local_root_beside_its_satyristes() {
        let root = tmp("local");
        fs::write(root.join(crate::satyristes::SATYRISTES_NAME), "(version 0.0.2)").unwrap();
        fs::create_dir_all(root.join(LOCAL_DIR)).unwrap();
        assert_eq!(discover(&root), Some(root.join(LOCAL_DIR)));
    }

    #[test]
    fn a_local_dir_without_a_satyristes_is_not_a_root() {
        // Otherwise a stray `.rustyfi/` high up the filesystem would capture
        // every document beneath it.
        let root = tmp("local-unmarked");
        fs::create_dir_all(root.join(LOCAL_DIR)).unwrap();
        assert_ne!(discover(&root), Some(root.join(LOCAL_DIR)));
    }

    #[test]
    fn the_nearest_directory_wins_over_a_higher_one() {
        // A project-local root in the directory you are standing in beats a
        // development tree further up.
        let outer = tmp("nearest");
        fs::create_dir_all(outer.join(DEV_DIR)).unwrap();
        let inner = outer.join("vendor/thing");
        fs::create_dir_all(&inner).unwrap();
        fs::write(inner.join(crate::satyristes::SATYRISTES_NAME), "(version 0.0.2)").unwrap();
        fs::create_dir_all(inner.join(LOCAL_DIR)).unwrap();
        assert_eq!(discover(&inner), Some(inner.join(LOCAL_DIR)));
    }

    #[test]
    fn a_development_tree_beats_a_local_root_in_the_same_directory() {
        let root = tmp("both");
        fs::create_dir_all(root.join(DEV_DIR)).unwrap();
        fs::write(root.join(crate::satyristes::SATYRISTES_NAME), "(version 0.0.2)").unwrap();
        fs::create_dir_all(root.join(LOCAL_DIR)).unwrap();
        assert_eq!(discover(&root), Some(root.join(DEV_DIR)));
    }

    #[test]
    fn a_project_local_root_layers_over_the_wider_one() {
        // The point of a search PATH: `.rustyfi/` carries what the project
        // installed for itself, and everything else still resolves from the
        // tree above it.
        let outer = tmp("layered");
        fs::create_dir_all(outer.join(DEV_DIR).join("dist/packages")).unwrap();
        let proj = outer.join("proj");
        fs::create_dir_all(proj.join(LOCAL_DIR).join("dist/packages")).unwrap();
        fs::write(proj.join(crate::satyristes::SATYRISTES_NAME), "(version 0.0.2)").unwrap();
        assert_eq!(
            discover_all(&proj),
            vec![proj.join(LOCAL_DIR), outer.join(DEV_DIR)]
        );
    }

    #[test]
    fn a_relative_start_still_walks_upward() {
        // `parent()` of a bare `doc.saty` is `""`, and `absolute("")` is an
        // error, so an unguarded walk sees only the working directory.
        let root = tmp("relative");
        fs::create_dir_all(root.join(DEV_DIR)).unwrap();
        let deep = root.join("a/b");
        fs::create_dir_all(&deep).unwrap();
        let here = std::env::current_dir().unwrap();
        std::env::set_current_dir(&deep).unwrap();
        let found = discover_all(Path::new(""));
        std::env::set_current_dir(here).unwrap();
        assert_eq!(found.len(), 1, "expected the tree above, got {found:?}");
        assert!(found[0].ends_with(DEV_DIR));
    }

    #[test]
    fn a_path_through_dotdot_does_not_re_enter_the_directory_it_left() {
        // `absolute()` only prefixes the working directory, so walking
        // `root/../other` upward would pass through `root` itself and find its
        // tree — a root the path was pointing AWAY from.
        let base = tmp("dotdot");
        let root = base.join("root");
        let other = base.join("other/doc");
        fs::create_dir_all(root.join(DEV_DIR)).unwrap();
        fs::create_dir_all(&other).unwrap();
        let here = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root).unwrap();
        let found = discover_all(Path::new("../other/./doc"));
        std::env::set_current_dir(here).unwrap();
        assert!(
            found.is_empty(),
            "should not see the tree it walked away from: {found:?}"
        );
    }

    #[test]
    fn prefix_roots_are_ordered_user_then_system() {
        let roots = prefix_roots();
        let user = roots.iter().position(|p| !p.starts_with("/usr"));
        let system = roots.iter().position(|p| p.starts_with("/usr"));
        if let (Some(u), Some(s)) = (user, system) {
            assert!(u < s, "the user prefix must be searched before the system one");
        }
        if !cfg!(windows) {
            assert!(roots.iter().any(|p| p == Path::new("/usr/local/lib/rustyfi")));
            assert!(roots.iter().any(|p| p == Path::new("/usr/lib/rustyfi")));
        }
    }
}

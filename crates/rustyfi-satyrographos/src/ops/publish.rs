//! `publish`: turn the project you are standing in into a package definition
//! inside a package repository — what `opam publish` does for an OCaml
//! package.
//!
//! The input is the project's own `Satyristes` (`(library (name …) (version …)
//! (lang …) (dependencies …))`), the output is one entry in a repository this
//! crate can install FROM. Two shapes exist and both are written:
//!
//! - **OPAM** — `packages/<id>/<id>.<version>/opam`, with the released tarball
//!   in the file's own `url { src: … checksum: … }` block. This is
//!   Satyrographos' own repository shape, so what is written here is
//!   installable by real Satyrographos as well as by this port
//!   (`registry::opam_index`).
//! - **TOML** — `packages/<id>.toml`, this port's native index
//!   (see [`crate::registry`]'s module doc). Emitted through the very struct
//!   the installer deserialises, so the two cannot drift.
//!
//! ## What it deliberately does NOT do
//!
//! - **Build or upload a tarball.** `--url` names an already-released archive,
//!   exactly as `opam publish` does; hosting it is the author's business. The
//!   digest is either given (`--sha256`) or computed from a local copy of that
//!   same archive (`--archive`).
//! - **Push, or open a pull request.** Both need credentials and a network.
//!   The write (and, with `--commit`, the commit) happen locally and the push
//!   command is PRINTED — a step the author runs, sees, and can undo.
//!
//! ## Why a published entry is read back before `publish` returns
//!
//! The one failure worth guarding is a definition this crate's own installer
//! cannot parse: it would look like a successful publish and fail only for
//! whoever tried to install it. So the last step re-reads the written entry
//! through [`crate::registry`]'s ordinary lookup path and checks the version,
//! the tarball URL and the digest survived. `Error::RegistryIndex` here means
//! the emitter and the reader disagree, and nothing was published that anyone
//! could use.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::manifest::Lang;
use crate::registry::{self, RegistryOptions};
use crate::satyristes::{self, LibraryMeta};
use crate::source::RegistryConfig;
use crate::util;

/// Which layout a package repository stores its definitions in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoShape {
    /// `packages/<id>/<id>.<version>/opam` — Satyrographos' own, and what
    /// upstream OPAM consumers can install from.
    Opam,
    /// `packages/<id>.toml` — this port's native index.
    Toml,
}

impl RepoShape {
    /// As the `--shape` flag spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            RepoShape::Opam => "opam",
            RepoShape::Toml => "toml",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "opam" => Some(RepoShape::Opam),
            "toml" => Some(RepoShape::Toml),
            _ => None,
        }
    }
}

/// What to publish, where, and how much of the git side to do.
#[derive(Debug, Default, Clone)]
pub struct PublishOptions {
    /// Directory to find the `Satyristes` from (searched upward). `None` is
    /// the current directory.
    pub project: Option<PathBuf>,
    /// Which `(library …)` block, when the manifest declares several.
    pub library: Option<String>,
    /// The released tarball's URL — recorded verbatim, never fetched.
    pub url: String,
    /// The tarball's SHA-256, as hex.
    pub sha256: Option<String>,
    /// A local copy of that same tarball, hashed to supply (or cross-check)
    /// [`Self::sha256`].
    pub archive: Option<PathBuf>,
    /// Force a repository shape instead of detecting it.
    pub shape: Option<RepoShape>,
    /// Publish under this id instead of the derived one.
    pub package_name: Option<String>,
    /// `synopsis:` (OPAM) / `description` (TOML).
    pub description: Option<String>,
    /// `maintainer:` — OPAM only; the TOML index has no such field.
    pub maintainer: Option<String>,
    /// Replace an already-published `<name>.<version>`.
    pub force: bool,
    /// `git add` + `git commit` the written file in the repository checkout.
    pub commit: bool,
    /// Commit on this branch, creating it if it does not exist. Read only
    /// alongside [`Self::commit`]: with no commit there is nothing to put on a
    /// branch, and the printed next steps stay the plain add/commit/push
    /// sequence.
    pub branch: Option<String>,
    /// Compose and check everything, write nothing.
    pub dry_run: bool,
}

/// What a publish did (or, under `--dry-run`, would have done).
#[derive(Debug, Clone)]
pub struct PublishReport {
    /// The `(library (name …))` this came from.
    pub library: String,
    /// The id it was published under — an OPAM repository's `satysfi-`-prefixed
    /// package id, or the TOML index's bare stem.
    pub package: String,
    /// The name `install` accepts for it, which is not always
    /// [`Self::package`] (see [`installable_name`]).
    pub installable: String,
    pub version: String,
    pub shape: RepoShape,
    /// The repository checkout written into.
    pub repository: PathBuf,
    /// The definition's path within [`Self::repository`] — what `git add`
    /// takes.
    pub relative: String,
    pub url: String,
    pub sha256: String,
    /// Exactly the bytes written, so `--dry-run` can show them.
    pub contents: String,
    /// The branch the commit landed on, when `--commit` was given.
    pub committed: Option<String>,
    /// The commands the author runs next, in order. Printed, never run.
    pub next_steps: Vec<String>,
    pub dry_run: bool,
}

/// Publish the project's library into the chosen repository.
///
/// `repos` is the configured repository list (a project's own `(registry …)`,
/// else the user's `config.toml`), consulted only when nothing more explicit
/// chose — see [`select_repository`].
pub fn publish(
    opts: &PublishOptions,
    reg_opts: &RegistryOptions,
    repos: &[RegistryConfig],
) -> Result<PublishReport, Error> {
    let manifest = find_manifest(opts.project.as_deref())?;
    let lib = select_library(&manifest, opts.library.as_deref())?;
    let sha256 = resolve_digest(opts)?;
    if opts.url.trim().is_empty() {
        return Err(Error::PublishInput {
            message: "no source url: `--url` names the released tarball, which is what \
                      an installing consumer downloads"
                .to_string(),
        });
    }
    check_opam_scalar("url", &opts.url)?;

    let repo = checkout(&select_repository(reg_opts, repos)?, reg_opts)?;
    let shape = detect_shape(&repo, opts.shape)?;
    let package = package_id(&lib, shape, opts.package_name.as_deref());
    let installable = installable_name(&package, &lib.name);

    let relative = definition_path(shape, &package, &lib.version);
    let path = repo.join(&relative);
    if !opts.force && already_published(&repo, shape, &package, &lib.version)? {
        return Err(Error::AlreadyPublished {
            name: package,
            version: lib.version,
            path,
        });
    }
    let contents = match shape {
        RepoShape::Opam => opam_text(&lib, opts, &opts.url, &sha256)?,
        RepoShape::Toml => toml_text(&path, &lib, opts, &opts.url, &sha256)?,
    };

    if !opts.dry_run {
        write_definition(&path, &contents)?;
        // The guard this whole operation exists to provide: a definition the
        // installer cannot read back is not a publish, it is a trap set for
        // whoever installs next.
        read_back(&repo, &installable, &lib.version, &opts.url, &sha256)?;
    }

    let committed = if opts.commit && !opts.dry_run {
        Some(git_commit(
            &repo,
            &relative,
            opts.branch.as_deref(),
            &format!("{package}: publish {}", lib.version),
        )?)
    } else {
        None
    };

    Ok(PublishReport {
        next_steps: next_steps(
            &repo,
            &relative,
            &package,
            &lib.version,
            committed.as_deref(),
        ),
        library: lib.name,
        package,
        installable,
        version: lib.version,
        shape,
        repository: repo,
        relative,
        url: opts.url.clone(),
        sha256,
        contents,
        committed,
        dry_run: opts.dry_run,
    })
}

// ---------------------------------------------------------------------------
// Input: the project's own Satyristes.
// ---------------------------------------------------------------------------

fn find_manifest(project: Option<&Path>) -> Result<PathBuf, Error> {
    let from = match project {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().map_err(|e| Error::io(Path::new("."), e))?,
    };
    // A `--project` pointing straight AT the file is accepted too: refusing it
    // would be a riddle, since that is the path this returns.
    if from.is_file() {
        return Ok(from);
    }
    satyristes::find_upward(&from).ok_or(Error::ProjectNotFound { from })
}

fn select_library(manifest: &Path, wanted: Option<&str>) -> Result<LibraryMeta, Error> {
    let libs = satyristes::read_libraries(manifest)?;
    let names = || {
        libs.iter()
            .map(|l| l.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    };
    if let Some(name) = wanted {
        return libs
            .iter()
            .find(|l| l.name == name)
            .cloned()
            .ok_or_else(|| Error::LibraryFilter { declared: names() });
    }
    match libs.len() {
        0 => Err(Error::Satyristes {
            path: manifest.to_path_buf(),
            message: "no `(library ...)` block found: nothing to publish".to_string(),
        }),
        1 => Ok(libs.into_iter().next().expect("length checked")),
        _ => Err(Error::AmbiguousLibrary { names: names() }),
    }
}

/// The tarball's SHA-256, from `--sha256`, from hashing `--archive`, or from
/// both (in which case they must agree — publishing a digest that does not
/// match the archive on hand is a mistake worth catching here rather than at
/// every consumer's install).
fn resolve_digest(opts: &PublishOptions) -> Result<String, Error> {
    let hashed = match &opts.archive {
        Some(path) => {
            if !path.is_file() {
                return Err(Error::PublishInput {
                    message: format!(
                        "--archive `{}` is not a regular file; it must be the released \
                         tarball itself, since what a consumer verifies is that file's \
                         own sha256",
                        path.display()
                    ),
                });
            }
            Some(util::sha256_file(path)?)
        }
        None => None,
    };
    let declared = match &opts.sha256 {
        Some(hex) => Some(normalize_sha256(hex)?),
        None => None,
    };
    match (declared, hashed) {
        (Some(d), Some(h)) if d != h => Err(Error::ChecksumMismatch {
            expected: d,
            actual: h,
        }),
        (Some(d), _) => Ok(d),
        (None, Some(h)) => Ok(h),
        (None, None) => Err(Error::PublishInput {
            message: "no source digest: pass `--sha256 HEX`, or `--archive PATH` \
                      naming a local copy of the tarball `--url` points at"
                .to_string(),
        }),
    }
}

/// Lowercased, and rejected unless it is 64 hex digits: a malformed digest
/// publishes a package nothing can ever verify.
fn normalize_sha256(hex: &str) -> Result<String, Error> {
    let hex = hex.trim().to_lowercase();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::PublishInput {
            message: format!("`{hex}` is not a sha256 (expected 64 hex digits)"),
        });
    }
    Ok(hex)
}

// ---------------------------------------------------------------------------
// Which repository, and getting a checkout of it.
// ---------------------------------------------------------------------------

/// The repository to publish into, ranked exactly as
/// `RegistryOptions::resolve_url` ranks a registry everywhere else: the
/// `--registry` flag, then `$RUSTYFI_REGISTRY`, then the project's `Satyristes`
/// `(registry …)`, then the user's config.
///
/// The one difference is what happens at the bottom rung with SEVERAL
/// configured. `search`/`install` consult them all in order, so "the first"
/// costs nothing there; a release goes into exactly one, and picking silently
/// would put it in a repository the author never named. So that case is
/// [`Error::AmbiguousRegistry`], which lists them and says how to choose.
pub fn select_repository(
    reg_opts: &RegistryOptions,
    repos: &[RegistryConfig],
) -> Result<String, Error> {
    if reg_opts.has_explicit_url() {
        return reg_opts.resolve_url(None);
    }
    let urls: Vec<&str> = repos.iter().filter_map(|r| r.url.as_deref()).collect();
    match urls.as_slice() {
        // `resolve_url(None)` rather than a bare `Err(NoRegistry)`, so the
        // "nothing configured anywhere" message stays the one word-for-word.
        [] => reg_opts.resolve_url(None),
        [one] => reg_opts.resolve_url(Some(one)),
        many => Err(Error::AmbiguousRegistry {
            urls: many.join(", "),
        }),
    }
}

/// A writable local checkout of `url`: a local directory is used in place, and
/// anything else is cloned through the same git-source cache a `{ git = … }`
/// dependency uses. `--offline` therefore refuses a clone it does not already
/// have, rather than reaching the network.
fn checkout(url: &str, reg_opts: &RegistryOptions) -> Result<PathBuf, Error> {
    if let Some(local) = registry::local_path_from_url(url) {
        if local.is_dir() {
            return Ok(local);
        }
        return Err(Error::PublishInput {
            message: format!(
                "repository `{}` is not a directory (a local repository must be an \
                 existing checkout; a remote one is given as a git URL)",
                local.display()
            ),
        });
    }
    Ok(registry::acquire_git_source(url, None, reg_opts)?.root)
}

/// Which shape `repo` stores definitions in, by looking at what it already
/// holds: a `packages/*.toml` file means the TOML index, a
/// `packages/<name>/<name>.<v>/opam` directory means the OPAM one.
///
/// A repository holding both, or holding nothing to judge by, is NOT guessed
/// at — publishing into the wrong half of a repository puts the entry
/// somewhere no consumer looks. `--shape` settles it, and (only then) a
/// missing `packages/` is created, which is how a fresh repository gets its
/// first package.
fn detect_shape(repo: &Path, forced: Option<RepoShape>) -> Result<RepoShape, Error> {
    let packages = repo.join("packages");
    if let Some(shape) = forced {
        std::fs::create_dir_all(&packages).map_err(|e| Error::io(&packages, e))?;
        return Ok(shape);
    }
    if !packages.is_dir() {
        return Err(Error::RepositoryShape {
            path: repo.to_path_buf(),
            message: "it has no `packages/` directory, so there is nothing to judge by; \
                      pass `--shape opam` or `--shape toml`"
                .to_string(),
        });
    }
    let mut has_toml = false;
    let mut has_opam = false;
    for entry in util::read_dir_paths(&packages)? {
        if entry.extension().and_then(|e| e.to_str()) == Some("toml") {
            has_toml = true;
        } else if entry.is_dir() && registry::opam_index::is_package_dir(&entry) {
            has_opam = true;
        }
    }
    match (has_opam, has_toml) {
        (true, false) => Ok(RepoShape::Opam),
        (false, true) => Ok(RepoShape::Toml),
        (true, true) => Err(Error::RepositoryShape {
            path: repo.to_path_buf(),
            message: "`packages/` holds BOTH opam package directories and \
                      `<name>.toml` index files; pass `--shape opam` or `--shape toml`"
                .to_string(),
        }),
        (false, false) => Err(Error::RepositoryShape {
            path: repo.to_path_buf(),
            message: "`packages/` is empty (or holds neither shape); pass \
                      `--shape opam` or `--shape toml`"
                .to_string(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Where a definition lives, and whether it is already there.
// ---------------------------------------------------------------------------

/// The definition's path relative to the repository root — also what `git add`
/// takes.
fn definition_path(shape: RepoShape, package: &str, version: &str) -> String {
    match shape {
        RepoShape::Opam => format!("packages/{package}/{package}.{version}/opam"),
        // One file per package, holding every version — hence the merge in
        // [`toml_text`].
        RepoShape::Toml => format!("packages/{package}.toml"),
    }
}

/// Whether this exact `<package>.<version>` is already published. Asked for
/// both shapes in one place because the RULE is one rule — that version is
/// what a consumer pins — even though the two store it differently: a whole
/// directory for OPAM, one table in a shared file for the TOML index.
fn already_published(
    repo: &Path,
    shape: RepoShape,
    package: &str,
    version: &str,
) -> Result<bool, Error> {
    let path = repo.join(definition_path(shape, package, version));
    match shape {
        RepoShape::Opam => Ok(path.is_file()),
        RepoShape::Toml => match path.is_file() {
            true => Ok(read_index(&path)?.versions.contains_key(version)),
            false => Ok(false),
        },
    }
}

// ---------------------------------------------------------------------------
// Naming.
// ---------------------------------------------------------------------------

/// The id to publish under.
///
/// For the TOML index that is the library name itself — `registry::lookup`
/// reads `packages/<name>.toml` by that name directly.
///
/// For an OPAM repository, Satyrographos publishes library `xpath` as opam
/// package `satysfi-xpath`, and `registry::lookup` resolves exactly those two
/// spellings (bare, then `satysfi-`-prefixed). The block's own `(opam "…")`
/// file supplies the id when it agrees with one of them; a THIRD spelling is
/// deliberately not adopted from it, because the entry would then be
/// unreachable from the library name a `@require:` or a dependency entry
/// writes. `--package-name` overrides all of this, for a repository with its
/// own conventions.
fn package_id(lib: &LibraryMeta, shape: RepoShape, forced: Option<&str>) -> String {
    if let Some(id) = forced {
        return id.to_string();
    }
    if shape == RepoShape::Toml {
        return lib.name.clone();
    }
    let prefixed = format!("satysfi-{}", lib.name);
    match lib.opam.as_deref().and_then(|f| f.strip_suffix(".opam")) {
        Some(stem) if stem == lib.name || stem == prefixed => stem.to_string(),
        _ if lib.name.starts_with("satysfi-") => lib.name.clone(),
        _ => prefixed,
    }
}

/// The name `install NAME` accepts for a published id — the inverse of
/// `registry::lookup`'s "try `<name>`, else `satysfi-<name>`" resolution, which
/// is also what `search` reports (`ops::search::installable_name`). An id that
/// is neither spelling of the library name is only reachable as itself.
pub fn installable_name(package: &str, library: &str) -> String {
    if package == library || package == format!("satysfi-{library}") {
        library.to_string()
    } else {
        package.to_string()
    }
}

// ---------------------------------------------------------------------------
// The OPAM definition.
// ---------------------------------------------------------------------------

/// Refuse a value this crate's own opam reader would mis-read.
///
/// [`crate::opam`] and `registry::opam_index` scan for the next `"` rather than
/// honouring escapes, and `opam_index`'s `url { … }` block ends at the first
/// `}`, so a quote, a backslash or a brace inside a value silently truncates
/// it. A newline breaks the line-oriented field lookup the same way. Refusing
/// is the only option that cannot publish a definition that reads back wrong.
fn check_opam_scalar(field: &str, value: &str) -> Result<(), Error> {
    if let Some(bad) = value
        .chars()
        .find(|c| matches!(c, '"' | '\\' | '{' | '}' | '\n' | '\r'))
    {
        return Err(Error::PublishInput {
            message: format!(
                "{field} contains `{}`, which this port's opam reader cannot round-trip \
                 (it scans for the next quote and ends a `url {{ … }}` block at the first \
                 brace, neither honouring escapes)",
                bad.escape_debug()
            ),
        });
    }
    Ok(())
}

fn quoted(value: &str) -> String {
    format!("\"{value}\"")
}

/// What `(lang …)` says about the typesetter, and nothing more: the block
/// declares a language GENERATION, so the dependency states that generation's
/// range rather than a specific release nobody wrote down.
fn satysfi_constraint(lang: Lang) -> &'static str {
    match lang {
        Lang::V0_0 => "< \"0.1.0\"",
        Lang::V0_1 => ">= \"0.1.0\"",
    }
}

/// A `Satyristes` dependency's opam package id. The whole registry names
/// SATySFi libraries this way, and `registry::lookup` strips the prefix back
/// off on the way in.
fn opam_dep_id(name: &str) -> String {
    if name.starts_with("satysfi-") {
        name.to_string()
    } else {
        format!("satysfi-{name}")
    }
}

/// `build:`/`install:` hand the work to Satyrographos itself, reading the
/// package's own `Satyristes` — upstream's convention, and what makes an entry
/// written here installable by real Satyrographos and not only by this port.
/// (`ops::prepare` recognises these lines as a delegation back to itself and
/// records them rather than running them.)
fn satyrographos_stanza(field: &str, library: &str) -> String {
    format!(
        "{field}: [\n  [\"satyrographos\" \"opam\" \"{verb}\"\n   \"--name\" {name}\n   \
         \"--prefix\" \"%{{prefix}}%\"\n   \"--script\" \"%{{build}}%/Satyristes\"]\n]\n",
        verb = field,
        name = quoted(library),
    )
}

/// The `opam` file's text. `url { … }` goes LAST: `opam_index`'s reader takes
/// the first `}` after `url {` as the block's end, and the `%{prefix}%` in the
/// stanzas above carries one.
fn opam_text(
    lib: &LibraryMeta,
    opts: &PublishOptions,
    url: &str,
    sha256: &str,
) -> Result<String, Error> {
    let mut out = String::from("opam-version: \"2.0\"\n");
    if let Some(text) = &opts.description {
        check_opam_scalar("--description", text)?;
        out.push_str(&format!("synopsis: {}\n", quoted(text)));
    }
    if let Some(text) = &opts.maintainer {
        check_opam_scalar("--maintainer", text)?;
        out.push_str(&format!("maintainer: {}\n", quoted(text)));
    }

    out.push_str("depends: [\n");
    out.push_str(&format!(
        "  \"satysfi\" {{{}}}\n",
        satysfi_constraint(lib.lang)
    ));
    out.push_str("  \"satyrographos\"\n");
    for dep in lib.dependencies.keys() {
        check_opam_scalar("a dependency name", dep)?;
        out.push_str(&format!("  {}\n", quoted(&opam_dep_id(dep))));
    }
    out.push_str("]\n");

    out.push_str(&satyrographos_stanza("build", &lib.name));
    out.push_str(&satyrographos_stanza("install", &lib.name));

    out.push_str(&format!(
        "url {{\n  src: {}\n  checksum: [\n    \"sha256={sha256}\"\n  ]\n}}\n",
        quoted(url)
    ));
    Ok(out)
}

// ---------------------------------------------------------------------------
// The TOML index entry.
// ---------------------------------------------------------------------------

/// An existing `packages/<id>.toml`, parsed exactly as the installer parses
/// it.
fn read_index(path: &Path) -> Result<registry::PackageIndex, Error> {
    let text = util::read_to_string(path)?;
    toml::from_str(&text).map_err(|source| Error::RegistryIndex {
        message: format!("{}: {source}", path.display()),
    })
}

/// The updated `packages/<id>.toml`, MERGED into whatever is already there: a
/// package's other released versions are what its existing consumers pin, so
/// publishing 1.1.0 must not retract 1.0.0.
fn toml_text(
    path: &Path,
    lib: &LibraryMeta,
    opts: &PublishOptions,
    url: &str,
    sha256: &str,
) -> Result<String, Error> {
    let mut index = match path.is_file() {
        true => read_index(path)?,
        false => registry::PackageIndex::default(),
    };
    if opts.description.is_some() {
        index.description.clone_from(&opts.description);
    }
    index.versions.insert(
        lib.version.clone(),
        registry::VersionEntry {
            tarball_url: url.to_string(),
            sha256: sha256.to_string(),
            sha512: None,
            // Keyed by LIBRARY name with this crate's constraint syntax (the
            // solver's vocabulary), not by opam id — the opam shape's
            // `depends:` is the one that speaks opam.
            dependencies: lib
                .dependencies
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<BTreeMap<_, _>>(),
        },
    );
    Ok(toml::to_string_pretty(&index).expect("the index struct serialises to TOML"))
}

// ---------------------------------------------------------------------------
// Writing, verifying, committing.
// ---------------------------------------------------------------------------

fn write_definition(path: &Path, contents: &str) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    // Atomic for the same reason every other write in this crate is: a
    // repository is a live tree, and a half-written definition is one a
    // concurrent reader would parse as malformed.
    util::write_atomic(path, contents.as_bytes())
}

/// Re-read what was just written through the ordinary installer path, so a
/// definition the reader cannot parse fails HERE rather than at every
/// consumer.
fn read_back(
    repo: &Path,
    installable: &str,
    version: &str,
    url: &str,
    sha256: &str,
) -> Result<(), Error> {
    let repo_url = repo.display().to_string();
    let reg = registry::acquire(&repo_url, &RegistryOptions::default())?;
    let index = registry::lookup(&reg, installable)?;
    let (found, entry) = registry::select_version(&index, installable, Some(version))?;
    let mismatch = |what: &str, want: &str, got: &str| Error::RegistryIndex {
        message: format!(
            "the entry just written to {} does not read back: {what} is `{got}`, expected \
             `{want}` (the publisher and the index reader disagree)",
            repo.display()
        ),
    };
    if found != version {
        return Err(mismatch("the version", version, &found));
    }
    if entry.tarball_url != url {
        return Err(mismatch("the tarball url", url, &entry.tarball_url));
    }
    if !entry.sha256.eq_ignore_ascii_case(sha256) {
        return Err(mismatch("the sha256", sha256, &entry.sha256));
    }
    Ok(())
}

/// `git add` + `git commit` the one written path, on `branch` when named
/// (created if it does not exist). Returns the branch actually committed on.
fn git_commit(
    repo: &Path,
    relative: &str,
    branch: Option<&str>,
    message: &str,
) -> Result<String, Error> {
    let dir = repo.to_string_lossy().into_owned();
    if let Some(branch) = branch {
        // `--quiet` so a missing ref is an exit code rather than a stderr line
        // reported as a git failure.
        let exists =
            registry::git_capture(&["-C", &dir, "rev-parse", "--verify", "--quiet", branch])
                .is_ok();
        let args: &[&str] = if exists {
            &["-C", &dir, "checkout", branch]
        } else {
            &["-C", &dir, "checkout", "-b", branch]
        };
        registry::run_git(args)?;
    }
    registry::run_git(&["-C", &dir, "add", "--", relative])?;
    // Pathspec-limited, so an unrelated staged change in the checkout (a
    // git-source cache directory is reused across publishes) is not swept into
    // this commit.
    registry::run_git(&["-C", &dir, "commit", "-m", message, "--", relative])?;
    registry::git_capture(&["-C", &dir, "rev-parse", "--abbrev-ref", "HEAD"])
}

/// The commands the author runs next. Printed rather than run: each one needs
/// credentials, a network, or both, and a publish that pushed by itself would
/// be one the author never got to look at.
fn next_steps(
    repo: &Path,
    relative: &str,
    package: &str,
    version: &str,
    committed: Option<&str>,
) -> Vec<String> {
    let dir = repo.display();
    match committed {
        Some(branch) => vec![
            format!("git -C {dir} push origin {branch}"),
            format!(
                "then open a pull request for `{package}.{version}` \
                 (`publish` does not open one: it needs credentials and a network)"
            ),
        ],
        None => vec![
            format!("git -C {dir} add -- {relative}"),
            format!("git -C {dir} commit -m '{package}: publish {version}'"),
            format!("git -C {dir} push origin HEAD"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib(name: &str, opam: Option<&str>) -> LibraryMeta {
        LibraryMeta {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            lang: Lang::V0_0,
            opam: opam.map(str::to_string),
            dependencies: BTreeMap::new(),
        }
    }

    #[test]
    fn opam_id_defaults_to_the_satysfi_prefix() {
        assert_eq!(
            package_id(&lib("xpath", None), RepoShape::Opam, None),
            "satysfi-xpath"
        );
        // Already prefixed: not doubled.
        assert_eq!(
            package_id(&lib("satysfi-xpath", None), RepoShape::Opam, None),
            "satysfi-xpath"
        );
        // The TOML index is keyed by the library name itself.
        assert_eq!(
            package_id(&lib("xpath", None), RepoShape::Toml, None),
            "xpath"
        );
    }

    #[test]
    fn an_opam_stem_is_adopted_only_when_it_stays_reachable() {
        // `registry::lookup` resolves `<name>` and `satysfi-<name>`; the
        // manifest's own file name is honoured when it is one of those.
        assert_eq!(
            package_id(
                &lib("xpath", Some("satysfi-xpath.opam")),
                RepoShape::Opam,
                None
            ),
            "satysfi-xpath"
        );
        assert_eq!(
            package_id(&lib("xpath", Some("xpath.opam")), RepoShape::Opam, None),
            "xpath"
        );
        // A third spelling would publish an entry `install xpath` cannot find.
        assert_eq!(
            package_id(
                &lib("xpath", Some("rustyfi-xpath.opam")),
                RepoShape::Opam,
                None
            ),
            "satysfi-xpath"
        );
        // Unless the author says so outright.
        assert_eq!(
            package_id(&lib("xpath", None), RepoShape::Opam, Some("rustyfi-xpath")),
            "rustyfi-xpath"
        );
    }

    #[test]
    fn installable_name_inverts_the_prefix_convention() {
        assert_eq!(installable_name("satysfi-xpath", "xpath"), "xpath");
        assert_eq!(installable_name("xpath", "xpath"), "xpath");
        assert_eq!(installable_name("rustyfi-xpath", "xpath"), "rustyfi-xpath");
    }

    #[test]
    fn a_malformed_digest_is_refused() {
        assert!(normalize_sha256("abc").is_err());
        assert!(normalize_sha256(&"z".repeat(64)).is_err());
        assert_eq!(normalize_sha256(&"A".repeat(64)).unwrap(), "a".repeat(64));
    }

    /// The `url { … }` block and the `depends:`/`build:` fields are read by
    /// two DIFFERENT scanners — `registry::opam_index` for the source,
    /// [`crate::opam`] for what `prepare` needs — and only the first is
    /// covered by the publish round-trip test. This pins the second.
    #[test]
    fn the_emitted_opam_parses_with_this_crates_own_opam_reader() {
        let mut meta = lib("great-package", None);
        meta.dependencies
            .insert("base".to_string(), "*".to_string());
        let text = opam_text(
            &meta,
            &PublishOptions::default(),
            "https://x.invalid/p.tar.gz",
            "ab",
        )
        .expect("compose");

        let dir =
            std::env::temp_dir().join(format!("rustyfi-publish-opamparse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("great-package.opam");
        std::fs::write(&path, &text).unwrap();
        let parsed = crate::opam::read(&path).expect("read");

        assert_eq!(
            parsed
                .depends
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>(),
            ["satysfi", "satyrographos", "satysfi-base"]
        );
        assert_eq!(parsed.depends[0].constraint.as_deref(), Some("< \"0.1.0\""));
        // `%{prefix}%`'s braces must not be mistaken for a filter clause, and
        // `install:` must not be read as `build:`.
        assert_eq!(
            parsed.build,
            vec![vec![
                "satyrographos",
                "opam",
                "build",
                "--name",
                "great-package",
                "--prefix",
                "%{prefix}%",
                "--script",
                "%{build}%/Satyristes",
            ]]
        );
        // No `extra-source`, so `prepare` has nothing to fetch or run itself.
        assert!(parsed.extra_sources.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_value_the_opam_reader_cannot_round_trip_is_refused() {
        // Each of these silently truncates a field in `opam::field_string` or
        // `opam_index::source_of` rather than failing.
        for bad in ["a\"b", "a\\b", "a}b", "a{b", "a\nb"] {
            assert!(
                check_opam_scalar("url", bad).is_err(),
                "{bad:?} should be refused"
            );
        }
        assert!(check_opam_scalar("url", "https://example.invalid/x-1.0.0.tar.gz").is_ok());
    }
}

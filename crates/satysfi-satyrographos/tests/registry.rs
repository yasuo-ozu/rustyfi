//! Phase-3 registry tests (plan §5.4, §9 phase 3) — **no network**. Every
//! fixture is built in a unique temp directory: a package `.tar.gz` served via
//! a `file://` URL, and a registry index in two forms —
//!
//! - a **plain-directory index** (`<dir>/packages/<name>.toml`, read in place);
//! - a **bare git index** (`git init` a work tree, `git clone --bare` it, and
//!   point `--registry` at `file://…/bare.git` so `acquire` shells out to
//!   `git clone` into a hermetic cache dir).
//!
//! Coverage:
//! - happy path (both index forms): registry install materialises files under
//!   `dist/` and records a `registry` receipt with the resolved version/sha256;
//! - **sha256 mismatch**: a corrupted index digest aborts the install and
//!   leaves `dist/` and the receipts directory completely untouched (step 3);
//! - reconcile of a `{ registry = … }` Satyrfile source locks the resolved
//!   `(version, url, sha256)` and re-materialises reproducibly *without*
//!   re-consulting the index (the index dir is deleted before the 2nd run);
//! - `search` substring matching and `update` upgrade reporting (§8).
//!
//! The default `cargo test` run exercises all of this with the `http` cargo
//! feature **off** (no HTTP client compiled in) — `file://` tarballs keep the
//! whole suite offline, matching plan §8.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use satysfi_satyrographos::{self as sg, InstallOptions, RegistryOptions, RootOptions};

// ---------------------------------------------------------------------------
// Temp-dir fixture helper (same pattern as tests/ops.rs / tests/reconcile.rs).
// ---------------------------------------------------------------------------

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "satyrographos-registry-{tag}-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            n
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, rel: &str, content: &str) -> PathBuf {
        let p = self.0.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&p, content).expect("write fixture file");
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------
// Fixture builders.
// ---------------------------------------------------------------------------

/// Build a `.tar.gz` of a manifest package `<name>` at `<tmp>/tarballs/
/// <name>-<version>.tar.gz`, returning `(path, lowercase-hex-sha256)`.
fn make_tarball(tmp: &TempDir, name: &str, version: &str, body: &str) -> (PathBuf, String) {
    // The package source tree.
    let src = tmp.path().join(format!("src/{name}-{version}"));
    fs::create_dir_all(src.join("packages")).unwrap();
    fs::write(
        src.join("satysfi-package.toml"),
        format!(
            "[package]\n\
             name = \"{name}\"\n\
             version = \"{version}\"\n\
             satysfi-version-compat = \">=0.0.6, <0.1\"\n\
             \n\
             [[files]]\n\
             kind = \"package-dir\"\n\
             src = \"packages\"\n"
        ),
    )
    .unwrap();
    fs::write(src.join(format!("packages/{name}.satyh")), body).unwrap();

    let tarball = tmp.path().join(format!("tarballs/{name}-{version}.tar.gz"));
    fs::create_dir_all(tarball.parent().unwrap()).unwrap();
    // `-C <src> .` → entries rooted at the package (manifest at the archive root).
    let status = Command::new("tar")
        .args([
            "-czf",
            tarball.to_str().unwrap(),
            "-C",
            src.to_str().unwrap(),
            ".",
        ])
        .status()
        .expect("run tar");
    assert!(status.success(), "tar failed");

    let sha = sha256_of(&tarball);
    (tarball, sha)
}

fn sha256_of(path: &Path) -> String {
    let out = Command::new("sha256sum")
        .arg(path)
        .output()
        .expect("run sha256sum");
    assert!(out.status.success(), "sha256sum failed");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .expect("sha256sum output")
        .to_string()
}

fn file_url(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// One `[versions."<v>"]` block; `sha` overrides the real digest (for the
/// mismatch test) when `Some`.
struct Ver<'a> {
    version: &'a str,
    tarball: &'a Path,
    real_sha: &'a str,
    bad_sha: Option<&'a str>,
}

/// Write a plain-directory index at `<tmp>/<index_rel>/packages/<name>.toml`,
/// returning the index root path.
fn write_index(
    tmp: &TempDir,
    index_rel: &str,
    name: &str,
    description: Option<&str>,
    versions: &[Ver],
) -> PathBuf {
    let mut toml = String::new();
    if let Some(d) = description {
        toml.push_str(&format!("description = \"{d}\"\n\n"));
    }
    for v in versions {
        let sha = v.bad_sha.unwrap_or(v.real_sha);
        toml.push_str(&format!(
            "[versions.\"{}\"]\n\
             tarball_url = \"{}\"\n\
             sha256 = \"{}\"\n\n",
            v.version,
            file_url(v.tarball),
            sha,
        ));
    }
    tmp.write(&format!("{index_rel}/packages/{name}.toml"), &toml);
    tmp.path().join(index_rel)
}

fn install_opts(root: &Path) -> InstallOptions {
    InstallOptions {
        dest: Some(root.to_path_buf()),
        ..Default::default()
    }
}

fn root_opts(root: &Path) -> RootOptions {
    RootOptions {
        dest: Some(root.to_path_buf()),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Happy path — plain-directory index.
// ---------------------------------------------------------------------------

#[test]
fn plain_directory_registry_install_happy_path() {
    let tmp = TempDir::new("plain-happy");
    let (tarball, sha) = make_tarball(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let index = write_index(
        &tmp,
        "index",
        "great-package",
        Some("A great SATySFi package"),
        &[Ver {
            version: "1.0.0",
            tarball: &tarball,
            real_sha: &sha,
            bad_sha: None,
        }],
    );
    let root = tmp.path().join("root");
    let reg = RegistryOptions {
        url: Some(file_url(&index)),
        ..Default::default()
    };

    let (report, resolved) =
        sg::install_registry("great-package", None, &install_opts(&root), &reg, None)
            .expect("registry install ok");

    assert_eq!(report.name, "great-package");
    assert_eq!(resolved.version, "1.0.0");
    assert_eq!(resolved.sha256, sha);

    // Files materialised under dist/.
    assert!(root
        .join("dist/packages/great-package/great-package.satyh")
        .is_file());

    // The receipt records a `registry` source with version/url/sha256.
    let receipt = fs::read_to_string(root.join(".satyrographos/receipts/great-package.toml"))
        .expect("receipt written");
    assert!(receipt.contains("kind = \"registry\""), "{receipt}");
    assert!(receipt.contains("value = \"great-package\""), "{receipt}");
    assert!(receipt.contains("version = \"1.0.0\""), "{receipt}");
    assert!(receipt.contains(&sha), "receipt records the sha256:\n{receipt}");
}

// ---------------------------------------------------------------------------
// sha256 mismatch — nothing touched (plan §5.4 step 3).
// ---------------------------------------------------------------------------

#[test]
fn sha256_mismatch_leaves_dist_and_receipts_untouched() {
    let tmp = TempDir::new("mismatch");
    let (tarball, sha) = make_tarball(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let bad = "0000000000000000000000000000000000000000000000000000000000000000";
    assert_ne!(sha, bad);
    let index = write_index(
        &tmp,
        "index",
        "great-package",
        None,
        &[Ver {
            version: "1.0.0",
            tarball: &tarball,
            real_sha: &sha,
            bad_sha: Some(bad),
        }],
    );
    let root = tmp.path().join("root");
    let reg = RegistryOptions {
        url: Some(file_url(&index)),
        ..Default::default()
    };

    let err = sg::install_registry("great-package", None, &install_opts(&root), &reg, None)
        .expect_err("checksum mismatch must fail");
    assert!(
        matches!(err, sg::Error::ChecksumMismatch { .. }),
        "expected ChecksumMismatch, got {err}"
    );

    // Nothing under dist/, and the receipts directory has no receipt.
    assert!(!root.join("dist").exists(), "dist/ must not be created on mismatch");
    let receipts = root.join(".satyrographos/receipts");
    let count = fs::read_dir(&receipts)
        .map(|rd| rd.filter_map(|e| e.ok()).count())
        .unwrap_or(0);
    assert_eq!(count, 0, "no receipt may be written on mismatch");

    // And the downloaded (unverified) tarball was cleaned out of tmp/.
    let tmp_dir = root.join(".satyrographos/tmp");
    let leftovers = fs::read_dir(&tmp_dir)
        .map(|rd| rd.filter_map(|e| e.ok()).count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0, "unverified download must be cleaned up");
}

// ---------------------------------------------------------------------------
// Happy path — bare git index (shells out to git).
// ---------------------------------------------------------------------------

fn git(args: &[&str], cwd: Option<&Path>) {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output().expect("run git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn bare_git_registry_install_happy_path() {
    let tmp = TempDir::new("git-happy");
    let (tarball, sha) = make_tarball(&tmp, "great-package", "1.0.0", "let great = 1\n");

    // A work tree that IS the index (holds packages/), committed to git.
    let work = write_index(
        &tmp,
        "work",
        "great-package",
        Some("A great SATySFi package"),
        &[Ver {
            version: "1.0.0",
            tarball: &tarball,
            real_sha: &sha,
            bad_sha: None,
        }],
    );
    git(&["init", "-q", work.to_str().unwrap()], None);
    git(&["-C", work.to_str().unwrap(), "add", "-A"], None);
    git(
        &[
            "-C",
            work.to_str().unwrap(),
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "index",
        ],
        None,
    );

    // A bare clone: `file://<bare.git>` has no `packages/` on disk, so `acquire`
    // takes the git-clone path.
    let bare = tmp.path().join("registry.git");
    git(
        &[
            "clone",
            "-q",
            "--bare",
            work.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
        None,
    );

    let root = tmp.path().join("root");
    let reg = RegistryOptions {
        url: Some(file_url(&bare)),
        cache_dir: Some(tmp.path().join("cache")),
        ..Default::default()
    };

    let (report, resolved) =
        sg::install_registry("great-package", Some("1.0.0"), &install_opts(&root), &reg, None)
            .expect("git registry install ok");
    assert_eq!(report.name, "great-package");
    assert_eq!(resolved.version, "1.0.0");
    assert!(root
        .join("dist/packages/great-package/great-package.satyh")
        .is_file());
}

// ---------------------------------------------------------------------------
// Reconcile a registry Satyrfile source → lockfile pins (version, url, sha256)
// and is reproducible without re-consulting the index.
// ---------------------------------------------------------------------------

#[test]
fn reconcile_registry_locks_and_is_reproducible_without_index() {
    let tmp = TempDir::new("reconcile");
    let (tarball, sha) = make_tarball(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let index = write_index(
        &tmp,
        "index",
        "great-package",
        Some("A great SATySFi package"),
        &[Ver {
            version: "1.0.0",
            tarball: &tarball,
            real_sha: &sha,
            bad_sha: None,
        }],
    );

    // A Satyrfile whose only dependency is a registry source, with the index
    // URL declared in a [registry] section (the fallback when no flag/env set).
    let manifest = tmp.write(
        "proj/Satyrfile.toml",
        &format!(
            "[registry]\nurl = \"{}\"\n\n\
             [[library]]\nname = \"great-package\"\n\
             source = {{ registry = \"great-package\", version = \"1.0.0\" }}\n",
            file_url(&index)
        ),
    );
    let root = tmp.path().join("root");

    let report =
        sg::install_manifest_reg(&manifest, &root_opts(&root), &RegistryOptions::default())
            .expect("first reconcile ok");
    assert_eq!(report.installed.len(), 1);
    assert!(root
        .join("dist/packages/great-package/great-package.satyh")
        .is_file());

    // The lockfile pins the resolved version, url, and sha256.
    let lock = fs::read_to_string(tmp.path().join("proj/Satyrfile.lock")).unwrap();
    assert!(lock.contains("version = \"1.0.0\""), "{lock}");
    assert!(lock.contains(&file_url(&tarball)), "lock pins the url:\n{lock}");
    assert!(lock.contains(&sha), "lock pins the sha256:\n{lock}");

    // Delete the index entirely: a second reconcile must NOT re-consult it.
    fs::remove_dir_all(&index).unwrap();
    let report =
        sg::install_manifest_reg(&manifest, &root_opts(&root), &RegistryOptions::default())
            .expect("second reconcile reproducible without index");
    assert_eq!(report.skipped, ["great-package"], "unchanged entry skipped");
    assert!(report.installed.is_empty());
}

// ---------------------------------------------------------------------------
// search
// ---------------------------------------------------------------------------

#[test]
fn search_matches_name_and_description() {
    let tmp = TempDir::new("search");
    let (gp, gp_sha) = make_tarball(&tmp, "great-package", "1.0.0", "let g = 1\n");
    let (ft, ft_sha) = make_tarball(&tmp, "fonts-theano", "0.2.0", "let f = 1\n");
    write_index(
        &tmp,
        "index",
        "great-package",
        Some("A great SATySFi package"),
        &[Ver { version: "1.0.0", tarball: &gp, real_sha: &gp_sha, bad_sha: None }],
    );
    let index = write_index(
        &tmp,
        "index",
        "fonts-theano",
        Some("Theano didot fonts"),
        &[Ver { version: "0.2.0", tarball: &ft, real_sha: &ft_sha, bad_sha: None }],
    );
    let reg = RegistryOptions {
        url: Some(file_url(&index)),
        ..Default::default()
    };

    // Match on name.
    let hits = sg::search("great", &reg, None).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "great-package");
    assert_eq!(hits[0].version, "1.0.0");
    assert_eq!(hits[0].description.as_deref(), Some("A great SATySFi package"));

    // Match on description ("didot" only appears in the font entry's text).
    let hits = sg::search("didot", &reg, None).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "fonts-theano");

    // Empty term lists everything, sorted by name.
    let hits = sg::search("", &reg, None).unwrap();
    let names: Vec<_> = hits.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, ["fonts-theano", "great-package"]);
}

// ---------------------------------------------------------------------------
// update
// ---------------------------------------------------------------------------

#[test]
fn update_reports_available_upgrade() {
    let tmp = TempDir::new("update");
    let (t10, sha10) = make_tarball(&tmp, "great-package", "1.0.0", "let g = 1\n");
    let (t11, sha11) = make_tarball(&tmp, "great-package", "1.1.0", "let g = 2\n");

    // Index initially offers 1.0.0 only.
    let index = write_index(
        &tmp,
        "index",
        "great-package",
        Some("A great SATySFi package"),
        &[Ver { version: "1.0.0", tarball: &t10, real_sha: &sha10, bad_sha: None }],
    );
    let manifest = tmp.write(
        "proj/Satyrfile.toml",
        &format!(
            "[registry]\nurl = \"{}\"\n\n\
             [[library]]\nname = \"great-package\"\n\
             source = {{ registry = \"great-package\", version = \"1.0.0\" }}\n",
            file_url(&index)
        ),
    );
    let root = tmp.path().join("root");
    sg::install_manifest_reg(&manifest, &root_opts(&root), &RegistryOptions::default())
        .expect("reconcile at 1.0.0");

    let reg = RegistryOptions {
        url: Some(file_url(&index)),
        ..Default::default()
    };

    // No newer version yet.
    let rep = sg::update(&manifest, &reg).expect("update ok");
    assert!(rep.upgrades.is_empty(), "no upgrade before 1.1.0 is published");
    assert_eq!(rep.up_to_date, ["great-package"]);

    // Publish 1.1.0 into the index.
    write_index(
        &tmp,
        "index",
        "great-package",
        Some("A great SATySFi package"),
        &[
            Ver { version: "1.0.0", tarball: &t10, real_sha: &sha10, bad_sha: None },
            Ver { version: "1.1.0", tarball: &t11, real_sha: &sha11, bad_sha: None },
        ],
    );

    let rep = sg::update(&manifest, &reg).expect("update ok");
    assert_eq!(rep.upgrades.len(), 1, "1.1.0 upgrade reported");
    assert_eq!(rep.upgrades[0].name, "great-package");
    assert_eq!(rep.upgrades[0].current, "1.0.0");
    assert_eq!(rep.upgrades[0].latest, "1.1.0");
}

// ---------------------------------------------------------------------------
// Error paths.
// ---------------------------------------------------------------------------

#[test]
fn no_registry_configured_errors() {
    let tmp = TempDir::new("no-reg");
    let root = tmp.path().join("root");
    // Ensure no ambient $SATYSFI_REGISTRY leaks in from the environment.
    std::env::remove_var("SATYSFI_REGISTRY");
    let err = sg::install_registry(
        "great-package",
        None,
        &install_opts(&root),
        &RegistryOptions::default(),
        None,
    )
    .expect_err("no registry → error");
    assert!(matches!(err, sg::Error::NoRegistry), "{err}");
}

#[test]
fn missing_version_errors() {
    let tmp = TempDir::new("missing-ver");
    let (tarball, sha) = make_tarball(&tmp, "great-package", "1.0.0", "let g = 1\n");
    let index = write_index(
        &tmp,
        "index",
        "great-package",
        None,
        &[Ver { version: "1.0.0", tarball: &tarball, real_sha: &sha, bad_sha: None }],
    );
    let root = tmp.path().join("root");
    let reg = RegistryOptions {
        url: Some(file_url(&index)),
        ..Default::default()
    };
    let err = sg::install_registry("great-package", Some("9.9.9"), &install_opts(&root), &reg, None)
        .expect_err("missing version → error");
    assert!(matches!(err, sg::Error::VersionNotFound { .. }), "{err}");
}

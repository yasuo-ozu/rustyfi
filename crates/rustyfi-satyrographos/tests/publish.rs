//! `publish`: writing a package definition that this crate's own installer can
//! read back.
//!
//! The load-bearing tests here are the two round trips — publish, then resolve
//! the very entry just written through `ops::registry_install::resolve`, the
//! same path `install NAME` takes. A definition its own reader cannot parse
//! would otherwise look like a successful publish and fail only for whoever
//! tried to install it.
//!
//! Everything is offline: `--url` is recorded verbatim and never fetched, and
//! every repository is a plain local directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_satyrographos as sg;

/// A syntactically valid sha256 that no test ever has to compute.
const SHA: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const URL: &str = "https://example.invalid/great-package-1.0.0.tar.gz";

fn tmp(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "rustyfi-publish-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).expect("temp dir");
    p
}

/// A project whose `Satyristes` is the upstream README's own shape.
fn project(dir: &Path, version: &str) -> PathBuf {
    let root = dir.join("project");
    fs::create_dir_all(root.join("packages")).unwrap();
    fs::write(
        root.join("Satyristes"),
        format!(
            "(version 0.0.2)\n\
             (library\n  \
               (name \"great-package\")\n  \
               (version \"{version}\")\n  \
               (lang 0.0)\n  \
               (sources ((packageDir \"packages\")))\n  \
               (opam \"satysfi-great-package.opam\")\n  \
               (dependencies ((base ()) (fonts-theano ()))))\n"
        ),
    )
    .unwrap();
    root
}

/// A repository already holding one OPAM package, so the shape is detectable.
fn opam_repo(dir: &Path) -> PathBuf {
    let repo = dir.join("repo");
    let existing = repo.join("packages/satysfi-other/satysfi-other.0.1.0");
    fs::create_dir_all(&existing).unwrap();
    fs::write(
        existing.join("opam"),
        "opam-version: \"2.0\"\nurl {\n  src: \"https://example.invalid/other-0.1.0.tar.gz\"\n  \
         checksum: [ \"sha512=aaaa\" ]\n}\n",
    )
    .unwrap();
    repo
}

/// The same, for this port's native TOML index.
fn toml_repo(dir: &Path) -> PathBuf {
    let repo = dir.join("repo");
    fs::create_dir_all(repo.join("packages")).unwrap();
    fs::write(
        repo.join("packages/other.toml"),
        format!("[versions.\"0.1.0\"]\ntarball_url = \"https://example.invalid/other.tar.gz\"\nsha256 = \"{SHA}\"\n"),
    )
    .unwrap();
    repo
}

fn opts(project: &Path) -> sg::PublishOptions {
    sg::PublishOptions {
        project: Some(project.to_path_buf()),
        url: URL.to_string(),
        sha256: Some(SHA.to_string()),
        ..Default::default()
    }
}

fn reg(repo: &Path) -> sg::RegistryOptions {
    sg::RegistryOptions {
        url: Some(repo.display().to_string()),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// The round trips.
// ---------------------------------------------------------------------------

#[test]
fn an_opam_publish_resolves_back_through_the_installer() {
    let dir = tmp("opam-roundtrip");
    let proj = project(&dir, "1.0.0");
    let repo = opam_repo(&dir);

    let report = sg::publish(&opts(&proj), &reg(&repo), &[]).expect("publish");
    assert_eq!(report.shape, sg::RepoShape::Opam);
    // Satyrographos publishes library `xpath` as opam package `satysfi-xpath`;
    // the id follows that, and the name a consumer types does not.
    assert_eq!(report.package, "satysfi-great-package");
    assert_eq!(report.installable, "great-package");
    assert_eq!(
        report.relative,
        "packages/satysfi-great-package/satysfi-great-package.1.0.0/opam"
    );
    assert!(repo.join(&report.relative).is_file());

    // The whole point: `install great-package` finds exactly what was written.
    let resolved = sg::ops::registry_install::resolve("great-package", None, &reg(&repo), None)
        .expect("the published entry should resolve by the library name");
    assert_eq!(resolved.version, "1.0.0");
    assert_eq!(resolved.url, URL);
    assert_eq!(resolved.sha256, SHA);
}

#[test]
fn a_toml_publish_resolves_back_through_the_installer() {
    let dir = tmp("toml-roundtrip");
    let proj = project(&dir, "1.0.0");
    let repo = toml_repo(&dir);

    let report = sg::publish(&opts(&proj), &reg(&repo), &[]).expect("publish");
    assert_eq!(report.shape, sg::RepoShape::Toml);
    // The native index is keyed by the library name itself — no opam prefix.
    assert_eq!(report.package, "great-package");
    assert_eq!(report.relative, "packages/great-package.toml");

    let resolved = sg::ops::registry_install::resolve("great-package", None, &reg(&repo), None)
        .expect("the published entry should resolve");
    assert_eq!(resolved.version, "1.0.0");
    assert_eq!(resolved.url, URL);
    assert_eq!(resolved.sha256, SHA);
}

#[test]
fn the_opam_definition_carries_the_manifest_dependencies_and_generation() {
    let dir = tmp("opam-fields");
    let proj = project(&dir, "1.0.0");
    let repo = opam_repo(&dir);
    let report = sg::publish(&opts(&proj), &reg(&repo), &[]).expect("publish");
    let text = fs::read_to_string(repo.join(&report.relative)).unwrap();

    // `(dependencies ((base ()) …))` becomes opam ids, the spelling the whole
    // registry uses and the one `registry::lookup` strips back off.
    assert!(text.contains("\"satysfi-base\""), "{text}");
    assert!(text.contains("\"satysfi-fonts-theano\""), "{text}");
    // `(lang 0.0)` and nothing more: the generation's range, not a release.
    assert!(text.contains("\"satysfi\" {< \"0.1.0\"}"), "{text}");
    // Installable by real Satyrographos, not only by this port.
    assert!(
        text.contains("[\"satyrographos\" \"opam\" \"build\""),
        "{text}"
    );
    assert!(
        text.contains("[\"satyrographos\" \"opam\" \"install\""),
        "{text}"
    );
}

#[test]
fn the_toml_entry_carries_the_dependencies_the_solver_reads() {
    let dir = tmp("toml-deps");
    let proj = project(&dir, "1.0.0");
    let repo = toml_repo(&dir);
    let report = sg::publish(&opts(&proj), &reg(&repo), &[]).expect("publish");
    let text = fs::read_to_string(repo.join(&report.relative)).unwrap();
    // Keyed by LIBRARY name in this crate's constraint syntax — the solver's
    // vocabulary, not opam's.
    assert!(text.contains("[versions.\"1.0.0\".dependencies]"), "{text}");
    assert!(text.contains("base = \"*\""), "{text}");
    assert!(text.contains("fonts-theano = \"*\""), "{text}");
}

#[test]
fn a_second_version_does_not_retract_the_first() {
    // A package's other released versions are what its existing consumers
    // pin; the TOML shape rewrites one file, so this is where that could go
    // wrong.
    let dir = tmp("merge");
    let repo = toml_repo(&dir);

    let first = project(&dir, "1.0.0");
    sg::publish(&opts(&first), &reg(&repo), &[]).expect("publish 1.0.0");
    let second = project(&dir, "1.1.0");
    sg::publish(&opts(&second), &reg(&repo), &[]).expect("publish 1.1.0");

    for want in ["1.0.0", "1.1.0"] {
        let resolved =
            sg::ops::registry_install::resolve("great-package", Some(want), &reg(&repo), None)
                .unwrap_or_else(|e| panic!("{want} should still resolve: {e}"));
        assert_eq!(resolved.version, want);
    }
    // The neighbouring package is untouched either way.
    assert!(repo.join("packages/other.toml").is_file());
}

// ---------------------------------------------------------------------------
// Refusals.
// ---------------------------------------------------------------------------

#[test]
fn republishing_a_version_needs_force() {
    for toml_shape in [false, true] {
        let dir = tmp("force");
        let proj = project(&dir, "1.0.0");
        let repo = if toml_shape {
            toml_repo(&dir)
        } else {
            opam_repo(&dir)
        };

        sg::publish(&opts(&proj), &reg(&repo), &[]).expect("first publish");
        let err = sg::publish(&opts(&proj), &reg(&repo), &[])
            .expect_err("a published version is what a consumer pins");
        assert!(
            matches!(err, sg::Error::AlreadyPublished { .. }),
            "toml_shape={toml_shape}: {err}"
        );

        let forced = sg::PublishOptions {
            force: true,
            url: "https://example.invalid/replaced.tar.gz".to_string(),
            ..opts(&proj)
        };
        let report = sg::publish(&forced, &reg(&repo), &[]).expect("--force replaces");
        assert_eq!(report.url, "https://example.invalid/replaced.tar.gz");
        let resolved = sg::ops::registry_install::resolve("great-package", None, &reg(&repo), None)
            .expect("resolve after force");
        assert_eq!(resolved.url, "https://example.invalid/replaced.tar.gz");
    }
}

#[test]
fn an_undecidable_repository_shape_is_refused_rather_than_guessed() {
    // Empty: nothing to judge by.
    let dir = tmp("shape-empty");
    let proj = project(&dir, "1.0.0");
    let repo = dir.join("repo");
    fs::create_dir_all(repo.join("packages")).unwrap();
    let err = sg::publish(&opts(&proj), &reg(&repo), &[]).expect_err("no shape to detect");
    assert!(matches!(err, sg::Error::RepositoryShape { .. }), "{err}");
    assert!(err.to_string().contains("--shape"), "{err}");

    // `--shape` settles it, and then a first package can land.
    let forced = sg::PublishOptions {
        shape: Some(sg::RepoShape::Opam),
        ..opts(&proj)
    };
    let report = sg::publish(&forced, &reg(&repo), &[]).expect("--shape opam");
    assert_eq!(report.shape, sg::RepoShape::Opam);

    // Both shapes at once: also refused, for the same reason.
    let dir = tmp("shape-both");
    let proj = project(&dir, "1.0.0");
    let repo = opam_repo(&dir);
    fs::write(
        repo.join("packages/other.toml"),
        format!("[versions.\"0.1.0\"]\ntarball_url = \"https://example.invalid/o.tar.gz\"\nsha256 = \"{SHA}\"\n"),
    )
    .unwrap();
    let err = sg::publish(&opts(&proj), &reg(&repo), &[]).expect_err("ambiguous shape");
    assert!(matches!(err, sg::Error::RepositoryShape { .. }), "{err}");
}

#[test]
fn several_configured_repositories_refuse_to_choose() {
    // `search`/`install` consult every repository in order, so "the first"
    // costs nothing there; a release goes into exactly one.
    assert!(
        std::env::var_os("RUSTYFI_REGISTRY").is_none(),
        "this test asserts the fallback list is consulted, which $RUSTYFI_REGISTRY \
         would pre-empt (it outranks the list, by design)"
    );
    let dir = tmp("many-repos");
    let proj = project(&dir, "1.0.0");
    let repo = opam_repo(&dir);
    let other = opam_repo(&tmp("many-repos-2"));

    let configured = |paths: &[&Path]| -> Vec<sg::RegistryConfig> {
        paths
            .iter()
            .map(|p| sg::RegistryConfig {
                url: Some(p.display().to_string()),
                ..Default::default()
            })
            .collect()
    };
    let no_choice = sg::RegistryOptions::default();

    let err = sg::publish(&opts(&proj), &no_choice, &configured(&[&repo, &other]))
        .expect_err("two repositories, no choice");
    match &err {
        sg::Error::AmbiguousRegistry { urls } => {
            assert!(urls.contains(&repo.display().to_string()), "{urls}");
            assert!(urls.contains(&other.display().to_string()), "{urls}");
        }
        other => panic!("expected AmbiguousRegistry, got {other}"),
    }
    assert!(err.to_string().contains("--registry"), "{err}");

    // Exactly one configured is not a choice to make.
    let report = sg::publish(&opts(&proj), &no_choice, &configured(&[&repo])).expect("one repo");
    assert_eq!(report.repository, repo);

    // And an explicit `--registry` settles a list of any length.
    let dir2 = tmp("many-repos-explicit");
    let proj2 = project(&dir2, "1.0.0");
    let report = sg::publish(&opts(&proj2), &reg(&other), &configured(&[&repo, &other]))
        .expect("--registry outranks the configured list");
    assert_eq!(report.repository, other);
}

#[test]
fn a_project_without_a_satyristes_is_refused_by_name() {
    let dir = tmp("no-manifest");
    let empty = dir.join("nowhere/deeper");
    fs::create_dir_all(&empty).unwrap();
    let repo = opam_repo(&dir);
    let err = sg::publish(
        &sg::PublishOptions {
            project: Some(empty.clone()),
            ..opts(&empty)
        },
        &reg(&repo),
        &[],
    )
    .expect_err("nothing to publish");
    // `tmp()` roots are under the system temp dir, which has no Satyristes
    // above it either, so the upward walk genuinely finds nothing.
    assert!(matches!(err, sg::Error::ProjectNotFound { .. }), "{err}");
    assert!(err.to_string().contains("Satyristes"), "{err}");
}

// ---------------------------------------------------------------------------
// The digest.
// ---------------------------------------------------------------------------

#[test]
fn a_local_archive_supplies_the_digest_and_cross_checks_a_declared_one() {
    let dir = tmp("digest");
    let proj = project(&dir, "1.0.0");
    let repo = opam_repo(&dir);
    let archive = dir.join("great-package-1.0.0.tar.gz");
    fs::write(&archive, b"not really a tarball, but it has a sha256").unwrap();

    // Hashed from the file: no --sha256 needed.
    let from_file = sg::PublishOptions {
        sha256: None,
        archive: Some(archive.clone()),
        ..opts(&proj)
    };
    let report = sg::publish(&from_file, &reg(&repo), &[]).expect("publish");
    let hashed = report.sha256.clone();
    assert_eq!(hashed.len(), 64, "{hashed}");
    let resolved = sg::ops::registry_install::resolve("great-package", None, &reg(&repo), None)
        .expect("resolve");
    assert_eq!(resolved.sha256, hashed);

    // Declared AND hashed, disagreeing: publishing a digest that does not
    // match the archive on hand is caught here, not at every consumer.
    let contradiction = sg::PublishOptions {
        sha256: Some(SHA.to_string()),
        archive: Some(archive),
        force: true,
        ..opts(&proj)
    };
    let err = sg::publish(&contradiction, &reg(&repo), &[]).expect_err("digests disagree");
    assert!(matches!(err, sg::Error::ChecksumMismatch { .. }), "{err}");
}

#[test]
fn a_publish_with_no_digest_at_all_is_refused() {
    let dir = tmp("no-digest");
    let proj = project(&dir, "1.0.0");
    let repo = opam_repo(&dir);
    let err = sg::publish(
        &sg::PublishOptions {
            sha256: None,
            ..opts(&proj)
        },
        &reg(&repo),
        &[],
    )
    .expect_err("no digest");
    assert!(matches!(err, sg::Error::PublishInput { .. }), "{err}");
    assert!(err.to_string().contains("--sha256"), "{err}");
}

// ---------------------------------------------------------------------------
// The git side: local only.
// ---------------------------------------------------------------------------

fn git(repo: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn commit_lands_on_the_named_branch_and_stops_short_of_pushing() {
    let dir = tmp("commit");
    let proj = project(&dir, "1.0.0");
    let repo = opam_repo(&dir);
    git(&repo, &["init", "-q", "."]);
    // A real repository has an identity; the fixture needs one for the same
    // reason.
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "seed"]);

    let report = sg::publish(
        &sg::PublishOptions {
            commit: true,
            branch: Some("publish-great-package".to_string()),
            ..opts(&proj)
        },
        &reg(&repo),
        &[],
    )
    .expect("publish --commit");

    assert_eq!(report.committed.as_deref(), Some("publish-great-package"));
    assert_eq!(
        git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "publish-great-package"
    );
    // The definition is IN the commit, not merely on disk.
    let tracked = git(&repo, &["show", "--name-only", "--format=", "HEAD"]);
    assert_eq!(tracked.trim(), report.relative, "{tracked}");
    // Nothing was pushed: the repository has no remote at all, so a push would
    // have failed loudly rather than succeeded quietly.
    assert_eq!(git(&repo, &["remote"]), "");
    assert!(
        report.next_steps[0].contains("push origin publish-great-package"),
        "{:?}",
        report.next_steps
    );
}

#[test]
fn a_dry_run_writes_nothing_but_shows_the_definition() {
    let dir = tmp("dry-run");
    let proj = project(&dir, "1.0.0");
    let repo = opam_repo(&dir);
    let report = sg::publish(
        &sg::PublishOptions {
            dry_run: true,
            ..opts(&proj)
        },
        &reg(&repo),
        &[],
    )
    .expect("dry run");
    assert!(report.dry_run);
    assert!(report.contents.contains(URL), "{}", report.contents);
    assert!(
        !repo.join(&report.relative).exists(),
        "--dry-run must not write {}",
        report.relative
    );
    assert!(
        sg::ops::registry_install::resolve("great-package", None, &reg(&repo), None).is_err(),
        "--dry-run must not publish anything resolvable"
    );
}

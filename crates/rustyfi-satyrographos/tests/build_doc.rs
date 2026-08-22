//! `satyrographos build` — running a `(libraryDoc ...)`'s own build commands.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_satyrographos as sg;

fn tmp(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "rustyfi-builddoc-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).expect("temp dir");
    p
}

/// A manifest whose "typesetter" is `cp`, so the build is observable without
/// depending on the compiler: it copies `in.txt` to the product.
fn project(tag: &str, docs: &str) -> PathBuf {
    let dir = tmp(tag);
    fs::write(dir.join("in.txt"), "hello").unwrap();
    fs::write(
        dir.join("Satyristes"),
        format!("(version 0.0.2)\n(library (name \"l\") (version \"1\") (sources ((packageDir \".\"))))\n{docs}"),
    )
    .unwrap();
    dir
}

#[test]
fn runs_the_build_commands_and_reports_the_product() {
    let dir = project(
        "one",
        r#"(libraryDoc (name "d") (version "1")
             (build ((cp "in.txt" "out.txt")))
             (sources ((doc "installed.txt" "out.txt"))))"#,
    );
    let reports = sg::build(&dir, &sg::BuildOptions::default()).expect("build");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].name, "d");
    assert_eq!(reports[0].commands, vec![vec!["cp", "in.txt", "out.txt"]]);
    assert_eq!(reports[0].products, vec![("out.txt".to_string(), true)]);
    assert!(dir.join("out.txt").is_file(), "the command actually ran");
}

#[test]
fn install_true_materialises_the_product_under_dist_doc() {
    // With `BuildOptions::install` set, the same product `products` already
    // confirmed exists is ALSO staged into the resolved root, at
    // `dist/doc/<name>/<dst>` — the same per-library-namespace convention
    // `md` uses.
    let dir = project(
        "install",
        r#"(libraryDoc (name "d") (version "1")
             (build ((cp "in.txt" "out.txt")))
             (sources ((doc "installed.txt" "out.txt"))))"#,
    );
    let root = dir.join("root");
    let opts = sg::BuildOptions {
        install: Some(sg::RootOptions { dest: Some(root.clone()), ..Default::default() }),
        ..Default::default()
    };
    let reports = sg::build(&dir, &opts).expect("build+install");
    assert_eq!(reports[0].installed, vec![PathBuf::from("dist/doc/d")]);
    let installed = root.join("dist/doc/d/installed.txt");
    assert!(installed.is_file(), "product was not installed");
    assert_eq!(fs::read_to_string(installed).unwrap(), "hello");
}

#[test]
fn rebuilding_with_install_replaces_the_previous_output() {
    // There is no `--force` for `build`: a doc target is a build artifact, so
    // rebuilding it and reinstalling over the previous run must just work,
    // not fail with "already installed".
    let dir = project(
        "reinstall",
        r#"(libraryDoc (name "d") (version "1")
             (build ((cp "in.txt" "out.txt")))
             (sources ((doc "installed.txt" "out.txt"))))"#,
    );
    let root = dir.join("root");
    let opts = sg::BuildOptions {
        install: Some(sg::RootOptions { dest: Some(root.clone()), ..Default::default() }),
        ..Default::default()
    };
    sg::build(&dir, &opts).expect("first build");
    sg::build(&dir, &opts).expect("second build replaces the first");
    assert!(root.join("dist/doc/d/installed.txt").is_file());
}

#[test]
fn no_install_option_installs_nothing() {
    let dir = project(
        "noinstall",
        r#"(libraryDoc (name "d") (version "1")
             (build ((cp "in.txt" "out.txt")))
             (sources ((doc "installed.txt" "out.txt"))))"#,
    );
    let reports = sg::build(&dir, &sg::BuildOptions::default()).expect("build");
    assert!(reports[0].installed.is_empty());
}

#[test]
fn a_failing_command_stops_the_build_and_says_which() {
    let dir = project(
        "fail",
        r#"(libraryDoc (name "d") (version "1")
             (build ((cp "missing.txt" "out.txt") (cp "in.txt" "second.txt"))))"#,
    );
    let err = sg::build(&dir, &sg::BuildOptions::default()).expect_err("should fail");
    assert!(matches!(err, sg::Error::DocBuild { .. }), "got {err:?}");
    assert!(err.to_string().contains("cp missing.txt out.txt"));
    assert!(
        !dir.join("second.txt").exists(),
        "a later command must not run after a failure"
    );
}

#[test]
fn several_targets_need_a_name() {
    let dir = project(
        "many",
        r#"(libraryDoc (name "a") (version "1") (build ((cp "in.txt" "a.txt"))))
           (libraryDoc (name "b") (version "1") (build ((cp "in.txt" "b.txt"))))"#,
    );
    let err = sg::build(&dir, &sg::BuildOptions::default()).expect_err("ambiguous");
    assert!(matches!(err, sg::Error::AmbiguousDoc { .. }), "got {err:?}");

    let opts = sg::BuildOptions {
        docs: vec!["b".to_string()],
        ..Default::default()
    };
    sg::build(&dir, &opts).expect("build b");
    assert!(dir.join("b.txt").is_file());
    assert!(!dir.join("a.txt").exists(), "only the named target runs");
}

#[test]
fn a_manifest_with_no_doc_target_says_so() {
    let dir = project("none", "");
    let err = sg::build(&dir, &sg::BuildOptions::default()).expect_err("nothing to build");
    assert!(matches!(err, sg::Error::NoDocTarget), "got {err:?}");
}

#[test]
fn the_typesetter_option_replaces_the_named_program() {
    // `(build ((satysfi ...)))` — a manifest written for upstream — runs the
    // binary the caller chose, not whatever `satysfi` is on PATH.
    let dir = project(
        "typesetter",
        r#"(libraryDoc (name "d") (version "1") (build ((satysfi "in.txt" "out.txt"))))"#,
    );
    let opts = sg::BuildOptions {
        typesetter: Some(PathBuf::from("cp")),
        ..Default::default()
    };
    sg::build(&dir, &opts).expect("build");
    assert!(dir.join("out.txt").is_file());
}

#[test]
fn working_directory_is_where_the_commands_run() {
    // Upstream keeps a doc's sources in `doc/` and builds there, so the
    // command names `in.txt` while the product is declared `doc/out.txt`.
    let dir = tmp("workdir");
    fs::create_dir_all(dir.join("doc")).unwrap();
    fs::write(dir.join("doc/in.txt"), "hello").unwrap();
    fs::write(
        dir.join("Satyristes"),
        r#"(version 0.0.2)
(library (name "l") (version "1") (sources ((packageDir "doc"))))
(libraryDoc (name "d") (version "1")
  (workingDirectory "doc")
  (build ((cp "in.txt" "out.txt")))
  (sources ((doc "installed.pdf" "doc/out.txt")))
  (opam "unmodelled.opam"))"#,
    )
    .unwrap();
    let reports = sg::build(&dir, &sg::BuildOptions::default()).expect("build");
    assert!(dir.join("doc/out.txt").is_file(), "ran inside doc/");
    assert!(!dir.join("out.txt").exists(), "not in the manifest's dir");
    assert_eq!(reports[0].products, vec![("doc/out.txt".to_string(), true)]);
}

#[test]
fn a_doc_only_manifest_builds_but_installs_nothing() {
    // No `(library ...)` at all: `build` is happy, and `install` says where the
    // targets it found are handled rather than only that libraries are absent.
    let dir = tmp("doconly");
    fs::write(dir.join("in.txt"), "hello").unwrap();
    fs::write(
        dir.join("Satyristes"),
        r#"(version 0.0.2)
(libraryDoc (name "docs-only") (version "1") (build ((cp "in.txt" "out.txt"))))"#,
    )
    .unwrap();

    sg::build(&dir, &sg::BuildOptions::default()).expect("a doc-only manifest builds");
    assert!(dir.join("out.txt").is_file());

    let err = sg::install(&dir, &sg::InstallOptions::default()).expect_err("nothing to install");
    let msg = err.to_string();
    assert!(msg.contains("docs-only"), "should name the doc target: {msg}");
    assert!(msg.contains("build"), "should point at `build`: {msg}");
}

#[test]
fn lib_root_reaches_the_build_commands_and_is_made_absolute() {
    // The child resolves its own root from the DOCUMENT's directory, which is
    // inside the package being documented — usually no root at all — so the
    // caller's root has to be handed down. Relative, it must still mean the
    // caller's, not the working directory the command runs in.
    let dir = tmp("libroot");
    fs::create_dir_all(dir.join("doc")).unwrap();
    fs::write(
        dir.join("Satyristes"),
        r#"(version 0.0.2)
(libraryDoc (name "d") (version "1")
  (workingDirectory "doc")
  (build ((sh "-c" "printf %s \"$RUSTYFI_LIB_ROOT\" > seen.txt"))))"#,
    )
    .unwrap();
    let opts = sg::BuildOptions {
        lib_root: Some(PathBuf::from("relative-root")),
        ..Default::default()
    };
    sg::build(&dir, &opts).expect("build");
    let seen = fs::read_to_string(dir.join("doc/seen.txt")).unwrap();
    assert!(
        std::path::Path::new(&seen).is_absolute(),
        "the child should see an absolute root, got {seen:?}"
    );
    assert!(seen.ends_with("relative-root"), "got {seen:?}");
    assert!(
        !seen.contains("/doc/"),
        "resolved against the caller, not the working directory: {seen:?}"
    );
}

#[test]
fn same_named_blocks_coexist_when_their_lang_differs() {
    // One manifest, one name, both generations. `--lang` is what picks, and
    // each installs into its own corpus with its own receipt.
    let dir = tmp("lang");
    for (d, body) in [("src06", "% 0.0\n"), ("src01", "% 0.1\n")] {
        fs::create_dir_all(dir.join(d)).unwrap();
        fs::write(dir.join(d).join("pkg.satyh"), body).unwrap();
    }
    fs::write(
        dir.join("Satyristes"),
        r#"(version 0.0.2)
(library (name "pkg") (version "1") (lang 0.0) (sources ((packageDir "src06"))))
(library (name "pkg") (version "1") (lang 0.1) (sources ((packageDir "src01"))))
(libraryDoc (name "d") (version "1") (lang 0.0) (build ((cp "src06/pkg.satyh" "out06.txt"))))
(libraryDoc (name "d") (version "1") (lang 0.1) (build ((cp "src01/pkg.satyh" "out01.txt"))))"#,
    )
    .unwrap();
    let root = dir.join("root");

    // A name alone does not identify a library.
    let err = sg::install(&dir, &sg::InstallOptions { dest: Some(root.clone()), ..Default::default() })
        .expect_err("ambiguous across generations");
    assert!(matches!(err, sg::Error::AmbiguousLibrary { .. }), "{err}");
    assert!(err.to_string().contains("lang 0.0"), "names the generation: {err}");

    for lang in [sg::Lang::V0_0, sg::Lang::V0_1] {
        sg::install(
            &dir,
            &sg::InstallOptions { dest: Some(root.clone()), lang: Some(lang), ..Default::default() },
        )
        .unwrap_or_else(|e| panic!("install {}: {e}", lang.as_str()));
    }
    assert!(root.join("dist/packages/pkg/pkg.satyh").is_file(), "0.0 corpus");
    assert!(root.join("dist-v01/packages/pkg/pkg.satyh").is_file(), "0.1 corpus");
    assert!(root.join(".satyrographos/receipts/pkg.toml").is_file());
    assert!(root.join(".satyrographos/receipts/pkg@0.1.toml").is_file());

    // Doc targets select the same way.
    let err = sg::build(&dir, &sg::BuildOptions::default()).expect_err("ambiguous doc");
    assert!(matches!(err, sg::Error::AmbiguousDoc { .. }), "{err}");
    sg::build(&dir, &sg::BuildOptions { lang: Some(sg::Lang::V0_1), ..Default::default() })
        .expect("build the 0.1 doc");
    assert!(dir.join("out01.txt").is_file(), "the 0.1 target ran");
    assert!(!dir.join("out06.txt").exists(), "the 0.0 target did not");
}

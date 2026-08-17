//! Library-level phase-1 tests (plan §9): fixture package trees built under
//! unique temp directories (cleaned on drop, no extra dev-dependencies — the
//! same `TempDir` pattern as `satysfi-loader/tests/loader.rs`), exercised
//! directly through the `ops::{install,uninstall,list,status}` API.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use satysfi_satyrographos::{self as sg, InstallOptions, RootOptions};

// ---------------------------------------------------------------------------
// Temp-dir fixture helper (transcribed from satysfi-loader/tests/loader.rs).
// ---------------------------------------------------------------------------

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "satyrographos-test-{tag}-{}-{}-{}",
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

/// Build the plan's canonical two-source `great-package`-style manifest fixture
/// (`package-dir` + `font-dir`) under `<tmp>/src`, declaring package `name`.
fn write_manifest_pkg(tmp: &TempDir, name: &str, version: &str, satyh_body: &str) {
    tmp.write(
        "src/satysfi-package.toml",
        &format!(
            "[package]\n\
             name = \"{name}\"\n\
             version = \"{version}\"\n\
             satysfi-version-compat = \">=0.0.6, <0.1\"\n\
             \n\
             [[files]]\n\
             kind = \"package-dir\"\n\
             src = \"packages\"\n\
             \n\
             [[files]]\n\
             kind = \"font-dir\"\n\
             src = \"fonts\"\n"
        ),
    );
    tmp.write(&format!("src/packages/{name}.satyh"), satyh_body);
    tmp.write("src/fonts/x.ttf", "ttf-bytes\n");
}

fn dest_opts(root: &Path) -> InstallOptions {
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
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn install_manifest_materializes_nested_layout_and_receipt() {
    let tmp = TempDir::new("manifest");
    write_manifest_pkg(&tmp, "bar", "1.0.0", "let bar = 1\n");
    let root = tmp.path().join("root");

    let report = sg::install(&tmp.path().join("src"), &dest_opts(&root)).expect("install ok");
    assert_eq!(report.name, "bar");
    assert_eq!(report.version, "1.0.0");

    assert!(root.join("dist/packages/bar/bar.satyh").is_file());
    assert!(root.join("dist/fonts/bar/x.ttf").is_file());

    let receipt = root.join(".satyrographos/receipts/bar.toml");
    assert!(receipt.is_file());
    let text = fs::read_to_string(&receipt).unwrap();
    assert!(text.contains("dist/packages/bar/bar.satyh"), "{text}");
    assert!(text.contains("dist/fonts/bar/x.ttf"), "{text}");
    assert!(text.contains("kind = \"path\""), "{text}");
}

#[test]
fn install_no_manifest_falls_back_to_flat_copy() {
    let tmp = TempDir::new("fallback");
    tmp.write("src/packages/foo.satyh", "let foo = 1\n");
    let root = tmp.path().join("root");

    sg::install(&tmp.path().join("src"), &dest_opts(&root)).expect("fallback install ok");
    // Flat, no per-library namespace (plan §5.1).
    assert!(root.join("dist/packages/foo.satyh").is_file());
    assert!(!root.join("dist/packages/foo/foo.satyh").exists());
}

#[test]
fn collision_refused_without_force_then_replaced_with_force() {
    let tmp = TempDir::new("collide");
    write_manifest_pkg(&tmp, "bar", "1.0.0", "let bar = 1\n");
    let root = tmp.path().join("root");
    let src = tmp.path().join("src");

    sg::install(&src, &dest_opts(&root)).expect("first install ok");

    // Second install without --force is refused.
    let err = sg::install(&src, &dest_opts(&root)).unwrap_err();
    assert!(matches!(err, sg::Error::AlreadyInstalled { .. }), "{err}");

    // Change the source, install with --force: the file is replaced.
    fs::write(src.join("packages/bar.satyh"), "let bar = 999\n").unwrap();
    let forced = InstallOptions {
        force: true,
        ..dest_opts(&root)
    };
    sg::install(&src, &forced).expect("forced install ok");
    let installed = fs::read_to_string(root.join("dist/packages/bar/bar.satyh")).unwrap();
    assert_eq!(installed, "let bar = 999\n");
}

#[test]
fn unmanaged_collision_refused() {
    let tmp = TempDir::new("unmanaged");
    write_manifest_pkg(&tmp, "bar", "1.0.0", "let bar = 1\n");
    let root = tmp.path().join("root");
    // Pre-place an unmanaged file where the install would land, with no
    // receipt claiming it.
    fs::create_dir_all(root.join("dist/packages/bar")).unwrap();
    fs::write(root.join("dist/packages/bar/bar.satyh"), "hand placed\n").unwrap();

    let err = sg::install(&tmp.path().join("src"), &dest_opts(&root)).unwrap_err();
    assert!(matches!(err, sg::Error::UnmanagedCollision { .. }), "{err}");
    // The hand-placed content is untouched.
    let kept = fs::read_to_string(root.join("dist/packages/bar/bar.satyh")).unwrap();
    assert_eq!(kept, "hand placed\n");
}

#[test]
fn uninstall_removes_only_receipted_files() {
    let tmp = TempDir::new("uninstall");
    write_manifest_pkg(&tmp, "bar", "1.0.0", "let bar = 1\n");
    let root = tmp.path().join("root");
    sg::install(&tmp.path().join("src"), &dest_opts(&root)).expect("install ok");

    // A user hand-adds an unrelated file under the package's own directory.
    let extra = root.join("dist/packages/bar/HAND_ADDED.txt");
    fs::write(&extra, "keep me\n").unwrap();

    sg::uninstall("bar", &root_opts(&root)).expect("uninstall ok");

    // Receipted files and the receipt are gone.
    assert!(!root.join("dist/packages/bar/bar.satyh").exists());
    assert!(!root.join(".satyrographos/receipts/bar.toml").exists());
    // The hand-added file survives (never rm -rf'd), so its parent dir must
    // survive too (pruning stops at any non-empty directory).
    assert!(extra.is_file(), "hand-added file must survive uninstall");
}

#[test]
fn uninstall_missing_package_errors() {
    let tmp = TempDir::new("uninstall-missing");
    let root = tmp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let err = sg::uninstall("nope", &root_opts(&root)).unwrap_err();
    assert!(matches!(err, sg::Error::NotInstalled { .. }), "{err}");
}

#[test]
fn list_shows_sorted_packages() {
    let tmp = TempDir::new("list");
    let root = tmp.path().join("root");

    // Empty root: no error, empty list.
    assert!(sg::list(&root_opts(&root)).unwrap().is_empty());

    // Two packages, installed out of alphabetical order.
    for (name, ver) in [("zebra", "0.1.0"), ("alpha", "3.2.1")] {
        let sub = TempDir::new(name);
        write_manifest_pkg(&sub, name, ver, "let x = 1\n");
        sg::install(&sub.path().join("src"), &dest_opts(&root)).expect("install ok");
    }

    let listed = sg::list(&root_opts(&root)).unwrap();
    let names: Vec<_> = listed.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["alpha", "zebra"], "list must be sorted");
    assert_eq!(listed[0].version, "3.2.1");
    assert_eq!(listed[0].file_count, 2);
}

#[test]
fn status_flags_missing_files() {
    let tmp = TempDir::new("status");
    write_manifest_pkg(&tmp, "bar", "1.0.0", "let bar = 1\n");
    let root = tmp.path().join("root");
    sg::install(&tmp.path().join("src"), &dest_opts(&root)).expect("install ok");

    // All present initially.
    let report = sg::status(None, &root_opts(&root)).unwrap();
    assert!(!report.any_missing());

    // Delete one recorded file; status must flag it.
    fs::remove_file(root.join("dist/fonts/bar/x.ttf")).unwrap();
    let report = sg::status(Some("bar"), &root_opts(&root)).unwrap();
    assert!(report.any_missing());
    let pkg = &report.packages[0];
    assert_eq!(pkg.missing_files.len(), 1);
    assert!(pkg.missing_files[0].ends_with("x.ttf"));
    assert_eq!(pkg.present_files(), 1);
}

#[test]
fn tar_gz_install_produces_nested_layout() {
    let tmp = TempDir::new("targz");
    write_manifest_pkg(&tmp, "greet", "2.1.0", "let greet = 1\n");
    let archive = tmp.path().join("greet.tar.gz");
    make_tar_gz(&tmp.path().join("src"), &archive);

    let root = tmp.path().join("root");
    let report = sg::install(&archive, &dest_opts(&root)).expect("tar.gz install ok");
    assert_eq!(report.name, "greet");
    assert!(root.join("dist/packages/greet/greet.satyh").is_file());

    let text = fs::read_to_string(root.join(".satyrographos/receipts/greet.toml")).unwrap();
    assert!(text.contains("kind = \"archive\""), "{text}");
}

#[test]
fn zip_slip_is_refused_and_writes_nothing() {
    let tmp = TempDir::new("zipslip");
    let archive = tmp.path().join("evil.tar.gz");
    make_malicious_tar_gz(&archive);
    let root = tmp.path().join("root");

    let err = sg::install(&archive, &dest_opts(&root)).unwrap_err();
    assert!(matches!(err, sg::Error::PathTraversal { .. }), "{err}");
    // The traversal target (a sibling of the extraction dir) was never
    // written.
    assert!(!tmp.path().join("escape.txt").exists());
    assert!(!root.join("dist").exists());
}

// ---------------------------------------------------------------------------
// Phase 4: Satyristes S-expression front-end (plan §5.5/§9).
// ---------------------------------------------------------------------------

/// The upstream README's own great-package `Satyristes`, verbatim (fetched
/// 2026-07-04 via `gh api repos/na4zagin3/satyrographos/contents/README.md`).
const README_GREAT_PACKAGE: &str = r#"
;; For Satyrographos 0.0.2 series
(version 0.0.2)

;; Library declaration
(library
  ;; Library name
  (name "great-package")
  ;; Library version
  (version "1.0")
  ;; Files
  (sources
    ((fontDir "fonts")
     (hash "fonts.satysfi-hash" "hash/fonts.satysfi-hash")
     (packageDir "packages")))
  ;; OPAM package file
  (opam "satysfi-great-package.opam")
  ;; Dependency
  (dependencies ((fonts-theano ()))))

;; Library doc declaration (parsed and ignored)
(libraryDoc
  (name "great-package-doc")
  (version "1.0")
  (workingDirectory "doc")
  (build ((satysfi "great-package.saty" "-o" "great-package.pdf")))
  (sources ((doc "great-package.pdf" "doc/great-package.pdf")))
  (opam "satysfi-great-package-doc.opam")
  (dependencies ((great-package ()))))
"#;

/// Materialise the great-package source tree the README's `Satyristes`
/// references, under `<tmp>/src`.
fn write_readme_great_package(tmp: &TempDir) {
    tmp.write("src/Satyristes", README_GREAT_PACKAGE);
    tmp.write("src/packages/great-package.satyh", "let gp = 1\n");
    tmp.write("src/fonts/interesting-font.ttf", "ttf-bytes\n");
    tmp.write("src/hash/fonts.satysfi-hash", "{\"fonts\": {}}\n");
}

#[test]
fn satyristes_readme_example_installs_per_kind_destinations() {
    let tmp = TempDir::new("satyristes-readme");
    write_readme_great_package(&tmp);
    let root = tmp.path().join("root");

    let report = sg::install(&tmp.path().join("src"), &dest_opts(&root)).expect("satyristes install");
    // §5.5 field-by-field: name/version straight off the (library ...) block.
    assert_eq!(report.name, "great-package");
    assert_eq!(report.version, "1.0");

    // packageDir -> recursively into dist/packages/<name>/.
    assert!(root.join("dist/packages/great-package/great-package.satyh").is_file());
    // fontDir -> recursively into dist/fonts/<name>/.
    assert!(root.join("dist/fonts/great-package/interesting-font.ttf").is_file());
    // hash -> FLAT dist/hash/<dst>, no per-library namespace (§5.5 asymmetry).
    assert!(root.join("dist/hash/fonts.satysfi-hash").is_file());
    assert!(!root.join("dist/hash/great-package/fonts.satysfi-hash").exists());

    // dependencies recorded (fonts-theano, wildcard); opam/libraryDoc ignored.
    let receipt = root.join(".satyrographos/receipts/great-package.toml");
    let text = fs::read_to_string(&receipt).unwrap();
    assert!(text.contains("dist/hash/fonts.satysfi-hash"), "{text}");
    assert!(text.contains("dist/packages/great-package/great-package.satyh"), "{text}");
}

#[test]
fn satyristes_positional_kinds_land_correctly() {
    // Exercise every remaining source kind (package/font/md/file) with the
    // positional `(kind "dst" "src")` form.
    let tmp = TempDir::new("satyristes-kinds");
    tmp.write(
        "src/Satyristes",
        r#"(library
  (name "kits")
  (version "0.2")
  (sources
    ((package "kits.satyh" "src/kits.satyh")
     (font "body.ttf" "assets/body.ttf")
     (md "guide.md" "docs/guide.md")
     (file "extra/notes.txt" "notes.txt"))))
"#,
    );
    tmp.write("src/src/kits.satyh", "let kits = 1\n");
    tmp.write("src/assets/body.ttf", "font\n");
    tmp.write("src/docs/guide.md", "# guide\n");
    tmp.write("src/notes.txt", "notes\n");
    let root = tmp.path().join("root");

    sg::install(&tmp.path().join("src"), &dest_opts(&root)).expect("install");
    assert!(root.join("dist/packages/kits/kits.satyh").is_file());
    assert!(root.join("dist/fonts/kits/body.ttf").is_file());
    assert!(root.join("dist/md/kits/guide.md").is_file());
    // `file` kind is arbitrary, root-relative under dist/.
    assert!(root.join("dist/extra/notes.txt").is_file());
}

#[test]
fn satyristes_multiple_libraries_require_selection() {
    let tmp = TempDir::new("satyristes-multi");
    tmp.write(
        "src/Satyristes",
        r#"(library (name "alpha") (version "1") (sources ((packageDir "packages"))))
(library (name "beta") (version "2") (sources ((packageDir "packages"))))
"#,
    );
    tmp.write("src/packages/x.satyh", "let x = 1\n");
    let root = tmp.path().join("root");

    // No --library filter, two libraries: ambiguous.
    let err = sg::install(&tmp.path().join("src"), &dest_opts(&root)).unwrap_err();
    assert!(matches!(err, sg::Error::AmbiguousLibrary { .. }), "{err}");

    // --library selects exactly one.
    let opts = InstallOptions {
        libraries: Some(vec!["beta".to_string()]),
        ..dest_opts(&root)
    };
    let report = sg::install(&tmp.path().join("src"), &opts).expect("selected install");
    assert_eq!(report.name, "beta");
    assert!(root.join("dist/packages/beta/x.satyh").is_file());
    // alpha was not materialised.
    assert!(!root.join("dist/packages/alpha").exists());
}

#[test]
fn satyristes_unknown_form_errors_naming_it() {
    let tmp = TempDir::new("satyristes-unknown");
    tmp.write(
        "src/Satyristes",
        "(library (name \"p\") (version \"1\") (sources ((packageDir \"packages\"))))\n(frobnicate 1)\n",
    );
    tmp.write("src/packages/x.satyh", "let x = 1\n");
    let root = tmp.path().join("root");

    let err = sg::install(&tmp.path().join("src"), &dest_opts(&root)).unwrap_err();
    match err {
        sg::Error::Satyristes { message, .. } => {
            assert!(message.contains("frobnicate"), "{message}");
        }
        other => panic!("expected Satyristes error, got {other}"),
    }
}

#[test]
fn both_manifests_are_ambiguous() {
    let tmp = TempDir::new("satyristes-both");
    write_readme_great_package(&tmp);
    // Also drop a satysfi-package.toml alongside the Satyristes.
    tmp.write(
        "src/satysfi-package.toml",
        "[package]\nname = \"x\"\nversion = \"1\"\nsatysfi-version-compat = \"*\"\n",
    );
    let root = tmp.path().join("root");
    let err = sg::install(&tmp.path().join("src"), &dest_opts(&root)).unwrap_err();
    assert!(matches!(err, sg::Error::AmbiguousSource { .. }), "{err}");
}

#[test]
fn satyristes_tar_gz_round_trip() {
    let tmp = TempDir::new("satyristes-targz");
    write_readme_great_package(&tmp);
    let archive = tmp.path().join("gp.tar.gz");
    make_tar_gz(&tmp.path().join("src"), &archive);
    let root = tmp.path().join("root");

    let report = sg::install(&archive, &dest_opts(&root)).expect("targz satyristes install");
    assert_eq!(report.name, "great-package");
    assert!(root.join("dist/hash/fonts.satysfi-hash").is_file());
}

// ---------------------------------------------------------------------------
// Archive-building helpers (tar + flate2 are normal deps of this crate, so
// they are available to integration tests without new dev-dependencies).
// ---------------------------------------------------------------------------

fn make_tar_gz(src_dir: &Path, out: &Path) {
    let file = fs::File::create(out).unwrap();
    let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(enc);
    // Archive the source directory's *contents* under a top-level "pkg/" dir.
    builder.append_dir_all("pkg", src_dir).unwrap();
    builder.into_inner().unwrap().finish().unwrap();
}

/// Build a tar.gz containing a single entry whose path escapes the extraction
/// root via `..`, written with a raw header name to bypass the `tar` crate's
/// own `set_path` sanitisation (a real attacker's tool has no such qualms).
fn make_malicious_tar_gz(out: &Path) {
    let file = fs::File::create(out).unwrap();
    let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(enc);

    let data = b"pwned\n";
    let mut header = tar::Header::new_gnu();
    {
        let name = b"../escape.txt";
        let old = header.as_old_mut();
        old.name[..name.len()].copy_from_slice(name);
    }
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append(&header, &data[..]).unwrap();
    builder.into_inner().unwrap().finish().unwrap();
}

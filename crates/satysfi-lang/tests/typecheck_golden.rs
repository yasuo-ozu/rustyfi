//! The L3 differential golden harness (`…/tmp/l3-typecheck-refactor.md`
//! §8.3): for every `.saty`/`.satyh`/`.satyg` fixture under
//! `crates/*/tests/fixtures/` and every one of the 29 bundled packages
//! under `lib-satysfi/dist/packages/` (loader-merged the same way `stdja`
//! loads today, via a synthetic `@require: <pkg>` entry), run
//! parse -> elaborate -> `typecheck_verbose` and print one deterministic
//! line per input: `OK <tag> <n-warnings> <warning debug strings…>` or
//! `ERR <tag> <Display-of-TypeError-or-earlier-stage-error>`.
//!
//! `scripts/typecheck-golden.sh` runs this test (via `--ignored
//! --nocapture`) once on the parent commit and once on the L3 commit and
//! diffs the two outputs — the diff must be empty (byte-identical
//! error/warning strings across the whole corpus). This is the "ordering
//! tripwire" §5 of the spec describes: it catches a swapped
//! `expected`/`found`, a dropped `?`, or an off-by-one arm move — anything
//! that would change an error string or a warning list anywhere in the
//! corpus, even where the individual unit-test suite's spot assertions
//! wouldn't notice.
//!
//! `#[ignore]`d: this walks the whole corpus (dozens of files, several of
//! them large stdlib packages) and is meant to be run explicitly by the
//! golden script, not on every `cargo test --workspace`.

use satysfi_lang::{elaborate, primitives, typecheck, v1};
use satysfi_loader::{LoadOptions, LoadedCst, LoadedFile, LoadedProgram};
use satysfi_syntax::SatysfiVersion;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// This repo's root, resolved relative to this crate's own manifest
/// directory (`crates/satysfi-lang`) — same convention as
/// `stdlib_tier0.rs`'s `lib_root`.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root should exist")
}

fn lib_root() -> PathBuf {
    repo_root().join("lib-satysfi")
}

/// Every `crates/*/tests/fixtures/**/*.saty(h|g)` file, sorted for
/// deterministic output order.
fn fixture_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let crates_dir = repo_root().join("crates");
    for entry in fs::read_dir(&crates_dir).expect("read crates/") {
        let entry = entry.expect("read_dir entry");
        let fixtures = entry.path().join("tests").join("fixtures");
        if fixtures.is_dir() {
            walk_saty_files(&fixtures, &mut out);
        }
    }
    out.sort();
    out
}

fn walk_saty_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let entry = entry.expect("read_dir entry");
        let path = entry.path();
        if path.is_dir() {
            walk_saty_files(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("saty") | Some("satyh") | Some("satyg")
        ) {
            out.push(path);
        }
    }
}

/// Every bundled package name under `lib-satysfi/dist/packages/` (file stem
/// only — the name `@require:` resolves), sorted for deterministic output
/// order. 29 at the L3 baseline (see the spec's §8.3).
fn package_names() -> Vec<String> {
    let mut out = Vec::new();
    let dir = lib_root().join("dist").join("packages");
    for entry in fs::read_dir(&dir).expect("read dist/packages/") {
        let entry = entry.expect("read_dir entry");
        let path = entry.path();
        if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("satyh") | Some("satyg")
        ) {
            out.push(
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .expect("package file stem")
                    .to_string(),
            );
        }
    }
    out.sort();
    out
}

/// A uniquely-named temp `.saty` entry file, cleaned up on drop — used only
/// for the synthetic `@require: <pkg> in ()` package-probe entries.
struct TempDoc(PathBuf);

impl TempDoc {
    fn new(tag: &str, src: &str) -> TempDoc {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "satysfi-lang-typecheck-golden-{tag}-{}-{}.saty",
            std::process::id(),
            n
        ));
        fs::write(&path, src).expect("write temp golden entry");
        TempDoc(path)
    }
}

impl Drop for TempDoc {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn as_v006(cst: &LoadedCst) -> &satysfi_syntax::cst::File {
    match cst {
        LoadedCst::V0_0_6(f) => f,
        LoadedCst::V0_1(_) => unreachable!("as_v006 called on a V0_1-loaded file"),
    }
}

/// Merge a loader-resolved V0.0.6 program's preludes into one synthetic
/// `cst::File` — mirrors `satysfi-cli`'s `merge_program` /
/// `stdlib_tier0.rs`'s helper of the same name.
fn merge_program_v006(program: LoadedProgram) -> satysfi_syntax::cst::File {
    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let entry_cst = as_v006(&entry.cst).clone();
    let mut prelude = Vec::new();
    for lib in &files {
        prelude.extend(as_v006(&lib.cst).prelude.clone());
    }
    prelude.extend(entry_cst.prelude);
    satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_cst.in_kw,
        body: entry_cst.body,
        eoi: entry_cst.eoi,
    }
}

fn as_v01(f: &LoadedFile) -> &satysfi_syntax::cst_v1::FileV1 {
    match &f.cst {
        LoadedCst::V0_1(cst) => cst,
        LoadedCst::V0_0_6(_) => unreachable!("as_v01 called on a V0_0_6-loaded file"),
    }
}

/// V0_1 analogue of `merge_program_v006` — mirrors `lib.rs`'s
/// `compile_document_v1_with_trials` assembly step (lowering each
/// dependency + the entry through `v1::lower`), stopping short of eval.
/// Also returns the X2a `v006_indices` set (mirrors production's — see
/// `elaborate::elaborate_program_with_versions`), so callers can wrap
/// spliced V0_0_6 bindings the same way production does.
fn merge_program_v01(
    files: &[LoadedFile],
) -> Result<(satysfi_syntax::cst::File, std::collections::HashSet<usize>), v1::lower::LowerError> {
    let (entry, deps) = files
        .split_last()
        .expect("loader always yields at least the entry file");
    let mut prelude = Vec::new();
    let mut v006_indices = std::collections::HashSet::new();
    for dep in deps {
        // X1/X2a (design-cross-version-import.md §5, §"Slice X2 — per-group
        // primitive environment"): mirror production's
        // `compile_document_v1_with_trials` dep loop, which is now a
        // MIXED-version list. A V0_1 dep is lowered as before; a V0_0_6 dep (a
        // 0.0.6-corpus `@require:` target reached under a V0_1 entry) splices
        // its `cst::File.prelude` bindings directly, and its contributed
        // top-level indices are recorded into `v006_indices` — mirroring
        // production's index bookkeeping — so `one_line`'s V0_1 branch can
        // call `elaborate::elaborate_program_with_versions` the same way
        // `compile_document_v1_with_trials` does, keeping the two in sync
        // for a mixed-version fixture's `Ast::VersionScope` shape. Production
        // ALSO runs the forked-name guard on this path, but this
        // typecheck-differential harness only needs to not panic + emit a
        // stable line, so it still splices unconditionally, guard-free. (The
        // V0_0_6-first branch in `one_line` — the 0.0.6 golden lines — never
        // reaches here and is untouched by X1/X2a.)
        match &dep.cst {
            LoadedCst::V0_1(cst) => prelude.extend(v1::lower::lower_file_v1(cst)?),
            LoadedCst::V0_0_6(cst) => {
                let start = prelude.len();
                prelude.extend(cst.prelude.iter().cloned());
                v006_indices.extend(start..prelude.len());
            }
        }
    }
    let entry_cst = as_v01(entry);
    let body = v1::lower::lower_document_v1(entry_cst)?;
    let eoi = match entry_cst {
        satysfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => unreachable!("lower_document_v1 already rejected a Library entry"),
    };
    Ok((
        satysfi_syntax::cst::File {
            headers: Vec::new(),
            prelude,
            in_kw: Some(satysfi_syntax::leaf::KwIn(satysfi_syntax::Span::default())),
            body: Some(body),
            eoi,
        },
        v006_indices,
    ))
}

/// Load + elaborate + `typecheck_verbose` one entry file, trying V0_0_6
/// first and falling back to V0_1 if the V0_0_6 load itself fails (the
/// corpus mixes both generations, e.g. `v01-*.saty` fixtures) — the version
/// used is baked into `tag` so the golden output stays stable across runs
/// regardless of which branch actually fired.
fn one_line(tag: &str, entry: &Path) -> String {
    let opts_006 = LoadOptions {
        lib_root: Some(lib_root()),
        version: SatysfiVersion::V0_0_6,
        ..Default::default()
    };
    match satysfi_loader::load(entry, &opts_006) {
        Ok(program) => {
            let file = merge_program_v006(program);
            render(tag, "0.0.6", || {
                let env = primitives::base_env();
                let scope = elaborate::Scope::new(env.names());
                let program = elaborate::elaborate_program(&file, &scope)
                    .map_err(|e| format!("elaborate: {e}"))?;
                typecheck::typecheck_verbose(&program).map_err(|e| format!("{e}"))
            })
        }
        Err(v006_err) => {
            let opts_01 = LoadOptions {
                lib_root: Some(lib_root()),
                version: SatysfiVersion::V0_1,
                ..Default::default()
            };
            match satysfi_loader::load(entry, &opts_01) {
                Ok(program) => match merge_program_v01(&program.files) {
                    Ok((file, v006_indices)) => render(tag, "0.1", || {
                        let env = primitives::base_env_with_version(SatysfiVersion::V0_1);
                        let scope = elaborate::Scope::new(env.names());
                        let program = elaborate::elaborate_program_with_versions(
                            &file,
                            &scope,
                            &v006_indices,
                            None,
                        )
                        .map_err(|e| format!("elaborate: {e}"))?;
                        typecheck::typecheck_verbose_with_version(&program, SatysfiVersion::V0_1)
                            .map_err(|e| format!("{e}"))
                    }),
                    Err(lower_err) => format!("ERR {tag} (0.1) lower: {lower_err}"),
                },
                // Neither version's loader could even parse/resolve this
                // entry — report the original V0.0.6 load error (stable,
                // version-independent framing).
                Err(_v01_err) => format!("ERR {tag} load: {v006_err}"),
            }
        }
    }
}

fn render(
    tag: &str,
    version_tag: &str,
    f: impl FnOnce() -> Result<Vec<typecheck::MatchWarning>, String>,
) -> String {
    match f() {
        Ok(warnings) => {
            let rendered: Vec<String> = warnings.iter().map(|w| format!("{w:?}")).collect();
            format!(
                "OK {tag} ({version_tag}) {} [{}]",
                warnings.len(),
                rendered.join("; ")
            )
        }
        Err(e) => format!("ERR {tag} ({version_tag}) {e}"),
    }
}

#[test]
#[ignore = "golden differential harness — run explicitly via scripts/typecheck-golden.sh"]
fn typecheck_golden() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let mut lines = Vec::new();

            for path in fixture_files() {
                let tag = path
                    .strip_prefix(repo_root())
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                lines.push(one_line(&tag, &path));
            }

            for pkg in package_names() {
                let src = format!("@require: {pkg}\nin\n()");
                let doc = TempDoc::new(&pkg, &src);
                let tag = format!("package:{pkg}");
                lines.push(one_line(&tag, &doc.0));
            }

            lines.sort();
            for line in &lines {
                println!("{line}");
            }
        })
        .expect("spawn big-stack thread")
        .join()
        .expect("golden harness thread panicked");
}

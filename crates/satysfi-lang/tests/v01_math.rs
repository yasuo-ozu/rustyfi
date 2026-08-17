//! math-split spec §6.3: the `math-text`/`math-boxes` split + `read-math` +
//! `val math` integration tests. Harness copied from `v01_slice1.rs:95-160`
//! (hand-built `LoadedFile`s + a `Mono` metrics stub +
//! `compile_document_v1_with_trials`, and `assert_geometry_equivalent`).

use std::path::{Path, PathBuf};

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::value::DocumentValue;
use satysfi_lang::{prim_types, primitives, CompileError};
use satysfi_loader::{LoadOptions, LoadedCst, LoadedFile, LoadedProgram};
use satysfi_syntax::SatysfiVersion;

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Same ASCII-advancing `FontMetrics` stub `v01_slice1.rs` uses — the
/// fixtures render real ASCII glyphs (`a^2 + b^2 = c^2`, digits, `x`, …),
/// so `advance` must return `Some` for every ASCII char. No MATH table
/// (`math_constants` defaults to `None`), matching every other base-14
/// fixture in this crate.
struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, c: char, size: Length) -> Option<Length> {
        if c.is_ascii() {
            Some(size * 0.5)
        } else {
            None
        }
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

fn v01_mini_file() -> LoadedFile {
    let path = repo("lib-satysfi/dist-v01/packages/v01-mini.satyh");
    let src = std::fs::read_to_string(&path).expect("read v01-mini.satyh");
    LoadedFile {
        path,
        cst: LoadedCst::V0_1(
            satysfi_syntax::parse_file_v1(&src).unwrap_or_else(|e| panic!("v01-mini.satyh: {e}")),
        ),
        origin: Default::default(),
        version: SatysfiVersion::V0_1,
    }
}

fn entry_file_from_disk(rel: &str) -> LoadedFile {
    let path = repo(rel);
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {rel}: {e}"));
    LoadedFile {
        path,
        cst: LoadedCst::V0_1(
            satysfi_syntax::parse_file_v1(&src).unwrap_or_else(|e| panic!("{rel}: {e}")),
        ),
        origin: Default::default(),
        version: SatysfiVersion::V0_1,
    }
}

/// An in-code V0_1 entry document (no on-disk fixture) — same `LoadedCst`
/// shape, just parsed straight from `src` with a placeholder path.
fn entry_file_inline(src: &str) -> LoadedFile {
    LoadedFile {
        path: PathBuf::from("<inline-test-entry>"),
        cst: LoadedCst::V0_1(
            satysfi_syntax::parse_file_v1(src).unwrap_or_else(|e| panic!("inline entry: {e}")),
        ),
        origin: Default::default(),
        version: SatysfiVersion::V0_1,
    }
}

/// A hand-built V0_1 *library* source, parsed the same way `v01_mini_file`
/// parses a real one — used by tests that need a `module … = struct … end`
/// wrapper (`val math` can only bind inside a library, never at document
/// top level — `FileV1::Document` is just an expression).
fn lib_file_inline(name: &str, src: &str) -> LoadedFile {
    LoadedFile {
        path: PathBuf::from(format!("<inline-test-lib:{name}>")),
        cst: LoadedCst::V0_1(
            satysfi_syntax::parse_file_v1(src).unwrap_or_else(|e| panic!("inline lib {name}: {e}")),
        ),
        origin: Default::default(),
        version: SatysfiVersion::V0_1,
    }
}

/// Merge a loader-resolved 0.0.6 program's preludes into one synthetic
/// `cst::File`, exactly like `satysfi-cli`'s private `merge_program`
/// (reproduced locally the same way `v01_slice1.rs` does).
fn merge_program_006(program: LoadedProgram) -> satysfi_syntax::cst::File {
    fn as_v006(cst: LoadedCst) -> satysfi_syntax::cst::File {
        match cst {
            LoadedCst::V0_0_6(f) => f,
            LoadedCst::V0_1(_) => unreachable!("this test's 0.0.6 merge path only"),
        }
    }
    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let entry_cst = as_v006(entry.cst);
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(as_v006(lib.cst).prelude);
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

/// The plan's "same core IR, different surface" bar (`v01_slice1.rs`'s own
/// helper, reproduced): equal page geometry, equal per-page line counts,
/// equal placed-line x/baseline_y sequences.
fn assert_geometry_equivalent(a: &DocumentValue, b: &DocumentValue) {
    assert_eq!(
        a.geometry, b.geometry,
        "page geometry must match exactly (both are A4)"
    );
    assert_eq!(a.pages.len(), b.pages.len(), "page count must match");
    for (i, (pa, pb)) in a.pages.iter().zip(b.pages.iter()).enumerate() {
        assert_eq!(
            pa.lines.len(),
            pb.lines.len(),
            "page {i}: line count must match"
        );
        for (j, (la, lb)) in pa.lines.iter().zip(pb.lines.iter()).enumerate() {
            assert!(
                (la.x.0 - lb.x.0).abs() < 1e-6,
                "page {i} line {j}: x mismatch ({} vs {})",
                la.x.0,
                lb.x.0
            );
            assert!(
                (la.baseline_y.0 - lb.baseline_y.0).abs() < 1e-6,
                "page {i} line {j}: baseline_y mismatch ({} vs {})",
                la.baseline_y.0,
                lb.baseline_y.0
            );
        }
    }
}

/// Test 1 (§6.3): `${a^2}` and `\frac` through `read-math` + the v01 math
/// prims produce IDENTICAL glyph geometry to the 0.0.6 twin's `as_math`
/// path — both feed the same layout engine. This is the capstone
/// assertion the math-split spec calls out.
#[test]
fn v01_math_matches_006_twin_geometry() {
    let files = vec![
        v01_mini_file(),
        entry_file_from_disk("crates/satysfi-cli/tests/fixtures/v01-math.saty"),
    ];
    let mono = Mono;
    let (doc_v1, _trials) = satysfi_lang::compile_document_v1_with_trials(&files, &mono)
        .unwrap_or_else(|e| panic!("compile_document_v1_with_trials: {e}"));
    assert_eq!(doc_v1.pages.len(), 1);
    assert!(
        doc_v1.pages[0].lines.len() >= 2,
        "two +p paragraphs, got {}",
        doc_v1.pages[0].lines.len()
    );

    let entry_006 = repo("crates/satysfi-cli/tests/fixtures/v01-math-equiv-006.saty");
    let lib_root_006 = repo("lib-satysfi");
    let program_006 = satysfi_loader::load(
        &entry_006,
        &LoadOptions {
            lib_root: Some(lib_root_006),
            version: SatysfiVersion::V0_0_6,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("loading v01-math-equiv-006.saty: {e}"));
    let merged_006 = merge_program_006(program_006);
    let doc_006 = satysfi_lang::compile_document_cst(&merged_006, &mono)
        .unwrap_or_else(|e| panic!("compile_document_cst on the 0.0.6 twin: {e}"));

    assert_geometry_equivalent(&doc_v1, &doc_006);
}

/// Test 2 (§6.3): the with-scripts smoke fixture compiles, produces one
/// page, and the `\lim` paragraph's line exists (no 0.0.6 twin — 0.0.6's
/// `\lim` scripting would render corner scripts, not limits).
#[test]
fn v01_scripts_render() {
    let files = vec![
        v01_mini_file(),
        entry_file_from_disk("crates/satysfi-cli/tests/fixtures/v01-math-scripts.saty"),
    ];
    let mono = Mono;
    let (doc, _trials) = satysfi_lang::compile_document_v1_with_trials(&files, &mono)
        .unwrap_or_else(|e| panic!("compile_document_v1_with_trials: {e}"));
    assert_eq!(doc.pages.len(), 1);
    assert!(
        !doc.pages[0].lines.is_empty(),
        "the \\lim paragraph's line must exist"
    );
}

/// Test 3 (§6.3): `math-text` and `math-boxes` are genuinely distinct V0_1
/// types — `read-math`'s OWN result (`math-boxes`) cannot be fed back into
/// `read-math`'s `math-text` argument slot. Negative test.
#[test]
fn read_math_result_cannot_be_used_as_math_text() {
    let entry = r#"
@require: v01-mini
let open V01Mini in
let ctx = get-initial-context 440pt (command \math) in
let x = read-math ctx ${a} in
let y = read-math ctx x in
0
"#;
    let files = vec![v01_mini_file(), entry_file_inline(entry)];
    let mono = Mono;
    let err = satysfi_lang::compile_document_v1_with_trials(&files, &mono)
        .expect_err("math-boxes fed where math-text is required must be a type error");
    assert!(
        matches!(err, CompileError::Type(_)),
        "expected a TypeError, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("MathText") || msg.contains("MathBoxes") || msg.contains("math"),
        "error should mention the type mismatch: {msg}"
    );
}

/// Test 4 (§6.3): `val math ctx \bad m = m` — returning the command's own
/// `math-text` PARAMETER directly, never routed through `read-math` — must
/// fail: a V0_1 math command's result must be `math-boxes`. `m`'s own type
/// is pinned to `math-text` by a throwaway `read-math ctx m` (with no
/// other use, `m` would stay a free/generalized variable and this scheme
/// would type-check vacuously — Milner generalization means a call-site
/// constraint can't retroactively narrow an already-generalized scheme).
#[test]
fn val_math_result_must_be_math_boxes() {
    let lib = "module BadMath = struct\n  \
               val math ctx \\bad m =\n    \
               let _ = read-math ctx m in\n    \
               m\nend";
    let files = vec![lib_file_inline("bad-math", lib), entry_file_inline("0")];
    let mono = Mono;
    let err = satysfi_lang::compile_document_v1_with_trials(&files, &mono)
        .expect_err("a `val math` command returning `math-text` must be a type error");
    assert!(
        matches!(err, CompileError::Type(_)),
        "expected a TypeError, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("math-boxes") || msg.contains("MathBoxes"),
        "error should mention `math-boxes`: {msg}"
    );
}

/// Test 5 (§6.3): the 6 prims 0.1 removes outright (`math-split` spec
/// §2.1) are unbound in BOTH the runtime env and the type table under
/// V0_1, and stay bound under V0_0_6 (the positive twin) — env and
/// type-env agree by construction (both key on
/// `SatysfiVersion::math_is_split`/`VersionSpan`).
#[test]
fn removed_prims_are_unbound_in_v01() {
    for name in [
        "get-axis-height",
        "math-pull-in-scripts",
        "math-color",
        "math-char-class",
        "math-variant-char",
        "text-in-math",
    ] {
        assert!(
            primitives::base_env_with_version(SatysfiVersion::V0_1)
                .lookup(name)
                .is_none(),
            "'{name}' should be unbound in the V0_1 runtime env"
        );
        assert!(
            prim_types::primitive_type_with_version(name, SatysfiVersion::V0_1).is_none(),
            "'{name}' should have no V0_1 type"
        );
        // Twin positive: unaffected under V0_0_6.
        assert!(
            primitives::base_env_with_version(SatysfiVersion::V0_0_6)
                .lookup(name)
                .is_some(),
            "'{name}' should stay bound in the V0_0_6 runtime env"
        );
        assert!(
            prim_types::primitive_type_with_version(name, SatysfiVersion::V0_0_6).is_some(),
            "'{name}' should keep its V0_0_6 type"
        );
    }
}

/// Test 6 (§6.3): the 8 prims 0.1 adds (math-split spec §2.2), plus G6's 4
/// table prims (`…/tmp/g6-g7-standins.md` §5.1), are unbound under V0_0_6
/// in both the runtime env and the type table, and bound under V0_1.
#[test]
fn added_prims_are_unbound_in_006() {
    for name in [
        "read-math",
        "stringify-math",
        "set-math-char",
        "set-math-char-class",
        "get-math-char-class",
        "embed-inline-to-math",
        "get-math-axis-height-ratio",
        "%math-attach-scripts",
        // G6 table prims:
        "load-hyphenation-dictionary",
        "load-unicode-char-database",
        "set-hyphenation-dictionary",
        "set-unicode-char-database",
    ] {
        assert!(
            primitives::base_env_with_version(SatysfiVersion::V0_0_6)
                .lookup(name)
                .is_none(),
            "'{name}' should be unbound in the V0_0_6 runtime env"
        );
        assert!(
            prim_types::primitive_type(name).is_none(),
            "'{name}' should have no V0_0_6 type"
        );
        assert!(
            primitives::base_env_with_version(SatysfiVersion::V0_1)
                .lookup(name)
                .is_some(),
            "'{name}' should be bound in the V0_1 runtime env"
        );
        assert!(
            prim_types::primitive_type_with_version(name, SatysfiVersion::V0_1).is_some(),
            "'{name}' should have a V0_1 type"
        );
    }
}

/// G6 (`…/tmp/g6-g7-standins.md` §5.1): `here` is a bare CONSTANT (not a
/// `prims!` table row), so it needs its own gating assertion — unbound (env
/// + type table) under V0_0_6, bound under V0_1.
#[test]
fn here_constant_is_unbound_in_006() {
    assert!(
        primitives::base_env_with_version(SatysfiVersion::V0_0_6)
            .lookup("here")
            .is_none(),
        "'here' should be unbound in the V0_0_6 runtime env"
    );
    assert!(
        prim_types::primitive_type("here").is_none(),
        "'here' should have no V0_0_6 type"
    );
    assert!(
        primitives::base_env_with_version(SatysfiVersion::V0_1)
            .lookup("here")
            .is_some(),
        "'here' should be bound in the V0_1 runtime env"
    );
    assert!(
        prim_types::primitive_type_with_version("here", SatysfiVersion::V0_1).is_some(),
        "'here' should have a V0_1 type"
    );
}

/// Like `compile_document_v1`, but returns the entry expression's RAW
/// evaluated `Value` instead of requiring it to be a `DocumentValue` — for
/// tests that need to inspect an intermediate value (e.g. `inline-boxes`
/// ink) rather than a full page-broken document. Mirrors
/// `compile_document_v1_with_trials`'s own assembly (each dep lowered via
/// `v1::lower::lower_file_v1`, the entry via `v1::lower::lower_document_v1`,
/// merged into one synthetic `cst::File`) but skips `compile::compile_program`
/// /the fixpoint-eval/hook-firing loop entirely (irrelevant for a bare
/// expression with no page break in it) — a single direct
/// `eval::Interp::eval` call over the elaborated `Ast` suffices.
fn eval_v01_raw_value(
    files: &[LoadedFile],
    metrics: &dyn FontMetrics,
) -> Result<satysfi_lang::value::Value, String> {
    let (entry, deps) = files.split_last().expect("at least one file");
    fn as_v01(f: &LoadedFile) -> &satysfi_syntax::cst_v1::FileV1 {
        match &f.cst {
            LoadedCst::V0_1(cst) => cst,
            LoadedCst::V0_0_6(_) => unreachable!("this helper is V0_1-only"),
        }
    }
    let mut prelude = Vec::new();
    for dep in deps {
        prelude.extend(
            satysfi_lang::v1::lower::lower_file_v1(as_v01(dep)).map_err(|e| format!("lib lower: {e}"))?,
        );
    }
    let entry_cst = as_v01(entry);
    let body = satysfi_lang::v1::lower::lower_document_v1(entry_cst)
        .map_err(|e| format!("entry lower: {e}"))?;
    let eoi = match entry_cst {
        satysfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => unreachable!("lower_document_v1 already rejected a Library entry"),
    };
    let file = satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: Some(satysfi_syntax::leaf::KwIn(satysfi_syntax::Span::default())),
        body: Some(body),
        eoi,
    };
    let env0 = primitives::base_env_with_version(SatysfiVersion::V0_1);
    let scope = satysfi_lang::elaborate::Scope::new_with_version(env0.names(), SatysfiVersion::V0_1);
    let program =
        satysfi_lang::elaborate::elaborate_program(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    // No `:>` sealing in any fixture this helper serves, so the public
    // `typecheck_verbose_with_version` (ordinary inference, no sig-
    // subsumption pass) is sufficient — `v1::module_check::check_program`
    // is `pub(crate)`, unreachable from this external integration test.
    satysfi_lang::typecheck::typecheck_verbose_with_version(&program, SatysfiVersion::V0_1)
        .map_err(|e| format!("typecheck: {e}"))?;
    let mut interp = satysfi_lang::eval::Interp::new(metrics);
    interp.version = SatysfiVersion::V0_1;
    interp.eval(&env0, &program.body).map_err(|e| format!("eval: {e}"))
}

// ============================================================================
// Math-package completion M2: the `t_paren` 0.1 retype (`length -> length ->
// context -> inline-boxes * (length -> length)`).
// ============================================================================

/// T-M2-seal: `val paren-left : paren` seals against a real 0.1-shaped
/// `paren-left hgt dpt ctx = …` body — pins `typecheck.rs`'s
/// `name_to_mono("paren", V0_1)` case (§3.3) resolving structurally to
/// `t_paren(V0_1)`, matching the impl's own inferred type by construction.
#[test]
fn t_m2_paren_seals_against_the_v01_shape() {
    let lib = "\
module M :> sig
  val paren-left : paren
end = struct
  val paren-left hgt dpt ctx =
    let color = get-text-color ctx in
    let graphics (x, y) =
      fill color (start-path (x, y) |> line-to (x +' 3pt, y) |> close-with-line)
    in
    let kerninfo _ = 0pt in
    (inline-graphics 3pt hgt dpt graphics, kerninfo)
end
";
    let files = vec![
        lib_file_inline("m2-seal", lib),
        entry_file_inline("1"),
    ];
    let mono = Mono;
    match satysfi_lang::compile_document_v1(&files, &mono) {
        Ok(_) | Err(CompileError::NotADocument(_)) => {}
        Err(other) => panic!("T-M2-seal: expected acceptance, got: {other}"),
    }
}

/// T-M2-type/T-M2-eval combined: `math-paren ctx paren-left paren-right m`
/// with REAL 0.1-shaped closures (`hgt dpt ctx -> (inline-boxes, kernf)`,
/// §3.1) type-checks and evaluates through `embed-math`/`read-inline`, and
/// — pinning the BIGGEST M2 risk (§8 risk 1: forgetting `c2.font_size =
/// size` in `make_paren_run`'s V0_1 arm) — the closure route actually ran
/// (not the font-glyph fallback): the produced graphics contain `Fill` ink,
/// which `paren_variant_fallback` never emits.
#[test]
fn t_m2_paren_closure_route_draws_fill_ink() {
    use satysfi_backend::{GraphicsElem, HorzBox, PureHorzBox};
    use satysfi_lang::value::Value;

    let lib = "\
module M = struct
  val paren-left hgt dpt ctx =
    let color = get-text-color ctx in
    let graphics (x, y) =
      fill color (start-path (x, y) |> line-to (x +' 3pt, y) |> line-to (x +' 3pt, y +' 3pt) |> close-with-line)
    in
    let kerninfo _ = 0pt in
    (inline-graphics 3pt hgt dpt graphics, kerninfo)
  val paren-right hgt dpt ctx = paren-left hgt dpt ctx
end
";
    let entry = "\
@require: v01-mini
let open V01Mini in
let ctx = get-initial-context 200pt (command \\math) in
embed-math ctx (math-paren ctx M.paren-left M.paren-right (read-math ctx ${x}))
";
    let files = vec![v01_mini_file(), lib_file_inline("m2-eval", lib), entry_file_inline(entry)];
    let mono = Mono;
    let v = eval_v01_raw_value(&files, &mono)
        .unwrap_or_else(|e| panic!("T-M2-eval: expected evaluation to succeed, got: {e}"));
    let boxes = match v {
        Value::InlineBoxes(b) => b,
        other => panic!("expected inline-boxes, got {other:?}"),
    };
    let mut saw_fill = false;
    for b in &boxes {
        if let HorzBox::Pure(PureHorzBox::Math { rules, .. }) = b {
            if rules.iter().any(|r| matches!(r, GraphicsElem::Fill(..))) {
                saw_fill = true;
            }
        }
    }
    assert!(
        saw_fill,
        "expected the paren closure route to draw Fill ink (not the font-glyph fallback), got {boxes:?}"
    );
}

// ============================================================================
// Math-package completion M3: 9 -> 14 `math-char-class` constructors.
// ============================================================================

/// T-M3-type/T-M3-remap: `set-math-char-class MathSansSerif`/
/// `MathTypewriter` type-check and evaluate under V0_1, and
/// `convert-string-for-math` remaps `A`/`a` to the expected codepoints
/// (`math.rs`'s capitals/smalls offsets, §4.1).
#[test]
fn t_m3_new_char_classes_typecheck_and_remap() {
    let entry = "\
@require: v01-mini
let open V01Mini in
let ctx = get-initial-context 200pt (command \\math) in
let ctx-sans = ctx |> set-math-char-class MathSansSerif in
let ctx-tt = ctx |> set-math-char-class MathTypewriter in
let sans-a = convert-string-for-math ctx-sans MathSansSerif (string-unexplode [65]) in
let sans-a-lower = convert-string-for-math ctx-sans MathSansSerif (string-unexplode [97]) in
let tt-a = convert-string-for-math ctx-tt MathTypewriter (string-unexplode [65]) in
(string-explode sans-a, string-explode sans-a-lower, string-explode tt-a)
";
    let files = vec![v01_mini_file(), entry_file_inline(entry)];
    let mono = Mono;
    let v = eval_v01_raw_value(&files, &mono)
        .unwrap_or_else(|e| panic!("T-M3-remap: expected evaluation to succeed, got: {e}"));
    let satysfi_lang::value::Value::Tuple(parts) = v else {
        panic!("expected a 3-tuple");
    };
    assert_eq!(parts.len(), 3);
    fn first_cp(v: &satysfi_lang::value::Value) -> i64 {
        match v {
            satysfi_lang::value::Value::List(cps) => match &cps[0] {
                satysfi_lang::value::Value::Int(n) => *n,
                other => panic!("expected an int codepoint, got {other:?}"),
            },
            other => panic!("expected a codepoint list, got {other:?}"),
        }
    }
    assert_eq!(first_cp(&parts[0]), 0x1D5A0, "MathSansSerif capital A");
    assert_eq!(first_cp(&parts[1]), 0x1D5BA, "MathSansSerif lowercase a");
    assert_eq!(first_cp(&parts[2]), 0x1D670, "MathTypewriter capital A");
}

/// T-M3-frozen: under V0_0_6, `MathSansSerif` stays an unknown constructor
/// (pins the registration gate — the frozen 0.0.6 surface never learns the
/// 5 new names).
#[test]
fn t_m3_new_char_classes_are_unknown_under_v006() {
    let src = "let m = MathSansSerif in 0";
    let file = satysfi_syntax::parse_file(src).unwrap_or_else(|e| panic!("0.0.6 parse: {e}"));
    let env = primitives::base_env();
    let scope = satysfi_lang::elaborate::Scope::new(env.names());
    let elaborated = satysfi_lang::elaborate::elaborate_program(&file, &scope)
        .unwrap_or_else(|e| panic!("0.0.6 elaborate: {e}"));
    let err = satysfi_lang::typecheck::typecheck(&elaborated)
        .expect_err("'MathSansSerif' must stay an unknown constructor under V0_0_6");
    assert!(
        err.to_string().contains("MathSansSerif") || err.to_string().contains("unknown"),
        "{err}"
    );
}

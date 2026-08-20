//! L5a (`…/tmp/prim-retype-sweep.md` §4.1): the scalar/string/IO non-math
//! primitive slice — bitwise ops, Unicode string ops, `read-file`,
//! `register-document-information`, the `get-initial-text-info` R1 fork,
//! and the bare-constant audit. Harness: direct `Interp::apply` chains
//! against `base_env`/`base_env_with_version` (the `font_scheme.rs`/
//! `images.rs` pattern) — a full compile is overkill for pure-eval
//! primitives with no context/layout dependency.

use rustyfi_backend::{Context, DocInfo, FontKey, FontMetrics, HyphenLang, Length};
use rustyfi_lang::eval::Interp;
use rustyfi_lang::value::Value;
use rustyfi_lang::{prim_types, primitives, types};
use rustyfi_syntax::RustyfiVersion;
use std::collections::BTreeMap;

struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, _c: char, size: Length) -> Option<Length> {
        Some(size * 0.5)
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

fn some_str(s: &str) -> Value {
    Value::Ctor(
        "Some".to_string(),
        Some(Box::new(Value::Str(s.to_string()))),
    )
}

fn none() -> Value {
    Value::Ctor("None".to_string(), None)
}

/// Apply a named primitive (looked up in `env`) to `args`, left to right.
fn call(
    interp: &mut Interp,
    env: &rustyfi_lang::value::BaseEnv,
    name: &str,
    args: Vec<Value>,
) -> Value {
    let mut f = env
        .lookup(name)
        .unwrap_or_else(|| panic!("{name} is not bound"));
    for a in args {
        f = interp
            .apply(f, a)
            .unwrap_or_else(|e| panic!("{name} application failed: {e}"));
    }
    f
}

fn try_call(
    interp: &mut Interp,
    env: &rustyfi_lang::value::BaseEnv,
    name: &str,
    args: Vec<Value>,
) -> Result<Value, rustyfi_lang::eval::EvalError> {
    let mut f = env
        .lookup(name)
        .unwrap_or_else(|| panic!("{name} is not bound"));
    for a in args {
        f = interp.apply(f, a)?;
    }
    Ok(f)
}

fn as_int(v: Value) -> i64 {
    match v {
        Value::Int(n) => n,
        other => panic!("expected int, got {other:?}"),
    }
}

// `Value` derives no `PartialEq` (see its own doc comment), so every
// comparison below unwraps to a plain Rust type first.
fn as_string(v: Value) -> String {
    match v {
        Value::Str(s) => s,
        other => panic!("expected string, got {other:?}"),
    }
}

fn as_string_list(v: Value) -> Vec<String> {
    match v {
        Value::List(items) => items.into_iter().map(as_string).collect(),
        other => panic!("expected a list, got {other:?}"),
    }
}

// ============================================================================
// 1. Bitwise ops (A1-A6)
// ============================================================================

#[test]
fn bitwise_ops_evaluate() {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);

    assert_eq!(
        as_int(call(
            &mut interp,
            &env,
            "band",
            vec![Value::Int(6), Value::Int(3)]
        )),
        2
    );
    assert_eq!(
        as_int(call(
            &mut interp,
            &env,
            "bor",
            vec![Value::Int(6), Value::Int(3)]
        )),
        7
    );
    assert_eq!(
        as_int(call(
            &mut interp,
            &env,
            "bxor",
            vec![Value::Int(6), Value::Int(3)]
        )),
        5
    );
    assert_eq!(
        as_int(call(&mut interp, &env, "bnot", vec![Value::Int(0)])),
        -1
    );
    assert_eq!(
        as_int(call(
            &mut interp,
            &env,
            "<<",
            vec![Value::Int(1), Value::Int(4)]
        )),
        16
    );
    // Logical (not arithmetic) shift right — pins `lsr` semantics on a
    // negative operand: -16 as u64 >> 2.
    assert_eq!(
        as_int(call(
            &mut interp,
            &env,
            ">>",
            vec![Value::Int(-16), Value::Int(2)]
        )),
        4611686018427387900,
    );

    let err = try_call(&mut interp, &env, "<<", vec![Value::Int(1), Value::Int(64)]).unwrap_err();
    assert!(
        err.msg.contains("Bit offset out of bounds for '<<'"),
        "got: {}",
        err.msg
    );
    let err = try_call(&mut interp, &env, ">>", vec![Value::Int(1), Value::Int(-1)]).unwrap_err();
    assert!(
        err.msg.contains("Bit offset out of bounds for '>>'"),
        "got: {}",
        err.msg
    );
}

// ============================================================================
// 2/3. Unicode string ops (A7-A9)
// ============================================================================

#[test]
fn normalization_round_trips() {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);

    // "e" + COMBINING ACUTE ACCENT (U+0301) --NFC--> "é" (U+00E9).
    let decomposed = "e\u{0301}";
    let precomposed = "\u{00e9}";
    let nfc = as_string(call(
        &mut interp,
        &env,
        "normalize-string-to-nfc",
        vec![Value::Str(decomposed.to_string())],
    ));
    assert_eq!(nfc, precomposed);

    let nfd = as_string(call(
        &mut interp,
        &env,
        "normalize-string-to-nfd",
        vec![Value::Str(precomposed.to_string())],
    ));
    assert_eq!(nfd, decomposed);

    // Hangul syllable GA (U+AC00) decomposes to two jamo, U+1100 U+1161.
    let composed = "\u{ac00}";
    let jamo = "\u{1100}\u{1161}";
    let d = as_string(call(
        &mut interp,
        &env,
        "normalize-string-to-nfd",
        vec![Value::Str(composed.to_string())],
    ));
    assert_eq!(d, jamo);
    let c = as_string(call(
        &mut interp,
        &env,
        "normalize-string-to-nfc",
        vec![Value::Str(jamo.to_string())],
    ));
    assert_eq!(c, composed);

    // NFC is idempotent: NFC(NFC(x)) == NFC(x).
    let twice = as_string(call(
        &mut interp,
        &env,
        "normalize-string-to-nfc",
        vec![Value::Str(nfc.clone())],
    ));
    assert_eq!(twice, nfc);
}

#[test]
fn grapheme_split_uax29() {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);

    // "a" + COMBINING ACUTE ACCENT + "b" -> 2 extended grapheme clusters.
    let v = call(
        &mut interp,
        &env,
        "split-grapheme-cluster",
        vec![Value::Str("a\u{0301}b".to_string())],
    );
    let items = as_string_list(v);
    assert_eq!(items, vec!["a\u{0301}".to_string(), "b".to_string()]);

    // Family emoji (man, woman, girl, boy joined by ZWJ) is ONE extended
    // grapheme cluster.
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let v = call(
        &mut interp,
        &env,
        "split-grapheme-cluster",
        vec![Value::Str(family.to_string())],
    );
    let items = as_string_list(v);
    assert_eq!(items.len(), 1, "expected 1 cluster, got {items:?}");

    // Empty string -> [].
    let v = call(
        &mut interp,
        &env,
        "split-grapheme-cluster",
        vec![Value::Str(String::new())],
    );
    let items = as_string_list(v);
    assert!(items.is_empty(), "expected [], got {items:?}");
}

// ============================================================================
// 4. `read-file` (A10)
// ============================================================================

#[test]
fn read_file_reads_lines() {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);

    // Fixture: `a\nb\r\nc\n` — keep `\r`, drop the trailing empty piece.
    let v = call(
        &mut interp,
        &env,
        "read-file",
        vec![Value::Str(
            "tests/fixtures/read-file-sample.txt".to_string(),
        )],
    );
    let strs = as_string_list(v);
    assert_eq!(
        strs,
        vec!["a".to_string(), "b\r".to_string(), "c".to_string()]
    );

    // `..` path components are rejected with upstream's exact message.
    let err = try_call(
        &mut interp,
        &env,
        "read-file",
        vec![Value::Str(
            "tests/fixtures/../fixtures/read-file-sample.txt".to_string(),
        )],
    )
    .unwrap_err();
    assert!(
        err.msg.contains("cannot access files by using '..'"),
        "got: {}",
        err.msg
    );

    // A missing file is a clean `EvalError`, not a panic.
    let err = try_call(
        &mut interp,
        &env,
        "read-file",
        vec![Value::Str("tests/fixtures/does-not-exist.txt".to_string())],
    )
    .unwrap_err();
    assert!(err.msg.contains("read-file"), "got: {}", err.msg);
}

// ============================================================================
// 5. `register-document-information` (A11)
// ============================================================================

fn doc_info_record(title: Value, subject: Value, author: Value, keywords: Vec<&str>) -> Value {
    let mut fields = BTreeMap::new();
    fields.insert("title".to_string(), title);
    fields.insert("subject".to_string(), subject);
    fields.insert("author".to_string(), author);
    fields.insert(
        "keywords".to_string(),
        Value::List(
            keywords
                .into_iter()
                .map(|s| Value::Str(s.to_string()))
                .collect(),
        ),
    );
    Value::Record(fields)
}

#[test]
fn register_document_information_registers() {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);

    assert!(interp.doc_info.is_none());

    let rec = doc_info_record(
        some_str("Title One"),
        none(),
        some_str("Author One"),
        vec!["a", "b"],
    );
    call(
        &mut interp,
        &env,
        "register-document-information",
        vec![rec],
    );
    assert_eq!(
        interp.doc_info,
        Some(DocInfo {
            title: Some("Title One".to_string()),
            subject: None,
            author: Some("Author One".to_string()),
            keywords: vec!["a".to_string(), "b".to_string()],
        })
    );

    // Last write wins.
    let rec2 = doc_info_record(some_str("Title Two"), some_str("Subj"), none(), vec![]);
    call(
        &mut interp,
        &env,
        "register-document-information",
        vec![rec2],
    );
    assert_eq!(
        interp.doc_info,
        Some(DocInfo {
            title: Some("Title Two".to_string()),
            subject: Some("Subj".to_string()),
            author: None,
            keywords: vec![],
        })
    );
}

// ============================================================================
// 6. `get-initial-text-info` — the R1 fork
// ============================================================================

#[test]
fn get_initial_text_info_forks() {
    // V0_0: `unit -> text-info` (unchanged).
    let v006_poly = prim_types::primitive_type("get-initial-text-info").unwrap();
    let v006_mono = types::instantiate(&v006_poly, 0);
    match v006_mono {
        types::MonoType::Func(_, _, cod) => {
            assert!(
                !matches!(*cod, types::MonoType::Func(..)),
                "v0.0.6 side must be 1-ary"
            );
        }
        other => panic!("expected a function type, got {other:?}"),
    }

    // V0_1: 2-ary (`inline [math-text] -> (…) -> text-info`).
    let v01_poly =
        prim_types::primitive_type_with_version("get-initial-text-info", RustyfiVersion::V0_1)
            .unwrap();
    let v01_mono = types::instantiate(&v01_poly, 0);
    match v01_mono {
        types::MonoType::Func(_, _, cod1) => match *cod1 {
            types::MonoType::Func(_, _, cod2) => {
                assert!(
                    !matches!(*cod2, types::MonoType::Func(..)),
                    "v0.1 side must be exactly 2-ary"
                );
            }
            other => panic!("expected a 2nd arrow, got {other:?}"),
        },
        other => panic!("expected a function type, got {other:?}"),
    }

    // V0_1 eval: apply 2 dummy args, get `TextInfo{indent: 0}` back.
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let env01 = primitives::base_env_with_version(RustyfiVersion::V0_1);
    let v = call(
        &mut interp,
        &env01,
        "get-initial-text-info",
        vec![Value::Unit, Value::Unit],
    );
    match v {
        Value::TextInfo(ti) => assert_eq!(ti.indent, 0),
        other => panic!("expected a text-info, got {other:?}"),
    }

    // V0_0 eval: unchanged 1-arg behavior.
    let env006 = primitives::base_env();
    let v = call(
        &mut interp,
        &env006,
        "get-initial-text-info",
        vec![Value::Unit],
    );
    match v {
        Value::TextInfo(ti) => assert_eq!(ti.indent, 0),
        other => panic!("expected a text-info, got {other:?}"),
    }
}

// ============================================================================
// 7. Version-gating: the 11 added names are unbound under V0_0
// ============================================================================

#[test]
fn added_prims_gate_by_version() {
    const NAMES: &[&str] = &[
        "<<",
        ">>",
        "band",
        "bor",
        "bxor",
        "bnot",
        "normalize-string-to-nfc",
        "normalize-string-to-nfd",
        "split-grapheme-cluster",
        "read-file",
        "register-document-information",
    ];
    let env006 = primitives::base_env();
    let env01 = primitives::base_env_with_version(RustyfiVersion::V0_1);
    for name in NAMES {
        assert!(
            env006.lookup(name).is_none(),
            "{name} must be unbound under V0_0"
        );
        assert!(
            prim_types::primitive_type(name).is_none(),
            "{name} must have no type under V0_0"
        );
        assert!(
            env01.lookup(name).is_some(),
            "{name} must be bound under V0_1"
        );
        assert!(
            prim_types::primitive_type_with_version(name, RustyfiVersion::V0_1).is_some(),
            "{name} must have a type under V0_1"
        );
    }
}

// ============================================================================
// 9. The bare-constant audit (§1.4)
// ============================================================================

#[test]
fn bare_constants_bound_under_v01() {
    let env01 = primitives::base_env_with_version(RustyfiVersion::V0_1);
    for name in [
        "inline-fil",
        "inline-nil",
        "block-nil",
        "omit-skip-after",
        "clear-page",
    ] {
        assert!(
            env01.lookup(name).is_some(),
            "{name} must stay bound under V0_1"
        );
    }
}

// ============================================================================
// 10. G6 (`…/tmp/g6-g7-standins.md` §5.2) — `load-unicode-char-database`/
// `set-unicode-char-database` are still ACCEPT-AND-RETURN stand-ins (never
// a `stringify-math`-style hard error), `here` resolves to the empty
// string, and `load-hyphenation-dictionary`/`set-hyphenation-dictionary`
// are now REAL (S1) — no longer no-ops.
// ============================================================================

fn as_context(v: Value) -> Context {
    match v {
        Value::Context(c) => *c,
        other => panic!("expected a context, got {other:?}"),
    }
}

#[test]
fn load_hyphenation_dictionary_parses_a_known_name_into_a_hyphen_lang_tag() {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);
    let v = call(
        &mut interp,
        &env,
        "load-hyphenation-dictionary",
        vec![Value::Str("english".to_string())],
    );
    assert!(
        matches!(v, Value::Hyphenation(HyphenLang::EnglishUS)),
        "expected Value::Hyphenation(EnglishUS), got {v:?}"
    );
}

#[test]
fn load_hyphenation_dictionary_also_accepts_the_real_stdlib_path_form() {
    // The real, vendored `hyph-english.satyh` stand-in package calls this
    // with `here ^ "/../hyph/english.rustyfi-hyph"` (an upstream-style
    // PATH, not a bare name) — `std-ja`/`std-ja-book`/`std-ja-report`/
    // `md-ja`'s `get-standard-context` all route through it, so this must
    // resolve to the same tag or every one of those real doc classes'
    // capstone e2e tests fails at `set-hyphenation-dictionary` time.
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);
    let v = call(
        &mut interp,
        &env,
        "load-hyphenation-dictionary",
        vec![Value::Str("/../hyph/english.rustyfi-hyph".to_string())],
    );
    assert!(
        matches!(v, Value::Hyphenation(HyphenLang::EnglishUS)),
        "expected Value::Hyphenation(EnglishUS), got {v:?}"
    );
}

#[test]
fn load_hyphenation_dictionary_parses_british_names_into_english_gb_tag() {
    // en-GB (en-GB option): all three accepted spellings must resolve to
    // the same tag as "english"/"en-US" does to `HyphenLang::EnglishUS`
    // above.
    let mono = Mono;
    for name in ["british", "en-GB", "british-english"] {
        let mut interp = Interp::new(&mono);
        let env = primitives::base_env_with_version(RustyfiVersion::V0_1);
        let v = call(
            &mut interp,
            &env,
            "load-hyphenation-dictionary",
            vec![Value::Str(name.to_string())],
        );
        assert!(
            matches!(v, Value::Hyphenation(HyphenLang::EnglishGB)),
            "{name:?}: expected Value::Hyphenation(EnglishGB), got {v:?}"
        );
    }
}

#[test]
fn load_hyphenation_dictionary_errors_on_an_unknown_name() {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);
    let result = try_call(
        &mut interp,
        &env,
        "load-hyphenation-dictionary",
        vec![Value::Str("klingon".to_string())],
    );
    assert!(
        result.is_err(),
        "expected an error for an unrecognized dictionary name, got {result:?}"
    );
}

#[test]
fn load_unicode_char_database_accepts_and_returns_unit() {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);
    let v = call(
        &mut interp,
        &env,
        "load-unicode-char-database",
        vec![
            Value::Str("Scripts.txt".to_string()),
            Value::Str("EastAsianWidth.txt".to_string()),
            Value::Str("LineBreak.txt".to_string()),
        ],
    );
    assert!(matches!(v, Value::Unit), "expected Value::Unit, got {v:?}");
}

#[test]
fn set_hyphenation_dictionary_is_real_but_set_unicode_char_database_is_still_a_no_op() {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);

    let ctx0 = as_context(call(
        &mut interp,
        &env,
        "get-initial-context",
        vec![Value::Length(Length::pt(100.0)), Value::Unit],
    ));
    // Upstream loads `english.satysfi-hyph` into `default_hyphen_dictionary` at
    // startup and gives it to every initial context (`primitives.cppo.ml:500,607`),
    // so English is the default here too. (This asserted `None` while the port
    // held hyphenation opt-in behind the D4 byte-identity gate.)
    assert_eq!(
        ctx0.hyphen_dictionary,
        Some(rustyfi_backend::HyphenLang::EnglishUS),
        "Context::initial must default to English, matching upstream"
    );

    // `set-hyphenation-dictionary` (S1) is now REAL: it writes
    // `Context::hyphen_dictionary`, no longer a no-op.
    let ctx1 = as_context(call(
        &mut interp,
        &env,
        "set-hyphenation-dictionary",
        vec![
            Value::Hyphenation(HyphenLang::EnglishUS),
            Value::Context(Box::new(ctx0.clone())),
        ],
    ));
    assert_eq!(ctx1.hyphen_dictionary, Some(HyphenLang::EnglishUS));
    assert_eq!(
        Context {
            hyphen_dictionary: ctx0.hyphen_dictionary,
            ..ctx1.clone()
        },
        ctx0,
        "every other field must be unchanged"
    );

    // `set-unicode-char-database` is untouched by this slice — still a
    // no-op.
    let ctx2 = as_context(call(
        &mut interp,
        &env,
        "set-unicode-char-database",
        vec![Value::Unit, Value::Context(Box::new(ctx0.clone()))],
    ));
    assert_eq!(
        ctx2, ctx0,
        "set-unicode-char-database must return ctx unchanged"
    );
}

#[test]
fn here_resolves_to_the_empty_string() {
    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);
    let v = env.lookup("here").expect("'here' must be bound under V0_1");
    assert_eq!(as_string(v), "");
}

use rustyfi_backend::{FontKey, FontMetrics, Length, PureHorzBox, VertBox};
use rustyfi_lang::value::{DocumentValue, Value};
use rustyfi_lang::{compile_document_cst, elaborate, eval, primitives};
use std::path::Path;
use std::rc::Rc;

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

fn eval_str(src: &str) -> Result<Value, rustyfi_lang::CompileError> {
    let file = rustyfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let ast = elaborate::elaborate(&file, &scope)?;
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    Ok(interp.eval(&env, &rustyfi_lang::ast::debrand(&ast, &store))?)
}

/// `document`/`+p`/`\emph` are no longer hardcoded Rust natives (phase 4):
/// they're now ordinary bindings in the real `stdja-mini` stdlib package
/// (`lib-rustyfi/dist/packages/stdja-mini.satyh`). Tests below that need
/// them compile `src` the same way the multi-file loader's `merge_program`
/// does — concatenate the package's prelude ahead of `src`'s own — rather
/// than pulling in the whole loader crate for a single-file test.
fn compile_document_with_stdlib(
    src: &str,
    metrics: &dyn rustyfi_backend::FontMetrics,
) -> Result<Rc<DocumentValue>, rustyfi_lang::CompileError> {
    let lib_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist/packages/stdja-mini.satyh");
    let lib_src = std::fs::read_to_string(&lib_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", lib_path.display()));
    let lib_file = rustyfi_syntax::parse_file(&lib_src)?;
    let doc_file = rustyfi_syntax::parse_file(src)?;

    let mut prelude = lib_file.prelude;
    prelude.extend(doc_file.prelude);
    let merged = rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: doc_file.in_kw,
        body: doc_file.body,
        eoi: doc_file.eoi,
    };
    compile_document_cst(&merged, metrics)
}

#[test]
fn arithmetic_free_basics() {
    assert!(matches!(eval_str("42").unwrap(), Value::Int(42)));
    assert!(matches!(eval_str("`s`").unwrap(), Value::Str(s) if s == "s"));
    assert!(matches!(eval_str("true").unwrap(), Value::Bool(true)));
    let Value::Length(l) = eval_str("10pt").unwrap() else {
        panic!()
    };
    assert_eq!(l, Length::pt(10.0));
}

#[test]
fn functions_and_lets() {
    let v = eval_str("let id = fun x -> x in id 7").unwrap();
    assert!(matches!(v, Value::Int(7)));
    let v = eval_str("let const a b = a in const 1 2").unwrap();
    assert!(matches!(v, Value::Int(1)));
    let v = eval_str("let x = 1\nlet y = x\nin y").unwrap();
    assert!(matches!(v, Value::Int(1)));
}

#[test]
fn records_and_lists() {
    let Value::Record(r) = eval_str("(| a = 1; b = `x` |)").unwrap() else {
        panic!()
    };
    assert_eq!(r.len(), 2);
    let Value::List(l) = eval_str("[1; 2; 3]").unwrap() else {
        panic!()
    };
    assert_eq!(l.len(), 3);
}

#[test]
fn unbound_variable_is_an_elab_error() {
    let err = eval_str("nope").unwrap_err();
    assert!(err.to_string().contains("unbound variable 'nope'"));
    let err = eval_str("{ \\nope{x} }").unwrap_err();
    assert!(err.to_string().contains("nope"));
}

#[test]
fn inline_text_quotes_until_read() {
    let v = eval_str("{ hello }").unwrap();
    assert!(matches!(v, Value::InlineText { .. }));
}

#[test]
fn document_produces_pages_with_lines() {
    let doc =
        compile_document_with_stdlib("document (| title = {T} |) '< +p { hello world } >", &Mono)
            .unwrap();
    assert_eq!(doc.pages.len(), 1);
    let line = &doc.pages[0].lines[0];
    let words: Vec<&str> = line
        .contents
        .iter()
        .filter_map(|(_, b)| match b {
            PureHorzBox::InnerString { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(words, vec!["hello", "world"]);
}

#[test]
fn emph_switches_font() {
    let doc = compile_document_with_stdlib("document (||) '< +p { a \\emph{b} } >", &Mono).unwrap();
    let line = &doc.pages[0].lines[0];
    let fonts: Vec<u16> = line
        .contents
        .iter()
        .filter_map(|(_, b)| match b {
            PureHorzBox::InnerString { info, .. } => Some(info.font.0),
            _ => None,
        })
        .collect();
    assert_eq!(fonts, vec![0, 2], "emph must run in the oblique face");
}

#[test]
fn long_paragraph_wraps() {
    let mut para = String::new();
    for i in 0..80 {
        para.push_str(&format!("word{i} "));
    }
    let src = format!("document (||) '< +p {{ {para} }} >");
    let doc = compile_document_with_stdlib(&src, &Mono).unwrap();
    assert!(
        doc.pages[0].lines.len() > 1,
        "80 words at 12pt monospace must wrap"
    );
}

#[test]
fn eval_matches_block_boxes_shape() {
    // +p through the public primitives: one paragraph = at least one Line.
    let doc = compile_document_with_stdlib("document (||) '< +p { x } +p { y } >", &Mono).unwrap();
    assert!(doc.pages[0].lines.len() >= 2);
    let _shape_check: Vec<VertBox> = Vec::new();
}

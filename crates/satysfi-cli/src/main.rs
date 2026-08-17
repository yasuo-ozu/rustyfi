use anyhow::Context as _;
use clap::Parser;
use std::path::PathBuf;

/// Compile a SATySFi (.saty) document to PDF.
#[derive(Parser)]
#[command(name = "satysfi-rust", version)]
struct Args {
    /// Input .saty file.
    input: PathBuf,

    /// Output PDF path (defaults to the input with a .pdf extension).
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Library root for `@require:` resolution (packages under
    /// `<lib-root>/dist/packages/`). Falls back to $SATYSFI_LIB_ROOT, then
    /// to the nearest `lib-satysfi/` directory found by searching upward
    /// from the input file's own directory (so running this from anywhere
    /// inside a checkout that has a top-level `lib-satysfi/`, e.g. this
    /// repo, needs no flag or environment variable at all).
    #[arg(long)]
    lib_root: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let output = args
        .output
        .unwrap_or_else(|| args.input.with_extension("pdf"));
    let lib_root = args
        .lib_root
        .or_else(|| std::env::var_os("SATYSFI_LIB_ROOT").map(PathBuf::from))
        .or_else(|| discover_lib_root(&args.input));

    let program = satysfi_loader::load(&args.input, &satysfi_loader::LoadOptions { lib_root })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let merged = merge_program(program);

    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document_cst(&merged, &metrics)
        .map_err(|e| anyhow::anyhow!("{}: {e}", args.input.display()))?;

    let bytes = satysfi_pdf::render_pdf(&doc.geometry, &doc.pages)?;
    std::fs::write(&output, bytes)
        .with_context(|| format!("cannot write {}", output.display()))?;

    let line_count: usize = doc.pages.iter().map(|p| p.lines.len()).sum();
    eprintln!(
        " ---- ---- ---- ----\n  output written on {} ({} page(s), {} line(s)).",
        output.display(),
        doc.pages.len(),
        line_count
    );
    Ok(())
}

/// The CLI's default `--lib-root` rule (used only when neither `--lib-root`
/// nor `$SATYSFI_LIB_ROOT` is given): starting at `input`'s own directory,
/// walk upward through its ancestors looking for a `lib-satysfi/`
/// subdirectory, returning the first one found. This is the simplest rule
/// that makes `satysfi-rust some/nested/doc.saty` "just work" from anywhere
/// inside a checkout that has one top-level `lib-satysfi/` (this repo
/// included), with no flag or environment variable needed, while still
/// resolving relative to the *document*, not the current working directory
/// (so it behaves the same regardless of where the command is run from).
fn discover_lib_root(input: &std::path::Path) -> Option<PathBuf> {
    let mut dir = input.parent()?.to_path_buf();
    loop {
        let candidate = dir.join("lib-satysfi");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Concatenate the dependency-ordered library preludes ahead of the entry
/// document's own prelude, producing one synthetic file for elaboration.
/// (The v0.0.6 analog type-checks each library into a shared environment in
/// dependency order; untyped elaboration gets the same scoping by prelude
/// concatenation.)
fn merge_program(program: satysfi_loader::LoadedProgram) -> satysfi_syntax::cst::File {
    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(lib.cst.prelude);
    }
    prelude.extend(entry.cst.prelude);
    satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry.cst.in_kw,
        body: entry.cst.body,
        eoi: entry.cst.eoi,
    }
}

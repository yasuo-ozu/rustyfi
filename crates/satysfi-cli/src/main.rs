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
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let output = args
        .output
        .unwrap_or_else(|| args.input.with_extension("pdf"));

    let src = std::fs::read_to_string(&args.input)
        .with_context(|| format!("cannot read {}", args.input.display()))?;

    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document(&src, &metrics)
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

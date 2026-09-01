import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import { findBinary } from './binary';
import { run } from './run';
import { parseBuildDiagnostics, unlocated } from './core/buildDiagnostics';

/**
 * `rustyfi.build` / `rustyfi.buildAndOpen`.
 *
 * A build is not the preview, and the differences are deliberate:
 *
 *   * it compiles the file ON DISK, so the PDF lands where the author expects
 *     and `@import:` resolves exactly as it will for anybody else;
 *   * the output is the compiler's own default path -- no `-o`, so this
 *     command holds no opinion the CLI does not already hold;
 *   * failures become DIAGNOSTICS in the Problems panel, which is this
 *     editor's answer to the vim side's quickfix list.
 *
 * The document is saved first. A build command that silently compiles
 * yesterday's bytes defeats its own purpose; `rustyfi.build.autoSave`
 * restores the cautious behaviour, and then a dirty document refuses.
 */

let diagnostics: vscode.DiagnosticCollection | undefined;
let inflight = false;

export function collection(): vscode.DiagnosticCollection {
  if (!diagnostics) {
    diagnostics = vscode.languages.createDiagnosticCollection('rustyfi-build');
  }
  return diagnostics;
}

export function dispose(): void {
  diagnostics?.dispose();
  diagnostics = undefined;
}

/** Where the compiler will write: alongside the document, same stem. */
export function outputPath(doc: vscode.TextDocument): string {
  const p = doc.uri.fsPath;
  return path.join(path.dirname(p), path.basename(p, path.extname(p)) + '.pdf');
}

export async function build(out: vscode.OutputChannel, open: boolean): Promise<void> {
  const ed = vscode.window.activeTextEditor;
  if (!ed || ed.document.languageId !== 'satysfi') {
    vscode.window.showInformationMessage('Open a SATySFi document first.');
    return;
  }
  const doc = ed.document;
  if (doc.uri.scheme !== 'file') {
    vscode.window.showErrorMessage('This document has no file on disk to build.');
    return;
  }
  if (inflight) {
    vscode.window.showInformationMessage('A build is already running.');
    return;
  }

  const cfg = vscode.workspace.getConfiguration('rustyfi');
  if (doc.isDirty) {
    if (!cfg.get<boolean>('build.autoSave', true)) {
      vscode.window.showErrorMessage(
        'The document has unsaved changes. Save it, or enable rustyfi.build.autoSave.',
      );
      return;
    }
    if (!(await doc.save())) {
      vscode.window.showErrorMessage('Could not save the document.');
      return;
    }
  }

  const bin = findBinary(out);
  if (!bin) return;

  const src = doc.uri.fsPath;
  const pdf = outputPath(doc);
  const args = [src];
  const libRoot = cfg.get<string>('libRoot', '');
  if (libRoot && libRoot.trim()) args.push('--lib-root', libRoot.trim());

  inflight = true;
  const started = Date.now();
  try {
    const res = await vscode.window.withProgress(
      {
        location: vscode.ProgressLocation.Window,
        title: `Building ${path.basename(src)}…`,
      },
      () =>
        run(bin.path, args, {
          cwd: path.dirname(src),
          timeoutMs: cfg.get<number>('build.timeout', 120000),
        }).result,
    );

    const ms = Date.now() - started;
    const text = [res.stderr, res.stdout].filter(Boolean).join('\n');

    if (res.code === 0) {
      // Clear on success: leaving the previous failure in the Problems panel
      // is how you chase an error you already fixed.
      collection().clear();
      out.appendLine(`[rustyfi] built ${pdf} in ${ms}ms`);
      if (open) await openExternally(pdf, out);
      else vscode.window.setStatusBarMessage(`$(check) Built ${path.basename(pdf)}`, 4000);
      return;
    }

    report(doc, text, out);
    vscode.window.showErrorMessage(
      `Build failed (exit ${res.code}). See the Problems panel.`,
    );
  } catch (e) {
    out.appendLine(`[rustyfi] build could not run: ${e}`);
    vscode.window.showErrorMessage(`Could not run rustyfi: ${e}`);
  } finally {
    inflight = false;
  }
}

/**
 * Turn compiler output into diagnostics, grouped by the file each names.
 *
 * A diagnostic can name a DEPENDENCY rather than the document being built --
 * the error may be in an `@import:`ed library -- so entries are grouped by
 * their own path instead of all being pinned to the active editor.
 */
function report(doc: vscode.TextDocument, text: string, out: vscode.OutputChannel): void {
  const col = collection();
  col.clear();

  const byFile = new Map<string, vscode.Diagnostic[]>();
  for (const d of parseBuildDiagnostics(text)) {
    // The compiler counts from 1 on both axes; VS Code counts from 0.
    const line = Math.max(0, d.line - 1);
    const from = Math.max(0, d.colStart - 1);
    const to = Math.max(from + 1, d.colEnd - 1);
    const diag = new vscode.Diagnostic(
      new vscode.Range(line, from, line, to),
      d.message,
      vscode.DiagnosticSeverity.Error,
    );
    diag.source = 'rustyfi build';
    const key = path.isAbsolute(d.file) ? d.file : path.resolve(path.dirname(doc.uri.fsPath), d.file);
    const list = byFile.get(key) ?? [];
    list.push(diag);
    byFile.set(key, list);
  }
  for (const [file, list] of byFile) col.set(vscode.Uri.file(file), list);

  // A failure with no located diagnostic -- an unresolvable `@require:`, a
  // missing library root -- still has to be visible. Pin it to line 1 of the
  // document being built rather than losing it to the output channel.
  const rest = unlocated(text);
  if (byFile.size === 0 && rest.length > 0) {
    const diag = new vscode.Diagnostic(
      new vscode.Range(0, 0, 0, 1),
      rest.join('\n'),
      vscode.DiagnosticSeverity.Error,
    );
    diag.source = 'rustyfi build';
    col.set(doc.uri, [diag]);
  }
  for (const line of rest) out.appendLine(`[rustyfi] ${line}`);
}

async function openExternally(pdf: string, out: vscode.OutputChannel): Promise<void> {
  if (!fs.existsSync(pdf)) {
    vscode.window.showErrorMessage(`The build reported success but ${pdf} is not there.`);
    return;
  }
  // The OS handler, not a VS Code tab: this command exists to hand the PDF
  // to a real viewer. `rustyfi.preview.format = pdf` is the in-editor route.
  const opened = await vscode.env.openExternal(vscode.Uri.file(pdf));
  if (!opened) {
    out.appendLine(`[rustyfi] the system declined to open ${pdf}`);
    vscode.window.showWarningMessage(`Could not open ${path.basename(pdf)} externally.`);
  }
}

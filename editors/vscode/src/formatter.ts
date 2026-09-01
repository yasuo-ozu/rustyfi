import * as vscode from 'vscode';
import * as path from 'path';
import { buildFormatArgs, type FormatSettings } from './core/fmtOptions';
import { decideFormat } from './core/fmtResult';
import { findBinary } from './binary';
import { run } from './run';

function settings(): FormatSettings {
  const c = vscode.workspace.getConfiguration('rustyfi.format');
  return {
    lang: c.get<string>('lang', 'auto'),
    maxWidth: c.get<number | null>('maxWidth', null),
    tabSpaces: c.get<number | null>('tabSpaces', null),
    maxBlankLines: c.get<number | null>('maxBlankLines', null),
    wrapComments: c.get<boolean | null>('wrapComments', null),
    wrapInlineText: c.get<boolean | null>('wrapInlineText', null),
  };
}

const warned = new Set<string>();

export class RustyfiFormatter implements vscode.DocumentFormattingEditProvider {
  constructor(private readonly out: vscode.OutputChannel) {}

  async provideDocumentFormattingEdits(
    doc: vscode.TextDocument,
    _options: vscode.FormattingOptions,
    token: vscode.CancellationToken,
  ): Promise<vscode.TextEdit[]> {
    const bin = findBinary(this.out);
    if (!bin) return [];

    const { args, warnings } = buildFormatArgs(settings());
    for (const w of warnings) {
      this.out.appendLine(`[rustyfi] ${w}`);
      if (!warned.has(w)) { warned.add(w); vscode.window.showWarningMessage(w); }
    }

    // The DOCUMENT TEXT goes to stdin, not the path on disk -- so formatting
    // works on an unsaved buffer and always matches what is on screen.
    const original = doc.getText();
    const cwd = doc.uri.scheme === 'file' ? path.dirname(doc.uri.fsPath) : undefined;

    const handle = run(bin.path, args, { cwd, stdin: original, timeoutMs: 15000 });
    const sub = token.onCancellationRequested(() => handle.cancel());

    let res;
    try {
      res = await handle.result;
    } catch (e) {
      this.out.appendLine(`[rustyfi] format failed to spawn: ${e}`);
      vscode.window.showErrorMessage(`rustyfi fmt could not run: ${e}`);
      return [];
    } finally {
      sub.dispose();
    }

    if (token.isCancellationRequested) return [];

    const decision = decideFormat(res, original);

    switch (decision.kind) {
      case 'unchanged':
        return [];

      case 'decline':
        // The whole point: return NO EDIT.  The buffer is left exactly as the
        // user typed it, and the reason goes to the status bar rather than a
        // modal, because a document being typed into declines constantly.
        this.out.appendLine(`[rustyfi] ${decision.reason} ${decision.detail}`);
        vscode.window.setStatusBarMessage(`$(circle-slash) ${decision.reason}`, 5000);
        return [];

      case 'error':
        this.out.appendLine(`[rustyfi] ${decision.reason} ${decision.detail}`);
        vscode.window.showErrorMessage(
          `${decision.reason}${decision.detail ? ' — ' + decision.detail : ''}`,
        );
        return [];

      case 'apply': {
        const full = new vscode.Range(
          doc.positionAt(0),
          doc.positionAt(original.length),
        );
        return [vscode.TextEdit.replace(full, decision.text)];
      }
    }
  }
}

import * as vscode from 'vscode';
import { RustyfiFormatter } from './formatter';
import { Preview } from './preview';
import * as lsp from './lsp';
import { resetBinaryWarning } from './binary';

const SELECTOR: vscode.DocumentSelector = { scheme: 'file', language: 'satysfi' };

let output: vscode.OutputChannel;
let formatterReg: vscode.Disposable | undefined;

function applyFormatterRegistration(ctx: vscode.ExtensionContext): void {
  formatterReg?.dispose();
  formatterReg = undefined;

  const provider = vscode.workspace
    .getConfiguration('rustyfi')
    .get<string>('format.provider', 'cli');

  if (provider === 'cli') {
    formatterReg = vscode.languages.registerDocumentFormattingEditProvider(
      SELECTOR,
      new RustyfiFormatter(output),
    );
    ctx.subscriptions.push(formatterReg);
  }
}

export function activate(ctx: vscode.ExtensionContext): void {
  output = vscode.window.createOutputChannel('rustyfi');
  ctx.subscriptions.push(output);

  applyFormatterRegistration(ctx);
  void lsp.start(output);

  ctx.subscriptions.push(
    vscode.commands.registerCommand('rustyfi.showPreview', () => {
      const ed = vscode.window.activeTextEditor;
      if (!ed || ed.document.languageId !== 'satysfi') {
        vscode.window.showInformationMessage('Open a SATySFi document first.');
        return;
      }
      Preview.show(ed.document, output);
    }),

    vscode.commands.registerCommand('rustyfi.refreshPreview', () => {
      const ed = vscode.window.activeTextEditor;
      const p = ed ? Preview.forDocument(ed.document) : undefined;
      if (p) p.refresh();
      else vscode.window.showInformationMessage('No rustyfi preview is open for this document.');
    }),

    vscode.commands.registerCommand('rustyfi.restartServer', async () => {
      resetBinaryWarning();
      await lsp.start(output);
      vscode.window.setStatusBarMessage('$(check) rustyfi language server restarted', 3000);
    }),

    vscode.commands.registerCommand('rustyfi.showOutput', () => output.show(true)),

    vscode.workspace.onDidChangeConfiguration(async (e) => {
      if (e.affectsConfiguration('rustyfi.format.provider')) {
        applyFormatterRegistration(ctx);
        // The server's formatting feature is suppressed at client-construction
        // time, so switching provider has to restart the client.
        await lsp.start(output);
      } else if (
        e.affectsConfiguration('rustyfi.serverPath') ||
        e.affectsConfiguration('rustyfi.lsp.enable')
      ) {
        resetBinaryWarning();
        await lsp.start(output);
      }
    }),

    new vscode.Disposable(() => Preview.disposeAll()),
  );
}

export async function deactivate(): Promise<void> {
  Preview.disposeAll();
  await lsp.stop();
}

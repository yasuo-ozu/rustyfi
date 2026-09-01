import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import { discoverBinary, configuredButMissing, type DiscoveryInput, type Discovered } from './core/discovery';

function isExecutable(p: string): boolean {
  try {
    const st = fs.statSync(p);
    if (!st.isFile()) return false;
    fs.accessSync(p, fs.constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function input(): DiscoveryInput {
  const cfg = vscode.workspace.getConfiguration('rustyfi');
  const exeName = process.platform === 'win32' ? 'rustyfi.exe' : 'rustyfi';
  // The active document's directory, when it is a real file on disk. This is
  // what makes `code path/to/doc.saty` -- no folder open at all -- resolve a
  // binary sitting in a checkout above that file.
  const doc = vscode.window.activeTextEditor?.document;
  const documentDir =
    doc && doc.uri.scheme === 'file' ? path.dirname(doc.uri.fsPath) : null;
  return {
    configured: cfg.get<string>('serverPath', ''),
    workspaceFolders: (vscode.workspace.workspaceFolders ?? []).map((f) => f.uri.fsPath),
    documentDir,
    pathEntries: (process.env.PATH ?? '').split(path.delimiter),
    exeName,
    isExecutable,
    join: path.join,
    dirname: path.dirname,
  };
}

let warned = false;

/** Resolve the binary, reporting clearly (once) when it cannot be found. */
export function findBinary(out: vscode.OutputChannel): Discovered | null {
  const i = input();
  const found = discoverBinary(i);
  if (found) {
    out.appendLine(`[rustyfi] using ${found.path} (found via ${found.source})`);
    return found;
  }

  const msg = configuredButMissing(i)
    ? `rustyfi.serverPath points at "${(i.configured ?? '').trim()}", which is not an executable file.`
    : 'Could not find the `rustyfi` binary: not on PATH, and no target/release/rustyfi at or above this document or in the workspace. Set "rustyfi.serverPath".';

  out.appendLine(`[rustyfi] ${msg}`);
  if (!warned) {
    warned = true;
    vscode.window.showErrorMessage(msg, 'Open Settings').then((pick) => {
      if (pick === 'Open Settings') {
        vscode.commands.executeCommand('workbench.action.openSettings', 'rustyfi.serverPath');
      }
    });
  }
  return null;
}

/** Let a later successful lookup warn again if things break afresh. */
export function resetBinaryWarning(): void { warned = false; }

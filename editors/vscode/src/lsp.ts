import * as vscode from 'vscode';
import {
  LanguageClient,
  type LanguageClientOptions,
  type ServerOptions,
  type StaticFeature,
  type DynamicFeature,
} from 'vscode-languageclient/node';
import { findBinary } from './binary';

/**
 * `rustyfi lsp` advertises `documentFormattingProvider: true`, and this
 * extension ALSO contributes a formatter that shells out to `rustyfi fmt -`.
 * Registering both makes VS Code ask the user to pick a default formatter
 * every time they format, and only one of the two honours the
 * `rustyfi.format.*` settings.  So exactly one is registered, chosen by
 * `rustyfi.format.provider`.
 *
 * The suppression has to be a module-level flag rather than a subclass field:
 * `BaseLanguageClient`'s CONSTRUCTOR calls `registerBuiltinFeatures()`, so a
 * field initialised in the subclass constructor is still undefined by the
 * time `registerFeature` runs.
 */
let suppressServerFormatting = false;

const FORMATTING_METHODS = new Set([
  'textDocument/formatting',
  'textDocument/rangeFormatting',
  'textDocument/onTypeFormatting',
]);

class RustyfiLanguageClient extends LanguageClient {
  public override registerFeature(feature: StaticFeature | DynamicFeature<unknown>): void {
    if (suppressServerFormatting) {
      const method = (feature as DynamicFeature<unknown>).registrationType?.method;
      if (method && FORMATTING_METHODS.has(method)) return;
    }
    super.registerFeature(feature);
  }
}

let client: RustyfiLanguageClient | undefined;

export function isRunning(): boolean {
  return client !== undefined;
}

export async function start(out: vscode.OutputChannel): Promise<void> {
  await stop();

  const cfg = vscode.workspace.getConfiguration('rustyfi');
  if (!cfg.get<boolean>('lsp.enable', true)) {
    out.appendLine('[rustyfi] language server disabled by rustyfi.lsp.enable');
    return;
  }

  const bin = findBinary(out);
  if (!bin) return;

  // NO `transport: TransportKind.stdio`. It reads like the right thing to
  // write for a server that speaks stdio, and it is a trap: for an
  // `Executable`, vscode-languageclient implements that transport by
  // APPENDING `--stdio` to argv. `rustyfi lsp` speaks stdio unconditionally
  // and has no such flag, so clap refused the argument and exited 2 -- five
  // times, until the client gave up with "crashed 5 times in the last 3
  // minutes". Omitting the field is plain stdio over the child's own pipes,
  // which is what this server wants.
  const serverOptions: ServerOptions = {
    run: { command: bin.path, args: ['lsp'] },
    debug: { command: bin.path, args: ['lsp'] },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: 'satysfi' }],
    outputChannel: out,
    // Keep the channel from stealing focus every time the server logs.
    revealOutputChannelOn: 4 /* RevealOutputChannelOn.Never */,
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher('**/*.{saty,satyh,satyg}'),
    },
  };

  suppressServerFormatting =
    cfg.get<string>('format.provider', 'cli') !== 'lsp';

  const c = new RustyfiLanguageClient(
    'rustyfi',
    'rustyfi language server',
    serverOptions,
    clientOptions,
  );

  try {
    await c.start();
    client = c;
    out.appendLine(
      `[rustyfi] language server started${suppressServerFormatting ? " (its formatting provider is suppressed; rustyfi.format.provider is not 'lsp')" : ''}`,
    );
  } catch (e) {
    out.appendLine(`[rustyfi] language server failed to start: ${e}`);
    vscode.window.showErrorMessage(`rustyfi language server failed to start: ${e}`);
  }
}

export async function stop(): Promise<void> {
  const c = client;
  client = undefined;
  if (!c) return;
  try {
    await c.stop();
  } catch {
    /* already gone */
  }
}

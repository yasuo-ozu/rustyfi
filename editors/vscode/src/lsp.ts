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
let watcher: vscode.FileSystemWatcher | undefined;

export function isRunning(): boolean {
  return client !== undefined;
}

/**
 * Start and stop are SERIALIZED, because `client` is only assigned after the
 * `await c.start()` handshake completes and every caller is asynchronous.
 *
 * `activate()` does `void lsp.start(output)` and does not await it; a
 * configuration change, or a second click on "rustyfi: Restart Language
 * Server", calls `start()` again while the first is still mid-handshake.  The
 * second call's `await stopInner()` then sees `client === undefined`, stops
 * nothing, and both handshakes finish -- leaving TWO `rustyfi lsp` processes
 * running with only the last one in `client`.  The other is unreachable: no
 * later restart and not even `deactivate()` can stop it, so it lives as long
 * as the window does, holding its pipes and its file-system watcher.
 *
 * A single-slot queue is enough; there is one server and the operations are
 * short.
 */
let queue: Promise<unknown> = Promise.resolve();
function serialize<T>(fn: () => Promise<T>): Promise<T> {
  const next = queue.then(fn, fn);
  queue = next.then(
    () => undefined,
    () => undefined,
  );
  return next;
}

export function start(out: vscode.OutputChannel): Promise<void> {
  return serialize(() => startInner(out));
}

export function stop(): Promise<void> {
  return serialize(() => stopInner());
}

async function startInner(out: vscode.OutputChannel): Promise<void> {
  await stopInner();

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

  // Owned by this module and disposed in `stopInner`: the client subscribes to
  // the watcher but does not own it, so one was leaked per start -- and start
  // runs again on every settings change and every manual restart.
  watcher = vscode.workspace.createFileSystemWatcher('**/*.{saty,satyh,satyg}');

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: 'satysfi' }],
    outputChannel: out,
    // Keep the channel from stealing focus every time the server logs.
    revealOutputChannelOn: 4 /* RevealOutputChannelOn.Never */,
    synchronize: { fileEvents: watcher },
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

async function stopInner(): Promise<void> {
  const c = client;
  const w = watcher;
  client = undefined;
  watcher = undefined;
  if (w) {
    try {
      w.dispose();
    } catch {
      /* already gone */
    }
  }
  if (!c) return;
  try {
    await c.stop();
  } catch {
    /* already gone */
  }
}

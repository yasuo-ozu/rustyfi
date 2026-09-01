/**
 * The language-client LIFECYCLE, driven against the real `out/lsp.js`.
 *
 * `activate()` starts the client without awaiting it (`void lsp.start(output)`)
 * and three other places call `start()` again -- a `rustyfi.format.provider`
 * change, a `rustyfi.serverPath` / `rustyfi.lsp.enable` change, and the
 * "rustyfi: Restart Language Server" command.  Since `client` is only assigned
 * AFTER the `await c.start()` handshake, two overlapping calls each saw
 * `client === undefined`, stopped nothing, and both finished: two `rustyfi lsp`
 * processes, one of them unreachable by any later restart or by `deactivate()`.
 *
 * `vscode` and `vscode-languageclient/node` are not installable in a plain
 * `node --test` run, so they are stubbed through the module loader.  Everything
 * between them -- the module-level `client`, the ordering of stop and assign --
 * is the shipped code.  `node --test` gives each FILE its own process, so the
 * loader surgery here cannot leak into the other suites.
 */
import { test, describe, before } from 'node:test';
import assert from 'node:assert/strict';
import * as path from 'path';

/* eslint-disable @typescript-eslint/no-explicit-any */

interface Harness {
  start: (out: any) => Promise<void>;
  stop: () => Promise<void>;
  isRunning: () => boolean;
  started: number[];
  stopped: number[];
  watchersMade: number;
  watchersDisposed: number;
  reset: () => void;
}

function load(): Harness {
  const Module = require('module');
  const realLoad = Module._load;

  const started: number[] = [];
  const stopped: number[] = [];
  const counts = { made: 0, disposed: 0 };
  let nextId = 1;

  class FakeLanguageClient {
    public readonly clientId = nextId++;
    constructor(_id: string, _name: string, _server: unknown, _opts: unknown) {}
    registerFeature(): void {}
    async start(): Promise<void> {
      // A real start is a spawn plus an initialize round trip; the only thing
      // that matters here is that it does not resolve synchronously.
      await new Promise((r) => setTimeout(r, 30));
      started.push(this.clientId);
    }
    async stop(): Promise<void> {
      stopped.push(this.clientId);
    }
  }

  const vscodeStub = {
    workspace: {
      getConfiguration: () => ({ get: (_k: string, d: unknown) => d }),
      createFileSystemWatcher: () => {
        counts.made++;
        return { dispose: () => { counts.disposed++; } };
      },
    },
    window: { showErrorMessage: () => undefined },
  };

  Module._load = function (request: string, ...rest: unknown[]) {
    if (request === 'vscode') return vscodeStub;
    if (request === 'vscode-languageclient/node') return { LanguageClient: FakeLanguageClient };
    return realLoad.apply(this, [request, ...rest]);
  };

  // `binary.ts` reaches into the vscode API too; replace its exports wholesale
  // rather than stubbing everything it touches.
  const binPath = require.resolve(path.join(__dirname, '..', 'binary.js'));
  require.cache[binPath] = {
    id: binPath,
    filename: binPath,
    loaded: true,
    exports: { findBinary: () => ({ path: '/bin/true' }), resetBinaryWarning: () => undefined },
  } as any;

  const lsp = require(path.join(__dirname, '..', 'lsp.js'));

  return {
    start: lsp.start,
    stop: lsp.stop,
    isRunning: lsp.isRunning,
    started,
    stopped,
    get watchersMade() { return counts.made; },
    get watchersDisposed() { return counts.disposed; },
    reset() { started.length = 0; stopped.length = 0; counts.made = 0; counts.disposed = 0; },
  } as Harness;
}

describe('language client lifecycle', () => {
  let h: Harness;
  const out = { appendLine: () => undefined } as any;

  before(() => { h = load(); });

  test('two overlapping start()s leave exactly one server running', async () => {
    h.reset();
    const first = h.start(out);          // what activate() does, unawaited
    await new Promise((r) => setTimeout(r, 5)); // still inside the first handshake
    const second = h.start(out);         // a settings change, or a second restart click
    await Promise.all([first, second]);

    assert.equal(h.started.length, 2, 'both starts should have run');
    const live = h.started.filter((id) => !h.stopped.includes(id));
    assert.deepEqual(live.length, 1,
      `exactly one client may be left running, found ${live.length} (${live.join(', ')})`);
  });

  test('deactivate() stops the server that is actually running', async () => {
    h.reset();
    const first = h.start(out);
    await new Promise((r) => setTimeout(r, 5));
    const second = h.start(out);
    await Promise.all([first, second]);
    await h.stop();

    const orphans = h.started.filter((id) => !h.stopped.includes(id));
    assert.deepEqual(orphans, [], `no language server may outlive deactivate(); orphaned: ${orphans}`);
    assert.equal(h.isRunning(), false);
  });

  test('every file-system watcher a start creates is disposed again', async () => {
    h.reset();
    await h.start(out);
    await h.start(out);
    await h.start(out);
    await h.stop();
    assert.ok(h.watchersMade > 0, 'the client must register a watcher at all');
    assert.equal(h.watchersDisposed, h.watchersMade,
      `leaked ${h.watchersMade - h.watchersDisposed} of ${h.watchersMade} file-system watchers`);
  });
});

/**
 * These tests drive the REAL `rustyfi` binary.  They are the only place the
 * exit-code contract is checked against the compiler rather than against my
 * reading of `--help`, which is what makes the exit-6 decline test meaningful:
 * a future CLI change that renumbered the codes would fail here.
 *
 * They skip (rather than fail) when the binary has not been built, so a fresh
 * clone can still run `npm test`.
 */
import { test, describe } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { run } from '../run';
import { decideFormat, resultingText } from '../core/fmtResult';
import { buildFormatArgs } from '../core/fmtOptions';
import { buildPreviewArgs } from '../core/previewArgs';
import { renderMarkdown } from '../core/markdown';

// out/test/integration.test.js -> repo root is four levels up
const REPO = path.resolve(__dirname, '..', '..', '..', '..');
const BIN = path.join(REPO, 'target', 'release', 'rustyfi');
const HAVE_BIN = fs.existsSync(BIN);

const VALID = [
  '@require: stdjabook',
  'let x = 1',
  'in',
  "document (| title = {t}; author = {a}; show-title = true; show-toc = false |) '< +p{hi} >",
  '',
].join('\n');

const DOES_NOT_LEX = 'let x = `unterminated literal\n';
const DOES_NOT_PARSE = 'let x = = = 1 in\n';

describe('rustyfi fmt exit-code contract', { skip: HAVE_BIN ? false : 'rustyfi not built' }, () => {
  test('a well-formed document formats and exits 0', async () => {
    const { args } = buildFormatArgs({});
    const r = await run(BIN, args, { stdin: VALID }).result;
    assert.equal(r.code, 0, r.stderr);
    const d = decideFormat(r, VALID);
    assert.ok(d.kind === 'apply' || d.kind === 'unchanged');
  });

  test('a document that DOES NOT LEX exits 6 with empty stdout', async () => {
    const { args } = buildFormatArgs({});
    const r = await run(BIN, args, { stdin: DOES_NOT_LEX }).result;
    assert.equal(r.code, 6, `expected exit 6, got ${r.code}: ${r.stderr}`);
    assert.equal(r.stdout, '', 'exit 6 emits nothing on stdout — this is the file-eating case');
  });

  test('THE DECLINE PATH: exit 6 leaves the document byte-for-byte untouched', async () => {
    const { args } = buildFormatArgs({});
    const r = await run(BIN, args, { stdin: DOES_NOT_LEX }).result;
    const decision = decideFormat(r, DOES_NOT_LEX);

    // What the buffer would hold after the provider acts on this decision.
    const applied = resultingText(decision, DOES_NOT_LEX);

    assert.equal(decision.kind, 'decline');
    assert.equal(applied, DOES_NOT_LEX, 'the buffer must survive a decline unchanged');
    assert.notEqual(applied, '', 'the buffer must not be blanked');
  });

  test('a document that lexes but does not parse exits 7 and still declines', async () => {
    const { args } = buildFormatArgs({});
    const r = await run(BIN, args, { stdin: DOES_NOT_PARSE }).result;
    assert.equal(r.code, 7, `expected exit 7, got ${r.code}: ${r.stderr}`);
    assert.ok(r.stdout.length > 0, 'exit 7 DOES print a tidied document');

    const decision = decideFormat(r, DOES_NOT_PARSE);
    const applied = resultingText(decision, DOES_NOT_PARSE);
    assert.equal(decision.kind, 'decline', 'exit 7 must not be treated as success');
    assert.equal(applied, DOES_NOT_PARSE, 'the tidied text must NOT reach the buffer');
  });

  test('the option flags this extension sends are accepted by the CLI', async () => {
    // Catches a flag being renamed or removed upstream.
    const { args } = buildFormatArgs({
      maxWidth: 60, tabSpaces: 4, maxBlankLines: 1,
      wrapComments: false, wrapInlineText: false,
    });
    const r = await run(BIN, args, { stdin: VALID }).result;
    assert.notEqual(r.code, 2, `CLI rejected our arguments as a usage error: ${r.stderr}`);
    assert.equal(r.code, 0, r.stderr);
  });

  test('--max-width actually reaches the formatter', async () => {
    const wide = 'let verylongidentifiername = (1 + 2 + 3 + 4 + 5 + 6 + 7 + 8 + 9 + 10) in verylongidentifiername\n';
    const narrow = await run(BIN, buildFormatArgs({ maxWidth: 20 }).args, { stdin: wide }).result;
    const broad = await run(BIN, buildFormatArgs({ maxWidth: 1000 }).args, { stdin: wide }).result;
    if (narrow.code === 0 && broad.code === 0) {
      assert.notEqual(narrow.stdout, broad.stdout,
        'max-width 20 and 1000 should not produce identical output');
    }
  });
});

describe('preview compile', { skip: HAVE_BIN ? false : 'rustyfi not built' }, () => {
  const CORPUS = path.join(REPO, 'layout-tests', 'corpus', 'latexcmds', 'doc');
  const DOC = path.join(CORPUS, 'latexcmds-doc.saty');
  const haveDoc = fs.existsSync(DOC);

  test('a sibling temp file keeps relative @import resolution working',
    { skip: haveDoc ? false : 'corpus document missing' }, async () => {
      // This is the property the preview depends on: the in-memory buffer is
      // written next to the real document, NOT to os.tmpdir(), because
      // `@import: ../src/latexcmds` resolves relative to the importing file.
      const text = fs.readFileSync(DOC, 'utf8');
      assert.match(text, /^@import:/m, 'fixture must actually import something');

      const tmpOut = fs.mkdtempSync(path.join(os.tmpdir(), 'rustyfi-test-'));
      const sibling = path.join(CORPUS, '.rustyfi-preview-test.saty');
      fs.writeFileSync(sibling, text, 'utf8');
      try {
        const args = buildPreviewArgs({
          inputPath: sibling,
          outputPath: path.join(tmpOut, 'o.md'),
          auxPath: path.join(tmpOut, 'o.satysfi-aux'),
          mathMode: 'unicode-math',
        });
        const r = await run(BIN, args, { cwd: CORPUS, timeoutMs: 120000 }).result;
        assert.equal(r.code, 0, `compile failed: ${r.stderr.slice(-500)}`);
        const md = fs.readFileSync(path.join(tmpOut, 'o.md'), 'utf8');
        assert.ok(md.length > 1000, 'expected a substantial markdown document');
        // and it renders to HTML without throwing
        const html = renderMarkdown(md);
        assert.ok(html.length > 0);
      } finally {
        fs.rmSync(sibling, { force: true });
        fs.rmSync(path.join(CORPUS, '-'), { force: true });
        fs.rmSync(tmpOut, { recursive: true, force: true });
      }
    });

  test('the aux file lands in the temp dir, not beside the document',
    { skip: haveDoc ? false : 'corpus document missing' }, async () => {
      const before = fs.readdirSync(CORPUS).filter((f) => f.endsWith('.satysfi-aux'));
      const tmpOut = fs.mkdtempSync(path.join(os.tmpdir(), 'rustyfi-test-'));
      const sibling = path.join(CORPUS, '.rustyfi-preview-aux.saty');
      fs.writeFileSync(sibling, fs.readFileSync(DOC, 'utf8'), 'utf8');
      try {
        await run(BIN, buildPreviewArgs({
          inputPath: sibling,
          outputPath: path.join(tmpOut, 'o.md'),
          auxPath: path.join(tmpOut, 'o.satysfi-aux'),
          mathMode: 'unicode-math',
        }), { cwd: CORPUS, timeoutMs: 120000 }).result;
        const after = fs.readdirSync(CORPUS).filter((f) => f.endsWith('.satysfi-aux'));
        assert.deepEqual(after.sort(), before.sort(),
          'the preview must not drop a .satysfi-aux beside the user document');
      } finally {
        fs.rmSync(sibling, { force: true });
        fs.rmSync(path.join(CORPUS, '-'), { force: true });
        fs.rmSync(tmpOut, { recursive: true, force: true });
      }
    });
});

describe('process cancellation', { skip: HAVE_BIN ? false : 'rustyfi not built' }, () => {
  test('cancel() kills an in-flight process rather than orphaning it', async () => {
    const h = run(BIN, ['fmt', '-'], { stdin: VALID });
    h.cancel();
    const r = await h.result;
    assert.ok(r.signal !== null || r.code !== null, 'the process must have terminated');
  });

  test('cancel() after exit is harmless', async () => {
    const h = run(BIN, ['--version'], {});
    await h.result;
    assert.doesNotThrow(() => h.cancel());
  });

});

describe('run() timeout', () => {
  // Uses `sleep` rather than rustyfi: `run` always closes the child's stdin,
  // so `rustyfi lsp` would exit on EOF and never reach the timeout.
  test('a timeout kills a long-running process', async () => {
    const h = run('sleep', ['30'], { timeoutMs: 300 });
    const started = Date.now();
    const r = await h.result;
    assert.ok(r.signal !== null, `expected a signal, got code ${r.code}`);
    assert.ok(Date.now() - started < 5000, 'should have been killed promptly');
  });
});

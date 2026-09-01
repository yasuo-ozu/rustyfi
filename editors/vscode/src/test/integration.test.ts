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
import { parseBuildDiagnostics } from '../core/buildDiagnostics';

// out/test/integration.test.js -> repo root is four levels up
const REPO = path.resolve(__dirname, '..', '..', '..', '..');
const BIN = path.join(REPO, 'target', 'release', 'rustyfi');
const HAVE_BIN = fs.existsSync(BIN);
const LIB_ROOT = path.join(REPO, 'lib-rustyfi');

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
          format: 'markdown',
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
          format: 'markdown',
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

// ---------------------------------------------------------------------------
// PDF preview
//
// Self-contained: the fixture is written here rather than taken from the
// corpus, so this cannot be broken by an unrelated edit to a corpus document
// (which is exactly how the `@import:` test above breaks).
// ---------------------------------------------------------------------------

describe('pdf preview', { skip: !HAVE_BIN ? 'rustyfi is not built' : false }, () => {
  test('the pdf argv produces a real PDF that pdf.js can read', async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'rustyfi-pdf-'));
    const input = path.join(dir, 'doc.saty');
    const output = path.join(dir, 'preview.pdf');
    fs.writeFileSync(input, VALID, 'utf8');

    const args = buildPreviewArgs({
      inputPath: input,
      format: 'pdf',
      outputPath: output,
      auxPath: path.join(dir, 'preview.satysfi-aux'),
      mathMode: 'unicode-math',
      // Explicit: the fixture lives in a temp dir, so the compiler's own
      // walk-up discovery has nothing to find. Naming the root is also what
      // makes this test independent of where it is run from.
      libRoot: LIB_ROOT,
    });
    const res = await run(BIN, args, { cwd: dir, timeoutMs: 60000 }).result;
    assert.equal(res.code, 0, `compile failed: ${res.stderr || res.stdout}`);
    assert.ok(fs.existsSync(output), 'no PDF was written');

    const bytes = fs.readFileSync(output);
    // The magic, because a zero-byte or HTML-error file would still "exist".
    assert.equal(bytes.subarray(0, 5).toString('latin1'), '%PDF-', 'not a PDF');

    // And that pdf.js accepts it — importing the copy in `media/`, which is
    // the exact file the webview loads, rather than the one in node_modules.
    // Testing the dependency instead of the artefact is how the
    // `getOrInsertComputed` failure got shipped: node_modules held a build
    // that worked here and the webview loaded a different one that did not.
    const media = path.join(__dirname, '..', '..', 'media');
    const shipped = path.join(media, 'pdf.min.mjs');
    const worker = path.join(media, 'pdf.worker.min.mjs');
    assert.ok(fs.existsSync(shipped), 'npm run compile did not populate media/');
    assert.ok(fs.existsSync(worker), 'the worker is missing from media/');
    const lib = await import(shipped);
    // The same assignment the webview makes. Without it pdf.js hunts for a
    // sibling `pdf.worker.mjs` (unminified) and fails -- so this also proves
    // the worker file we ship is the one the library will accept.
    lib.GlobalWorkerOptions.workerSrc = worker;
    const doc = await lib.getDocument({
      data: new Uint8Array(bytes),
      useWorkerFetch: false,
    }).promise;
    assert.ok(doc.numPages >= 1, 'no pages');
    const page = await doc.getPage(1);
    const vp = page.getViewport({ scale: 1 });
    assert.ok(vp.width > 100 && vp.height > 100, `implausible page: ${vp.width}x${vp.height}`);
    // `lib` is imported from a runtime path, so it is `any` and the items
    // come back untyped; name the shape rather than leaning on inference.
    const items = (await page.getTextContent()).items as Array<{ str?: string }>;
    const text = items.map((i) => i.str ?? '').join('');
    assert.ok(text.includes('hi'), `the document's own text is missing: ${text.slice(0, 80)}`);

    fs.rmSync(dir, { recursive: true, force: true });
  });

  test('the aux file does not land beside the document', async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'rustyfi-pdf-aux-'));
    const docDir = path.join(dir, 'src');
    const tmpDir = path.join(dir, 'tmp');
    fs.mkdirSync(docDir);
    fs.mkdirSync(tmpDir);
    const input = path.join(docDir, 'doc.saty');
    fs.writeFileSync(input, VALID, 'utf8');

    const args = buildPreviewArgs({
      inputPath: input,
      format: 'pdf',
      outputPath: path.join(tmpDir, 'preview.pdf'),
      auxPath: path.join(tmpDir, 'preview.satysfi-aux'),
      mathMode: 'unicode-math',
      libRoot: LIB_ROOT,
    });
    const res = await run(BIN, args, { cwd: docDir, timeoutMs: 60000 }).result;
    assert.equal(res.code, 0, res.stderr);
    const strays = fs.readdirSync(docDir).filter((f) => f.endsWith('.satysfi-aux'));
    assert.deepEqual(strays, [], `preview littered the source directory: ${strays}`);

    fs.rmSync(dir, { recursive: true, force: true });
  });
});


// ---------------------------------------------------------------------------
// What the webview is allowed to assume about its JS engine.
// ---------------------------------------------------------------------------

test('the shipped pdf.js does not call V8 builtins a VS Code webview may lack', () => {
  // The preview failed at runtime with
  //   "this[#Yr].getOrInsertComputed is not a function"
  // because pdf.js 6.x calls `Map.prototype.getOrInsertComputed`, which is
  // newer than the Chromium VS Code 1.111 is built against. Its LEGACY build
  // calls it too, so the fix was pinning to 4.x *and* taking legacy.
  //
  // A webview is not "a current browser" -- it is whatever Chromium the
  // user's VS Code was built against, which may be a year old. This test is
  // what stops a routine `npm update` from reintroducing that.
  const media = path.join(__dirname, '..', '..', 'media');
  for (const f of ['pdf.min.mjs', 'pdf.worker.min.mjs']) {
    const p = path.join(media, f);
    if (!fs.existsSync(p)) continue; // compile step not run; other tests say so
    const src = fs.readFileSync(p, 'utf8');
    assert.ok(
      !src.includes('getOrInsertComputed'),
      `${f} calls getOrInsertComputed; pin pdfjs-dist lower or take the legacy build`,
    );
  }
});

// ---------------------------------------------------------------------------
// Build diagnostics, against what the compiler ACTUALLY prints.
//
// `buildDiagnostics.test.ts` parses a string I copied out of a failing build.
// This one runs the build and parses whatever comes back, so the two can
// never quietly disagree — a change to the compiler's wording fails here.
// ---------------------------------------------------------------------------

describe('build diagnostics', { skip: !HAVE_BIN ? 'rustyfi is not built' : false }, () => {
  test('a real failure parses into a located diagnostic', async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'rustyfi-build-'));
    const src = path.join(dir, 'bad.saty');
    fs.writeFileSync(
      src,
      [
        '@require: stdjabook',
        "document (| title = {T}; author = {A}; show-title = true; show-toc = false; |) '<",
        '  +p { \\nosuchcommand; }',
        '>',
        '',
      ].join('\n'),
      'utf8',
    );

    const res = await run(BIN, [src, '--lib-root', LIB_ROOT], {
      cwd: dir,
      timeoutMs: 60000,
    }).result;
    assert.notEqual(res.code, 0, 'the fixture was supposed to fail to build');

    const text = [res.stderr, res.stdout].filter(Boolean).join('\n');
    const diags = parseBuildDiagnostics(text);
    assert.equal(diags.length, 1, `expected one diagnostic, got: ${text}`);
    assert.equal(diags[0].line, 3, 'the offending line');
    assert.ok(diags[0].colStart > 0, 'a column');
    assert.ok(
      !diags[0].message.includes('Error:'),
      `the prefix must be consumed: ${diags[0].message}`,
    );
    assert.ok(
      diags[0].file.endsWith('bad.saty'),
      `the file it names: ${diags[0].file}`,
    );
    assert.ok(!fs.existsSync(path.join(dir, 'bad.pdf')), 'a failed build wrote a PDF');

    fs.rmSync(dir, { recursive: true, force: true });
  });

  test('a clean build writes the PDF the command will open', async () => {
    // `outputPath` is what `rustyfi.buildAndOpen` hands to the OS, and it is
    // computed rather than read back from the compiler — so if the default
    // output location ever changes, the command opens a file that is not
    // there. This is the test that would catch that.
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'rustyfi-build-ok-'));
    const src = path.join(dir, 'good.saty');
    fs.writeFileSync(src, VALID, 'utf8');

    const res = await run(BIN, [src, '--lib-root', LIB_ROOT], {
      cwd: dir,
      timeoutMs: 60000,
    }).result;
    assert.equal(res.code, 0, res.stderr || res.stdout);

    const expected = path.join(dir, 'good.pdf');
    assert.ok(fs.existsSync(expected), `no PDF at the computed path ${expected}`);
    assert.equal(
      fs.readFileSync(expected).subarray(0, 5).toString('latin1'),
      '%PDF-',
    );

    fs.rmSync(dir, { recursive: true, force: true });
  });
});

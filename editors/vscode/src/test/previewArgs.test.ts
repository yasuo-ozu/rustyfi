import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildPreviewArgs, humanizeDiagnostic, type MathMode } from '../core/previewArgs';

const opts = {
  inputPath: '/d/.rustyfi-preview-ab.saty',
  outputPath: '/tmp/x/preview.md',
  auxPath: '/tmp/x/preview.satysfi-aux',
  mathMode: 'unicode-math' as MathMode,
};

test('the default is markdown with unicode math, as asked for', () => {
  assert.deepEqual(buildPreviewArgs(opts), [
    '/d/.rustyfi-preview-ab.saty',
    '--format', 'markdown',
    '--unicode-math',
    '-o', '/tmp/x/preview.md',
    '--aux-file', '/tmp/x/preview.satysfi-aux',
  ]);
});

test('each math mode maps to its own flag, none silently substituted', () => {
  const expect: Record<MathMode, string> = {
    'unicode-math': '--unicode-math',
    'svg-math': '--svg-math',
    'svg-outline-math': '--svg-outline-math',
    'katex': '--katex',
    'mathml': '--mathml',
  };
  for (const [mode, flag] of Object.entries(expect) as [MathMode, string][]) {
    const a = buildPreviewArgs({ ...opts, mathMode: mode });
    assert.ok(a.includes(flag), `${mode} should pass ${flag}`);
    // exactly one math flag, so nothing is silently substituted or doubled
    const all = Object.values(expect);
    assert.equal(a.filter((x) => all.includes(x)).length, 1, `${mode} passed more than one math flag`);
  }
});

test('an unknown math mode falls back to unicode-math rather than passing junk', () => {
  const a = buildPreviewArgs({ ...opts, mathMode: 'nonsense' as MathMode });
  assert.ok(a.includes('--unicode-math'));
});

test('the aux file is kept out of the document directory', () => {
  const a = buildPreviewArgs(opts);
  const aux = a[a.indexOf('--aux-file') + 1];
  assert.ok(!aux.startsWith('/d/'), 'aux must not land beside the user document');
});

test('libRoot is forwarded only when set', () => {
  assert.ok(!buildPreviewArgs(opts).includes('--lib-root'));
  assert.ok(!buildPreviewArgs({ ...opts, libRoot: '   ' }).includes('--lib-root'));
  const a = buildPreviewArgs({ ...opts, libRoot: '/lib' });
  assert.deepEqual(a.slice(-2), ['--lib-root', '/lib']);
});

test('a diagnostic naming the temp file is rewritten to the real document', () => {
  const tmp = '/proj/doc/.rustyfi-preview-a1b2c3.saty';
  const msg = `Error: ${tmp}: line 2, characters 8-9: parse error: unexpected \`=\``;
  const out = humanizeDiagnostic(msg, tmp, 'thesis.saty');
  assert.ok(!out.includes('.rustyfi-preview-'), out);
  assert.match(out, /thesis\.saty: line 2/);
});

test('the bare basename is rewritten too (the compiler may print either)', () => {
  const tmp = '/proj/doc/.rustyfi-preview-a1b2c3.saty';
  const out = humanizeDiagnostic('Error: .rustyfi-preview-a1b2c3.saty: type mismatch', tmp, 'thesis.saty');
  assert.equal(out, 'Error: thesis.saty: type mismatch');
});

test('a path containing regex metacharacters is replaced literally', () => {
  const tmp = '/p+r(o)j/.rustyfi-preview-x.saty';
  const out = humanizeDiagnostic(`Error: ${tmp}: boom`, tmp, 'a.saty');
  assert.equal(out, 'Error: a.saty: boom');
});

test('a message naming no temp path is left alone', () => {
  const out = humanizeDiagnostic('Error: cannot resolve @require: foo', '/t/.p.saty', 'a.saty');
  assert.equal(out, 'Error: cannot resolve @require: foo');
});

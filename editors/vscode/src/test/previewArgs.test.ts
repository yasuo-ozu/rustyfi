import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  buildPreviewArgs,
  humanizeDiagnostic,
  outputExtension,
  type MathMode,
  type PreviewFormat,
} from '../core/previewArgs';

const opts = {
  inputPath: '/d/.rustyfi-preview-ab.saty',
  format: 'markdown' as PreviewFormat,
  outputPath: '/tmp/x/preview.md',
  auxPath: '/tmp/x/preview.satysfi-aux',
  mathMode: 'unicode-math' as MathMode,
};

test('markdown mode carries the math flag', () => {
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


// --- PDF mode --------------------------------------------------------------

test('pdf mode asks for a pdf and carries NO math flag', () => {
  // The math flags choose how an equation is re-expressed in a format that
  // has no maths of its own. A PDF is typeset by the same engine that lays
  // the document out, so there is nothing to re-express -- and
  // `--unicode-math` is documented "Markdown only".
  assert.deepEqual(
    buildPreviewArgs({ ...opts, format: 'pdf', outputPath: '/tmp/x/preview.pdf' }),
    [
      '/d/.rustyfi-preview-ab.saty',
      '--format', 'pdf',
      '-o', '/tmp/x/preview.pdf',
      '--aux-file', '/tmp/x/preview.satysfi-aux',
    ],
  );
});

test('pdf mode still forwards a configured lib root', () => {
  const args = buildPreviewArgs({ ...opts, format: 'pdf', libRoot: '/lib' });
  assert.ok(args.includes('--lib-root'));
  assert.equal(args[args.indexOf('--lib-root') + 1], '/lib');
});

test('markdown mode keeps svg-outline-math when asked for it', () => {
  // The pairing the user asked for: Markdown preview WITH SVG outline math.
  const args = buildPreviewArgs({ ...opts, mathMode: 'svg-outline-math' });
  assert.ok(args.includes('--svg-outline-math'), args.join(' '));
  assert.ok(!args.includes('--unicode-math'), args.join(' '));
});

test('the output extension matches the format', () => {
  // Guards a real failure mode rather than restating the function: the
  // compiler picks its writer from `--format`, not from the path, so a
  // mismatch here means reading a .md that holds a PDF.
  assert.equal(outputExtension('pdf'), '.pdf');
  assert.equal(outputExtension('markdown'), '.md');
});

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { decideFormat, resultingText, type ProcessResult } from '../core/fmtResult';

const DOC = 'let x = 1 in x\n';
const res = (o: Partial<ProcessResult>): ProcessResult =>
  ({ code: 0, stdout: '', stderr: '', signal: null, ...o });

test('exit 0 with changed text applies it', () => {
  const d = decideFormat(res({ code: 0, stdout: 'let x = 1\nin\nx\n' }), DOC);
  assert.equal(d.kind, 'apply');
  assert.equal(d.kind === 'apply' && d.text, 'let x = 1\nin\nx\n');
});

test('exit 0 with identical text is "unchanged", not a no-op edit', () => {
  assert.equal(decideFormat(res({ code: 0, stdout: DOC }), DOC).kind, 'unchanged');
});

// ---------------------------------------------------------------------------
// The decline paths.  These are the tests that matter: a wrong answer here
// replaces the user's file with nothing.
// ---------------------------------------------------------------------------

test('exit 6 (does not lex) DECLINES and carries no text to apply', () => {
  const d = decideFormat(
    res({ code: 6, stdout: '', stderr: 'error: <stdin>: declined — the file does not lex' }),
    DOC,
  );
  assert.equal(d.kind, 'decline');
  assert.ok(!('text' in d), 'a decline must not carry replacement text');
  assert.match((d as { reason: string }).reason, /does not lex/);
});

test('exit 6 stays a decline even if stdout somehow had content', () => {
  // Guard against a future CLI change: the EXIT CODE is what decides, never
  // the presence of output.
  const d = decideFormat(res({ code: 6, stdout: 'garbage' }), DOC);
  assert.equal(d.kind, 'decline');
});

test('exit 7 (lexed, did not parse) DECLINES even though stdout has a document', () => {
  // This is the subtle one: exit 7 really does print a whitespace-tidied
  // document.  Treating it as success would overwrite a mid-edit file with a
  // tidy that drops the layout the user expected.
  const d = decideFormat(
    res({ code: 7, stdout: 'let x = = = 1 in\n', stderr: 'warning: parse error' }),
    'let x = = = 1 in\n',
  );
  assert.equal(d.kind, 'decline');
  assert.ok(!('text' in d));
  assert.match((d as { reason: string }).reason, /does not parse/);
});

test('exit 0 with EMPTY output on a non-empty document is refused, not applied', () => {
  // The catastrophic case, guarded explicitly.
  const d = decideFormat(res({ code: 0, stdout: '' }), DOC);
  assert.equal(d.kind, 'error');
  assert.match((d as { reason: string }).reason, /no output/);
});

test('an empty document formatting to empty is legitimately "unchanged"', () => {
  assert.equal(decideFormat(res({ code: 0, stdout: '' }), '').kind, 'unchanged');
});

test('exit 2 (usage) and 5 (filesystem) are errors, not declines', () => {
  assert.equal(decideFormat(res({ code: 2 }), DOC).kind, 'error');
  assert.equal(decideFormat(res({ code: 5 }), DOC).kind, 'error');
});

test('an unknown exit code is an error and never applies', () => {
  const d = decideFormat(res({ code: 42, stdout: 'something' }), DOC);
  assert.equal(d.kind, 'error');
});

test('a killed process is an error, never an apply', () => {
  const d = decideFormat(res({ code: null, signal: 'SIGKILL', stdout: 'partial' }), DOC);
  assert.equal(d.kind, 'error');
});

test('no exit code other than 0 can ever produce an apply', () => {
  for (const code of [1, 2, 3, 4, 5, 6, 7, 8, 42, 255]) {
    const d = decideFormat(res({ code, stdout: 'REPLACEMENT' }), DOC);
    assert.notEqual(d.kind, 'apply', `exit ${code} must not apply an edit`);
  }
});

test('resultingText: no decision except apply can change the buffer', () => {
  const kinds = [
    decideFormat(res({ code: 6, stdout: '' }), DOC),
    decideFormat(res({ code: 7, stdout: 'tidied' }), DOC),
    decideFormat(res({ code: 2 }), DOC),
    decideFormat(res({ code: 5 }), DOC),
    decideFormat(res({ code: 0, stdout: '' }), DOC),
    decideFormat(res({ code: null, signal: 'SIGKILL' }), DOC),
  ];
  for (const d of kinds) assert.equal(resultingText(d, DOC), DOC);
  assert.equal(resultingText(decideFormat(res({ code: 0, stdout: 'NEW' }), DOC), DOC), 'NEW');
});

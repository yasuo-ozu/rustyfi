import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  parseBuildDiagnostics,
  unlocated,
  hasLocated,
} from '../core/buildDiagnostics';

// Real output, copied from a failing build rather than invented.
const REAL =
  "Error: /tmp/x/bad.saty: line 3, characters 7-21: unbound inline command '\\nosuchcommand'";

test('the Error: prefix is consumed, not left in the message', () => {
  // The exact mistake the vim side made first: anchoring at the path, so
  // nothing matched and a failed build reported no diagnostics at all.
  const [d] = parseBuildDiagnostics(REAL);
  assert.equal(d.file, '/tmp/x/bad.saty');
  assert.equal(d.line, 3);
  assert.equal(d.colStart, 7);
  assert.equal(d.colEnd, 21);
  assert.ok(!d.message.includes('Error:'), d.message);
  assert.ok(d.message.includes('unbound inline command'), d.message);
});

test('a single-character location parses too', () => {
  const [d] = parseBuildDiagnostics('Error: /a/b.saty: line 9, character 4: oops');
  assert.deepEqual(
    { line: d.line, colStart: d.colStart, colEnd: d.colEnd, message: d.message },
    { line: 9, colStart: 4, colEnd: 4, message: 'oops' },
  );
});

test('a path containing a colon still parses', () => {
  // The file part is non-greedy and the tail is anchored on ` line N,`, so a
  // colon in a directory name does not split the match in the wrong place.
  const [d] = parseBuildDiagnostics('Error: /tmp/a:b/doc.saty: line 2, characters 1-3: bad');
  assert.equal(d.file, '/tmp/a:b/doc.saty');
  assert.equal(d.line, 2);
});

test('a message with no location is kept, not dropped', () => {
  // An unresolvable @require: has no file or line, and that message IS the
  // explanation. Dropping it leaves a failed build with nothing to show.
  const text = 'Error: cannot resolve `@require: nope`; searched: (no candidates)';
  assert.deepEqual(parseBuildDiagnostics(text), []);
  assert.deepEqual(unlocated(text), [text]);
  assert.equal(hasLocated(text), false);
});

test('located and unlocated lines are partitioned, not double-counted', () => {
  const text = ['warming up', REAL, 'and something else'].join('\n');
  assert.equal(parseBuildDiagnostics(text).length, 1);
  assert.deepEqual(unlocated(text), ['warming up', 'and something else']);
});

test('blank lines produce nothing on either side', () => {
  assert.deepEqual(parseBuildDiagnostics('\n\n  \n'), []);
  assert.deepEqual(unlocated('\n\n  \n'), []);
});

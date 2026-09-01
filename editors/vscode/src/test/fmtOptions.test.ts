import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildFormatArgs } from '../core/fmtOptions';

test('an unset setting emits no flag, so $RUSTYFI_FMT_* still wins', () => {
  const { args, warnings } = buildFormatArgs({});
  assert.deepEqual(args, ['fmt', '-']);
  assert.deepEqual(warnings, []);
});

test('lang "auto" is omitted; 0.0 and 0.1 are passed', () => {
  assert.deepEqual(buildFormatArgs({ lang: 'auto' }).args, ['fmt', '-']);
  assert.deepEqual(buildFormatArgs({ lang: '0.1' }).args, ['fmt', '-', '--lang', '0.1']);
  assert.deepEqual(buildFormatArgs({ lang: '0.0' }).args, ['fmt', '-', '--lang', '0.0']);
});

test('all five formatter options map to their documented flags', () => {
  const { args } = buildFormatArgs({
    maxWidth: 80, tabSpaces: 4, maxBlankLines: 1,
    wrapComments: false, wrapInlineText: true,
  });
  assert.deepEqual(args, [
    'fmt', '-',
    '--max-width', '80',
    '--tab-spaces', '4',
    '--max-blank-lines', '1',
    '--wrap-comments', 'false',
    '--wrap-inline-text', 'true',
  ]);
});

test('maxBlankLines 0 is a real value, not a falsy omission', () => {
  assert.ok(buildFormatArgs({ maxBlankLines: 0 }).args.includes('--max-blank-lines'));
  assert.deepEqual(buildFormatArgs({ maxBlankLines: 0 }).args.slice(-2), ['--max-blank-lines', '0']);
});

test('wrapComments false is forwarded, not dropped as falsy', () => {
  assert.deepEqual(buildFormatArgs({ wrapComments: false }).args.slice(-2),
    ['--wrap-comments', 'false']);
});

test('an out-of-range value is dropped with a warning, never forwarded', () => {
  // The CLI REFUSES an out-of-range value and writes nothing, so forwarding
  // one would break formatting entirely rather than degrade it.
  for (const [k, v] of [['maxWidth', 10], ['maxWidth', 5000], ['tabSpaces', 0],
                        ['tabSpaces', 99], ['maxBlankLines', -1], ['maxBlankLines', 100]] as const) {
    const r = buildFormatArgs({ [k]: v } as Record<string, number>);
    assert.deepEqual(r.args, ['fmt', '-'], `${k}=${v} should emit no flag`);
    assert.equal(r.warnings.length, 1, `${k}=${v} should warn`);
  }
});

test('a non-integer numeric setting is dropped', () => {
  const r = buildFormatArgs({ maxWidth: 80.5 });
  assert.deepEqual(r.args, ['fmt', '-']);
  assert.match(r.warnings[0], /not an integer/);
});

test('range boundaries are inclusive', () => {
  assert.ok(buildFormatArgs({ maxWidth: 20 }).args.includes('--max-width'));
  assert.ok(buildFormatArgs({ maxWidth: 1000 }).args.includes('--max-width'));
  assert.ok(buildFormatArgs({ tabSpaces: 1 }).args.includes('--tab-spaces'));
  assert.ok(buildFormatArgs({ tabSpaces: 16 }).args.includes('--tab-spaces'));
});

test('an unknown lang is refused rather than guessed at', () => {
  const r = buildFormatArgs({ lang: '0.2' });
  assert.deepEqual(r.args, ['fmt', '-']);
  assert.equal(r.warnings.length, 1);
});

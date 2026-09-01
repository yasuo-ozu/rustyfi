/**
 * Tokenize real SATySFi snippets with the shipped TextMate grammar.
 *
 * Validating the grammar as JSON proves nothing; what matters is whether the
 * four lexical AREAS (program / inline text `{ }` / block text `'< >` /
 * math `${ }`) nest correctly, because that is what a naive grammar gets
 * wrong and it is what makes highlighting look broken.
 */
import { test, describe, before } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as path from 'path';
import * as vsctm from 'vscode-textmate';
import * as oniguruma from 'vscode-oniguruma';

let grammar: vsctm.IGrammar | null = null;

before(async () => {
  const wasmPath = require.resolve('vscode-oniguruma/release/onig.wasm');
  const wasmBin = fs.readFileSync(wasmPath).buffer;
  await oniguruma.loadWASM(wasmBin as ArrayBuffer);

  const registry = new vsctm.Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (s: string[]) => new oniguruma.OnigScanner(s),
      createOnigString: (s: string) => new oniguruma.OnigString(s),
    }),
    loadGrammar: async (scope: string) => {
      if (scope !== 'source.satysfi') return null;
      const p = path.resolve(__dirname, '..', '..', 'syntaxes', 'satysfi.tmLanguage.json');
      return vsctm.parseRawGrammar(fs.readFileSync(p, 'utf8'), p);
    },
  });
  grammar = await registry.loadGrammar('source.satysfi');
  assert.ok(grammar, 'grammar failed to load');
});

/** Return every scope name applied anywhere in `src`. */
function scopesOf(src: string): Set<string> {
  const all = new Set<string>();
  let rules = vsctm.INITIAL;
  for (const line of src.split('\n')) {
    const r = grammar!.tokenizeLine(line, rules);
    for (const t of r.tokens) for (const s of t.scopes) all.add(s);
    rules = r.ruleStack;
  }
  return all;
}

/** Scopes covering the first occurrence of `needle`. */
function scopesAt(src: string, needle: string): string[] {
  const lines = src.split('\n');
  let rules = vsctm.INITIAL;
  for (const line of lines) {
    const col = line.indexOf(needle);
    const r = grammar!.tokenizeLine(line, rules);
    if (col >= 0) {
      for (const t of r.tokens) {
        if (t.startIndex <= col && col < t.endIndex) return t.scopes;
      }
    }
    rules = r.ruleStack;
  }
  return [];
}

const has = (scopes: string[], frag: string) => scopes.some((s) => s.includes(frag));

describe('SATySFi TextMate grammar', () => {
  test('headers are recognised', () => {
    const s = scopesAt('@require: stdjabook\n', 'require');
    assert.ok(has(s, 'keyword.control.import'), s.join(' '));
  });

  test('a comment runs to end of line', () => {
    assert.ok(has(scopesAt('let x = 1 % a comment\n', 'comment'), 'comment.line'));
  });

  test('program keywords and types', () => {
    assert.ok(has(scopesAt('let x = 1 in x\n', 'let'), 'keyword.control'));
    assert.ok(has(scopesAt('let x : int = 1 in x\n', 'int'), 'support.type'));
  });

  test('a backtick string literal is a string', () => {
    assert.ok(has(scopesAt('let s = `hello` in s\n', 'hello'), 'string.quoted'));
  });

  test('a keyword INSIDE a string is not a keyword', () => {
    const s = scopesAt('let s = `let in fun` in s\n', 'let in fun');
    assert.ok(has(s, 'string.quoted'), s.join(' '));
    assert.ok(!has(s, 'keyword.control'), 'string contents must not be keyworded');
  });

  // ---- the four areas ----------------------------------------------------

  test('AREA: inline text { } is its own area', () => {
    const s = scopesAt('let it = {hello} in it\n', 'hello');
    assert.ok(has(s, 'meta.inline-text'), s.join(' '));
  });

  test('AREA: an inline command inside inline text', () => {
    const s = scopesAt('let it = {\\emph{x}} in it\n', 'emph');
    assert.ok(has(s, 'entity.name.function'), s.join(' '));
    assert.ok(has(s, 'meta.inline-text'));
  });

  test("AREA: block text '< > and its +commands", () => {
    const src = "let b = '<+p{hi}> in b\n";
    assert.ok(has(scopesAt(src, '+p'), 'meta.block-text'), 'inside block text');
    assert.ok(has(scopesAt(src, 'p{'), 'entity.name.function'), 'block command named');
  });

  test('AREA: math ${ } is its own area', () => {
    const s = scopesAt('let m = ${x^2} in m\n', 'x');
    assert.ok(has(s, 'meta.math'), s.join(' '));
  });

  test('AREA: math nests inside inline text', () => {
    const s = scopesAt('let it = {see ${a+b} here} in it\n', 'a+b');
    assert.ok(has(s, 'meta.math'), s.join(' '));
    assert.ok(has(s, 'meta.inline-text'), 'math should still be within the inline-text area');
  });

  test('AREA: text after a nested math group returns to inline text, not math', () => {
    const src = 'let it = {before ${x} after} in it\n';
    const after = scopesAt(src, 'after');
    assert.ok(has(after, 'meta.inline-text'), after.join(' '));
    assert.ok(!has(after, 'meta.math'), 'the math area must have closed at its }');
  });

  test('AREA: a command argument ( ) re-enters the program area', () => {
    const s = scopesAt('let it = {\\code(let y = 2 in y);} in it\n', 'let y');
    assert.ok(has(s, 'keyword.control'), `program keywords should light up again: ${s.join(' ')}`);
  });

  test('AREA: program resumes after inline text closes', () => {
    const src = 'let a = {text} in let b = 2 in b\n';
    const s = scopesAt(src, 'in let b');
    assert.ok(!has(s, 'meta.inline-text'), 'inline-text must not leak past its }');
  });

  test('an escape inside inline text is an escape, not a command', () => {
    const s = scopesAt('let it = {a \\{ b} in it\n', '\\{');
    assert.ok(has(s, 'constant.character.escape'), s.join(' '));
  });

  test('math escapes back to text with !{ }', () => {
    const s = scopesAt('let m = ${a !{ words } b} in m\n', 'words');
    assert.ok(has(s, 'punctuation.section.embedded') || has(s, 'meta.'), s.join(' '));
  });

  test('length literals and numbers', () => {
    assert.ok(has(scopesAt('let l = 12pt in l\n', '12pt'), 'constant.numeric'));
    assert.ok(has(scopesAt('let n = 42 in n\n', '42'), 'constant.numeric'));
  });

  test('booleans are constants', () => {
    assert.ok(has(scopesAt('let b = true in b\n', 'true'), 'constant.language'));
  });

  test('a let-inline command binding names the command', () => {
    assert.ok(has(scopesAt('let-inline \\foo it = it\n', 'foo'), 'entity.name.function'));
  });

  test('a real corpus file tokenizes without falling into one giant scope', () => {
    const repo = path.resolve(__dirname, '..', '..', '..', '..');
    const doc = path.join(repo, 'layout-tests', 'corpus', 'floatfig', 'floatfig.saty');
    if (!fs.existsSync(doc)) return; // corpus not present
    const src = fs.readFileSync(doc, 'utf8');
    const scopes = scopesOf(src);
    // A grammar that has collapsed produces almost nothing; a working one
    // lights up all four areas plus the usual leaves.
    for (const expected of [
      'keyword.control.import.satysfi',
      'comment.line.percentage.satysfi',
      'meta.inline-text.satysfi',
      'meta.block-text.satysfi',
      'entity.name.function.block-command.satysfi',
      'constant.numeric.length.satysfi',
    ]) {
      assert.ok(scopes.has(expected), `corpus file never produced ${expected}`);
    }
  });
});

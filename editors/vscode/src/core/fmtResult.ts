/**
 * Turning a `rustyfi fmt -` process result into a decision about the buffer.
 *
 * This is the file where a mistake is expensive.  Returning a TextEdit that
 * replaces the whole document with an empty string silently destroys the
 * user's work, and `rustyfi fmt` exits 6 with EMPTY STDOUT whenever the
 * document does not lex -- which, for a file being typed in, is most of the
 * time.  So the rule here is: apply an edit only for an exit code that is
 * documented to mean "this is your formatted text", and never apply an empty
 * replacement for a non-empty input.
 *
 * Exit codes, from `rustyfi fmt --help` (v0.1.6):
 *
 *     0  clean
 *     1  --check found files needing reformatting   (we never pass --check)
 *     2  usage
 *     5  filesystem
 *     6  declined: the file does not lex, so there is no token stream to re-emit
 *     7  the file LEXED but did not PARSE, so it was only tidied by the older
 *        whitespace formatter rather than laid out
 *
 * 6 and 7 are both treated as DECLINES.  6 is obvious.  7 is the subtle one:
 * the process exits non-zero but DOES print a document on stdout, so a
 * provider that keyed on "is there output" would happily overwrite a
 * half-written file with a whitespace-only tidy of itself, throwing away the
 * layout the user expected and doing it while the file is mid-edit.  Treating
 * 7 as success is the quiet version of the same bug as treating 6 as success.
 */

export type FormatDecision =
  | { kind: 'apply'; text: string }
  | { kind: 'unchanged' }
  | { kind: 'decline'; reason: string; detail: string }
  | { kind: 'error'; reason: string; detail: string };

export interface ProcessResult {
  code: number | null;
  /** Non-null when the process died on a signal instead of exiting. */
  signal?: string | null;
  stdout: string;
  stderr: string;
}

function trimDetail(stderr: string): string {
  const t = stderr.trim();
  if (!t) return '';
  // The CLI prefixes stdin diagnostics with `<stdin>:`; drop that, the user
  // knows which buffer they are in.
  const first = t.split('\n').find((l) => l.trim().length > 0) ?? t;
  return first.replace(/^(error|warning):\s*/, '').replace(/^<stdin>:\s*/, '');
}

/**
 * @param original the document text that was fed to stdin, needed for the
 *        "did this actually change anything" and empty-output guards.
 */
export function decideFormat(res: ProcessResult, original: string): FormatDecision {
  if (res.signal) {
    return {
      kind: 'error',
      reason: `The formatter was killed by signal ${res.signal}.`,
      detail: trimDetail(res.stderr),
    };
  }

  switch (res.code) {
    case 0:
      break; // fall through to the guards below
    case 6:
      return {
        kind: 'decline',
        reason: 'Not formatted: the document does not lex.',
        detail: trimDetail(res.stderr),
      };
    case 7:
      return {
        kind: 'decline',
        reason:
          'Not formatted: the document lexes but does not parse, so only whitespace could be tidied.',
        detail: trimDetail(res.stderr),
      };
    case 2:
      return {
        kind: 'error',
        reason: 'The formatter rejected its arguments (check the rustyfi.format.* settings).',
        detail: trimDetail(res.stderr),
      };
    case 5:
      return {
        kind: 'error',
        reason: 'The formatter hit a filesystem error.',
        detail: trimDetail(res.stderr),
      };
    default:
      return {
        kind: 'error',
        reason: `The formatter exited with code ${res.code}.`,
        detail: trimDetail(res.stderr),
      };
  }

  // ---- exit 0: still not trusted unconditionally -------------------------

  // The catastrophic case.  If the compiler ever exits 0 with no output for a
  // document that had content, applying it would blank the file.  Refuse.
  if (res.stdout.length === 0 && original.length > 0) {
    return {
      kind: 'error',
      reason: 'The formatter exited 0 but produced no output; leaving the document unchanged.',
      detail: trimDetail(res.stderr),
    };
  }

  if (res.stdout === original) return { kind: 'unchanged' };

  return { kind: 'apply', text: res.stdout };
}

/**
 * The text the buffer holds once a decision has been acted on.
 *
 * This mirrors exactly what the formatting provider does -- ONLY an `apply`
 * replaces anything -- and exists so the invariant "a decline cannot change
 * the buffer" is a statement in code that tests can exercise directly,
 * rather than a property re-derived in each test.
 */
export function resultingText(d: FormatDecision, original: string): string {
  return d.kind === 'apply' ? d.text : original;
}

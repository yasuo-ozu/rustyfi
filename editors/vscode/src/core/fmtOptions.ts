/**
 * Mapping VS Code settings onto `rustyfi fmt` command-line flags.
 *
 * No `vscode` import lives in this file on purpose: it is pure data-in /
 * argv-out so it can be exercised by a plain node test runner.
 *
 * The flags below were read off `rustyfi fmt --help` (v0.1.6) rather than
 * assumed.  The compiler documents this precedence for each of the five
 * formatting options:
 *
 *     the flag  >  the RUSTYFI_FMT_* environment variable  >  a built-in default
 *
 * and it resolves each option SEPARATELY.  That is why every option here is
 * nullable and why `null` means "emit no flag": passing a flag unconditionally
 * would silently outrank a user's environment variable.  Only a setting the
 * user actually set should reach the command line.
 */

export interface FormatSettings {
  lang?: string | null;
  maxWidth?: number | null;
  tabSpaces?: number | null;
  maxBlankLines?: number | null;
  wrapComments?: boolean | null;
  wrapInlineText?: boolean | null;
}

/** Inclusive ranges the CLI documents; it REFUSES an out-of-range value
 *  rather than clamping, and writes nothing when it does.  We therefore drop
 *  a bad value here instead of forwarding it, so a typo in settings.json
 *  degrades to "the default" rather than to "formatting is broken". */
const RANGES: Record<string, [number, number]> = {
  maxWidth: [20, 1000],
  tabSpaces: [1, 16],
  maxBlankLines: [0, 32],
};

export interface BuildResult {
  args: string[];
  /** Settings that were dropped, with the reason — surfaced to the user once. */
  warnings: string[];
}

function numeric(
  key: keyof typeof RANGES,
  flag: string,
  value: number | null | undefined,
  out: string[],
  warnings: string[],
): void {
  if (value === null || value === undefined) return;
  if (!Number.isFinite(value) || !Number.isInteger(value)) {
    warnings.push(`rustyfi.format.${key}: ${value} is not an integer; ignoring it.`);
    return;
  }
  const [lo, hi] = RANGES[key];
  if (value < lo || value > hi) {
    warnings.push(
      `rustyfi.format.${key}: ${value} is outside the ${lo}..=${hi} the formatter accepts; ignoring it.`,
    );
    return;
  }
  out.push(flag, String(value));
}

/**
 * Build the argument vector for `rustyfi fmt -`.
 *
 * `-` is the documented stdin/stdout mode: it reads the buffer on stdin and
 * writes the formatted text on stdout, touching no file.  That is what lets
 * the provider format an unsaved buffer.
 */
export function buildFormatArgs(s: FormatSettings): BuildResult {
  const args: string[] = ['fmt', '-'];
  const warnings: string[] = [];

  if (s.lang && s.lang !== 'auto') {
    if (s.lang === '0.0' || s.lang === '0.1') {
      args.push('--lang', s.lang);
    } else {
      warnings.push(`rustyfi.format.lang: "${s.lang}" is not 0.0 or 0.1; ignoring it.`);
    }
  }

  numeric('maxWidth', '--max-width', s.maxWidth, args, warnings);
  numeric('tabSpaces', '--tab-spaces', s.tabSpaces, args, warnings);
  numeric('maxBlankLines', '--max-blank-lines', s.maxBlankLines, args, warnings);

  // The two booleans take an explicit value (`--wrap-comments true|false`);
  // the CLI also accepts the bare flag as `true`, but spelling the value out
  // keeps `false` expressible and the two arms symmetric.
  if (s.wrapComments !== null && s.wrapComments !== undefined) {
    args.push('--wrap-comments', s.wrapComments ? 'true' : 'false');
  }
  if (s.wrapInlineText !== null && s.wrapInlineText !== undefined) {
    args.push('--wrap-inline-text', s.wrapInlineText ? 'true' : 'false');
  }

  return { args, warnings };
}

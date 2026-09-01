/**
 * Parsing the compiler's build diagnostics.
 *
 * The shape, measured rather than assumed:
 *
 *     Error: /path/doc.saty: line 3, characters 7-21: unbound inline command …
 *
 * One line, and note the `Error: ` prefix -- a pattern anchored at the path
 * matches nothing, which is how the vim side's first errorformat reported an
 * empty list for a build that had plainly failed.
 *
 * Kept out of `build.ts` and away from `vscode` so it can be tested against
 * real compiler output rather than against a reading of it.
 */

export interface BuildDiagnostic {
  /** Absolute path as the compiler printed it. */
  file: string;
  /** 1-based, as written; the caller converts to VS Code's 0-based. */
  line: number;
  /** 1-based inclusive start column. */
  colStart: number;
  /** 1-based EXCLUSIVE end column, or `colStart` when only one was given. */
  colEnd: number;
  message: string;
}

const RANGE = /^(?:Error:\s*)?(.+?): line (\d+), characters (\d+)-(\d+):\s*(.*)$/;
const POINT = /^(?:Error:\s*)?(.+?): line (\d+), character (\d+):\s*(.*)$/;

/**
 * Every diagnostic the output carries, in order.
 *
 * A line that matches neither shape is NOT dropped by the caller -- see
 * `unlocated` -- because a diagnostic nobody can see is worse than one that
 * cannot be jumped to.
 */
export function parseBuildDiagnostics(output: string): BuildDiagnostic[] {
  const out: BuildDiagnostic[] = [];
  for (const raw of output.split(/\r?\n/)) {
    const line = raw.trimEnd();
    if (!line) continue;
    const r = RANGE.exec(line);
    if (r) {
      out.push({
        file: r[1],
        line: Number(r[2]),
        colStart: Number(r[3]),
        colEnd: Number(r[4]),
        message: r[5],
      });
      continue;
    }
    const p = POINT.exec(line);
    if (p) {
      const col = Number(p[3]);
      out.push({
        file: p[1],
        line: Number(p[2]),
        colStart: col,
        colEnd: col,
        message: p[4],
      });
    }
  }
  return out;
}

/**
 * The lines that carried no location, so a caller can still surface them.
 *
 * A build can fail for reasons with no file at all -- an unresolvable
 * `@require:`, a missing library root -- and those messages are the whole
 * explanation.
 */
export function unlocated(output: string): string[] {
  return output
    .split(/\r?\n/)
    .map((l) => l.trimEnd())
    .filter((l) => l.length > 0 && !RANGE.test(l) && !POINT.test(l));
}

/** Did the compiler say anything that looks like a located diagnostic? */
export function hasLocated(output: string): boolean {
  return parseBuildDiagnostics(output).length > 0;
}

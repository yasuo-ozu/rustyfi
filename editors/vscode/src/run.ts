import { spawn } from 'child_process';
import type { ProcessResult } from './core/fmtResult';

export interface RunOptions {
  cwd?: string;
  stdin?: string;
  timeoutMs?: number;
  env?: NodeJS.ProcessEnv;
}

export interface RunHandle {
  result: Promise<ProcessResult>;
  /** Kill the process.  Safe to call after it has already exited. */
  cancel: () => void;
}

/**
 * Spawn a process, feed it stdin, collect stdout/stderr.
 *
 * Returns a handle rather than a bare promise so the caller can cancel: the
 * preview must be able to kill an in-flight compile when a newer keystroke
 * supersedes it or the panel closes, otherwise a fast typist accumulates
 * compiler processes.
 */
export function run(cmd: string, args: string[], opts: RunOptions = {}): RunHandle {
  const child = spawn(cmd, args, {
    cwd: opts.cwd,
    env: opts.env ?? process.env,
    stdio: ['pipe', 'pipe', 'pipe'],
  });

  let killed = false;
  let timer: NodeJS.Timeout | undefined;

  const result = new Promise<ProcessResult>((resolve, reject) => {
    const outChunks: Buffer[] = [];
    const errChunks: Buffer[] = [];

    child.stdout.on('data', (d: Buffer) => outChunks.push(d));
    child.stderr.on('data', (d: Buffer) => errChunks.push(d));

    child.on('error', (e) => {
      if (timer) clearTimeout(timer);
      reject(e);
    });

    child.on('close', (code, signal) => {
      if (timer) clearTimeout(timer);
      resolve({
        code,
        signal: killed ? (signal ?? 'SIGKILL') : signal,
        stdout: Buffer.concat(outChunks).toString('utf8'),
        stderr: Buffer.concat(errChunks).toString('utf8'),
      });
    });

    if (opts.stdin !== undefined) {
      // EPIPE is expected when the child exits before draining stdin (a
      // decline on a huge buffer, say); it must not become an unhandled error.
      child.stdin.on('error', () => { /* ignore */ });
      child.stdin.end(opts.stdin, 'utf8');
    } else {
      child.stdin.end();
    }
  });

  if (opts.timeoutMs && opts.timeoutMs > 0) {
    timer = setTimeout(() => { killed = true; child.kill('SIGKILL'); }, opts.timeoutMs);
  }

  return {
    result,
    cancel: () => {
      if (child.exitCode === null && child.signalCode === null) {
        killed = true;
        child.kill('SIGKILL');
      }
      if (timer) clearTimeout(timer);
    },
  };
}

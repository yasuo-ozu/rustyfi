import { test } from 'node:test';
import assert from 'node:assert/strict';
import { discoverBinary, configuredButMissing, type DiscoveryInput } from '../core/discovery';

const base = (over: Partial<DiscoveryInput>): DiscoveryInput => ({
  configured: '',
  workspaceFolders: ['/repo'],
  pathEntries: ['/usr/bin', '/usr/local/bin'],
  exeName: 'rustyfi',
  isExecutable: () => false,
  join: (...p) => p.join('/'),
  ...over,
});

test('an explicit setting wins over everything else', () => {
  const d = discoverBinary(base({
    configured: '/opt/rustyfi',
    isExecutable: (p) => p === '/opt/rustyfi' || p === '/usr/bin/rustyfi',
  }));
  assert.deepEqual(d, { path: '/opt/rustyfi', source: 'setting' });
});

test('a configured path that is not executable resolves to nothing, not a fallback', () => {
  // Falling back would hand the user a different binary than the one they
  // named, with no indication why their setting was ignored.
  const i = base({ configured: '/opt/missing', isExecutable: (p) => p === '/usr/bin/rustyfi' });
  assert.equal(discoverBinary(i), null);
  assert.equal(configuredButMissing(i), true);
});

test('PATH is searched before the workspace build directory', () => {
  const d = discoverBinary(base({
    isExecutable: (p) => p === '/usr/local/bin/rustyfi' || p === '/repo/target/release/rustyfi',
  }));
  assert.deepEqual(d, { path: '/usr/local/bin/rustyfi', source: 'path' });
});

test('PATH order is respected', () => {
  const d = discoverBinary(base({ isExecutable: () => true }));
  assert.equal(d!.path, '/usr/bin/rustyfi');
});

test('the workspace target/release build is the last resort', () => {
  const d = discoverBinary(base({ isExecutable: (p) => p === '/repo/target/release/rustyfi' }));
  assert.deepEqual(d, { path: '/repo/target/release/rustyfi', source: 'workspace' });
});

test('nothing anywhere resolves to null, and that is not "configured but missing"', () => {
  const i = base({});
  assert.equal(discoverBinary(i), null);
  assert.equal(configuredButMissing(i), false);
});

test('empty PATH entries are skipped', () => {
  const d = discoverBinary(base({
    pathEntries: ['', '/usr/bin'],
    isExecutable: (p) => p === '/usr/bin/rustyfi',
  }));
  assert.equal(d!.path, '/usr/bin/rustyfi');
});

test('the windows executable name is honoured', () => {
  const d = discoverBinary(base({
    exeName: 'rustyfi.exe',
    isExecutable: (p) => p === '/usr/bin/rustyfi.exe',
  }));
  assert.equal(d!.path, '/usr/bin/rustyfi.exe');
});

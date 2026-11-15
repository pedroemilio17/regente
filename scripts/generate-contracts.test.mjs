import { test } from 'node:test';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import { contractsMatch, type } from './generate-contracts.mjs';

test('resolves local and cross-file references and rejects missing definitions', () => {
  assert.equal(type({ $ref: '#/$defs/AgentLive' }, 'api.schema.json'), 'AgentLive');
  assert.equal(type({ $ref: 'event.schema.json' }, 'api.schema.json'), 'AgentEvent');
  assert.throws(() => type({ $ref: '#/$defs/Missing' }, 'api.schema.json'), /Unknown schema reference/);
  assert.throws(() => type({ $ref: 'missing.json' }, 'api.schema.json'), /Unknown schema reference/);
});

test('preserves required, optional, nullable and closed object fields', () => {
  assert.equal(type({ type: 'object', additionalProperties: false, required: ['a'], properties: { a: { type: ['string', 'null'] }, b: { type: 'integer' } } }, 'api.schema.json'), '{ "a": string | null; "b"?: number; }');
});

test('detects even small changes or a missing generated file', () => {
  assert.equal(contractsMatch('export type A = string;\n', 'export type A = string;\n'), true);
  assert.equal(contractsMatch('export type A = number;\n', 'export type A = string;\n'), false);
  assert.equal(contractsMatch('', 'export type A = string;\n'), false);
});

test('treats objects as open by default and rejects unsupported schema keywords', () => {
  assert.equal(type({ type: 'object', properties: { a: { type: 'string' } } }, 'api.schema.json'), '{ "a"?: string; [key: string]: unknown; }');
  assert.throws(() => type({ type: 'string', not: { const: 'x' } }, 'api.schema.json'), /Unsupported schema keyword/);
});

test('check command detects drift without rewriting its output', async () => {
  const dir = await mkdtemp(path.join(tmpdir(), 'regente-contracts-'));
  const output = path.join(dir, 'contracts.ts');
  const script = fileURLToPath(new URL('./generate-contracts.mjs', import.meta.url));
  const run = (...args) => spawnSync(process.execPath, [script, '--output', output, ...args], { encoding: 'utf8' });
  try {
    assert.equal(run().status, 0);
    assert.equal(run('--check').status, 0);
    await writeFile(output, 'stale\n');
    assert.equal(run('--check').status, 1);
    assert.equal(await readFile(output, 'utf8'), 'stale\n');
  } finally { await rm(dir, { recursive: true, force: true }); }
});

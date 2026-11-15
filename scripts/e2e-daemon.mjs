import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { spawn } from 'node:child_process';

const root = resolve(import.meta.dirname, '..');
const home = mkdtempSync(join(tmpdir(), 'regente-e2e-home-'));
const child = spawn(join(root, 'target/debug/regented'), [], {
  cwd: root,
  stdio: 'inherit',
  env: { ...process.env, REGENTE_HOME: home, REGENTE_PORT: process.env.REGENTE_E2E_PORT ?? '17717', REGENTE_UI_DIR: join(root, 'ui/dist'), REGENTE_FIXTURES: join(root, 'fixtures'), REGENTE_FAKE_STEP_MS: '50' },
});
let stopping = false;
function stop() { if (!stopping) { stopping = true; child.kill('SIGTERM'); } }
process.on('SIGTERM', stop);
process.on('SIGINT', stop);
child.on('exit', (code) => { rmSync(home, { recursive: true, force: true }); process.exit(stopping ? 0 : code ?? 1); });
child.on('error', (error) => { console.error(error); rmSync(home, { recursive: true, force: true }); process.exit(1); });

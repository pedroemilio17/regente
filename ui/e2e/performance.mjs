// Run against an isolated daemon with the built UI. Measures Chromium process CPU
// through CDP; daemon CPU is reported separately when REGENTE_DAEMON_PID is set.
import { chromium, request } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const baseURL = process.env.REGENTE_E2E_URL ?? 'http://127.0.0.1:17717';
const duration = Number(process.env.REGENTE_PERF_MS ?? '30000');
// REGENTE_PERF_AGENTS=30 profiles the canvas with a large team.
const count = Number(process.env.REGENTE_PERF_AGENTS ?? '4');
// Account concurrency caps how many run at once; at least four must be active.
const required = Math.min(count, 4);
const repo = mkdtempSync(join(tmpdir(), 'regente-perf-'));
const browser = await chromium.launch({ headless: process.env.REGENTE_PERF_HEADED !== '1', ignoreDefaultArgs: ['--disable-background-timer-throttling', '--disable-backgrounding-occluded-windows', '--disable-renderer-backgrounding'], executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined });
const api = await request.newContext({ baseURL });
const session = await browser.newBrowserCDPSession();
let created = null;
const cpu = async () => (await session.send('SystemInfo.getProcessInfo')).processInfo;
const daemonCpu = () => {
  if (!process.env.REGENTE_DAEMON_PID) return null;
  const stat = readFileSync(`/proc/${process.env.REGENTE_DAEMON_PID}/stat`, 'utf8');
  const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
  return Number(fields[11]) + Number(fields[12]);
};
try {
  execFileSync('git', ['init', '-b', 'main', repo]);
  execFileSync('git', ['-C', repo, 'config', 'user.name', 'Regente Test']);
  execFileSync('git', ['-C', repo, 'config', 'user.email', 'test@example.invalid']);
  writeFileSync(join(repo, 'README.md'), '# Performance\n');
  execFileSync('git', ['-C', repo, 'add', 'README.md']);
  execFileSync('git', ['-C', repo, 'commit', '-m', 'initial']);
  const agents = Array.from({ length: count }, (_, i) => ({ name: `Perf ${i + 1}`, cli: 'fake', is_orchestrator: i === 0, settings: { fake_scenario: 'performance' } }));
  const response = await api.post('/api/v1/projects', { data: { name: 'Performance', root: repo, base_branch: 'main', agents } });
  if (!response.ok()) throw new Error(await response.text());
  created = await response.json();
  for (const agent of created.agents) {
    const task = await api.post('/api/v1/tasks', { data: { project_id: created.project.id, agent_id: agent.id, title: `Performance ${agent.name}`, budget_usd: 1000 } });
    if (!task.ok()) throw new Error(await task.text());
  }
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${baseURL}/#/projeto/${created.project.id}`);
  await page.getByTestId('canvas-node').nth(count - 1).waitFor();
  await page.screenshot({ path: process.env.REGENTE_PERF_SCREENSHOT ?? '/tmp/perf.png', fullPage: true });
  const foreground = await context.newPage();
  await foreground.goto('about:blank');
  await foreground.bringToFront();
  const active = (snapshot) => snapshot.agents.filter((agent) => agent.project_id === created.project.id && ['working', 'thinking'].includes(agent.live.status)).length;
  const sampleActive = async () => active(await (await api.get('/api/v1/snapshot')).json());
  const deadline = Date.now() + 10000;
  while (await sampleActive() < required && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 100));
  const activeAtStart = await sampleActive();
  if (activeAtStart < required) throw new Error(`Expected at least ${required} active agents, got ${activeAtStart}`);
  const pageDiagnostics = () => page.evaluate(() => ({ visibilityState: document.visibilityState, hidden: document.hidden, hasFocus: document.hasFocus(), animationGatePaused: document.documentElement.classList.contains('anim-paused'), animations: document.getAnimations().reduce((counts, animation) => { counts[animation.playState] = (counts[animation.playState] ?? 0) + 1; return counts; }, {}) }));
  const pageStateAtStart = await pageDiagnostics();
  const startCpu = await cpu();
  const startDaemon = daemonCpu();
  const start = performance.now();
  const activeSamples = [activeAtStart];
  while (performance.now() - start < duration) {
    await new Promise((resolve) => setTimeout(resolve, Math.min(1000, Math.max(0, duration - (performance.now() - start)))));
    activeSamples.push(await sampleActive());
  }
  const elapsed = (performance.now() - start) / 1000;
  const endSnapshot = await (await api.get('/api/v1/snapshot')).json();
  const pageStateAtEnd = await pageDiagnostics();
  const endCpu = await cpu();
  const endDaemon = daemonCpu();
  const startById = new Map(startCpu.map((process) => [process.id, process.cpuTime]));
  const byType = {};
  for (const process of endCpu) {
    if (!startById.has(process.id)) continue;
    byType[process.type] = (byType[process.type] ?? 0) + Math.max(0, process.cpuTime - startById.get(process.id)) / elapsed * 100;
  }
  const hz = Number(execFileSync('getconf', ['CLK_TCK'], { encoding: 'utf8' }).trim());
  console.log(JSON.stringify({ elapsedSeconds: elapsed, headless: process.env.REGENTE_PERF_HEADED !== '1', pageStateAtStart, pageStateAtEnd, activeAgentsAtStart: activeAtStart, activeAgentsAtEnd: active(endSnapshot), activeAgentsSamples: activeSamples, agents: count, allSamplesActive: activeSamples.every((active) => active >= required), chromiumPercentOfOneCoreByProcessType: byType, daemonPercentOfOneCore: startDaemon === null || endDaemon === null ? null : (endDaemon - startDaemon) / hz / elapsed * 100, gpuUtilization: 'not measured; GPU process CPU is not GPU utilization', screenshot: process.env.REGENTE_PERF_SCREENSHOT ?? '/tmp/perf.png', limitations: 'Chromium, a second tab brought forward, sampled process CPU; check page visibility and focus to verify background state; browser and GPU process CPU can include the foreground blank tab and other browser work. Not a Tauri or GPU utilization measurement. New/exited browser processes are excluded.' }, null, 2));
} finally {
  if (created) {
    await Promise.allSettled(created.agents.map((agent) => api.post(`/api/v1/agents/${encodeURIComponent(agent.id)}/pause`, { data: { reason: 'Fim da medição' } })));
    const deadline = Date.now() + 10000;
    let running = true;
    while (running && Date.now() < deadline) {
      const tasks = await (await api.get(`/api/v1/tasks?project=${encodeURIComponent(created.project.id)}`)).json();
      running = tasks.some((task) => task.status === 'running');
      if (running) await new Promise((resolve) => setTimeout(resolve, 100));
    }
    if (running) console.error(`Tasks still running; preserving repository: ${repo}`);
    else rmSync(repo, { recursive: true, force: true });
  } else {
    rmSync(repo, { recursive: true, force: true });
  }
  await api.dispose();
  await browser.close();
}

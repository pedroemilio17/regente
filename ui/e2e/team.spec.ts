// Real daemon (scripts/e2e-daemon.mjs) with fake agents only. Covers the
// Regente 2.0 flows: A (task to the Maestro changes the node), B (popover →
// Atividade → back to chat with the draft intact), C (real delegation draws an
// edge in Execution) and D (stream drop shows "Reconectando…" and recovers).
import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

function gitRepo(prefix: string): string {
  const repo = mkdtempSync(join(tmpdir(), prefix));
  execFileSync('git', ['init', '-q', '-b', 'main', repo]);
  execFileSync('git', ['-C', repo, 'config', 'user.name', 'Regente Test']);
  execFileSync('git', ['-C', repo, 'config', 'user.email', 'test@example.invalid']);
  writeFileSync(join(repo, 'README.md'), '# Projeto de teste\n');
  execFileSync('git', ['-C', repo, 'add', 'README.md']);
  execFileSync('git', ['-C', repo, 'commit', '-q', '-m', 'initial']);
  return repo;
}

const node = (page: Page, name: string) => page.locator(`[data-testid="canvas-node"][data-agent="${name}"]`);

test('fluxos A–D: Maestro, popover, delegação no fluxo e reconexão', async ({ page, request }) => {
  const repo = gitRepo('regente-e2e-project-');
  try {
    const initialHead = execFileSync('git', ['-C', repo, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
    const agents = [
      { name: 'Orquestrador', cli: 'fake', is_orchestrator: true, settings: { fake_scenario: 'orchestrator' } },
      ...['Backend', 'Frontend', 'QA'].map((name) => ({ name, cli: 'fake', settings: { fake_scenario: name.toLowerCase() } })),
    ];
    const created = await request.post('/api/v1/projects', { data: { name: 'Projeto E2E', root: repo, base_branch: 'main', agents } });
    expect(created.ok(), await created.text()).toBeTruthy();
    const { project } = await created.json();

    // The stream goes through a proxy so the test can drop it (Fluxo D).
    let streamConnections = 0;
    let blocked = false;
    let disconnectStream: (() => void) | undefined;
    await page.routeWebSocket('**/api/v1/stream', (socket) => {
      streamConnections++;
      if (blocked) {
        socket.close();
        return;
      }
      const server = socket.connectToServer();
      disconnectStream = () => server.close();
    });
    // Every status text the Maestro node shows, so a fast fake turn is not missed.
    await page.addInitScript(() => {
      const seen = new Set<string>();
      (window as unknown as { maestroStatuses: Set<string> }).maestroStatuses = seen;
      new MutationObserver(() => {
        const text = document.querySelector('[data-agent="Orquestrador"] [data-testid="node-status"]')?.textContent;
        if (text) seen.add(text.trim());
      }).observe(document, { subtree: true, childList: true, characterData: true });
    });

    await page.goto(`/#/projeto/${project.id}`);
    await expect(page.getByTestId('connection')).toHaveText('Conectado');
    for (const agent of agents) await expect(node(page, agent.name)).toBeVisible();
    await expect(node(page, 'Orquestrador').getByTestId('node-status')).toContainText('Disponível');
    // Structure: agents created with the project hang under the Maestro.
    await expect(page.locator('.react-flow__edge.cv-edge-allowed')).toHaveCount(3);

    // Fluxo A: the task goes to the Maestro through the chat; its node leaves "Disponível".
    const recipient = page.getByLabel('Destinatário');
    await expect(recipient.locator('option:checked')).toHaveText('Orquestrador (Maestro)');
    const box = page.getByLabel('Mensagem para Orquestrador');
    await box.fill('Adicionar rota health e tela de status');
    await box.press('Enter');
    await expect(page.getByRole('log', { name: 'Conversa com Orquestrador' })).toContainText('Adicionar rota health e tela de status');
    await expect
      .poll(() => page.evaluate(() => [...(window as unknown as { maestroStatuses: Set<string> }).maestroStatuses].some((text) => !text.includes('Disponível'))), { timeout: 20_000 })
      .toBe(true);
    await expect(node(page, 'Backend').locator('.cv-last')).toBeVisible({ timeout: 20_000 });

    // Fluxo C: real delegations (task_assigned with task id) become edges in Execution.
    await page.getByRole('button', { name: 'Execução' }).click();
    const delegated = page.locator('.react-flow__edge.cv-edge-delegated');
    await expect(delegated).toHaveCount(3, { timeout: 20_000 });
    await expect(page.locator('.react-flow__edge.cv-edge-allowed')).toHaveCount(0);
    await page.locator('.react-flow__edge.cv-edge-delegated[aria-label="Delegou"]').first().focus();
    await page.keyboard.press('Enter');
    const edgePopover = page.getByTestId('edge-popover');
    await expect(edgePopover).toBeVisible();
    await expect(edgePopover).toContainText('Orquestrador');
    await edgePopover.getByRole('button', { name: 'Fechar' }).click();
    await page.getByRole('button', { name: 'Estrutura' }).click();

    // The merge approval closes the run and commits to the repository.
    const approval = page.getByTestId('approval').first();
    await expect(approval).toBeVisible({ timeout: 30_000 });
    await approval.getByRole('button', { name: 'Aprovar' }).click();
    await expect(approval).toHaveCount(0);
    const healthPath = join(repo, 'src', 'health.ts');
    await expect.poll(() => existsSync(healthPath), { timeout: 30_000 }).toBe(true);
    expect(readFileSync(healthPath, 'utf8')).toContain('export function health');
    await expect.poll(() => execFileSync('git', ['-C', repo, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(), { timeout: 30_000 }).not.toBe(initialHead);

    // Fluxo B: popover → Abrir atividade → Voltar ao chat; draft and recipient intact.
    await box.fill('rascunho que não pode sumir');
    await node(page, 'Backend').click();
    const popover = page.getByRole('dialog', { name: 'Inspetor de Backend' });
    await expect(popover).toBeVisible();
    await expect(recipient.locator('option:checked')).toHaveText('Orquestrador (Maestro)');
    await popover.getByRole('button', { name: 'Abrir atividade' }).click();
    await expect(page.getByRole('tab', { name: 'Atividade' })).toHaveAttribute('aria-selected', 'true');
    await expect(page.getByRole('region', { name: 'Atividade de Backend' })).toBeVisible();
    await page.getByRole('button', { name: 'Voltar ao chat' }).click();
    await expect(page.getByRole('tab', { name: 'Chat' })).toHaveAttribute('aria-selected', 'true');
    await expect(box).toHaveValue('rascunho que não pode sumir');
    await expect(recipient.locator('option:checked')).toHaveText('Orquestrador (Maestro)');
    await page.keyboard.press('Escape');
    await expect(popover).toHaveCount(0);

    // Fluxo D: the stream drops; the badge says so, nothing is cleared, and it recovers.
    await expect.poll(() => streamConnections).toBeGreaterThan(0);
    const before = streamConnections;
    blocked = true;
    disconnectStream?.();
    await expect(page.getByTestId('connection')).toHaveText('Reconectando…');
    for (const agent of agents) await expect(node(page, agent.name)).toBeVisible();
    await expect(node(page, 'Backend').getByTestId('node-status')).toContainText('Sem conexão');
    blocked = false;
    await expect(page.getByTestId('connection')).toHaveText('Conectado', { timeout: 20_000 });
    expect(streamConnections).toBeGreaterThan(before);
    for (const agent of agents) await expect(node(page, agent.name)).toBeVisible();

    // The terminal opens its socket only on demand.
    let terminalSockets = 0;
    page.on('websocket', (socket) => {
      if (socket.url().includes('/pty')) terminalSockets++;
    });
    await node(page, 'Backend').click();
    await page.getByRole('dialog', { name: 'Inspetor de Backend' }).getByRole('button', { name: 'Configurar' }).click();
    expect(terminalSockets).toBe(0);
    await page.getByRole('button', { name: 'Abrir terminal' }).click();
    await expect(page.getByRole('region', { name: 'Terminal' })).toBeVisible();
    await expect.poll(() => terminalSockets).toBeGreaterThan(0);
  } finally {
    rmSync(repo, { recursive: true, force: true });
  }
});

test('creates a project through the form and restores it after reload', async ({ page }) => {
  const repo = gitRepo('regente-e2e-create-');
  try {
    await page.goto('/');
    await page.getByRole('button', { name: 'Criar' }).click();
    await page.getByRole('menuitem', { name: /Novo projeto/ }).click();
    const form = page.getByRole('form', { name: 'Novo projeto' });
    await form.getByLabel('Nome', { exact: true }).fill('Projeto criado pela interface');
    await form.getByLabel('Pasta (caminho absoluto)').fill(repo);
    await form.getByLabel('Descrição (opcional)').fill('Contexto de teste');
    await form.getByRole('button', { name: 'Criar projeto' }).click();
    await expect(page.getByRole('heading', { name: 'Projeto criado pela interface' })).toBeVisible();
    await page.reload();
    await expect(page.getByRole('heading', { name: 'Projeto criado pela interface' })).toBeVisible();
    await page.getByTestId('canvas').getByRole('button', { name: 'Novo agente' }).click();
    const agentForm = page.getByRole('form', { name: 'Novo agente' });
    await agentForm.getByLabel('Motor').selectOption('fake');
    await agentForm.getByRole('button', { name: 'Criar agente' }).click();
    await expect(page.getByTestId('canvas-node')).toHaveCount(1);
    await page.reload();
    await expect(page.getByTestId('canvas-node')).toHaveCount(1);
  } finally {
    rmSync(repo, { recursive: true, force: true });
  }
});

test('saves account settings and restores them after reload', async ({ page, request }) => {
  const accounts = await (await request.get('/api/v1/accounts')).json();
  const original = accounts.find((account: { provider: string }) => account.provider === 'codex');
  expect(original).toBeTruthy();
  try {
    await page.goto('/#/perfis');
    const account = page.getByRole('form', { name: 'Conta ChatGPT' });
    await account.getByText('Avançado').click();
    await account.getByLabel('Máx. de agentes simultâneos').fill('4');
    await account.getByRole('button', { name: 'Salvar' }).click();
    await expect(account.getByRole('status')).toHaveText('Alterações salvas.');
    await page.reload();
    await account.getByText('Avançado').click();
    await expect(account.getByLabel('Máx. de agentes simultâneos')).toHaveValue('4');
  } finally {
    const restored = await request.patch(`/api/v1/accounts/${original.id}`, { data: original });
    expect(restored.ok(), await restored.text()).toBeTruthy();
  }
});

test('renders fake agent updates on the canvas within one second of daemon emission', async ({ page, request }, testInfo) => {
  const repo = gitRepo('regente-e2e-latency-');
  let agentId: string | undefined;
  let taskId: string | undefined;
  try {
    const response = await request.post('/api/v1/projects', { data: { name: 'Latência', root: repo, base_branch: 'main', agents: [{ name: 'Medidor', cli: 'fake', is_orchestrator: true, settings: { fake_scenario: 'performance' } }] } });
    expect(response.ok(), await response.text()).toBeTruthy();
    const { project, agents } = await response.json();
    agentId = agents[0].id;
    const emitted = new Map<string, number>();
    page.on('websocket', (socket) => {
      if (!socket.url().includes('/stream')) return;
      socket.on('framereceived', ({ payload }) => {
        if (typeof payload !== 'string') return;
        const frame = JSON.parse(payload);
        if (frame.type !== 'events') return;
        for (const event of frame.items) {
          if (event.agent_id === agents[0].id && event.kind === 'thinking' && event.summary.startsWith('Medição de desempenho ')) emitted.set(event.summary, Date.parse(event.ts));
        }
      });
    });
    await page.addInitScript(() => {
      const times: Record<string, number> = {};
      (window as unknown as { renderedTimes: Record<string, number> }).renderedTimes = times;
      new MutationObserver(() => {
        const text = document.querySelector('[data-testid="canvas-node"][data-agent="Medidor"] .cv-last-text')?.textContent;
        if (text?.startsWith('Medição de desempenho ') && !(text in times)) times[text] = Date.now();
      }).observe(document, { subtree: true, childList: true, characterData: true });
    });
    await page.goto(`/#/projeto/${project.id}`);
    await expect(node(page, 'Medidor')).toBeVisible();
    const task = await request.post('/api/v1/tasks', { data: { project_id: project.id, agent_id: agents[0].id, title: 'Medir latência', budget_usd: 1000 } });
    expect(task.ok(), await task.text()).toBeTruthy();
    taskId = (await task.json()).id;
    await expect.poll(async () => Object.keys(await page.evaluate(() => (window as unknown as { renderedTimes: Record<string, number> }).renderedTimes)).length, { timeout: 15_000 }).toBeGreaterThanOrEqual(5);
    const rendered = await page.evaluate(() => (window as unknown as { renderedTimes: Record<string, number> }).renderedTimes);
    const samples = Object.entries(rendered).filter(([summary]) => emitted.has(summary)).map(([summary, at]) => ({ summary, latencyMs: at - emitted.get(summary)! }));
    await testInfo.attach('latencia-daemon-dom.json', { body: JSON.stringify(samples, null, 2), contentType: 'application/json' });
    expect(samples.length).toBeGreaterThanOrEqual(5);
    for (const sample of samples) {
      expect(sample.latencyMs).toBeGreaterThanOrEqual(0);
      expect(sample.latencyMs).toBeLessThan(1000);
    }
  } finally {
    if (agentId) {
      const paused = await request.post(`/api/v1/agents/${agentId}/pause`, { data: { reason: 'Fim da medição' } });
      expect(paused.ok(), await paused.text()).toBeTruthy();
    }
    if (taskId) {
      await expect.poll(async () => {
        const tasks = await (await request.get('/api/v1/tasks')).json();
        return tasks.find((task: { id: string }) => task.id === taskId)?.status;
      }).not.toBe('running');
    }
    rmSync(repo, { recursive: true, force: true });
  }
});

test('shows the reason when a fake agent exceeds its budget', async ({ page, request }) => {
  const repo = gitRepo('regente-e2e-budget-');
  try {
    const created = await request.post('/api/v1/projects', { data: { name: 'Projeto orçamento', root: repo, base_branch: 'main', agents: [{ name: 'Gastador', cli: 'fake', is_orchestrator: true, settings: { fake_scenario: 'spender' } }] } });
    expect(created.ok(), await created.text()).toBeTruthy();
    const { project } = await created.json();
    await page.goto(`/#/projeto/${project.id}`);
    await page.getByPlaceholder('Descreva o que o time deve fazer').fill('Tarefa com limite');
    await page.getByPlaceholder('opcional').fill('1');
    await page.getByRole('button', { name: 'Enviar tarefa' }).click();
    await expect(node(page, 'Gastador').getByTestId('node-status')).toContainText('Pausado', { timeout: 20_000 });
    const openActivity = async () => {
      await node(page, 'Gastador').click();
      await page.getByRole('dialog', { name: 'Inspetor de Gastador' }).getByRole('button', { name: 'Abrir atividade' }).click();
      await expect(page.getByTestId('pause-box')).toContainText(/orçamento/i);
    };
    await openActivity();
    await page.reload();
    await openActivity();
  } finally {
    rmSync(repo, { recursive: true, force: true });
  }
});

# Épico: Regente 2.0 — workspace com canvas, chat e observabilidade

Origem: especificação "REGENTE 2.0 — UX, Design e Observabilidade" (15/11/2026).
Resumo: canvas para observar, chat para comandar, sidebar para administrar.

## Fase 0 — Inventário (feito)

| Spec pede | Já existe | Decisão |
|---|---|---|
| Log append-only com `seq` | `Event.seq` global no SQLite (`daemon/src/store`) | Reusar. `seq` é global, não por run; serve igual. |
| Snapshot + stream + replay | `GET /snapshot`, WS `/stream` manda snapshot ao conectar; reconexão com backoff (`ui/src/api/stream.ts`) | Reusar. Reconectar = novo snapshot; sem `afterSeq` até medir que pesa. |
| SSE vs WS | WS já existe | Sem SSE (a própria spec manda reusar). |
| `task.delegated` estruturado | MCP emite `task_assigned` com tarefa real (`daemon/src/mcp/mod.rs:318`) | É a aresta `delegated`. Texto livre nunca vira aresta. |
| `run` | `Task.root_task_id` | Run = árvore de tarefas de uma raiz. Sem tabela `runs`. |
| Topologia configurada | `Agent.parent_id`, `is_orchestrator` | Aresta `allowed` = `parent_id`. |
| Dependências | `Task.depends_on` | Aresta `depends_on`. |
| Estado de tarefa | `queued, running, waiting, blocked, done, cancelled` | Mapear na UI (`done→completed`, `waiting→review`). Não renomear no banco. |
| Aprovação, timeline, diff, terminal | `Approval`, `Timeline.tsx`, `DiffView.tsx`, `TerminalPanel.tsx` | Reusar dentro do Inspector. |
| Avatares pixel art | `Pet.tsx`, `useAnimationGate` (reduced motion) | Reusar no nó. |
| Store local | zustand | Reusar. |
| displayName/role/provider/modelId | `name`, `role`, `cli`, `model` | Já separados. |

## Cortes (não fazer agora)

elkjs (layout em árvore próprio basta para ≤30 agentes), TanStack Query/Virtual (só após profiling), Radix/shadcn, SSE, rotas `/api/runs/*` e `/api/projects/:id/layout` (layout fica em `localStorage`: app local de um usuário), Postgres/multiusuário, command palette, mini-map, replay temporal, comparação de runs. Fase 6 inteira fica fora.

## Tarefas

Cada tarefa fecha com: `pnpm --filter ui test`, `pnpm --filter ui build` e, se tocar o daemon, `cargo test -p daemon`, `cargo clippy -p daemon --all-targets -- -D warnings`, `cargo fmt --all --check`. Sem CLI real.

- [x] **T1 — Tokens e AppShell** (P0) — sem dependência
  Grid Topbar 52px / Sidebar 60↔240px / Canvas / Chat 400px (320–520, arrastável). Tokens da §11.1 em `styles.css`. Abaixo de 1100px o chat vira painel sobreposto. Largura do chat e sidebar persistidas por projeto em `localStorage`.
  Arquivos: `ui/src/App.tsx`, `ui/src/styles.css`, `ui/src/components/Sidebar.tsx`, novo `ui/src/components/Topbar.tsx`.
  Aceite: trocar de seção na sidebar não desmonta o chat; layout correto a 1024px; largura sobrevive a reload.

- [x] **T2 — Estado de conexão e frescor** (P0) — depende de T1
  Topbar mostra `online / reconnecting / offline / degraded` a partir do `stream.ts`. Em queda, nada é zerado; nós mostram "Sem atualização" com hora do último evento do agente.
  Arquivos: `ui/src/api/stream.ts`, `ui/src/store.ts`, `Topbar.tsx`.
  Aceite: teste derruba o WS fake → badge muda, estado anterior preservado, reconexão não duplica eventos.

- [x] **T3 — Projeção de estado do agente** (P0) — sem dependência
  Função pura `agentStatus(agent, tasks, events, now)` → `idle | working | waiting | paused | error | offline | unknown`, mais `currentTask` e `lastActivity`. Agente volta a `idle` quando não há tarefa ativa. Mapa de status de tarefa (`done→completed` etc.).
  Arquivos: novo `ui/src/lib/projection.ts` + teste.
  Aceite: testes cobrem tarefa concluída → `idle`, pausado, erro, sem heartbeat → `unknown`.

- [x] **T4 — Canvas modo Estrutura** (P0) — depende de T1, T3
  Adicionar `@xyflow/react` (registrar em `docs/decisoes.md`). Nós `orchestrator` e `agent` (pet + nome + papel + status em texto/forma + tarefa + última atividade). Arestas `allowed` de `parent_id`. Layout em árvore próprio (Maestro no topo), recalculado só quando a topologia muda. Posições fixadas, viewport por modo em `localStorage`; `fitView` só sem viewport salvo.
  Arquivos: novo `ui/src/canvas/` (`Canvas.tsx`, `AgentNode.tsx`, `layout.ts` + teste), substitui o grid de cards de `ui/src/screens/Overview.tsx`.
  Aceite: novo agente não move nós fixados; clique seleciona sem mudar o destinatário do chat; Esc limpa seleção; F ajusta à tela.

- [x] **T5 — Canvas modo Execução** (P0) — depende de T4
  Alternador Estrutura/Execução. Arestas `delegated` vindas de `task_assigned` (com `task_id`), `depends_on` tracejada, `blocked_by` âmbar, `failed` vermelha estável. Destaque de ~700ms na criação, desligado em reduced motion. Clique na aresta mostra origem, destino, hora, tarefa, resultado.
  Arquivos: `ui/src/canvas/`, `ui/src/lib/projection.ts`.
  Aceite: mensagem de texto "vou delegar" sem `task_assigned` não cria aresta (teste); 5 agentes em paralelo atualizam independentemente.

- [x] **T6 — Chat do Maestro como painel fixo** (P0) — depende de T1
  Chat à direita com cabeçalho "Maestro · <motor>". Destinatário só muda por seletor ou `@nome`, com chip "Para: X". Rascunho por projeto+destinatário em `localStorage`. Autoscroll só se estiver no fim; senão contador de novas. Bloco de delegação com "Ver no fluxo" seleciona a aresta (T5).
  Arquivos: `ui/src/components/Chat.tsx`, `ui/src/App.tsx`.
  Aceite: clicar num nó não altera destinatário; rascunho sobrevive a troca de aba e reload.

- [x] **T7 — Inspector rápido e profundo** (P1) — depende de T4, T6
  Clique → popover junto ao nó (tarefa, último evento, dependências, ações). Duplo clique/Enter → aba "Atividade" no painel direito, reusando `Timeline`, `DiffView`, `TerminalPanel` e config de `screens/Agent.tsx`. "Voltar ao chat" preserva rascunho e scroll.
  Arquivos: novo `ui/src/components/Inspector.tsx`, `ui/src/screens/Agent.tsx` (extrair partes), `Chat.tsx`.
  Aceite: Fluxo B da spec passa em teste.

- [x] **T8 — Visão em lista acessível** (P1) — depende de T3
  Alternativa ao grafo: lista/árvore com os mesmos status e ações, navegável por teclado.
  Arquivos: `ui/src/canvas/AgentList.tsx`.
  Aceite: todos os agentes e status alcançáveis só com Tab/setas.

- [x] **T9 — Isolamento entre projetos** (P1) — depende de T2
  Trocar de projeto filtra eventos/arestas pelo `project_id` e restaura layout e rascunho do projeto.
  Aceite: teste com dois projetos: evento de A nunca aparece em B.

- [x] **T10 — Polimento e E2E** (P1) — depende de T5, T7
  Atalhos, foco visível, contraste WCAG dos tokens, Playwright para Fluxos A–E com agentes falsos, perfil de pan/zoom com 30 agentes (`fixtures/fake/performance.json`).
  Feito: `ui/e2e/team.spec.ts` cobre Fluxos A–D com daemon real e agentes falsos; anel de foco accent; `--muted` passa AA (6,3:1 sobre `--raised`); reduced motion desliga pets e destaque de aresta; CSS morto removido; `e2e/performance.mjs` aceita `REGENTE_PERF_AGENTS=30`.
  Pendente: Fluxo E sem E2E; perfil de pan/zoom (só CPU com 30 agentes); arestas `depends_on` entre irmãos passam por baixo dos nós.

## Ordem

T1 e T3 em paralelo → T2, T4, T6 → T5, T8, T9 → T7 → T10.

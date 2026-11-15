# Regente — progresso

Um relatório curto por marco: o que foi feito, o que ficou e as decisões tomadas.

## M0 — Validar os CLIs (portão)

**Feito**

- Conferidas as flags de Claude Code 2.1.318, Codex 0.166.0 e agy 1.5.3 no `--help` e numa execução real pequena de cada um (autorizada pelo Pedro).
- Seis fixtures gravadas em `fixtures/` (uma tarefa completa por CLI e três casos de negação), mais o servidor MCP de teste.
- `docs/clis.md` com a tabela de respostas, o mapa de eventos e 11 lacunas, cada uma com plano.

**Achados que mudam o plano**

- O Claude Code informa o custo em dólar e a utilização das janelas de 5h e de 7 dias; Codex e agy não informam nenhum dos dois, então ficam como estimativa.
- O agy só aceita MCP na configuração global. O mensageiro vai para ele por uma ponte stdio (`regented mcp-stdio`).
- Codex e agy não trazem contagem de linhas nas edições; o daemon calcula pelo `git diff` do worktree.

**Ficou**

- Confirmar no M8 que o agy repassa as variáveis de ambiente ao processo MCP filho.

**Portão**: o Pedro escolheu seguir direto pelos portões nesta rodada; a revisão do M0 fica por este relatório e por `docs/clis.md`.

## M1 — Fundação do daemon

**Feito**

- Workspace Cargo com o crate `daemon` (binários `regented` e `regente`), config em `~/.config/regente/` (`config.json`, `prices.json`, `regente.db`).
- SQLite em modo WAL com migração numerada (`daemon/migrations/0001_init.sql`) e backup automático antes de migrar um banco existente.
- Eventos só de acréscimo; `usage` e `file_changes` são derivadas e podem ser reconstruídas (`rebuild_derived`).
- API REST e WebSocket com snapshot e lotes de 250 ms; a UI estática é servida pelo próprio daemon.
- Contratos em `shared/schema/` e um teste que valida a saída real do daemon contra eles.
- Adaptador falso com 12 cenários em `fixtures/fake/`.

**Aceite**: `POST /tasks` com agente falso gera os eventos na ordem certa pelo WebSocket (`task_events_reach_the_websocket_in_order`); reiniciar o daemon reconstrói o estado (`restart_rebuilds_state_from_events`, `running_tasks_are_requeued_on_restart`).

## M2 — Adaptadores reais

**Feito**

- `claude.rs`, `codex.rs` e `agy.rs` com testes sobre as fixtures gravadas; um processo por turno, retomada pelo id da sessão, timeout e cancelamento; custo `reported` (Claude Code) ou `estimated` (tabela `prices.json`).
- O Claude Code informa a utilização de 5h e de 7 dias; o daemon guarda e mostra esse número no lugar da estimativa.

**Aceite**: os tipos de evento de cada CLI aparecem nos testes com as fixtures. Rodada real de uma tarefa pequena por CLI, pelo daemon, em 14/11/2026: os três criaram o arquivo, rodaram `ls`, chamaram `report_done` pelo mensageiro e deixaram o merge esperando aprovação.

**Achados da rodada real, já corrigidos**

- O Codex escreveu o arquivo por comando de shell, sem `file_change`. Agora o daemon compara o `git status` do worktree no fim do turno e cria o `file_edit` que faltou.
- Faltava o status "ocioso" ao fechar uma tarefa por `report_done`.
- O agy lê o arquivo de descrição da ferramenta MCP antes de cada chamada; essa leitura não vira mais evento.

## M3 — Worktrees e segurança base (portão)

**Feito**

- Worktree por agente e tarefa em `.regente/wt/`, branch `regente/<tarefa>/<agente>`, faixa de 10 portas por worktree, `.env.regente` com valores falsos, checkpoint automático por turno com autor `Regente/<agente>`, merge squash e limpeza.
- Ambiente limpo ao subir cada CLI; regras de negação geradas para Claude Code, Codex e agy a partir dos padrões bloqueados.
- Só hashes SHA-256 dos valores do `.env`; observador de arquivos nos worktrees; alerta crítico e pausa do agente.
- `regente run -- <comando>`: segredos só no processo filho, saída mascarada, aprovação para ação irreversível.
- Aprovações que nunca vencem sozinhas.
- Barreira extra: um evento que carregue um valor de segredo é apagado antes de ser gravado, e o agente é pausado.

**Aceite (os 3 testes pedidos)**

1. Agente falso lendo `.env` é bloqueado: `reading_a_secret_file_is_blocked_and_writing_a_secret_pauses`.
2. Escrever um valor de segredo gera alerta e pausa, e o commit é barrado: o mesmo teste, mais `writing_a_secret_value_alerts_and_pauses` (observador de arquivos).
3. `git push` fica parado esperando aprovação: `git_push_waits_for_approval` e `denied_approval_does_not_run_the_command`.

**Portão**: seguido direto (decisão D1). Para revisar: rode `cargo test -p daemon`.

## M4 — MCP mensageiro e orquestração

**Feito**

- Mensageiro em `POST /mcp/{agent_id}` com as 5 ferramentas; registrado por execução no Claude Code e no Codex, e pela ponte stdio no agy.
- Laço do orquestrador, fila com dependências, limite de 20 mensagens por tarefa, pausa após 3 rodadas sem progresso, limite de agentes simultâneos por conta.
- Mensagens entregues marcadas como dado não confiável; varredura de segredos no texto.

**Aceite**: `orchestrator_delegates_agents_talk_and_merge_waits_for_approval` (orquestrador delega a 3 agentes, Frontend e Backend trocam mensagens, QA só começa depois dos dois, merge espera aprovação); `a_conversation_loop_is_cut_by_the_message_limit`; `three_rounds_without_progress_pause_the_agent`; `account_concurrency_limit_is_respected`.

## M6 e M7 — parte do daemon

- Janelas de 5h, dia e semana por conta, com teste da virada de janela; orçamentos por tarefa, agente e dia, aviso em 80% e pausa em 100% (`an_agent_over_budget_shows_up_paused_with_the_reason`).
- `POST /projects` grava `.regente/team.yaml`, `CLAUDE.md` e `AGENTS.md` (preservando o que o usuário já escreveu) e aplica os padrões bloqueados (`creating_a_project_writes_team_files_and_runs_with_its_permissions`).

**Números**: 87 testes do daemon passando; `cargo clippy -- -D warnings` e `cargo fmt --check` limpos.

## M5 — medição preliminar de desempenho

Em 14/11/2026, uma medição de 30,004 s com quatro agentes falsos ativos em todas as 30 amostras usou Chromium headless; diagnóstico posterior no mesmo modo mostrou a aba do Regente visível e com foco, apesar de outra aba ter sido trazida à frente. Portanto, esta não é uma medição válida de segundo plano. A CPU dos processos existentes do Chromium somou cerca de 4,07% de um núcleo: renderer 2,27%, processo do navegador 0,27%, processo GPU 1,40% e rede 0,13%. O processo GPU aqui é CPU, não utilização da GPU. O PID do daemon não foi informado, então sua CPU não foi medida. A captura ficou em `/tmp/perf.png` na máquina de teste.

A meta de menos de 2% em segundo plano **ainda não foi demonstrada**: a medição inclui processos compartilhados pelo navegador e uma aba vazia, usa Chromium headless com a aba visível e não mede o app Tauri nem a utilização da GPU. O procedimento reproduzível está em `ui/e2e/performance.mjs`; a fixture `fixtures/fake/performance.json` mantém os agentes ativos sem chamadas pagas.

Uma segunda medição de 30 s com Chromium em modo gráfico também encontrou a página visível e com foco, com 18 animações em execução; quatro agentes ficaram ativos em todas as amostras. CPU por núcleo: renderer 2,10%, navegador 0,67%, processo GPU 1,73%, rede 0,10% e daemon 0,80%. Ela tampouco comprova desempenho em segundo plano. O teste foi ajustado para abrir a aba adicional no mesmo contexto e usar o tratamento normal do Chromium para abas em segundo plano; é necessário repetir e conferir a visibilidade efetiva.

## M5 a M7 — interface e validação

A interface cobre visão geral, detalhes e timeline dos agentes, aprovações, terminal sob demanda, contas, orçamentos e criação de projetos e agentes. Em 14/11/2026 passaram build e lint da UI, 13 testes Vitest e 3 cenários Playwright com daemon isolado e agentes falsos. Os cenários cobrem delegação, aprovação, reconexão, terminal, criação de projeto e pausa por orçamento. Também passaram 97 testes do daemon (73 unitários, 2 de contrato e 22 de integração), formatação e Clippy do daemon; a formatação do app desktop foi verificada. A medição nativa posterior de CPU está registrada abaixo; a utilização de GPU ainda não foi medida.

Projetos exigem raiz de repositório Git e branch existente. O adaptador falso aplica as regras de acesso às pastas; para CLIs reais, projetos com pastas somente leitura ou sem acesso têm a execução recusada antes de criar worktree, pois as regras dos CLIs não garantem isolamento de subpastas. O rollback da criação de projeto cobre falhas normais, sem garantir recuperação de queda do processo ou concorrência externa.

## M8 — preparação para aceite real

O script `scripts/prepare-m8.py` foi executado com sucesso em 14/11/2026: criou um repositório temporário, daemon isolado e configuração dos quatro agentes, sem iniciar tarefas ou CLIs pagos. A execução real e os pacotes Linux foram posteriormente verificados conforme registrado abaixo. O aceite completo e a validação de pacotes macOS permanecem pendentes.

Uma nova tentativa de 5 s, com duas abas no mesmo contexto e tratamento padrão de fundo do Chromium, ainda indicou a página visível, com foco e 18 animações em execução. O ambiente automatizado não conseguiu ocultar a aba; essa tentativa não mede o Tauri em segundo plano.

### Primeira tentativa real autorizada — Codex e agy

Na tentativa de 14/11/2026, sem Claude por falta de créditos, os quatro cards apareceram sem erros JavaScript. Backend e Frontend trabalharam em worktrees paralelos e trocaram mensagens sobre o contrato; o backend passou três testes e a rota `GET /health` respondeu no repositório. Os custos estimados dos três trabalhadores com uso registrado somaram US$ 0,287873; o orquestrador foi cancelado sem relatório de uso, portanto esse valor não é o custo total da tentativa.

A rodada revelou uma corrida: o Frontend chamou `report_done` às 23:10:13.980, o QA iniciou às 23:10:14.777 e o checkpoint e status concluído do Frontend só ocorreram às 23:10:38.839. O QA recebeu o trabalho do Backend, mas não os arquivos do Frontend presentes em sua branch. Os quatro agentes foram pausados e o merge não foi aprovado. A barreira entre conclusão, checkpoint e início de dependentes foi corrigida e testada; o aceite M8 permanece pendente. Evidências temporárias: `/tmp/regente-m8-_qmxegu1`.

### Segunda tentativa real — Codex e agy

Após a correção da barreira, a segunda rodada usou Codex no orquestrador, Backend e QA, e agy no Frontend, sem Claude. Os quatro cards apareceram sem erros JavaScript; foram observados 71 eventos, três mensagens e nenhuma ocorrência do valor de segredo falso auditado. O QA verificou cinco arquivos e passou seis casos em dois arquivos de teste. A rota respondeu por HTTP e a tela integrada foi verificada no navegador. O uso registrado foi estimado em US$ 0,303781625. O merge ficou aguardando aprovação (`ap_9ecff179c37d4f4e`); o cenário completo com Claude e o aceite final permanecem pendentes. Evidências temporárias: `/tmp/regente-m8-7l10tq79`.

### Desktop Linux e medição nativa

Os pacotes DEB (6,84 MiB) e RPM (6,85 MiB) foram gerados com um sysroot temporário, sem instalar bibliotecas no sistema. Ambos incluem os três binários, interface e fixtures; as dependências dinâmicas foram resolvidas. No smoke do RPM extraído, o app iniciou seu próprio daemon, serviu a interface, fechou normalmente por `WM_DELETE_WINDOW` com código 0 e deixou o daemon saudável. Artefatos em `app/src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/`. A instalação completa e pacotes macOS não foram validados.

Em uma medição nativa de 30 s, após aquecimento, a janela Tauri permaneceu minimizada e quatro agentes falsos ativos em todas as 30 amostras. Os mesmos processos do app e WebKit estavam presentes no início e no fim: CPU do app/WebKit 0,667% de um núcleo, daemon 0,267%, total 0,933%. Isso atende à meta de CPU nesta configuração; a utilização de GPU não foi medida. O procedimento está em `scripts/native-performance.py`.

Na repetição final de 30,000 s, com 5 s de aquecimento, a janela permaneceu minimizada e quatro agentes ativos em todas as 30 amostras; os mesmos processos Tauri, WebKit de conteúdo e WebKit de rede permaneceram presentes. O app/WebKit usou 0,667% de um núcleo e o daemon 0,233%, totalizando 0,900%: a meta de CPU abaixo de 2% foi demonstrada nesta configuração Linux. O contador Intel RC6 indicou 96,253% do intervalo em residência ociosa **global da GPU**; outras aplicações podem afetá-lo. Seu complemento não mede utilização, e a utilização de GPU atribuível ao Regente não foi medida.

### Continuação M8 — Botão de login por plataforma (R2) e alinhamento de ambiente

Em 15/11/2026, foram implementados os requisitos complementares do marco M8 para atender à definição de pronto do MVP:
1. **Login por plataforma na interface (R2)**:
   - Implementado no componente `AccountCard` (`ui/src/screens/Profiles.tsx`) e módulo de autenticação (`ui/src/lib/auth.ts`) o botão de login interativo para Claude, ChatGPT (Codex) e Antigravity (agy).
   - O fluxo direciona para o login de cada plataforma (`https://claude.ai/login`, `https://chatgpt.com/`, `https://antigravity.google/` para assinaturas, ou páginas de chave de API correspondentes: Console Anthropic, OpenAI Platform, Google AI Studio) com `target="_blank"` e `rel="noopener noreferrer"`.
   - São exibidas as orientações e comandos CLI para cada plataforma (`claude auth login --claudeai`, `codex login`, `agy`) e os nomes de variáveis de ambiente (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`).
   - Cobertura de testes unitários adicionada em `ui/src/screens/Profiles.test.tsx` (7 testes cobrindo renderização, atributos de links e reatividade ao alterar tipo de login).
2. **Formatação Rust e compatibilidade de filtros de build**:
   - Ajustada formatação de `daemon/src/adapters/process.rs` e `daemon/src/security/folder_sandbox.rs` via `cargo fmt`; validação com `cargo fmt --check` passando com 0 diffs.
   - Nome do pacote na pasta `ui` alinhado para `"name": "ui"`, permitindo resolução idêntica de comandos `pnpm --filter ui test` e `pnpm --filter ui lint`.
3. **Validação da suíte canônica de testes**:
   - `cargo test -p daemon`: 98 testes passaram (74 unitários, 2 de contrato, 22 de integração, 1 ignorado que exige bwrap sem privilégios).
   - `cargo clippy -- -D warnings`: 0 erros ou avisos.
   - `cargo fmt --check`: 0 diffs.
   - `pnpm --filter ui test`: 21 testes passaram em 5 arquivos de teste.
   - `pnpm --filter ui lint`: 0 erros de tipo e lint.
   - `pnpm test:contracts`: 5 testes de contrato de esquemas passaram.
   - `scripts/prepare-m8.py`: executado com sucesso preparando repositório temporário isolado e daemon funcional.

## Regente 2.0 — T10 (polimento e E2E)

Em 15/11/2026, `ui/e2e/team.spec.ts` foi reescrito para a nova interface e cobre os Fluxos A–D com daemon real e agentes falsos: tarefa ao Maestro pelo chat muda o nó, popover → Atividade → Voltar ao chat com rascunho e destinatário intactos, três arestas `delegated` no modo Execução e badge "Reconectando…" com recuperação. Os 5 testes E2E passaram duas vezes seguidas. O QA visual em 1440x900 e 1024x768 levou a estas mudanças: canvas na largura toda logo abaixo das aprovações, agentes sem `parent_id` pendurados no Maestro, `fitView` limitado a zoom 1 com margem para a barra e os botões, filhos em grade quando passam de seis, "Disponível" como rótulo único de ocioso e "Aguardando" enquanto há aprovação de merge pendente. Também entraram anel de foco accent, Enter na aresta abrindo os detalhes e a remoção de CSS morto. O `--muted` já passava em AA (6,3:1 sobre `--raised`). Ficaram pendentes o Fluxo E, o perfil de pan/zoom com 30 agentes (só CPU foi medida) e o traçado das arestas `depends_on` entre irmãos.

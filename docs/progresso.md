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

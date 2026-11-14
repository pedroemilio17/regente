# Regente — validação dos CLIs (M0)

14/11/2026. Versões instaladas nesta máquina: Claude Code 2.1.318, Codex CLI 0.166.0, Antigravity CLI (`agy`) 1.5.3.

Tudo abaixo foi conferido no `--help` de cada CLI e numa execução real pequena (ler um arquivo, editar, rodar `ls`, chamar a ferramenta `ping` de um MCP de teste). As saídas estão em `fixtures/<cli>/`, com caminhos e nome de usuário trocados por valores neutros.

## Respostas por CLI

| Pergunta | Claude Code | Codex | agy |
| --- | --- | --- | --- |
| Modo headless | `claude -p "<prompt>"` | `codex exec "<prompt>"` | `agy -p "<prompt>"` |
| Saída estruturada (JSON por linha) | `--output-format stream-json --verbose` | `--json` | `--output-format stream-json` |
| Id da sessão | `session_id` no evento `system/init` | `thread_id` no evento `thread.started` | `conversation_id` no evento `init` |
| Retomar sessão | `--resume <session_id>` (ou fixar com `--session-id <uuid>`) | `codex exec resume <thread_id> "<prompt>" --json` | `--conversation <id>` |
| Configurar MCP | Por execução: `--mcp-config '<json>' --strict-mcp-config` | Por execução: `-c mcp_servers.<nome>.command=…` / `.args=…` (ou `.url`) | Só global: `agy mcp add` grava em `~/.gemini/config/mcp_config.json` |
| Aprovar ferramenta MCP sem prompt | `--allowedTools mcp__<servidor>__<ferramenta>` | `-c mcp_servers.<nome>.default_tools_approval_mode="approve"` | `--settings '{"permissions":{"allow":["mcp(<servidor>/<ferramenta>)"]}}'` |
| Permissões e sandbox | `--settings '<json>'` com `permissions.allow/ask/deny`; `--permission-mode acceptEdits`; `--permission-prompts none` nega o que pediria confirmação | `-s read-only \| workspace-write \| danger-full-access`; aprovação `never` no `exec` | `--mode accept-edits`; `permissions.allow` com `command(<alvo>)` e `mcp(<alvo>)`; `--sandbox` |
| Sem regra de permissão em headless | Nega (com `--permission-prompts none`) | Falha a chamada e segue | Nega e lista em `result.denied_actions` |
| Arquivo de contexto do projeto | `CLAUDE.md` | `AGENTS.md` | `AGENTS.md` |
| Tokens | `usage` em cada mensagem e no `result` | `turn.completed.usage` | `usage` em cada `step_update` de resposta e no `result` |
| Custo em dólar | Sim: `result.total_cost_usd` → `reported` | Não → `estimated` | Não → `estimated` |
| % de uso do plano | Sim: `rate_limit_event` traz a utilização de 5h e de 7 dias | Não aparece no `exec --json` | Não aparece |

## Como cada evento do contrato aparece

| `kind` do Regente | Claude Code | Codex | agy |
| --- | --- | --- | --- |
| `thinking` | bloco `thinking` em `assistant` | `agent_message` intermediário / `reasoning` | `step_update` de `agent_response` |
| `file_read` | `tool_use` `Read` | `command_execution` com `cat`/`sed`/`rg` (sem evento próprio) | ferramenta `read_file` |
| `file_edit` | `tool_use` `Edit`/`Write` + `structuredPatch` no resultado | `file_change` (`changes[].path`, `kind`) | ferramenta `edit_file` |
| `command` | `tool_use` `Bash` | `item.started` de `command_execution` | ferramenta `run_command` em `ACTIVE` |
| `command_result` | `tool_result` com `stdout`/`stderr` | `item.completed` com `aggregated_output` e `exit_code` | `run_command` em `DONE`, com `output` |
| chamada MCP | `tool_use` `mcp__<servidor>__<ferramenta>` | `mcp_tool_call` | ferramenta `call_mcp_tool` |
| `error` | `result` com `is_error` | `mcp_tool_call`/`command_execution` com `status: failed` | `step_update` em `ERROR`; `denied_actions` |
| fim do turno | `result` | `turn.completed` | `result` |

## Lacunas e plano para cada uma

| # | CLI | Lacuna | Plano |
| --- | --- | --- | --- |
| 1 | agy | MCP só na configuração global; não há flag por execução | Registrar uma vez um servidor stdio `regente` (`regented mcp-stdio`), que lê a identidade do agente de variáveis de ambiente do processo e repassa ao daemon. A validar no M8 se o agy repassa o ambiente ao filho MCP |
| 2 | agy | `edit_file` não traz diff nem contagem de linhas | Calcular linhas pelo `git diff` do worktree depois do evento |
| 3 | Codex | `file_change` não traz contagem de linhas | Mesmo cálculo pelo `git diff` |
| 4 | Codex | Leitura de arquivo não tem evento próprio | Classificar `command_execution` de `cat`, `sed -n`, `head`, `rg` como `file_read` |
| 5 | Codex | Sem regra de negação de leitura por caminho | O `.env` não existe no worktree; ambiente limpo; varredura de vazamento; sandbox `workspace-write` |
| 6 | Codex e agy | Custo não informado | Estimar pela tabela de preços editável (`cost_source: estimated`) |
| 7 | Codex e agy | % do plano não informada | Estimar pelo registro de tokens do próprio Regente e mostrar como estimativa |
| 8 | Claude Code | Hooks e plugins do usuário entram nas execuções headless e podem bloquear ferramentas | Subir com `--setting-sources project` e passar as regras do Regente por `--settings` |
| 9 | Claude Code | Ferramentas MCP chegam adiadas e geram um `ToolSearch` antes da chamada | Ignorar `ToolSearch` no adaptador |
| 10 | Codex | Cada turno reenvia ~130 mil tokens de entrada (quase tudo em cache) | Contar `cached_input_tokens` em `tokens_cache` e cobrar pelo preço de cache |
| 11 | agy | Sandbox real não confirmado (`--sandbox` só restringe o terminal) | Confiar no worktree sem segredos + regras `permissions.allow` fechadas; contêiner fica para depois do MVP |

## Comandos usados nas gravações

```bash
claude -p "<tarefa>" --output-format stream-json --verbose --model haiku \
  --max-budget-usd 0.50 --setting-sources project --strict-mcp-config \
  --mcp-config '{"mcpServers":{"regente_test":{"command":"node","args":["test-mcp.mjs"]}}}' \
  --permission-mode acceptEdits \
  --allowedTools Read Edit "Bash(ls:*)" mcp__regente_test__ping

codex exec --json -s workspace-write -C <pasta> \
  -c 'mcp_servers.regente_test.command="node"' \
  -c 'mcp_servers.regente_test.args=["test-mcp.mjs"]' \
  -c 'mcp_servers.regente_test.default_tools_approval_mode="approve"' "<tarefa>"

agy mcp add regente_test node test-mcp.mjs   # removido logo depois
agy -p "<tarefa>" --output-format stream-json --mode accept-edits --print-timeout 150s \
  --settings '{"permissions":{"allow":["command(ls)","mcp(regente_test/ping)"]}}'
```

## Fixtures gravadas

| Arquivo | O que mostra |
| --- | --- |
| `fixtures/claude/basic.jsonl` | Tarefa completa, custo informado (US$ 0,0032) e uso do plano |
| `fixtures/codex/basic.jsonl` | Tarefa completa, com a chamada MCP aprovada |
| `fixtures/codex/mcp-denied.jsonl` | Chamada MCP recusada por falta de aprovação |
| `fixtures/agy/basic.jsonl` | Tarefa completa, com comando e MCP liberados |
| `fixtures/agy/command-denied.jsonl` | Comando negado em headless |
| `fixtures/agy/mcp-denied.jsonl` | Chamada MCP negada em headless |
| `fixtures/tools/test-mcp.mjs` | Servidor MCP de teste (ferramenta `ping`) |

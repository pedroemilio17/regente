# Regente — decisões e desvios do plano

Cada entrada: o que foi encontrado, o que foi escolhido e por quê.

## D1 — Portões sem parada (14/11/2026)

- **Encontrado**: o plano manda parar no fim do M0, M3 e M5.
- **Escolhido**: seguir direto. O Pedro pediu o trabalho completo e, perguntado, respondeu "Seguir direto".
- **Efeito**: cada portão vira um relatório em `docs/progresso.md` para revisão posterior.

## D2 — MCP mensageiro sem a biblioteca rmcp

- **Encontrado**: o mensageiro tem 5 ferramentas e só precisa de `initialize`, `tools/list` e `tools/call` sobre HTTP.
- **Escolhido**: implementar o JSON-RPC direto numa rota do axum, em vez de usar o SDK `rmcp` (3.10.0, ativo).
- **Por quê**: é a forma mais simples que passa nos critérios de aceite (regra 5 do plano), tira uma dependência grande e deixa a identidade do agente na própria URL. Trocar por `rmcp` depois fica contido em `daemon/src/mcp/`.

## D3 — MCP no agy por ponte stdio

- **Encontrado**: o agy 1.5.3 só lê MCP da configuração global (`~/.gemini/config/mcp_config.json`); não há flag por execução.
- **Escolhido**: registrar uma vez o comando `regented mcp-stdio`, que lê `REGENTE_MCP_URL` do ambiente do agente e repassa as chamadas ao daemon.
- **Risco**: depende de o agy repassar o ambiente ao filho MCP. A validar no M8.

## D4 — Dependências fora da tabela da stack

| Dependência | Motivo |
| --- | --- |
| `sha2` | Guardar só o hash dos valores do `.env` (M3) |
| `uuid` | Ids de eventos, tarefas e sessões |
| `chrono` | Carimbos de tempo e matemática das janelas de uso |
| `thiserror` | Exigido pelas regras de qualidade do plano |
| `tokio-tungstenite` (só testes) | Cliente WebSocket nos testes de integração |
| `tower` (só testes) | Chamar as rotas do axum sem abrir porta |
| `tempfile` (só testes) | Repositórios git temporários nos testes |

## D5 — Versões fixadas

- `notify` na série 8 (a 9 ainda é release candidate).
- `tauri` na série 2 (a 3 ainda é alpha).
- `rusqlite` com o recurso `bundled`, para não depender do SQLite do sistema.

## D6 — Hooks do usuário nas execuções do Claude Code

- **Encontrado**: hooks e plugins de `~/.claude/settings.json` entram nas execuções headless e podem barrar ferramentas dos agentes.
- **Escolhido**: o adaptador sobe o Claude Code com `--setting-sources project` e passa as regras do Regente por `--settings`.

## D7 — Integração só no fim, com aprovação

- **Encontrado**: o system design descreve merge de cada etapa na branch da tarefa e depois na base.
- **Escolhido**: cada agente trabalha na própria branch; uma etapa que depende de outras recebe o merge delas ao criar o worktree; a integração na branch base acontece uma vez, quando a tarefa raiz termina e o Pedro aprova (squash de cada branch de agente).
- **Por quê**: cobre o cenário de aceitação (QA vê o trabalho de Backend e Frontend; merge final espera aprovação) sem manter uma branch intermediária.

## D8 — Preparar worktree sem instalar dependências

- **Escolhido**: "preparar" gera o `.env.regente` e reserva a faixa de portas. Instalar dependências a partir de cache compartilhado fica para depois do MVP.

## D9 — Tarefa fecha sozinha quando o CLI não chama `report_done`

- **Encontrado**: um CLI pode encerrar o turno sem chamar `report_done`.
- **Escolhido**: sem etapas abertas, sem mensagens pendentes e sem pergunta enviada no turno, a tarefa fecha com a última resposta do CLI como resumo. Se o agente enviou uma mensagem, a tarefa fica aguardando a resposta.

## D10 — Resposta a mensagem vira uma tarefa curta

- **Encontrado**: um agente pode receber uma pergunta depois de já ter fechado a própria tarefa.
- **Escolhido**: o daemon abre uma tarefa "Responder a <agente>", filha da mesma tarefa, que fecha após um turno.

## D11 — Terminal sob demanda é um shell no worktree

- **Encontrado**: em modo headless não existe um terminal do CLI para anexar.
- **Escolhido**: "Abrir terminal" cria um PTY com um shell no worktree mais recente do agente (ambiente limpo), destruído ao fechar.

## D12 — Formatação padrão do rustfmt

- Sem `rustfmt.toml`: largura padrão de 100 colunas.

## D13 — Registro do mensageiro no agy

- Na primeira execução de um agente agy, o daemon roda `agy mcp add regente <regented> mcp-stdio` (configuração global do agy) e marca isso em `~/.config/regente/agy-mcp-registered`. Validado na rodada real: o agy repassa `REGENTE_MCP_URL` ao processo MCP filho, então a decisão D3 se confirmou.

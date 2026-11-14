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

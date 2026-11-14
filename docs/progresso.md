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

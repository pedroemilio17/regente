# Plano: simplificar criação e login

## Objetivo

Criar projeto, agente e conectar conta em poucos segundos. Só o essencial é obrigatório; o resto tem padrão e pode ser ajustado depois pelo orquestrador via MCP do sistema.

## Entregas

### E1 — Modais mínimos (UI)
- **Novo projeto**: modal com Nome* e Pasta* (caminho absoluto); abaixo, Descrição opcional (vira `context`). Branch base, pastas, bloqueios, orçamento e time somem do formulário (padrões do daemon).
- **Novo agente**: modal com Motor (Claude, Codex, Antigravity, Teste) e Função (Maestro / Agente do time). Projeto = o atual. Nome, papel, pet, modelo, instruções, orçamento e líder ficam com padrão.
- Arquivos: `ui/src/screens/NewProject.tsx`, `ui/src/screens/NewAgent.tsx`, `ui/src/App.tsx`, `ui/src/styles.css`, testes da UI.

### E2 — Padrões e MCP do sistema (daemon)
- `POST /projects`: `base_branch` ausente = branch atual do repositório.
- `POST /agents`: `name` ausente = rótulo do motor com sufixo único no projeto ("Claude", "Claude 2"); `role` ausente = "Orquestrador" ou "Agente".
- Novas ferramentas no MCP existente (`/mcp/{agent_id}`), só para o orquestrador: `create_agent`, `update_agent`, `update_project` (contexto, pastas, bloqueios, orçamento, branch base) e `get_project`. Reaproveitam a mesma lógica das rotas REST.
- Arquivos: `daemon/src/mcp/mod.rs`, `daemon/src/api/mod.rs` (somente handlers de projeto/agente), `daemon/src/app.rs`, `shared/schema/api.schema.json`, testes.

### E3 — Login de conta simples
- Cada conta mostra status (Conectado / Não conectado) e um botão **Entrar**.
- **Entrar** abre um terminal (PTY já existente) rodando o login do próprio CLI (`claude auth login`, `codex login`, ...), que abre o navegador. Ao fechar, a UI consulta o status de novo.
- Daemon: `GET /accounts/{id}/status` e PTY de login por conta; comandos numa tabela por provedor, com o adaptador falso nos testes.
- Tipo de login, caminho do CLI, modelo, concorrência e limites ficam num bloco "Avançado" recolhido.
- Arquivos: `ui/src/screens/Profiles.tsx`, `ui/src/lib/auth.ts`, `ui/src/screens/Profiles.test.tsx`, `daemon/src/api/pty.rs`, rotas de conta em `daemon/src/api/mod.rs`, `daemon/src/adapters/`.

## Fora do escopo agora
- UTCP (o MCP já existente cobre a necessidade).
- ~~Seletor nativo de pasta~~: feito depois, pelo daemon (D22).

## Aceite
- `cargo test -p daemon`, `cargo clippy -p daemon --all-targets -- -D warnings`, `cargo fmt --all --check`, `pnpm --filter ui test` e `pnpm --filter ui build` verdes.
- Nenhum teste com CLI real; nenhum `.env*` lido.
- Decisões novas registradas em `docs/decisoes.md` (D20+).

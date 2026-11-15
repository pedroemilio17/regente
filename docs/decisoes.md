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
| `rfd` (só Linux) | Seletor nativo de pasta via xdg-desktop-portal (D22) |

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

## D14 — Dependências da interface

- TypeScript 5.9.3 foi fixado para manter compatibilidade com o parser do typescript-eslint instalado; a versão 7 causava falha no lint. Fontes Inter e JetBrains Mono locais e xterm.js foram acrescentados à stack para tipografia sem rede e terminal sob demanda.

## D15 — Origem local e permissões de pastas

- O daemon aceita Host e Origin locais válidos, rejeitando origens não confiáveis e tentativas de DNS rebinding.
- O projeto deve apontar para a raiz de um repositório Git com branch existente. As permissões de pastas são verificadas pelo adaptador falso. CLIs reais são recusados antes do worktree quando existem pastas somente leitura ou sem acesso, pois suas regras não garantem isolamento de subpastas. Pastas com leitura e escrita são permitidas.

## D16 — Criação de projeto e worktrees

- Falhas normais na criação revertem os dados e arquivos gerados; isso não garante rollback após interrupção abrupta nem diante de alterações externas concorrentes. A limpeza automática dos worktrees ocorre em respostas concluídas sem commits e sem alterações; outros casos preservam o trabalho para revisão.

## D17 — Desktop isolado

- A casca Tauri usa workspace e lockfile próprios para não impor dependências desktop aos testes do daemon. Os pacotes Linux foram gerados com sysroot temporário; a instalação completa e o macOS ainda precisam de validação nas plataformas de destino.

## D18 — Barreira de conclusão das dependências

- A conclusão anunciada pelo agente não libera dependentes antes do checkpoint e da finalização bem-sucedida do turno. A correção foi motivada pela corrida observada na primeira rodada real de M8 e validada por testes e uma segunda rodada real.

## D19 — Botão de login por plataforma e alinhamento do filtro de pacotes (R2)

- O nome do pacote na pasta `ui` foi alinhado para `"name": "ui"`, garantindo que comandos de verificação com filtro pnpm (`pnpm --filter ui test` e `pnpm --filter ui lint`) executem diretamente sem erros de incompatibilidade de filtro.
- No componente `AccountCard` (`ui/src/screens/Profiles.tsx`), foi implementada a orientação e botão de login interativo para cada provedor suportado (Claude, ChatGPT/Codex e Antigravity/agy), conforme o tipo de login selecionado (`subscription` ou `api_key`).
- Os botões de login abrem em nova aba com `target="_blank"` e `rel="noopener noreferrer"`, direcionando para o portal de login da plataforma (ex.: `claude.ai/login`, `chatgpt.com/`, `antigravity.google/` ou consoles de chaves de API) e exibem o comando correspondente no terminal (`claude auth login --claudeai`, `codex login`, `agy`) ou nome da variável de ambiente requerida.

## D20 — Padrões de criação e MCP do sistema

- `POST /projects` sem `base_branch` usa a branch atual do repositório (`git rev-parse --abbrev-ref HEAD`), mantendo a verificação da D15. `POST /agents` aceita `name` ausente (rótulo do motor com sufixo único no projeto: "Claude", "Claude 2") e `role` ausente ("Orquestrador" ou "Agente"); sem `parent_id`, um agente que não é orquestrador fica abaixo do orquestrador raiz, se houver.
- O MCP existente ganhou `get_project`, `update_project`, `create_agent` e `update_agent`, recusadas a quem não é orquestrador. Elas chamam as mesmas funções das rotas REST e regeneram os arquivos do time. Pelo MCP só se alteram nome, papel, modelo e instruções (`settings.instructions`) de um agente, nunca permissões, conta ou pausa.
- Pelo MCP o orquestrador só aperta a segurança do projeto (acrescenta bloqueios, restringe ou acrescenta pastas, reduz ou define o orçamento do dia), pois lê conteúdo não confiável; a rota REST do usuário continua livre.

## D21 — Login de conta pelo próprio CLI (substitui os links da D19)

- Cada conta mostra o status do login e um botão **Entrar** (ou **Reconectar**). `GET /accounts/{id}/status` roda o comando de status do CLI (`cli_path` da conta, ambiente limpo, 5 s): `claude auth status` (JSON `loggedIn`) e `codex login status` (código de saída). O agy não tem comando de status, então fica `unknown`; contas com chave de API também, pois o daemon não lê chaves.
- **Entrar** abre `GET /accounts/{id}/login`, o mesmo PTY do terminal sob demanda (D11), rodando `claude auth login --claudeai`, `codex login` ou `agy` (que pede o login ao abrir). Ao fechar, a UI consulta o status de novo. A tabela fica em `adapters::auth_commands`; o provedor `fake` roda `cli_path` sem argumentos, para os testes.
- Tipo de login, caminho do CLI, modelo, concorrência e limites ficam num bloco "Avançado" recolhido. Os links para portais e as instruções da D19 saíram.

## D22 — Seletor de pasta aberto pelo daemon

- **Encontrado**: o navegador não entrega caminho absoluto (`showDirectoryPicker` só dá um handle), e a UI roda tanto no navegador quanto na casca Tauri. O plano de simplificação deixava o seletor nativo fora do escopo por exigir plugin do Tauri.
- **Escolhido**: `POST /api/v1/pick-folder`, atrás do `loopback_guard`, faz o daemon (local, na sessão gráfica do usuário) abrir o diálogo de pasta e devolver `{ "path": "/abs" }` ou `{ "path": null }` se cancelado. Começa em `$HOME`, sem limite de tempo para o usuário escolher. Só um diálogo por vez (409); sem sessão gráfica, 503 "Não foi possível abrir o seletor de pastas; digite o caminho." O campo Pasta continua editável.
- **Linux**: crate `rfd` 0.18 com `default-features = false` e só o recurso `xdg-portal` (sem GTK nem Wayland). O rfd carrega `libdbus-1` em tempo de execução e cai no `zenity` se o portal faltar, então não há dependência de compilação. Como o rfd devolve falha como cancelamento, o daemon confere antes se há tela (`WAYLAND_DISPLAY`/`DISPLAY`) e barramento de sessão; uma falha do portal depois disso aparece como cancelamento.
- **macOS**: o rfd precisa do laço principal do AppKit, que o daemon não roda (o diálogo travaria), então o daemon usa `osascript` (`choose folder`), sem dependência nova. Falta validar na plataforma, como na D17. Outros sistemas respondem 503.

## D23 — Modelos reais, raciocínio e uso em % disponível

- **Modelos**: `GET /accounts/{id}/models` roda o comando de lista do provedor (ambiente limpo, 8 s, cache de 5 min por conta): `codex debug models` (JSON; só `visibility == "list"`; `slug`, `display_name`, `supported_reasoning_levels[].effort`, `default_reasoning_level`) e `agy models` (uma linha `slug<TAB>Nome`; linhas sem TAB são ignoradas). Claude usa a lista fixa do `claude --help` (`opus`, `sonnet`, `haiku`, `fable`). Falha gera lista vazia e `detail`; a UI mantém "Padrão da conta" e "Outro…". Formatos confirmados em 15/11/2026 com codex-cli 0.166.0 e agy 1.5.9; as fixtures em `fixtures/models/` são sintéticas no mesmo formato.
- **Raciocínio**: `agent.settings.effort`, validado contra `low, medium, high, xhigh, max, ultra` antes de virar argumento: Codex `-c model_reasoning_effort=<x>`; Claude `--effort <x>` (existe no `claude --help` 2.1.x; `ultra` não é enviado); agy não tem campo, pois o nível já vem no slug do modelo.
- **Uso**: a UI mostra "X% disponível" (100 − usado) nas janelas de 5h e semana, com "reinicia às …"; sem % do CLI, "sem dados de cota". A estimativa gasto ÷ limite em US$ e os limites em dólares da conta saíram (só alimentavam essa estimativa; o daemon não bloqueia por eles). `pct_source` ganhou `none`. Claude: `rate_limit_event`. Codex: o daemon lê o último `rate_limits` de um `token_count` no arquivo de sessão mais recente em `$CODEX_HOME/sessions` (padrão `~/.codex/sessions`), só os números (`used_percent`, `window_minutes` 300/10080, `resets_at`); janelas já reiniciadas são ignoradas. O agy não informa cota.
- Fontes: https://learn.chatgpt.com/docs/developer-commands?surface=cli , https://learn.chatgpt.com/docs/config-file/config-reference , https://antigravity.google/docs/cli/headless.md
- Pendente: o `token_count` no stdout do `codex exec --json` não foi confirmado; o uso vem do arquivo de sessão, que também reflete outros clientes da mesma conta.

## D24 — Canvas com @xyflow/react

- **Escolhido**: `@xyflow/react` 12.12.1, fixado sem `^`, é a única dependência nova do canvas (T4/T5 do `docs/plano-regente-2.md`). Pan, zoom, arrastar, foco por teclado e arestas vêm prontos; sem elkjs/dagre: `ui/src/canvas/layout.ts` faz uma árvore simples (orquestrador no topo), recalculada só quando ids ou `parent_id` mudam.
- **Layout salvo**: em `localStorage` (`regente.canvas.<projeto>`), com try/catch. Posições fixadas por arrasto valem nos dois modos (Execução é somente leitura e mostra o mesmo arranjo); o viewport é guardado por modo. `fitView` só quando não há viewport salvo; um agente novo não move nós fixados.
- **Arestas**: Estrutura = `Agent.parent_id`. Execução = `task_assigned` com `task_id` (do dono da tarefa-pai ao destinatário), `depends_on` tracejada, `blocked_by` âmbar, `failed` vermelha quando há evento `error` da tarefa ainda aberta. Texto de mensagem nunca vira aresta. Os eventos vêm de `subscribeEvents` mais as últimas 100 entradas da timeline de cada agente, pois o store não guarda eventos.
- **Status do agente**: projeção pura em `ui/src/lib/projection.ts`; sem tarefa aberta o agente fica Disponível, mesmo depois de concluir.

## D25 — Editar projeto depois de criado

- **Escolhido**: `PATCH /projects/{id}` reaproveita `api::update_project` (as mesmas validações da criação) e o modal `NewProject` em modo edição, aberto por "Configurações do projeto" na topbar. O usuário edita nome, pasta e descrição.
- **Pasta**: passa pelas mesmas checagens da criação (caminho absoluto, raiz de um repositório Git, arquivos de configuração sem links simbólicos). A branch base e as pastas com permissão atuais precisam continuar válidas no novo repositório. A troca é recusada enquanto houver tarefa que não esteja `done`/`cancelled`, porque os worktrees moram em `<pasta>/.regente/`. Os arquivos do time são gravados de novo na pasta nova; os da pasta antiga ficam como estão.
- **Orquestrador**: pelo MCP continua sem poder mudar nome ou pasta (`update_project(..., by_user = false)`).

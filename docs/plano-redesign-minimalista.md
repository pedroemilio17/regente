# Épico: redesign minimalista (canvas em tela cheia, chat sobreposto, barra no topo)

Para o Maestro e o time do Regente. Leia também `CLAUDE.md`, `docs/plano-regente-2.md` (o que já existe) e `docs/decisoes.md`.

## Objetivo

O canvas ocupa a janela inteira, de borda a borda. Todo o resto flutua sobre ele: barra de ferramentas no topo, painel de projetos à esquerda, chat à direita e zoom no canto inferior direito. O chat continua sendo a peça principal: é por ele que o usuário comanda o Maestro.

## Referência visual (descrita em texto; os agentes não veem as imagens)

A inspiração é o app Maestri. Use só o padrão de layout, nunca nome, logo ou textos dele.

- **Fundo**: grade de pontos discreta sobre `--canvas`, cobrindo 100% da janela. Não há coluna fixa nem barra que roube área do canvas.
- **Barra superior central**: uma "pílula" flutuante, centralizada no topo (~12px da borda), com raio grande, fundo `--panel`, borda `--border` e sombra leve. Contém só ícones (com `aria-label` e tooltip): selecionar, mover (mão/pan), novo agente, nova tarefa, alternar Grafo/Lista, alternar Estrutura/Execução e configurações.
- **Barra contextual**: com um agente selecionado, aparece logo abaixo da barra superior uma segunda pílula menor, só com ícones: editar agente, falar com ele no chat, abrir atividade, pausar ou retomar, e reorganizar. Some quando a seleção é limpa.
- **Painel esquerdo**: lista de projetos flutuante, ~240px, recolhível para um botão. Tem campo "Filtrar", botão "+" (novo projeto) e menu de contexto por projeto (clique direito ou botão "⋯"): Configurações do projeto, Copiar caminho, Novo agente.
- **Chat direito**: painel sobreposto ao canvas, como o chat do VS Code: preso à direita com margem de ~8px, altura cheia, cantos arredondados e sombra. Pode ser redimensionado (320–520px), recolhido para um botão e reaberto. As abas Chat / Atividade / Arquivos continuam.
- **Canto inferior direito**: controles flutuantes de zoom (−, porcentagem, +, ajustar à tela).
- **Configurações**: um modal grande com abas no topo (Projeto, Agentes, Contas, Uso), cada aba com sub-abas em "segmented control" quando precisar. Linhas no formato rótulo à esquerda, controle à direita e descrição em cinza embaixo.
- **Editar agente**: um modal médio com "Cancelar" à esquerda, título no centro e "Salvar" à direita, e três abas: Detalhes (nome, papel, motor, modelo, raciocínio), Aparência (pet) e Agente (instruções, permissões, orçamento, se é Maestro).

## O que sai da tela

- O cabeçalho grande ("Orquestrador", chips de contagem e gasto), o card "Aprovações pendentes" fixo e a caixa "Nova tarefa" acima do canvas.
- As aprovações viram blocos no chat, com botões Aprovar/Negar e confirmação, mais um contador na barra superior. Tarefas novas são criadas pelo chat ou pelo ícone "nova tarefa", que abre um modal pequeno.
- A tela de Perfis/Contas como rota própria passa para a aba Contas das configurações.

## Regras para todas as tarefas

- Mudança mínima e completa. Reaproveite `Canvas`, `MaestroChat`, `Inspector`, `ModelEffort`, `NewProject` (que já tem modo edição), `UsageBars`, `Modal` e a API existente. Sem dependência nova, exceto se a tarefa disser.
- Use os tokens de `ui/src/styles.css` (`--app`, `--canvas`, `--panel`, `--raised`, `--border`, `--text`, `--muted`, `--accent`, `--success`, `--warning`, `--danger`, `--info`, `--radius-*`). Não crie cores novas.
- **Para evitar conflito entre branches**: só a R1 edita `ui/src/styles.css`. Cada outra tarefa põe seu CSS num arquivo próprio, importado pelo próprio componente, e em `App.tsx` mexe só na linha que monta o seu componente.
- Acessibilidade: tudo alcançável por teclado, foco visível, `aria-label` nos botões só de ícone, nada que dependa só de hover e `prefers-reduced-motion` respeitado.
- Segurança: o texto dos agentes não é confiável (o chat já usa `AgentMarkdown`, sem HTML cru); nenhuma ação destrutiva sem confirmação; nada de `.env*`.
- No worktree, antes de testar: `pnpm install --frozen-lockfile`. Para entregar, estes três precisam passar: `pnpm --filter ui test`, `pnpm --filter ui build` e `pnpm --filter ui lint`. Testes vitest novos para o comportamento novo.
- Mensagem de commit em inglês, no padrão conventional (`feat(ui): ...`).

## Time

| Agente | Motor | Modelo | Raciocínio | Papel |
|---|---|---|---|---|
| Maestro | Claude | (atual) | (atual) | orquestra, cria subtarefas com `depends_on`, integra |
| Haiku 1, Haiku 2 | Claude | `haiku` | medium | codar tarefas pequenas e médias |
| Luna 1, Luna 2 | Codex | `gpt-6-luna` | high | codar layout e lógica |
| Hydrogen 1, Hydrogen 2 | Antigravity | `gemini-4-hydrogen-high` | (no nome do modelo) | codar componentes de interação |
| Revisor | Claude | `sonnet` | medium | revisar cada branch antes da integração |
| Testador | Codex | `gpt-6.1-sol` | high | testar funcionalidade, acessibilidade e segurança |

Limites das contas: Claude roda 3 agentes ao mesmo tempo, Codex 2 e Antigravity 2. O que passar disso fica na fila (dá para aumentar em Contas).

## Tarefas

### Onda 1 (base; todas as outras dependem dela)

**R1 — Shell em tela cheia** · Luna 1
- `App.tsx` e `styles.css`: o canvas vira camada de fundo `position: fixed; inset: 0`. Topbar e sidebar deixam de ser colunas do grid e passam a camadas flutuantes (`position: absolute`, com `z-index` em escala definida por tokens `--z-*`).
- Crie as áreas de montagem: topo central, contextual abaixo dela, esquerda, direita (chat) e canto inferior direito. Por enquanto monte nelas os componentes atuais.
- A grade de pontos do canvas cobre a tela toda.
- Remova de `Overview.tsx` o cabeçalho grande, os chips e a caixa "Nova tarefa"; as aprovações continuam visíveis provisoriamente até a R6.
- Aceite:
  - nenhuma barra de rolagem na página;
  - o canvas recebe pan e zoom em toda a janela, inclusive atrás dos painéis (só a área fora dos painéis recebe o arrasto);
  - funciona em 1024×768 e 1440×900;
  - testes atualizados.

### Onda 2 (em paralelo, depois da R1)

**R2 — Barra superior e barra contextual** · Hydrogen 1
- Novo `ui/src/canvas/Toolbar.tsx` (+ `toolbar.css`), com a barra superior e a contextual descritas acima.
- Ferramenta "mover": arrastar no fundo faz pan. Ferramenta "selecionar": arrastar no fundo faz seleção por área, se o React Flow oferecer isso pronto; senão, a ferramenta só seleciona por clique.
- Atalhos: V (selecionar), H (mover), N (novo agente), F (ajustar à tela), ignorados enquanto o foco está num campo de texto.
- Os alternadores Grafo/Lista e Estrutura/Execução saem da toolbar interna de `Canvas.tsx` e passam para cá. Exponha no store o que precisar, sem duplicar estado.
- Aceite: navegação por teclado entre os ícones (roving tabindex ou Tab) e testes dos atalhos.

**R3 — Chat sobreposto, estilo VS Code** · Haiku 1
- `components/Chat.tsx` e um `chat.css` novo.
- Painel flutuante à direita, com margem, sombra e raio. Botão de recolher/abrir que lembra o estado por projeto (a largura já é lembrada).
- Composer embaixo, com textarea que cresce sozinha até ~8 linhas; Enter envia e Shift+Enter quebra linha.
- Respostas em Markdown já existem (`AgentMarkdown`): ajuste só o espaçamento das bolhas para leitura confortável, com resposta do agente sem bolha (texto corrido, como no Claude Code) e mensagem do usuário em bolha discreta.
- Aceite: rascunho e scroll preservados ao recolher/abrir e testes do Enter/Shift+Enter.

**R4 — Modal de configurações com abas** · Haiku 2
- Novo `components/Settings.tsx` (+ `settings.css`) com as abas:
  - **Projeto**: o formulário de `NewProject` em modo edição, embutido;
  - **Agentes**: lista de agentes do projeto com botão Editar, que abre o modal da R5;
  - **Contas**: o conteúdo atual de `screens/Profiles.tsx`;
  - **Uso**: `UsageBars`.
- Aberto pelo ícone de configurações da barra superior e pelo menu do projeto.
- Remova a rota de Perfis se ela ficar sem uso.
- Aceite: Esc fecha, o foco volta ao gatilho e testes de troca de aba.

**R5 — Modal "Editar agente"** · Luna 2
- Novo `components/AgentEditor.tsx` (+ css), com o cabeçalho Cancelar / título / Salvar e as abas:
  - **Detalhes**: nome, papel, motor (só leitura), modelo e raciocínio via `ModelEffort`;
  - **Aparência**: pet, reaproveitando `lib/pets.ts`;
  - **Agente**: instruções, permissões e orçamento (o que `Inspector.tsx` já edita; mova, não duplique).
- Salva com `api.patchAgent`.
- Aberto pela barra contextual (R2), pela aba Agentes (R4) e pelo "Configurar" do Inspector.
- Aceite: os erros da API aparecem no modal e os testes cobrem salvar e cancelar.

**R6 — Painel de projetos e aprovações no chat** · Hydrogen 2
- Painel esquerdo flutuante (`components/ProjectPanel.tsx` + css) no lugar da `Sidebar`, com filtro, "+" e o menu de contexto por projeto. "Copiar caminho" usa `navigator.clipboard`, com fallback e aviso.
- Aprovações pendentes: viram um bloco no chat (componente `ApprovalMessage`), com contexto, Aprovar e Negar; o merge pede confirmação. Ganham também um contador na barra superior, que leva ao bloco.
- Remova o card fixo de aprovações do `Overview.tsx`.
- Aceite:
  - o menu de contexto abre por teclado (Shift+F10 ou botão "⋯");
  - aprovar ou negar pelo chat chama a API existente (`decide_approval`);
  - testes de ambos.

### Onda 3

**R7 — Controles de zoom e acabamento** · Haiku 1 (depois da R3)
- Controles flutuantes no canto inferior direito (−, %, +, ajustar), substituindo os `Controls` padrão do React Flow, com a porcentagem atualizada no pan/zoom.
- Garanta que nenhum painel flutuante cubra os outros em 1024×768.

## Revisão (Revisor · sonnet medium)

Cada tarefa R1–R7 ganha uma subtarefa "Revisar Rn" com `depends_on: [Rn]`. O Revisor:
- lê o diff da branch contra `main`;
- confere as regras acima: escopo, arquivo de CSS próprio, tokens, acessibilidade, testes e nenhuma dependência nova não pedida;
- procura bugs de estado: seleção versus destinatário do chat, rascunho e listeners sem cleanup.

Ele responde "aprovado" ou uma lista objetiva de correções, cada uma com arquivo e linha. Com correções, o Maestro devolve a tarefa ao mesmo agente que a fez. Só é integrado o que o Revisor aprovou.

## Teste (Testador · gpt-6.1-sol high)

Uma tarefa final com `depends_on` em todas as revisões aprovadas.

**Funcional:** atualize e rode `ui/e2e/team.spec.ts` (Playwright, com o daemon falso de `scripts/e2e-daemon.mjs`) cobrindo:
- canvas em tela cheia sem rolagem;
- pan e zoom pelos controles;
- barra contextual ao selecionar um agente;
- editar agente e salvar;
- configurações com troca de abas;
- aprovar pelo chat;
- recolher e abrir o chat preservando o rascunho;
- Enter/Shift+Enter;
- menu de contexto do projeto pelo teclado.

**Visual:** screenshots em 1440×900 e 1024×768, no estado normal, com seleção, com o chat recolhido e com as configurações abertas. Relate sobreposições e cortes.

**Segurança:**
- uma mensagem de agente com `<script>`, `<img src=x onerror>`, link `javascript:` e imagem remota não executa nada nem busca nada;
- aprovar exige clique e confirmação, e nunca acontece só por texto de agente;
- "Copiar caminho" não expõe nada além do caminho;
- nenhum segredo do `.env` aparece na UI;
- rode `pnpm audit --prod` e relate só o que for alto ou crítico.

**Entrega:** relatório com o que passou, o que falhou (com passos para reproduzir) e os caminhos dos screenshots. Não corrige código, só relata; o Maestro distribui as correções.

## Fim

O Maestro integra com aprovação do usuário (D7), atualiza `docs/progresso.md` e registra em `docs/decisoes.md` (D27 em diante) as decisões novas, como a escala de `z-index` e a remoção da rota de Perfis.

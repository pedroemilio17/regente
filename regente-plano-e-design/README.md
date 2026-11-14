# Regente — plano e design

Documentos de planejamento do Regente, o orquestrador local de agentes (Claude Code, Codex e Antigravity CLI).

## Conteúdo

| Pasta | O que tem |
| --- | --- |
| `plano/plano-mvp.md` | Plano de construção do MVP para o Fable: escopo, stack, regras, contratos, marcos M0–M8, definição de pronto e prompt inicial |
| `system-design/system-design.md` | System design: arquitetura, tecnologia e GPU, segurança, tokens, acesso remoto, banco, cache, worktrees, versionamento e escala |
| `*/diagramas/` | Diagramas dos dois documentos em PNG |
| `design/previews/` | Imagens de cada tela do MVP |
| `design/*.dc.html` | Telas do canvas de design. Abra direto no navegador: o `support.js` desta pasta renderiza cada uma, com os botões interativos |
| `design/canvas.json` | Posição e tamanho de cada tela no canvas original |

## Para começar com o Fable

Copie `plano/plano-mvp.md` para `docs/plano-mvp.md` e `system-design/system-design.md` para `docs/system-design.md` num repositório vazio chamado `regente`, junto com as pastas de diagramas e a pasta `design/` como referência visual. Depois cole o prompt do fim do plano.

Observação: as telas de design ainda mostram "Gemini CLI"; o plano e o system design já usam o Antigravity CLI (`agy`) no lugar dele.

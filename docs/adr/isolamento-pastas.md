# Isolamento de pastas por processo: proposta experimental

Estado: protótipo, não integrado. A recusa atual de CLIs reais para projetos com restrições permanece obrigatória até as condições abaixo serem comprovadas. O objetivo é aplicar `write`, `read` e `none` a todos os descendentes do processo, inclusive comandos de shell, não apenas às ferramentas do agente.

## Fronteira proposta para Linux

Criar um namespace de montagem com bubblewrap, namespace de usuário e PID, `/proc` novo e `/dev` mínimo. Não montar `/` do host. Montar somente runtimes e executáveis explicitamente identificados, em leitura, em caminhos que não contenham a raiz do projeto nem o worktree. Criar `/tmp` e `HOME` privados por turno. Preservar a rede para o provedor e o MCP de loopback; a disponibilidade de rede não concede acesso ao filesystem do host, mas serviços locais que leem arquivos em nome do agente precisam impor as mesmas regras.

Montar o worktree real no seu caminho absoluto para evitar sincronização de escrita, com sobreposições de leitura ou máscaras vazias para `read` e `none`, respectivamente. A regra mais restritiva prevalece em sobreposições. Não montar a raiz original ou o diretório de dados do daemon. Como um bind expõe os mesmos inodes, antes de iniciar é necessário rejeitar links simbólicos que escapem da árvore e arquivos com múltiplos hard links, inclusive em diretórios protegidos; impedir alterações concorrentes entre a inspeção e o uso também é obrigatório. Uma verificação simples antes de iniciar não elimina a corrida. Sem um mecanismo efetivo de exclusão ou uma projeção mediada pelo kernel, esta arquitetura não está pronta para produção.

O `.git` do worktree é um arquivo que aponta para metadados reais, cujo object store dá acesso a todo o histórico, inclusive conteúdo `none`. Substituí-lo por um diretório Git privado, criado em uma área temporária fora do worktree, contendo apenas uma árvore inicial filtrada. A lista de exclusões deve aplicar `none` e padrões bloqueados a todos os descendentes, sem seguir symlinks; não copiar histórico, refs, configuração, hooks ou alternates. Não usar o índice ou o object store real. O daemon continua a usar o Git real fora do namespace para checkpoints. O Git privado é deliberadamente uma visão inicial, não preserva o histórico nem as branches reais; testes e experiência do produto precisam avaliar essa limitação. Mesmo um histórico privado deve ser inacessível por aliases como a raiz original e caminhos de configuração do Git.

Para `none`, montar uma máscara vazia sobre o diretório existente; uma entrada ausente deve ser tratada de modo que não possa ser criada pelo processo, ou a configuração deve ser rejeitada. Para `read`, usar bind somente leitura recursivo. Verificar erros de montagem e iniciar apenas após confirmação de que todas as restrições foram aplicadas. Arquivos especiais, pontos de montagem aninhados, caminhos com symlink, sobreposição de pasta do projeto com runtime/perfil e hard links exigem tratamento seguro explícito ou recusa. O daemon não deve aceitar uma descrição de sucesso simplesmente porque o processo bubblewrap iniciou.

## Estado dos provedores e plataformas

| Ambiente | Plano | Situação |
| --- | --- | --- |
| Linux / Claude | Descobrir executável e dependências; provisionar somente configuração e autenticação necessárias em HOME privado. | Pendente: mapear estado, dependências e renovação de autenticação. |
| Linux / Codex | Mesmo procedimento, incluindo configuração MCP local e mecanismos do sandbox aninhado. | Pendente: verificar compatibilidade de sandbox aninhado e estado de sessão. |
| Linux / agy | Mesmo procedimento; registrar bridge MCP sem usar o HOME global do host. | Pendente: definir contrato de registro e credenciais. |
| macOS | Investigar mecanismo nativo que satisfaça as mesmas propriedades, incluindo histórico Git e aliases. | Não comprovado; `sandbox-exec` não é considerado solução por si só. Continuar recusando. |
| Outros ou Linux sem namespaces | Nenhuma garantia disponível. | Continuar recusando com erro claro. |

A autenticação do próprio provedor pode ser lida pelo processo do provedor; o requisito aqui é impedir exposição das pastas do projeto. Copiar somente os arquivos de autenticação/configuração necessários, sem histórico, cache ou logs antigos. Caminhos e dependências de executáveis instalados pelo usuário não podem trazer diretórios inteiros arbitrários ao sandbox. O daemon não deve registrar conteúdos de credenciais.

## Critérios de aceitação antes de integrar

- Testar processos reais com leitura e escrita permitidas, `read` sem escrita, `none` sem leitura nem escrita, regras sobrepostas e restrição na raiz.
- Verificar caminhos absolutos para a raiz original, symlinks internos e externos, hard links, `/proc`, caminhos de Git alternativos, `git show` e `git cat-file`, incluindo conteúdo apenas histórico.
- Verificar que ferramentas MCP e subprocessos não executam operações de arquivo fora da mesma fronteira; considerar serviços locais acessíveis e descritores herdados.
- Comprovar proteção contra corrida ao modificar caminhos durante a execução, inclusive entre verificação e montagem; testar montagem aninhada e manipulação de diretórios ancestrais.
- Testar cada CLI real autenticado, rede e MCP, cancelamento, isolamento de estado, ambiente sem bubblewrap e falhas parciais. Não utilizar credenciais reais nem fazer chamadas pagas na fixture.
- Avaliar limite de confiança: processos locais externos com acesso de escrita ao worktree podem modificar seu conteúdo concorrentemente; o modelo e sua mitigação devem ser explícitos.

O módulo experimental `security/folder_sandbox.rs` só constrói argumentos para uma topologia controlada; não prepara Git filtrado, autenticação, verificação de aliases ou defesa contra corridas. Não é uma fronteira de segurança e não deve ser conectado ao caminho de produção antes dos critérios acima.

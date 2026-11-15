# Regente

Aplicativo local para coordenar agentes de programação. O daemon Rust mantém tarefas e eventos em SQLite e oferece uma API HTTP/WebSocket; a interface React apresenta o estado e a casca Tauri abre essa interface no desktop.

## Desenvolvimento

São necessários Rust/Cargo, Node.js e pnpm. Instale as dependências JavaScript com `pnpm install`; em seguida:

```sh
cargo test -p daemon
pnpm --filter ui build
cargo run -p daemon --bin regented
```

Abra `http://127.0.0.1:7717/`. O daemon usa `~/.config/regente/` para dados e configuração; `REGENTE_HOME`, `REGENTE_PORT`, `REGENTE_UI_DIR` e `REGENTE_FIXTURES` permitem alterar os caminhos e a porta. Para desenvolvimento com Vite, inicie o daemon e execute `pnpm dev` em outro terminal; o proxy encaminha a API ao daemon na porta 7717.

## Desktop e distribuição

A casca é um workspace Cargo independente em `app/src-tauri`, para que testes do daemon não precisem resolver as dependências Tauri. Para compilar é necessário acesso ao registro Cargo e ao npm na primeira execução, a CLI Tauri 2, e os [pré-requisitos oficiais do Tauri](https://v2.tauri.app/start/prerequisites/) para a plataforma. No Linux eles incluem WebKitGTK 4.1 e GTK 3 de desenvolvimento; no macOS são necessárias as ferramentas Xcode. O script obtém a CLI Tauri 2 via `pnpm dlx` e compila o daemon e a interface antes de preparar o pacote:

```sh
./scripts/build-desktop.sh
```

Execute no Linux para gerar os formatos Linux e no macOS para gerar os formatos macOS; os resultados ficam em `app/src-tauri/target/<target>/release/bundle/`. O script usa o alvo do host ou `TAURI_ENV_TARGET_TRIPLE`, que deve corresponder à plataforma de compilação e às dependências instaladas. A distribuição e o funcionamento dos pacotes ainda precisam ser verificados nas respectivas plataformas; não há binários publicados neste repositório.

Ao abrir, o aplicativo conecta-se ao daemon configurado ou inicia uma cópia incluída no pacote. O daemon serve a interface e as fixtures incluídas usando caminhos absolutos. Se já houver um daemon ativo, ele precisa estar servindo uma interface compatível: configure `REGENTE_UI_DIR` antes de iniciá-lo. Fechar a janela não encerra o daemon nem seus agentes; para pará-lo, encerre o processo `regented` manualmente após concluir as tarefas. Caso um aplicativo instalado seja removido ou atualizado, reinicie o daemon antes de abrir a nova versão para que passe a usar os recursos atualizados. Configure o `PATH` do ambiente de sessão gráfica para que os CLIs dos agentes estejam disponíveis ao daemon; aplicativos gráficos podem não herdar configurações do shell.

A validação com CLIs reais e a medição de CPU/GPU fazem parte do aceite do MVP e não são substituídas pela compilação do aplicativo.

## Inspecionar um pacote Linux sem instalar

Para um pacote Debian, use `ar` e `tar` em um diretório temporário. Ajuste o caminho para o pacote recém-gerado:

```sh
pacote="$PWD/app/src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/deb/Regente_0.1.0_amd64.deb"
extracao="$(mktemp -d)"
(cd "$extracao" && ar x "$pacote" && tar -xf data.tar.*)
find "$extracao/usr" -type f | sort
```

Confirme `usr/bin/regente-desktop`, `usr/bin/regented`, `usr/bin/regente`, `usr/lib/Regente/ui/index.html` e `usr/lib/Regente/fixtures/`. Para um RPM, `rpm -qip pacote.rpm` e `rpm -qlp pacote.rpm` inspecionam metadados e arquivos; para extrair sem instalar, execute `rpm2cpio /caminho/absoluto/pacote.rpm | (cd "$extracao" && cpio -idm --quiet)` com um diretório temporário vazio.

Com uma sessão gráfica e as bibliotecas de execução instaladas, o aplicativo extraído pode ser aberto isoladamente:

```sh
REGENTE_HOME="$extracao/dados-teste" REGENTE_PORT=18717 "$extracao/usr/bin/regente-desktop"
```

Escolha uma porta local livre. Nesse modo o Tauri encontra recursos relativos ao executável; ao iniciar seu próprio daemon, a casca define os caminhos da UI e das fixtures incluídas no pacote. Se já houver um daemon compatível na porta escolhida, a casca o reutiliza. Fechar a janela deixa esse daemon ativo; encerre-o manualmente após o teste e preserve o diretório extraído até ele parar.

## Primeiros passos sem gasto

1. Compile a interface e inicie o daemon como na seção de desenvolvimento. Abra `http://127.0.0.1:7717/`.
2. Crie um repositório Git de teste com um commit inicial e branch `main`. Na interface, crie um projeto apontando para a raiz desse repositório.
3. Adicione um agente com motor `fake` e marque-o como orquestrador. Crie uma tarefa simples na visão geral e acompanhe os eventos. Os cenários falsos usam as fixtures locais e não chamam serviços pagos.

## Preparar o cenário real M8

O cenário real exige autorização antes de iniciar qualquer tarefa e pode gerar cobranças. Para o time com Codex e Antigravity, primeiro compile `cargo build -p daemon --bins` e `pnpm --filter ui build`. Depois execute `python3 scripts/prepare-m8.py --team codex-agy`. Esse time usa Codex no orquestrador, backend e QA, e agy no frontend; portanto não valida o cenário original com três CLIs. Sem a opção, o script prepara o time clássico com Claude Code, Codex e agy. O script cria um repositório temporário com um valor falso de teste ignorado pelo Git, um diretório de dados isolado e os quatro agentes reais; ele não cria tarefa nem inicia CLIs. Mostra a URL, o PID e o diretório preservado para revisão.

Após a autorização, abra a URL e crie uma tarefa para o orquestrador com orçamento **US$ 2** e o texto: “Adicionar uma rota GET /health que retorna a versão, uma tela que mostra esse status e um teste para cada uma. Delegue backend e frontend em paralelo, faça o frontend perguntar ao backend pelo mensageiro sobre o formato da resposta e inicie QA após ambas as etapas.” O contexto preparado especifica Node.js com biblioteca padrão, testes node:test e arquivos separados para backend e frontend. Acompanhe delegação, mensagens, worktrees, custos e a aprovação final pela interface, e confira os demais itens em `docs/plano-mvp.md`. O limite depende dos valores de uso informados pelos CLIs; não é garantia de cobrança máxima pelo provedor. A primeira execução do agy pode alterar seu registro MCP global, conforme D13 em `docs/decisoes.md`. Não aprove o merge até revisar o resultado. Para encerrar, use o PID exibido pelo script; o diretório temporário permanece disponível até remoção manual.

Neste ambiente Fedora 44, um pacote Debian foi gerado sem instalar pacotes de desenvolvimento no sistema: foram extraídos em um diretório temporário RPMs oficiais de desenvolvimento compatíveis com os runtimes existentes, incluindo dependências transitivas. Seus arquivos `.pc` e links para bibliotecas foram disponibilizados por `PKG_CONFIG_PATH` e `LIBRARY_PATH`. Esse procedimento é específico das versões do ambiente; prefira os pré-requisitos da distribuição. A execução do pacote e os pacotes RPM e macOS ainda precisam de verificação. A automação `.github/workflows/desktop.yml` instala os requisitos no runner Linux e produz artefatos temporários de CI, sem publicá-los como release.

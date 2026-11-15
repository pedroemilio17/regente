# Regente: orientação para desenvolvimento

Leia `docs/plano-mvp.md` e `docs/decisoes.md` antes de alterar comportamento. O daemon Rust/SQLite está em `daemon/`; a interface React/TypeScript/Vite em `ui/`; a casca Tauri 2 em `app/src-tauri/`, com workspace Cargo independente. Contratos ficam em `shared/` e saídas falsas em `fixtures/`.

Comandos comuns: `cargo test -p daemon`, `cargo fmt --all --check`, `cargo clippy -p daemon --all-targets -- -D warnings`, `pnpm --filter regente-ui test`, `pnpm --filter regente-ui build` e `./scripts/build-desktop.sh`. Os testes usam agentes falsos; testes com CLIs reais exigem autorização. Não leia, imprima nem copie `.env*`, chaves ou segredos. Não faça push ou deploy sem autorização. Preserve o processo do daemon ao fechar a janela desktop.

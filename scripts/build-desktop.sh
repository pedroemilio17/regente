#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
command -v cargo >/dev/null || { echo 'cargo não encontrado' >&2; exit 1; }
command -v pnpm >/dev/null || { echo 'pnpm não encontrado' >&2; exit 1; }
triple="${TAURI_ENV_TARGET_TRIPLE:-$(rustc -vV | sed -n 's/^host: //p')}"
case "$triple" in
  *-linux-*|*-apple-darwin) ;;
  *) echo "Plataforma não suportada: $triple" >&2; exit 1 ;;
esac
pnpm --filter ui --fail-if-no-match build
cargo build --release -p daemon --bins --target "$triple"
for binary in regented regente; do
  install -m 755 "target/$triple/release/$binary" "app/src-tauri/binaries/$binary-$triple"
done
cd app/src-tauri
# dlx downloads the Tauri CLI on first use; Cargo resolves the Tauri crates.
pnpm dlx @tauri-apps/cli@2 build --target "$triple" "$@"

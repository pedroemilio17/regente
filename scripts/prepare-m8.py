#!/usr/bin/env python3
"""Prepare M8 with real-agent definitions; no task or CLI is started here."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
from urllib.error import URLError, HTTPError
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parent.parent
DAEMON = ROOT / "target/debug/regented"
UI = ROOT / "ui/dist"


def api(port, path, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    request = Request(f"http://127.0.0.1:{port}/api/v1{path}", data=data,
                      headers={"Content-Type": "application/json"} if data else {})
    with urlopen(request, timeout=5) as response:
        return json.load(response)


def main():
    parser = argparse.ArgumentParser(description="Prepara um cenário M8 sem iniciar agentes.")
    parser.add_argument("--team", choices=("classico", "codex-agy"), default="classico")
    args = parser.parse_args()
    if not DAEMON.is_file() or not (ROOT / "target/debug/regente").is_file() or not (UI / "index.html").is_file():
        sys.exit("Compile antes: cargo build -p daemon --bins && pnpm --filter ui build")
    workspace = Path(tempfile.mkdtemp(prefix="regente-m8-"))
    repo = workspace / "repo"
    repo.mkdir()
    # Values are deliberately fake, excluded from Git, and never printed.
    (repo / ".env").write_text("REGENTE_M8_FAKE=valor-falso-de-teste\n")
    (repo / ".gitignore").write_text(".env\n")
    (repo / "README.md").write_text("# Repositório temporário do cenário M8\n")
    subprocess.run(["git", "init", "-b", "main", str(repo)], check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["git", "-C", str(repo), "add", "README.md", ".gitignore"], check=True)
    subprocess.run(["git", "-C", str(repo), "-c", "user.name=Regente M8", "-c", "user.email=m8@example.invalid", "commit", "-m", "Cenário inicial"], check=True, stdout=subprocess.DEVNULL)
    # Reserve an ephemeral port briefly; fail safely if another process wins the race.
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    env = os.environ.copy()
    env["PATH"] = os.pathsep.join([str(ROOT / "target/debug"), str(Path.home() / ".cargo/bin"), env.get("PATH", "")])
    env.update(REGENTE_HOME=str(workspace / "home"), REGENTE_PORT=str(port),
               REGENTE_UI_DIR=str(UI), REGENTE_FIXTURES=str(ROOT / "fixtures"))
    proc = subprocess.Popen([str(DAEMON), "serve"], env=env, stdin=subprocess.DEVNULL,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            start_new_session=True)
    (workspace / "daemon.pid").write_text(str(proc.pid) + "\n")
    print(f"Diretório preservado: {workspace}", flush=True)
    try:
        for _ in range(100):
            if proc.poll() is not None:
                raise RuntimeError("daemon terminou ao iniciar")
            try:
                if (workspace / "home/regente.db").exists() and api(port, "/health").get("api_version") == "1":
                    break
            except (OSError, URLError, ValueError):
                pass
            time.sleep(0.1)
        else:
            raise RuntimeError("daemon não ficou pronto")
        team = [
            {"name": "Orquestrador", "cli": "claude", "role": "Orquestrar", "is_orchestrator": True},
            {"name": "Backend", "cli": "codex", "role": "Backend"},
            {"name": "Frontend", "cli": "agy", "role": "Frontend"},
            {"name": "QA", "cli": "claude", "role": "QA"},
        ]
        if args.team == "codex-agy":
            team[0]["cli"] = "codex"
            team[3]["cli"] = "codex"
        context = (
            "Repositório temporário para o cenário de aceite. "
            "Implemente usando apenas Node.js e sua biblioteca padrão, sem baixar dependências. "
            "Backend: servidor HTTP com GET /health retornando a versão; arquivos backend/ e teste node:test. "
            "Frontend: HTML que consulta e exibe o status; arquivos frontend/ e teste node:test. "
            "O frontend deve perguntar ao backend pelo mensageiro qual é o formato da resposta. "
            "Delegue backend e frontend em paralelo, cada um em seus arquivos; "
            "QA só começa após ambos concluírem e verifica os testes. "
            "Não faça push, deploy ou chamadas pagas adicionais."
        )
        result = api(port, "/projects", {"name": "Aceite M8", "root": str(repo),
                    "context": context,
                    "day_budget_usd": 2, "agents": team})
        print(f"Projeto criado: {result['project']['id']}")
        print(f"Interface: http://127.0.0.1:{port}/")
        print(f"Daemon PID: {proc.pid}")
        print("Nenhuma tarefa foi criada e nenhum CLI foi executado. Após autorização, crie na interface a tarefa documentada no README, com orçamento US$ 2.")
    except (OSError, URLError, HTTPError, RuntimeError, ValueError, KeyError) as error:
        # Avoid dumping HTTP bodies or daemon output, which may include sensitive data.
        proc.terminate()
        print(f"Falha na preparação ({type(error).__name__}); repositório preservado em {workspace}.", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()

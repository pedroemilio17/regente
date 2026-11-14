//! `regented`: the Regente daemon.
//!
//! - `regented` — serve the API on 127.0.0.1 (default port 7717).
//! - `regented mcp-stdio` — stdio bridge to the messenger, for CLIs that only
//!   accept stdio MCP servers (reads `REGENTE_MCP_URL`).

use std::process::ExitCode;

use regente_daemon::config::Config;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("mcp-stdio") => {
            let Ok(url) = std::env::var("REGENTE_MCP_URL") else {
                eprintln!("REGENTE_MCP_URL is not set: this command is started by an agent run by Regente");
                return ExitCode::FAILURE;
            };
            match regente_daemon::mcp::stdio_bridge(&url) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("mcp bridge failed: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("--version") | Some("-V") => {
            println!("regented {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--help") | Some("-h") => {
            println!("regented [serve]      serve the API (REGENTE_PORT, REGENTE_HOME)\nregented mcp-stdio    stdio bridge to the messenger MCP");
            ExitCode::SUCCESS
        }
        None | Some("serve") => {
            // Structured logs, one JSON object per line.
            tracing_subscriber::fmt()
                .json()
                .with_writer(std::io::stderr)
                .init();
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    eprintln!("could not start the async runtime: {error}");
                    return ExitCode::FAILURE;
                }
            };
            match runtime.block_on(regente_daemon::serve(Config::from_env())) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    tracing::error!(%error, "daemon stopped");
                    ExitCode::FAILURE
                }
            }
        }
        Some(other) => {
            eprintln!("unknown command: {other}");
            ExitCode::FAILURE
        }
    }
}

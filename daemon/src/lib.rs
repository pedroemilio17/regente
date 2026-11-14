//! Regente daemon: runs coding agents, stores their events and serves the
//! API. The interface is only a client of this process.

pub mod adapters;
pub mod api;
pub mod app;
pub mod config;
pub mod mcp;
pub mod model;
pub mod orchestrator;
pub mod security;
pub mod store;
pub mod team;
pub mod usage;
pub mod worktree;

use std::net::SocketAddr;
use std::sync::Arc;

use app::{App, AppError, AppResult};
use config::Config;

/// Start the daemon on an already-bound listener: event batcher, scheduler and
/// HTTP server. Returns the shared state and the address in use.
pub async fn start(
    cfg: Config,
    listener: tokio::net::TcpListener,
) -> AppResult<(Arc<App>, SocketAddr)> {
    let (app, events) = App::open(cfg)?;
    let addr = listener.local_addr()?;
    *app.bound_port.write().unwrap_or_else(|p| p.into_inner()) = addr.port();
    app.recover()?;
    app::spawn_batcher(app.clone(), events);
    tokio::spawn(orchestrator::run_scheduler(app.clone()));
    let router = api::router(app.clone());
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            tracing::error!(%error, "http server stopped");
        }
    });
    Ok((app, addr))
}

/// Bind to localhost only: the daemon never opens a port to the network.
pub async fn serve(cfg: Config) -> AppResult<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", cfg.port)).await?;
    let (_app, addr) = start(cfg, listener).await?;
    tracing::info!(%addr, "regented listening");
    tokio::signal::ctrl_c().await.map_err(AppError::Io)?;
    tracing::info!("shutting down");
    Ok(())
}

use std::net::{Ipv4Addr, SocketAddr};

use anyhow::Context;
use kotonoha_server::config::{self, Config};
use kotonoha_server::llm::LlamaServer;
use kotonoha_server::{app, auth};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("kotonoha-server {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .with_max_level(config::log_level(
            std::env::var("LOG_LEVEL").ok().as_deref(),
        ))
        .init();

    let config = Config::from_env(|k| std::env::var(k).ok())?;
    let content = std::fs::read_to_string(&config.api_keys_file)
        .with_context(|| format!("read API keys from {}", config.api_keys_file.display()))?;
    let keys = auth::parse_keys(&content);
    anyhow::ensure!(
        !keys.is_empty(),
        "no API key in {}",
        config.api_keys_file.display()
    );

    let llm = LlamaServer::new(&config.llama_server_url)?;
    let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, config.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, upstream = %config.llama_server_url, keys = keys.len(), "listening");
    axum::serve(listener, app::router(llm, keys))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

/// systemd の SIGTERM と端末の Ctrl-C で止まる。
async fn shutdown() {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install SIGTERM handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

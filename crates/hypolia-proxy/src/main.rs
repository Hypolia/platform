mod lobbies;
mod network;
mod orchestrator;

use std::time::Duration;

use rspigot_proxy_plugin::{Proxy, run_with};
use tokio::sync::mpsc;

use crate::network::Network;
use crate::orchestrator::Orchestrator;

const SYNC_EVERY: Duration = Duration::from_secs(10);
const PERMISSIONS_EVERY: u32 = 3;

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let orchestrator = Orchestrator::new(
        &env("ORCHESTRATOR_URL").unwrap_or_else(|| "http://127.0.0.1:8080".to_owned()),
        env("ORCHESTRATOR_TOKEN"),
        env("HYPOLIA_VERSION").unwrap_or_else(|| "dev".to_owned()),
    );
    let network = Network::new(
        orchestrator.clone(),
        env("LOBBY_SERVER").unwrap_or_else(|| "lobby".to_owned()),
    );
    let proxy = Proxy::default();
    tokio::spawn(sync(network.clone(), orchestrator.clone(), proxy.clone()));
    tokio::spawn(follow_events(network.clone(), orchestrator, proxy.clone()));
    let url =
        env("RSPIGOT_PROXY_URL").unwrap_or_else(|| rspigot_proxy_plugin::DEFAULT_URL.to_owned());
    if let Err(error) = run_with(network, url, env("RSPIGOT_PROXY_TOKEN"), proxy).await {
        tracing::error!("{error}");
        std::process::exit(1);
    }
}

async fn sync(network: Network, orchestrator: Orchestrator, proxy: Proxy) {
    let mut tick: u32 = 0;
    loop {
        if proxy.is_connected() {
            match orchestrator.state().await {
                Ok(state) => {
                    network.apply_servers(&proxy, &state.lobby_server_name, &state.servers);
                    for transfer in state.pending_transfers {
                        network.transfer(&proxy, transfer);
                    }
                }
                Err(error) => tracing::debug!("Orchestrator state skipped: {error}"),
            }
            if tick.is_multiple_of(PERMISSIONS_EVERY) {
                for player in proxy.players().await.unwrap_or_default() {
                    network.refresh_permissions(&proxy, &player.id).await;
                }
            }
            tick = tick.wrapping_add(1);
        }
        tokio::time::sleep(SYNC_EVERY).await;
    }
}

async fn follow_events(network: Network, orchestrator: Orchestrator, proxy: Proxy) {
    let (sender, mut events) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            if let Err(error) = orchestrator.stream_events(&sender).await {
                tracing::debug!("Orchestrator events interrupted: {error}");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
    while let Some(event) = events.recv().await {
        network.apply_event(&proxy, event);
    }
}

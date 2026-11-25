use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use hypolia_protocol::{PlayerRank, ProxyEvent, ProxyServerEntry, ServerKind, TransferCommand};
use rspigot_proxy_plugin::wire::{ArgumentKind, CommandSpec};
use rspigot_proxy_plugin::{
    ConnectOutcome, Hook, Invocation, Player, Plugin, Proxy, Verdict, command, rich,
};
use uuid::Uuid;

use crate::lobbies;
use crate::orchestrator::Orchestrator;
use crate::social::{self, Social};

pub const SERVER_PERMISSION: &str = "hypolia.command.server";
const HANDLED_TRANSFERS: usize = 4096;

#[derive(Default)]
pub struct Fleet {
    lobby_service: String,
    servers: BTreeMap<String, String>,
    lobbies: BTreeSet<String>,
    handled: HashSet<Uuid>,
}

impl Fleet {
    fn is_lobby(&self, name: &str) -> bool {
        name == self.lobby_service || self.lobbies.contains(name)
    }
}

#[derive(Clone)]
pub struct Network {
    orchestrator: Orchestrator,
    fleet: Arc<Mutex<Fleet>>,
    social: Social,
}

impl Network {
    pub fn new(orchestrator: Orchestrator, lobby_service: String) -> Self {
        Self {
            social: Social::new(orchestrator.clone()),
            orchestrator,
            fleet: Arc::new(Mutex::new(Fleet {
                lobby_service,
                ..Fleet::default()
            })),
        }
    }

    fn fleet(&self) -> MutexGuard<'_, Fleet> {
        self.fleet.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn apply_servers(&self, proxy: &Proxy, lobby_service: &str, entries: &[ProxyServerEntry]) {
        let mut fleet = self.fleet();
        fleet.lobby_service = lobby_service.to_owned();
        let wanted: BTreeSet<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        let stale: Vec<String> = fleet
            .servers
            .keys()
            .filter(|name| !wanted.contains(name.as_str()))
            .cloned()
            .collect();
        for name in stale {
            fleet.servers.remove(&name);
            fleet.lobbies.remove(&name);
            proxy.unregister_server(&name);
        }
        for entry in entries {
            upsert(&mut fleet, proxy, &entry.name, &entry.address, entry.kind);
        }
    }

    pub fn apply_event(&self, proxy: &Proxy, event: ProxyEvent) {
        match event {
            ProxyEvent::ServerUpsert {
                name,
                address,
                kind,
                ..
            } => upsert(&mut self.fleet(), proxy, &name, &address, kind),
            ProxyEvent::ServerRemove { name } => {
                let mut fleet = self.fleet();
                fleet.lobbies.remove(&name);
                if fleet.servers.remove(&name).is_some() {
                    proxy.unregister_server(&name);
                }
            }
            ProxyEvent::TransferPlayer {
                transfer_id,
                player_uuid,
                target_server,
                reason,
            } => self.transfer(
                proxy,
                TransferCommand {
                    transfer_id,
                    player_uuid,
                    target_server,
                    reason,
                },
            ),
        }
    }

    pub fn transfer(&self, proxy: &Proxy, transfer: TransferCommand) {
        if !proxy.is_connected() {
            return;
        }
        {
            let mut fleet = self.fleet();
            if fleet.handled.len() >= HANDLED_TRANSFERS {
                fleet.handled.clear();
            }
            if !fleet.handled.insert(transfer.transfer_id) {
                return;
            }
        }
        let (proxy, orchestrator) = (proxy.clone(), self.orchestrator.clone());
        tokio::spawn(async move {
            let player = transfer.player_uuid.to_string();
            let target = transfer.target_server;
            proxy.message(&player, &format!("§7Transfert vers §d{target}§7…"));
            let (success, message) = match proxy.connect(&player, &target).await {
                Ok(result) => match result.outcome {
                    ConnectOutcome::Connected | ConnectOutcome::AlreadyThere => (true, None),
                    outcome => (
                        false,
                        Some(
                            result
                                .message
                                .unwrap_or_else(|| format!("{outcome:?} for {target}")),
                        ),
                    ),
                },
                Err(error) => (false, Some(error.to_string())),
            };
            orchestrator
                .ack(transfer.transfer_id, success, message)
                .await;
        });
    }

    pub async fn refresh_permissions(&self, proxy: &Proxy, player: &str) {
        let Ok(id) = Uuid::parse_str(player) else {
            return;
        };
        match self.orchestrator.profile(id).await {
            Ok(profile) if is_staff(profile.rank) => {
                proxy.set_permissions(player, &[SERVER_PERMISSION], &[]);
            }
            Ok(_) => {
                proxy.set_permissions(player, &[], &[]);
            }
            Err(error) => tracing::debug!("No profile for {player}: {error}"),
        }
    }

    async fn emptiest_lobby(&self, proxy: &Proxy, excluded: Option<&str>) -> Option<String> {
        let servers = proxy.servers().await.ok()?;
        let lobbies = self.fleet().lobbies.clone();
        lobbies::least_loaded(&servers, &lobbies, excluded).map(|server| server.name.clone())
    }

    async fn hub(&self, proxy: &Proxy, player: &Player) {
        let on_lobby = player
            .server
            .as_deref()
            .is_some_and(|server| self.fleet().is_lobby(server));
        if on_lobby {
            proxy.message(&player.id, "§eTu es déjà au hub.");
            return;
        }
        let target = match self.emptiest_lobby(proxy, None).await {
            Some(lobby) => lobby,
            None => self.fleet().lobby_service.clone(),
        };
        report(proxy, player, &target).await;
    }

    async fn server(&self, proxy: &Proxy, player: &Player, target: Option<&str>) {
        let Some(target) = target else {
            let servers = proxy.servers().await.unwrap_or_default();
            let mut message = rich().text(&format!(
                "§7Tu es sur §b{}§7. Serveurs : ",
                player.server.as_deref().unwrap_or("?")
            ));
            for server in servers {
                message = message.command(
                    &format!("§a{} §8({}) ", server.name, server.players),
                    &format!("/server {}", server.name),
                    Some(&format!("Aller sur {}", server.name)),
                );
            }
            proxy.message_rich(&player.id, message.build());
            return;
        };
        report(proxy, player, target).await;
    }
}

fn upsert(fleet: &mut Fleet, proxy: &Proxy, name: &str, address: &str, kind: ServerKind) {
    let lobby = kind == ServerKind::Lobby;
    let changed = fleet.servers.get(name).map(String::as_str) != Some(address);
    let was_lobby = fleet.lobbies.contains(name);
    if lobby {
        fleet.lobbies.insert(name.to_owned());
    } else {
        fleet.lobbies.remove(name);
    }
    if changed || was_lobby != lobby {
        fleet.servers.insert(name.to_owned(), address.to_owned());
        proxy.register_server(name, address, lobby);
    }
}

fn is_staff(rank: PlayerRank) -> bool {
    matches!(rank, PlayerRank::Admin | PlayerRank::Mod | PlayerRank::Dev)
}

async fn report(proxy: &Proxy, player: &Player, target: &str) {
    let text = match proxy.connect(&player.id, target).await {
        Ok(result) => match result.outcome {
            ConnectOutcome::Connected => return,
            ConnectOutcome::AlreadyThere => format!("§eTu es déjà sur {target}."),
            ConnectOutcome::UnknownServer => format!("§c{target} n'existe pas."),
            ConnectOutcome::Unreachable => format!("§c{target} ne répond pas pour le moment."),
            ConnectOutcome::Denied => result
                .message
                .unwrap_or_else(|| format!("§cTu ne peux pas rejoindre {target}.")),
            _ => format!("§cImpossible de rejoindre {target}."),
        },
        Err(error) => format!("§cImpossible de rejoindre {target} : {error}"),
    };
    proxy.message(&player.id, &text);
}

impl Plugin for Network {
    const NAME: &'static str = "hypolia";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn subscriptions(&self) -> Vec<(Hook, bool)> {
        vec![
            (Hook::Login, false),
            (Hook::Disconnect, false),
            (Hook::InitialServer, true),
            (Hook::Kicked, true),
        ]
    }

    fn commands(&self) -> Vec<CommandSpec> {
        let mut commands = vec![
            command("hub").alias("lobby").build(),
            command("server")
                .permission(SERVER_PERMISSION)
                .optional("server", ArgumentKind::Server)
                .build(),
        ];
        commands.extend(social::commands());
        commands
    }

    async fn login(&self, proxy: &Proxy, player: &Player) {
        self.refresh_permissions(proxy, &player.id).await;
        if let Some(me) = social::player_ref(player) {
            self.social.greet(proxy, &me).await;
        }
    }

    async fn disconnect(&self, proxy: &Proxy, player: &Player) {
        if let Some(me) = social::player_ref(player) {
            self.social.leave_on_disconnect(proxy, &me).await;
            self.social.farewell(proxy, &me).await;
            self.orchestrator.disconnected(me.uuid).await;
        }
    }

    async fn initial_server(
        &self,
        proxy: &Proxy,
        _player: &Player,
        _candidates: &[String],
    ) -> Verdict {
        match self.emptiest_lobby(proxy, None).await {
            Some(server) => Verdict::Redirect {
                server,
                message: None,
            },
            None => Verdict::Allow,
        }
    }

    async fn kicked(
        &self,
        proxy: &Proxy,
        _player: &Player,
        server: &str,
        reason: &str,
        _fallbacks: &[String],
    ) -> Verdict {
        if self.fleet().is_lobby(server) {
            return Verdict::Allow;
        }
        match self.emptiest_lobby(proxy, Some(server)).await {
            Some(lobby) => Verdict::Redirect {
                server: lobby,
                message: Some(format!("§eRetour au hub : §r{reason}")),
            },
            None => Verdict::Allow,
        }
    }

    async fn command(&self, proxy: &Proxy, invocation: &Invocation) {
        let Some(player) = &invocation.player else {
            return;
        };
        let first = invocation.arguments.first().map(String::as_str);
        match invocation.command.as_str() {
            "hub" => self.hub(proxy, player).await,
            "server" => self.server(proxy, player, first).await,
            name => {
                self.social
                    .command(proxy, player, name, &invocation.arguments)
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staff_ranks_get_the_server_command() {
        assert!(is_staff(PlayerRank::Admin));
        assert!(is_staff(PlayerRank::Mod));
        assert!(is_staff(PlayerRank::Dev));
        assert!(!is_staff(PlayerRank::Vip));
        assert!(!is_staff(PlayerRank::Member));
        assert!(!is_staff(PlayerRank::Guest));
    }

    #[test]
    fn the_lobby_service_and_lobby_pods_count_as_lobbies() {
        let mut fleet = Fleet {
            lobby_service: "lobby".to_owned(),
            ..Fleet::default()
        };
        fleet.lobbies.insert("hypolia-lobby-abc".to_owned());
        assert!(fleet.is_lobby("lobby"));
        assert!(fleet.is_lobby("hypolia-lobby-abc"));
        assert!(!fleet.is_lobby("sheepwars-1"));
    }
}

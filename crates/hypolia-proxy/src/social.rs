use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use hypolia_protocol::social::{
    FriendOutcome, FriendRequest, PARTY_MAX_SIZE, Party, PartyOutcome, PartyRequest, PlayerRef,
};
use rspigot_proxy_plugin::wire::{ArgumentKind, CommandSpec};
use rspigot_proxy_plugin::{Player, Proxy, command, rich};
use uuid::Uuid;

use crate::orchestrator::Orchestrator;

const FRIEND_ACTIONS: [&str; 5] = ["add", "accept", "deny", "remove", "list"];
const GROUP_ACTIONS: [&str; 5] = ["invite", "accept", "leave", "kick", "list"];

#[derive(Clone)]
pub struct Social {
    orchestrator: Orchestrator,
    last_whisper: Arc<Mutex<HashMap<Uuid, String>>>,
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        command("friend")
            .alias("f")
            .choice("action", &FRIEND_ACTIONS, true)
            .optional("player", ArgumentKind::Player)
            .build(),
        command("group")
            .alias("g")
            .alias("party")
            .choice("action", &GROUP_ACTIONS, true)
            .optional("player", ArgumentKind::Player)
            .build(),
        command("gc")
            .argument("message", ArgumentKind::GreedyText)
            .build(),
        command("msg")
            .alias("tell")
            .alias("w")
            .argument("player", ArgumentKind::Player)
            .argument("message", ArgumentKind::GreedyText)
            .build(),
        command("r")
            .alias("reply")
            .argument("message", ArgumentKind::GreedyText)
            .build(),
    ]
}

fn reference(player: &Player) -> Option<PlayerRef> {
    Some(PlayerRef {
        uuid: Uuid::parse_str(&player.id).ok()?,
        name: player.name.clone(),
    })
}

fn names(players: &[PlayerRef]) -> String {
    players
        .iter()
        .map(|player| player.name.as_str())
        .collect::<Vec<_>>()
        .join("§7, §f")
}

impl Social {
    pub fn new(orchestrator: Orchestrator) -> Self {
        Self {
            orchestrator,
            last_whisper: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn whispers(&self) -> MutexGuard<'_, HashMap<Uuid, String>> {
        self.last_whisper
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    async fn online(&self, proxy: &Proxy) -> HashMap<Uuid, Player> {
        proxy
            .players()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|player| Some((Uuid::parse_str(&player.id).ok()?, player)))
            .collect()
    }

    pub async fn command(
        &self,
        proxy: &Proxy,
        player: &Player,
        name: &str,
        args: &[String],
    ) -> bool {
        let Some(me) = reference(player) else {
            return false;
        };
        let first = args.first().map(String::as_str);
        let second = args.get(1).map(String::as_str);
        match name {
            "friend" => {
                self.friend(proxy, &me, first.unwrap_or("list"), second)
                    .await
            }
            "group" => {
                self.group(proxy, &me, first.unwrap_or("list"), second)
                    .await
            }
            "gc" => self.group_chat(proxy, &me, &args.join(" ")).await,
            "msg" => {
                let text = args.get(1..).map(|rest| rest.join(" ")).unwrap_or_default();
                self.whisper(proxy, &me, first.unwrap_or_default(), &text)
                    .await;
            }
            "r" => {
                let last = self.whispers().get(&me.uuid).cloned();
                match last {
                    Some(target) => self.whisper(proxy, &me, &target, &args.join(" ")).await,
                    None => {
                        proxy.message(&player.id, "§cPersonne à qui répondre.");
                    }
                }
            }
            _ => return false,
        }
        true
    }

    async fn whisper(&self, proxy: &Proxy, me: &PlayerRef, target: &str, text: &str) {
        let me_id = me.uuid.to_string();
        if text.trim().is_empty() {
            proxy.message(&me_id, "§cUsage : /msg <joueur> <message>");
            return;
        }
        let Ok(found) = proxy.player(target).await else {
            proxy.message(&me_id, &format!("§c{target} n'est pas connecté."));
            return;
        };
        if found.id == me_id {
            proxy.message(&me_id, "§cTu ne peux pas t'écrire à toi-même.");
            return;
        }
        proxy.message(&found.id, &format!("§d{} §8» §7moi §8: §f{text}", me.name));
        proxy.message(&me_id, &format!("§7moi §8» §d{} §8: §f{text}", found.name));
        if let Ok(found_id) = Uuid::parse_str(&found.id) {
            let mut whispers = self.whispers();
            whispers.insert(found_id, me.name.clone());
            whispers.insert(me.uuid, found.name.clone());
        }
    }

    async fn friend(&self, proxy: &Proxy, me: &PlayerRef, action: &str, target: Option<&str>) {
        let me_id = me.uuid.to_string();
        if action == "list" {
            self.list_friends(proxy, me).await;
            return;
        }
        let Some(target) = target else {
            proxy.message(&me_id, &format!("§cUsage : /friend {action} <joueur>"));
            return;
        };
        let request = FriendRequest {
            player: me.clone(),
            target_name: target.to_owned(),
        };
        let outcome = match self.orchestrator.friend(action, &request).await {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!("friend {action} for {}: {error}", me.name);
                proxy.message(&me_id, "§cLes amis sont indisponibles, réessaie plus tard.");
                return;
            }
        };
        let online = self.online(proxy).await;
        match outcome {
            FriendOutcome::RequestSent { target } => {
                proxy.message(
                    &me_id,
                    &format!("§aDemande d'ami envoyée à §f{}§a.", target.name),
                );
                if online.contains_key(&target.uuid) {
                    proxy.message_rich(
                        &target.uuid.to_string(),
                        rich()
                            .text(&format!("§e{} veut être ton ami. ", me.name))
                            .command(
                                "§a[Accepter]",
                                &format!("/friend accept {}", me.name),
                                Some("Accepter la demande"),
                            )
                            .text(" ")
                            .command(
                                "§c[Refuser]",
                                &format!("/friend deny {}", me.name),
                                Some("Refuser la demande"),
                            )
                            .build(),
                    );
                }
            }
            FriendOutcome::Accepted { friend } => {
                proxy.message(
                    &me_id,
                    &format!("§aTu es maintenant ami avec §f{}§a.", friend.name),
                );
                if online.contains_key(&friend.uuid) {
                    proxy.message(
                        &friend.uuid.to_string(),
                        &format!("§aTu es maintenant ami avec §f{}§a.", me.name),
                    );
                }
            }
            FriendOutcome::Denied { target } => {
                proxy.message(
                    &me_id,
                    &format!("§7Demande de §f{} §7refusée.", target.name),
                );
            }
            FriendOutcome::Removed { target } => {
                proxy.message(
                    &me_id,
                    &format!("§7{} §7n'est plus dans tes amis.", target.name),
                );
            }
            FriendOutcome::AlreadyFriends { target } => {
                proxy.message(
                    &me_id,
                    &format!("§eTu es déjà ami avec §f{}§e.", target.name),
                );
            }
            FriendOutcome::AlreadyRequested { target } => {
                proxy.message(
                    &me_id,
                    &format!("§eTu as déjà envoyé une demande à §f{}§e.", target.name),
                );
            }
            FriendOutcome::NoRequest => {
                proxy.message(&me_id, &format!("§c{target} ne t'a pas envoyé de demande."));
            }
            FriendOutcome::NotFriends => {
                proxy.message(&me_id, &format!("§c{target} n'est pas dans tes amis."));
            }
            FriendOutcome::UnknownPlayer => {
                proxy.message(
                    &me_id,
                    &format!("§c{target} ne s'est jamais connecté sur Hypolia."),
                );
            }
            FriendOutcome::Yourself => {
                proxy.message(&me_id, "§cTu ne peux pas être ton propre ami.");
            }
            FriendOutcome::TooManyFriends => {
                proxy.message(&me_id, "§cTu as atteint la limite d'amis.");
            }
        }
    }

    async fn list_friends(&self, proxy: &Proxy, me: &PlayerRef) {
        let me_id = me.uuid.to_string();
        let list = match self.orchestrator.friends(me.uuid).await {
            Ok(list) => list,
            Err(error) => {
                tracing::warn!("friends of {}: {error}", me.name);
                proxy.message(&me_id, "§cLes amis sont indisponibles, réessaie plus tard.");
                return;
            }
        };
        let online = self.online(proxy).await;
        if list.friends.is_empty() && list.incoming.is_empty() {
            proxy.message(
                &me_id,
                "§7Tu n'as pas encore d'amis. §f/friend add <joueur>",
            );
            return;
        }
        let (connected, offline): (Vec<_>, Vec<_>) = list
            .friends
            .iter()
            .partition(|friend| online.contains_key(&friend.uuid));
        proxy.message(
            &me_id,
            &format!(
                "§6Amis §8(§a{}§8/§7{}§8)",
                connected.len(),
                list.friends.len()
            ),
        );
        for friend in connected {
            let server = online
                .get(&friend.uuid)
                .and_then(|player| player.server.clone())
                .unwrap_or_else(|| "?".to_owned());
            proxy.message_rich(
                &me_id,
                rich()
                    .text(&format!("§a● §f{} §8- §7{server} ", friend.name))
                    .suggest(
                        "§8[msg]",
                        &format!("/msg {} ", friend.name),
                        Some("Envoyer un message"),
                    )
                    .text(" ")
                    .command(
                        "§8[groupe]",
                        &format!("/group invite {}", friend.name),
                        Some("Inviter dans ton groupe"),
                    )
                    .build(),
            );
        }
        for friend in offline {
            proxy.message(&me_id, &format!("§8● §7{}", friend.name));
        }
        for request in &list.incoming {
            proxy.message_rich(
                &me_id,
                rich()
                    .text(&format!("§eDemande de §f{} ", request.name))
                    .command(
                        "§a[Accepter]",
                        &format!("/friend accept {}", request.name),
                        None,
                    )
                    .text(" ")
                    .command(
                        "§c[Refuser]",
                        &format!("/friend deny {}", request.name),
                        None,
                    )
                    .build(),
            );
        }
    }

    async fn group(&self, proxy: &Proxy, me: &PlayerRef, action: &str, target: Option<&str>) {
        let me_id = me.uuid.to_string();
        if action == "list" {
            self.list_group(proxy, me).await;
            return;
        }
        let target_ref = match (action, target) {
            ("leave", _) => None,
            (_, Some(target)) => match proxy.player(target).await {
                Ok(found) => reference(&found),
                Err(_) => {
                    proxy.message(&me_id, &format!("§c{target} n'est pas connecté."));
                    return;
                }
            },
            (_, None) => {
                proxy.message(&me_id, &format!("§cUsage : /group {action} <joueur>"));
                return;
            }
        };
        let request = PartyRequest {
            player: me.clone(),
            target: target_ref,
        };
        match self.orchestrator.party(action, &request).await {
            Ok(outcome) => self.announce(proxy, me, outcome),
            Err(error) => {
                tracing::warn!("group {action} for {}: {error}", me.name);
                proxy.message(
                    &me_id,
                    "§cLes groupes sont indisponibles, réessaie plus tard.",
                );
            }
        }
    }

    pub async fn leave_on_disconnect(&self, proxy: &Proxy, me: &PlayerRef) {
        let request = PartyRequest {
            player: me.clone(),
            target: None,
        };
        if let Ok(outcome) = self.orchestrator.party("leave", &request).await
            && outcome != PartyOutcome::NotInParty
        {
            self.announce(proxy, me, outcome);
        }
    }

    fn tell_party(proxy: &Proxy, party: &Party, except: Option<Uuid>, text: &str) {
        for member in &party.members {
            if Some(member.uuid) != except {
                proxy.message(&member.uuid.to_string(), text);
            }
        }
    }

    fn announce(&self, proxy: &Proxy, me: &PlayerRef, outcome: PartyOutcome) {
        let me_id = me.uuid.to_string();
        match outcome {
            PartyOutcome::Invited { party, target } => {
                Self::tell_party(
                    proxy,
                    &party,
                    None,
                    &format!(
                        "§d{} §7a invité §f{} §7dans le groupe.",
                        me.name, target.name
                    ),
                );
                proxy.message_rich(
                    &target.uuid.to_string(),
                    rich()
                        .text(&format!("§d{} t'invite dans son groupe. ", me.name))
                        .command(
                            "§a[Rejoindre]",
                            &format!("/group accept {}", me.name),
                            Some("Rejoindre le groupe"),
                        )
                        .text(" §8(60 s)")
                        .build(),
                );
            }
            PartyOutcome::Joined { party } => {
                Self::tell_party(
                    proxy,
                    &party,
                    None,
                    &format!(
                        "§d{} §7a rejoint le groupe §8({}/{PARTY_MAX_SIZE})",
                        me.name,
                        party.members.len()
                    ),
                );
            }
            PartyOutcome::Left { party, player } => {
                proxy.message(&player.uuid.to_string(), "§7Tu as quitté le groupe.");
                if let Some(party) = party {
                    Self::tell_party(
                        proxy,
                        &party,
                        Some(player.uuid),
                        &format!(
                            "§d{} §7a quitté le groupe. Chef : §f{}",
                            player.name, party.leader.name
                        ),
                    );
                }
            }
            PartyOutcome::Kicked { party, target } => {
                proxy.message(
                    &target.uuid.to_string(),
                    &format!("§c{} t'a exclu du groupe.", me.name),
                );
                Self::tell_party(
                    proxy,
                    &party,
                    None,
                    &format!("§d{} §7a été exclu du groupe.", target.name),
                );
            }
            PartyOutcome::Disbanded { members } => {
                for member in members {
                    proxy.message(&member.uuid.to_string(), "§7Le groupe est dissous.");
                }
            }
            PartyOutcome::NotInParty => {
                proxy.message(&me_id, "§cTu n'es pas dans un groupe.");
            }
            PartyOutcome::NotLeader => {
                proxy.message(&me_id, "§cSeul le chef du groupe peut faire ça.");
            }
            PartyOutcome::AlreadyInParty => {
                proxy.message(&me_id, "§cTu es déjà dans un groupe. §f/group leave");
            }
            PartyOutcome::TargetInParty => {
                proxy.message(&me_id, "§cCe joueur est déjà dans un groupe.");
            }
            PartyOutcome::PartyFull => {
                proxy.message(
                    &me_id,
                    &format!("§cLe groupe est plein ({PARTY_MAX_SIZE} joueurs)."),
                );
            }
            PartyOutcome::NoInvite => {
                proxy.message(
                    &me_id,
                    "§cTu n'as pas d'invitation de ce joueur, ou elle a expiré.",
                );
            }
            PartyOutcome::Yourself => {
                proxy.message(&me_id, "§cTu ne peux pas faire ça avec toi-même.");
            }
        }
    }

    async fn list_group(&self, proxy: &Proxy, me: &PlayerRef) {
        let me_id = me.uuid.to_string();
        match self.orchestrator.party_of(me.uuid).await {
            Ok(Some(party)) => {
                proxy.message(
                    &me_id,
                    &format!(
                        "§6Groupe §8({}/{PARTY_MAX_SIZE}) §7chef : §f{}",
                        party.members.len(),
                        party.leader.name
                    ),
                );
                proxy.message(&me_id, &format!("§7Membres : §f{}", names(&party.members)));
                if !party.invited.is_empty() {
                    proxy.message(&me_id, &format!("§7Invités : §f{}", names(&party.invited)));
                }
            }
            Ok(None) => {
                proxy.message(
                    &me_id,
                    "§7Tu n'es pas dans un groupe. §f/group invite <joueur>",
                );
            }
            Err(error) => {
                tracing::warn!("party of {}: {error}", me.name);
                proxy.message(
                    &me_id,
                    "§cLes groupes sont indisponibles, réessaie plus tard.",
                );
            }
        }
    }

    async fn group_chat(&self, proxy: &Proxy, me: &PlayerRef, text: &str) {
        let me_id = me.uuid.to_string();
        match self.orchestrator.party_of(me.uuid).await {
            Ok(Some(party)) => {
                Self::tell_party(
                    proxy,
                    &party,
                    None,
                    &format!("§d[Groupe] §f{} §8» §f{text}", me.name),
                );
            }
            Ok(None) => {
                proxy.message(&me_id, "§cTu n'es pas dans un groupe.");
            }
            Err(_) => {
                proxy.message(
                    &me_id,
                    "§cLes groupes sont indisponibles, réessaie plus tard.",
                );
            }
        }
    }

    pub async fn greet(&self, proxy: &Proxy, me: &PlayerRef) {
        let Ok(list) = self.orchestrator.friends(me.uuid).await else {
            return;
        };
        let online = self.online(proxy).await;
        let mut connected = 0;
        for friend in &list.friends {
            if online.contains_key(&friend.uuid) {
                connected += 1;
                proxy.message(
                    &friend.uuid.to_string(),
                    &format!("§a{} §7s'est connecté.", me.name),
                );
            }
        }
        let me_id = me.uuid.to_string();
        if !list.friends.is_empty() {
            proxy.message(
                &me_id,
                &format!(
                    "§7Amis en ligne : §a{connected}§7/{}. §f/friend",
                    list.friends.len()
                ),
            );
        }
        if !list.incoming.is_empty() {
            proxy.message(
                &me_id,
                &format!(
                    "§eTu as {} demande(s) d'ami. §f/friend",
                    list.incoming.len()
                ),
            );
        }
    }

    pub async fn farewell(&self, proxy: &Proxy, me: &PlayerRef) {
        let Ok(list) = self.orchestrator.friends(me.uuid).await else {
            return;
        };
        let online = self.online(proxy).await;
        for friend in &list.friends {
            if online.contains_key(&friend.uuid) {
                proxy.message(
                    &friend.uuid.to_string(),
                    &format!("§7{} s'est déconnecté.", me.name),
                );
            }
        }
        self.whispers().remove(&me.uuid);
    }
}

pub fn player_ref(player: &Player) -> Option<PlayerRef> {
    reference(player)
}

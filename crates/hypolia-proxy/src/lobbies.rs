use std::collections::BTreeSet;

use rspigot_proxy_plugin::Server;

pub fn least_loaded<'a>(
    servers: &'a [Server],
    lobbies: &BTreeSet<String>,
    excluded: Option<&str>,
) -> Option<&'a Server> {
    servers
        .iter()
        .filter(|server| lobbies.contains(&server.name))
        .filter(|server| Some(server.name.as_str()) != excluded)
        .min_by(|a, b| a.players.cmp(&b.players).then_with(|| a.name.cmp(&b.name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, players: u32) -> Server {
        Server {
            name: name.to_owned(),
            address: "10.0.0.1:25565".to_owned(),
            players,
        }
    }

    fn lobbies(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_emptiest_lobby_wins_and_names_break_ties() {
        let servers = [
            server("lobby-b", 3),
            server("lobby-a", 3),
            server("lobby-c", 5),
            server("sheepwars-1", 0),
        ];
        let set = lobbies(&["lobby-a", "lobby-b", "lobby-c"]);
        assert_eq!(least_loaded(&servers, &set, None).unwrap().name, "lobby-a");
        assert_eq!(
            least_loaded(&servers, &set, Some("lobby-a")).unwrap().name,
            "lobby-b"
        );
    }

    #[test]
    fn game_servers_are_never_picked() {
        let servers = [server("sheepwars-1", 0)];
        assert!(least_loaded(&servers, &lobbies(&["lobby-a"]), None).is_none());
    }
}

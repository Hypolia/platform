use std::time::Duration;

use hypolia_orchestrator::api::{AppState, build_router};
use hypolia_orchestrator::engine::{FleetEngine, FleetPolicy};
use hypolia_protocol::social::{
    FriendOutcome, FriendsList, Party, PartyOutcome, PartyRequest, PlayerRef,
};
use hypolia_protocol::{
    FleetStatusSnapshot, GameKind, JoinQueueRequest, JoinQueueResponse, QueueJoinStatus,
};
use reqwest::Client;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio::net::TcpListener;
use uuid::Uuid;

fn policy() -> FleetPolicy {
    FleetPolicy {
        game: GameKind::SheepWars,
        lobby_server_name: "lobby".to_owned(),
        queue_min_players: 8,
        max_players_per_server: 8,
        min_replicas: 1,
        max_replicas: 4,
        standby_servers: 1,
        idle_ttl: Duration::from_secs(60),
        starting_timeout: Duration::from_secs(60),
        transfer_timeout: Duration::from_secs(15),
        empty_grace: Duration::from_secs(30),
        ending_timeout: Duration::from_secs(60),
        heartbeat_timeout: Duration::from_secs(30),
        towerwars_gather_delay: Duration::ZERO,
    }
}

async fn serve() -> String {
    let app = build_router(AppState::new(FleetEngine::new(policy()), None));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

fn player(id: u128, name: &str) -> PlayerRef {
    PlayerRef {
        uuid: Uuid::from_u128(id),
        name: name.to_owned(),
    }
}

async fn post<B: Serialize, T: DeserializeOwned>(base: &str, path: &str, body: &B) -> T {
    let response = Client::new()
        .post(format!("{base}{path}"))
        .json(body)
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "{path}: {}",
        response.status()
    );
    response.json().await.unwrap()
}

async fn get<T: DeserializeOwned>(base: &str, path: &str) -> T {
    Client::new()
        .get(format!("{base}{path}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

fn party_request(me: PlayerRef, target: Option<PlayerRef>) -> PartyRequest {
    PartyRequest { player: me, target }
}

fn join(who: &PlayerRef) -> JoinQueueRequest {
    JoinQueueRequest {
        player_uuid: who.uuid,
        player_name: who.name.clone(),
        game: GameKind::SheepWars,
        match_size: 0,
    }
}

#[tokio::test]
async fn the_party_leader_queues_the_whole_party_and_members_cannot() {
    let base = serve().await;
    let steve = player(1, "Steve");
    let alex = player(2, "Alex");
    let invited: PartyOutcome = post(
        &base,
        "/api/v1/parties/invite",
        &party_request(steve.clone(), Some(alex.clone())),
    )
    .await;
    assert!(matches!(invited, PartyOutcome::Invited { .. }));
    let joined: PartyOutcome = post(
        &base,
        "/api/v1/parties/accept",
        &party_request(alex.clone(), Some(steve.clone())),
    )
    .await;
    assert!(matches!(joined, PartyOutcome::Joined { .. }));
    let party: Option<Party> = get(&base, &format!("/api/v1/players/{}/party", alex.uuid)).await;
    assert_eq!(party.unwrap().leader, steve);

    let refused: JoinQueueResponse = post(&base, "/api/v1/queue/join", &join(&alex)).await;
    assert_eq!(refused.status, QueueJoinStatus::NotPartyLeader);
    assert_eq!(refused.target_server.as_deref(), Some("Steve"));
    let status: FleetStatusSnapshot = get(&base, "/api/v1/status").await;
    assert!(status.queued_players.is_empty());

    let queued: JoinQueueResponse = post(&base, "/api/v1/queue/join", &join(&steve)).await;
    assert_eq!(queued.status, QueueJoinStatus::Queued);
    let status: FleetStatusSnapshot = get(&base, "/api/v1/status").await;
    let names: Vec<&str> = status
        .queued_players
        .iter()
        .map(|queued| queued.player_name.as_str())
        .collect();
    assert_eq!(names, ["Steve", "Alex"]);

    let left: PartyOutcome = post(
        &base,
        "/api/v1/parties/leave",
        &party_request(alex.clone(), None),
    )
    .await;
    assert!(matches!(left, PartyOutcome::Disbanded { .. }));
}

#[tokio::test]
async fn friends_go_through_the_api() {
    let base = serve().await;
    let empty: FriendsList = get(
        &base,
        &format!("/api/v1/players/{}/friends", Uuid::from_u128(1)),
    )
    .await;
    assert!(empty.friends.is_empty());
    let unknown: FriendOutcome = post(
        &base,
        "/api/v1/friends/add",
        &json!({ "player": player(1, "Steve"), "target_name": "Nobody" }),
    )
    .await;
    assert_eq!(unknown, FriendOutcome::UnknownPlayer);
    let missing = Client::new()
        .post(format!("{base}/api/v1/friends/hug"))
        .json(&json!({ "player": player(1, "Steve"), "target_name": "Alex" }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
}

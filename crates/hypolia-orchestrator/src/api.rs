use std::convert::Infallible;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::Utc;
use futures_util::stream::{self, Stream};
use hypolia_progression::{
    EquipError, MemoryProgressionRepository, ProgressionStoreError, Unlock, UnlockError,
};
use hypolia_protocol::{
    AdminCoinGrant, DrainResponse, EquipRequest, FleetStatusSnapshot, GameKind, HeartbeatRequest,
    HeartbeatResponse, JoinQueueRequest, JoinQueueResponse, Leaderboard, LeaveQueueRequest,
    LeaveQueueResponse, MatchReport, MatchReportAck, PROXY_VERSION_HEADER, PhaseUpdateRequest,
    PlayerIdentityUpdate, PlayerProfile, PlayerQueueStatus, ProxyDisconnectRequest, ProxyEvent,
    ProxyStateSnapshot, PurchaseRequest, SendToLobbyRequest, SpectateRequest, TransferAckRequest,
};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::engine::{EngineError, FleetEngine};
use crate::games::{GameError, Games};
use crate::lobby::DrainError;
use crate::progression::{ProgressionError, ProgressionService, ProgressionStore};
use crate::queue::{
    NewQueueEntry, QueueRepository, QueueStore, QueueStoreError, normalize_match_size,
    persist as persist_mutation,
};
use crate::social::{FriendStore, FriendStoreError, Friends, PartyBook};
use hypolia_protocol::social::{
    FriendOutcome, FriendRequest, FriendsList, Party, PartyOutcome, PartyRequest,
};

#[derive(Clone)]
pub struct AppState {
    pub games: Arc<Mutex<Games>>,
    pub proxy_tx: broadcast::Sender<ProxyEvent>,
    pub api_token: Option<String>,
    pub version: Option<String>,
    pub progression: ProgressionService<ProgressionStore>,
    pub queue: QueueStore,
    pub friends: Friends,
    pub parties: Arc<Mutex<PartyBook>>,
    persist_lock: Arc<tokio::sync::Mutex<()>>,
    proxy_version: Arc<Mutex<Option<String>>>,
}

impl AppState {
    pub fn new(engine: FleetEngine, api_token: Option<String>) -> Self {
        Self::from_games(Games::new(engine), api_token)
    }

    pub fn from_games(games: Games, api_token: Option<String>) -> Self {
        let (proxy_tx, _) = broadcast::channel(512);
        Self {
            games: Arc::new(Mutex::new(games)),
            proxy_tx,
            api_token,
            version: None,
            progression: ProgressionService::new(ProgressionStore::Memory(
                MemoryProgressionRepository::default(),
            )),
            queue: QueueStore::default(),
            friends: Friends::default(),
            parties: Arc::new(Mutex::new(PartyBook::default())),
            persist_lock: Arc::new(tokio::sync::Mutex::new(())),
            proxy_version: Arc::new(Mutex::new(None)),
        }
    }

    #[must_use]
    pub fn with_game_engine(self, engine: FleetEngine) -> Self {
        self.lock_games().insert_engine(engine);
        self
    }

    pub fn lock_games(&self) -> MutexGuard<'_, Games> {
        self.games.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[must_use]
    pub fn with_queue(mut self, queue: QueueStore) -> Self {
        self.queue = queue;
        self
    }

    pub async fn flush_queue(&self) -> Result<(), QueueStoreError> {
        let _serialized = self.persist_lock.lock().await;
        loop {
            let batch = self.lock_games().take_mutations();
            if batch.is_empty() {
                return Ok(());
            }
            for (done, mutation) in batch.iter().enumerate() {
                if let Err(err) = persist_mutation(&self.queue, mutation).await {
                    self.lock_games().restore_mutations(batch[done..].to_vec());
                    return Err(err);
                }
            }
        }
    }

    pub async fn flush_queue_logged(&self) {
        if let Err(err) = self.flush_queue().await {
            tracing::warn!(error = %err, "Écriture de la file d'attente différée");
        }
    }

    fn record_proxy_version(&self, headers: &HeaderMap) {
        if let Some(version) = headers
            .get(PROXY_VERSION_HEADER)
            .and_then(|v| v.to_str().ok())
        {
            *self.proxy_version.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(version.to_owned());
        }
    }

    #[must_use]
    pub fn with_friends(mut self, friends: Friends) -> Self {
        self.friends = friends;
        self
    }

    pub fn lock_parties(&self) -> MutexGuard<'_, PartyBook> {
        self.parties.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[must_use]
    pub fn with_progression(mut self, service: ProgressionService<ProgressionStore>) -> Self {
        self.progression = service;
        self
    }

    #[must_use]
    pub fn with_version(mut self, version: Option<String>) -> Self {
        self.version = version;
        self
    }

    pub fn emit_proxy_events(&self, events: Vec<ProxyEvent>) {
        for ev in events {
            let _ = self.proxy_tx.send(ev);
        }
    }
}

pub enum ApiError {
    Unauthorized,
    NotFound(String),
    BadRequest(String),
    Conflict(String),
    Progression(ProgressionError),
    Queue(QueueStoreError),
    Drain(DrainError),
    Friends(FriendStoreError),
}

fn unlock_json(unlock: Unlock) -> serde_json::Value {
    match unlock {
        Unlock::Free => json!({ "free": null }),
        Unlock::Price(price) => json!({ "price": price.get() }),
        Unlock::Rank(rank) => json!({ "rank": rank }),
        Unlock::NetworkLevel(level) => json!({ "network_level": level.get() }),
    }
}

fn progression_response(error: ProgressionError) -> Response {
    let (status, body) = match error {
        ProgressionError::Store(err) => {
            tracing::warn!(error = %err, "Stockage de progression indisponible");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                json!({ "error": "progression_unavailable" }),
            )
        }
        ProgressionError::EmptyMatch => (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "error": "empty_match" }),
        ),
        ProgressionError::Unlock(err) => match err {
            UnlockError::AlreadyUnlocked => {
                (StatusCode::CONFLICT, json!({ "error": "already_unlocked" }))
            }
            UnlockError::InsufficientCoins { missing } => (
                StatusCode::PAYMENT_REQUIRED,
                json!({ "error": "insufficient_coins", "missing": missing.get() }),
            ),
            UnlockError::RankRequired(rank) => (
                StatusCode::FORBIDDEN,
                json!({ "error": "rank_required", "rank": rank }),
            ),
            UnlockError::LevelRequired(level) => (
                StatusCode::FORBIDDEN,
                json!({ "error": "level_required", "level": level.get() }),
            ),
            UnlockError::UnknownCosmetic => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "error": "unknown_cosmetic" }),
            ),
        },
        ProgressionError::Equip(err) => match err {
            EquipError::NotUnlocked { requires } => (
                StatusCode::FORBIDDEN,
                json!({ "error": "not_unlocked", "requires": unlock_json(requires) }),
            ),
            EquipError::WrongSlot => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "error": "wrong_slot" }),
            ),
        },
    };
    (status, Json(body)).into_response()
}

impl From<ProgressionError> for ApiError {
    fn from(value: ProgressionError) -> Self {
        Self::Progression(value)
    }
}

impl From<ProgressionStoreError> for ApiError {
    fn from(value: ProgressionStoreError) -> Self {
        Self::Progression(ProgressionError::Store(value))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            Self::Progression(err) => return progression_response(err),
            Self::Queue(err) => {
                tracing::warn!(error = %err, "File d'attente indisponible");
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({ "error": "queue_unavailable" })),
                )
                    .into_response();
            }
            Self::Drain(err) => {
                let status = match err {
                    DrainError::UnknownServer => StatusCode::NOT_FOUND,
                    DrainError::NotALobby | DrainError::NoOtherLobby => StatusCode::CONFLICT,
                };
                return (status, Json(json!({ "error": err.to_string() }))).into_response();
            }
            Self::Friends(err) => {
                tracing::warn!(error = %err, "Amis indisponibles");
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({ "error": "friends_unavailable" })),
                )
                    .into_response();
            }
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "jeton orchestrateur invalide".to_owned(),
            ),
            Self::NotFound(m) => (StatusCode::NOT_FOUND, m),
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            Self::Conflict(m) => (StatusCode::CONFLICT, m),
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}

impl From<QueueStoreError> for ApiError {
    fn from(value: QueueStoreError) -> Self {
        Self::Queue(value)
    }
}

impl From<FriendStoreError> for ApiError {
    fn from(err: FriendStoreError) -> Self {
        Self::Friends(err)
    }
}

impl From<DrainError> for ApiError {
    fn from(value: DrainError) -> Self {
        Self::Drain(value)
    }
}

impl From<GameError> for ApiError {
    fn from(value: GameError) -> Self {
        match value {
            GameError::Inactive(_) => Self::BadRequest(value.to_string()),
            GameError::Engine(err) => err.into(),
        }
    }
}

impl From<EngineError> for ApiError {
    fn from(value: EngineError) -> Self {
        match value {
            EngineError::UnknownServer(id) => Self::NotFound(format!("serveur inconnu : {id}")),
            EngineError::NotJoinable(id) => Self::Conflict(format!("serveur non joignable : {id}")),
        }
    }
}

fn check_auth(headers: &HeaderMap, expected: Option<&str>) -> Result<(), ApiError> {
    let Some(expected_token) = expected else {
        return Ok(());
    };
    let supplied = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| raw.strip_prefix("Bearer "))
        .map(str::trim);
    if supplied == Some(expected_token) {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/api/v1/status", get(get_status))
        .route("/api/v1/queue", get(get_status))
        .route("/api/v1/queue/join", post(join_queue))
        .route("/api/v1/queue/leave", post(leave_queue))
        .route("/api/v1/queue/player/{uuid}", get(player_status))
        .route("/api/v1/servers", get(list_servers))
        .route("/api/v1/servers/heartbeat", post(server_heartbeat))
        .route("/api/v1/servers/{server_id}/phase", post(update_phase))
        .route("/api/v1/servers/{server_id}/drain", post(drain_server))
        .route(
            "/api/v1/servers/{server_id}/spectate",
            post(spectate_server),
        )
        .route(
            "/api/v1/players/{uuid}/send-lobby",
            post(send_player_to_lobby),
        )
        .route("/api/v1/matches", post(report_match))
        .route("/api/v1/players/{uuid}/profile", get(player_profile))
        .route("/api/v1/leaderboards/{game}", get(leaderboard))
        .route("/api/v1/players/{uuid}/identity", post(set_identity))
        .route("/api/v1/players/{uuid}/purchases", post(purchase_cosmetic))
        .route("/api/v1/players/{uuid}/equipped", put(equip_cosmetic))
        .route("/api/v1/players/{uuid}/coins", post(grant_coins))
        .route("/api/v1/proxy/state", get(proxy_state))
        .route("/api/v1/proxy/events", get(proxy_events))
        .route(
            "/api/v1/proxy/transfers/{transfer_id}/ack",
            post(ack_transfer),
        )
        .route(
            "/api/v1/proxy/player-disconnected",
            post(proxy_player_disconnected),
        )
        .route("/api/v1/players/{uuid}/friends", get(list_friends))
        .route("/api/v1/friends/{action}", post(friend_action))
        .route("/api/v1/players/{uuid}/party", get(player_party))
        .route("/api/v1/parties/{action}", post(party_action))
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "ok": true })))
}

async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    let snap = state.lock_games().snapshot(Instant::now());
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "ready_servers": snap.ready_servers,
            "queue_len": snap.queue_len,
        })),
    )
}

async fn get_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<FleetStatusSnapshot>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let mut snap = state.lock_games().snapshot(Instant::now());
    snap.orchestrator_version.clone_from(&state.version);
    snap.proxy_version = state
        .proxy_version
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    Ok(Json(snap))
}

async fn list_servers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<FleetStatusSnapshot>, ApiError> {
    get_status(State(state), headers).await
}

async fn join_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<JoinQueueRequest>,
) -> Result<Json<JoinQueueResponse>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    if req.player_name.trim().is_empty() {
        return Err(ApiError::BadRequest("player_name vide".to_owned()));
    }
    let party = state
        .lock_parties()
        .party_of(Instant::now(), req.player_uuid)
        .filter(|party| party.members.len() > 1);
    if let Some(party) = party {
        return join_queue_as_party(&state, req, party).await;
    }
    let seq = enqueue_seq(&state, &req).await?;
    let (resp, events) = state.lock_games().join(Instant::now(), req, seq)?;
    state.flush_queue().await?;
    state.emit_proxy_events(events);
    Ok(Json(resp))
}

async fn enqueue_seq(state: &AppState, req: &JoinQueueRequest) -> Result<Option<u64>, ApiError> {
    if !state
        .lock_games()
        .needs_enqueue(req.game, req.player_uuid)?
    {
        return Ok(None);
    }
    Ok(Some(
        state
            .queue
            .insert(NewQueueEntry {
                player_uuid: req.player_uuid,
                name: &req.player_name,
                joined_at: Utc::now(),
                seq: None,
                game: req.game,
                match_size: normalize_match_size(req.game, req.match_size),
            })
            .await?,
    ))
}

async fn join_queue_as_party(
    state: &AppState,
    req: JoinQueueRequest,
    party: Party,
) -> Result<Json<JoinQueueResponse>, ApiError> {
    if party.leader.uuid != req.player_uuid {
        return Ok(Json(JoinQueueResponse {
            status: hypolia_protocol::QueueJoinStatus::NotPartyLeader,
            position: 0,
            total_queued: 0,
            min_players: 0,
            target_server: Some(party.leader.name),
            ready_servers: 0,
            active_servers: 0,
            max_servers: 0,
        }));
    }
    let mut members = Vec::with_capacity(party.members.len());
    let ordered = std::iter::once(party.leader.clone()).chain(
        party
            .members
            .iter()
            .filter(|member| member.uuid != party.leader.uuid)
            .cloned(),
    );
    for member in ordered {
        let request = JoinQueueRequest {
            player_uuid: member.uuid,
            player_name: member.name.clone(),
            game: req.game,
            match_size: req.match_size,
        };
        let seq = enqueue_seq(state, &request).await?;
        members.push((request, seq));
    }
    let joined = state
        .lock_games()
        .join_party(Instant::now(), req.game, party.id, members)?;
    let Some((resp, events)) = joined else {
        return Err(ApiError::BadRequest("groupe vide".to_owned()));
    };
    state.flush_queue().await?;
    state.emit_proxy_events(events);
    Ok(Json(resp))
}

async fn leave_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LeaveQueueRequest>,
) -> Result<Json<LeaveQueueResponse>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let resp = state
        .lock_games()
        .leave_queue(req.game, req.player_uuid, req.reason)?;
    state.flush_queue().await?;
    Ok(Json(resp))
}

#[derive(Debug, Default, Deserialize)]
struct GameQuery {
    #[serde(default)]
    game: GameKind,
}

async fn player_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
    Query(query): Query<GameQuery>,
) -> Result<Json<PlayerQueueStatus>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let status = state
        .lock_games()
        .player_status(Instant::now(), uuid, query.game)?;
    Ok(Json(status))
}

async fn server_heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Response, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let server_id = req.server_id.clone();
    let (events, expected_players, teams) = {
        let mut games = state.lock_games();
        let events = games.apply_heartbeat(Instant::now(), req)?;
        (
            events,
            games.expected_players(&server_id),
            games.teams(&server_id),
        )
    };
    state.flush_queue_logged().await;
    state.emit_proxy_events(events);
    Ok(match expected_players {
        Some(expected) => Json(HeartbeatResponse {
            expected_players: Some(expected),
            teams,
        })
        .into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

async fn drain_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(server_id): Path<String>,
) -> Result<Json<DrainResponse>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let result = state.lock_games().drain_lobby(Instant::now(), &server_id)?;
    state.emit_proxy_events(result.events);
    Ok(Json(DrainResponse {
        remaining_players: result.remaining_players,
    }))
}

async fn update_phase(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(server_id): Path<String>,
    Json(req): Json<PhaseUpdateRequest>,
) -> Result<StatusCode, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let events = state
        .lock_games()
        .update_server_phase(Instant::now(), &server_id, req.phase)?;
    state.flush_queue_logged().await;
    state.emit_proxy_events(events);
    Ok(StatusCode::NO_CONTENT)
}

async fn spectate_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(server_id): Path<String>,
    Json(req): Json<SpectateRequest>,
) -> Result<StatusCode, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let event =
        state
            .lock_games()
            .queue_spectate_transfer(Instant::now(), &server_id, req.player_uuid)?;
    state.flush_queue_logged().await;
    state.emit_proxy_events(vec![event]);
    Ok(StatusCode::ACCEPTED)
}

async fn send_player_to_lobby(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
    Json(req): Json<SendToLobbyRequest>,
) -> Result<StatusCode, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let event = state
        .lock_games()
        .queue_lobby_transfer(Instant::now(), uuid, req.reason);
    state.flush_queue_logged().await;
    state.emit_proxy_events(vec![event]);
    Ok(StatusCode::ACCEPTED)
}

async fn proxy_state(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ProxyStateSnapshot>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    state.record_proxy_version(&headers);
    let snap = state.lock_games().proxy_snapshot();
    Ok(Json(snap))
}

async fn proxy_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    state.record_proxy_version(&headers);
    let rx = state.proxy_tx.subscribe();
    let stream = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if let Ok(json) = serde_json::to_string(&ev) {
                        return Some((Ok(Event::default().event("proxy").data(json)), rx));
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        skipped,
                        "Flux SSE proxy en retard, resynchronisation nécessaire"
                    );
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn ack_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(transfer_id): Path<Uuid>,
    Json(req): Json<TransferAckRequest>,
) -> Result<StatusCode, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    if !req.success {
        tracing::warn!(%transfer_id, message = ?req.message, "Transfert proxy échoué");
    }
    state
        .lock_games()
        .ack_transfer(Instant::now(), transfer_id, req.success);
    state.flush_queue_logged().await;
    Ok(StatusCode::NO_CONTENT)
}

async fn proxy_player_disconnected(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ProxyDisconnectRequest>,
) -> Result<Json<LeaveQueueResponse>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let resp = state.lock_games().disconnect(req.player_uuid);
    state.flush_queue().await?;
    Ok(Json(resp))
}

async fn report_match(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(report): Json<MatchReport>,
) -> Result<Json<MatchReportAck>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let seen = state.lock_games().seen_players(&report.server_id);
    let ack = state.progression.report_match(&report, &seen).await?;
    Ok(Json(ack))
}

async fn player_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
) -> Result<Json<PlayerProfile>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    Ok(Json(state.progression.profile(uuid).await?))
}

const LEADERBOARD_DEFAULT_LIMIT: u32 = 10;
const LEADERBOARD_MAX_LIMIT: u32 = 50;

#[derive(Debug, Deserialize)]
struct LeaderboardQuery {
    limit: Option<u32>,
}

async fn leaderboard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(game): Path<String>,
    Query(query): Query<LeaderboardQuery>,
) -> Result<Json<Leaderboard>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let game = GameKind::from_slug(&game)
        .ok_or_else(|| ApiError::NotFound(format!("jeu inconnu : {game}")))?;
    let limit = query
        .limit
        .unwrap_or(LEADERBOARD_DEFAULT_LIMIT)
        .min(LEADERBOARD_MAX_LIMIT);
    Ok(Json(state.progression.leaderboard(game, limit).await?))
}

async fn set_identity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
    Json(req): Json<PlayerIdentityUpdate>,
) -> Result<StatusCode, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    state.progression.set_identity(uuid, &req).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn purchase_cosmetic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
    Json(req): Json<PurchaseRequest>,
) -> Result<Json<PlayerProfile>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    Ok(Json(state.progression.purchase(uuid, req.cosmetic).await?))
}

async fn equip_cosmetic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
    Json(req): Json<EquipRequest>,
) -> Result<Json<PlayerProfile>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    Ok(Json(state.progression.equip(uuid, &req).await?))
}

async fn grant_coins(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
    Json(req): Json<AdminCoinGrant>,
) -> Result<Json<PlayerProfile>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    Ok(Json(state.progression.grant(uuid, &req).await?))
}

async fn list_friends(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
) -> Result<Json<FriendsList>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    Ok(Json(state.friends.list(uuid).await?))
}

async fn friend_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(action): Path<String>,
    Json(req): Json<FriendRequest>,
) -> Result<Json<FriendOutcome>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let outcome = match action.as_str() {
        "add" => state.friends.add(&req).await?,
        "accept" => state.friends.accept(&req).await?,
        "deny" => state.friends.deny(&req).await?,
        "remove" => state.friends.remove(&req).await?,
        other => return Err(ApiError::NotFound(format!("action inconnue : {other}"))),
    };
    Ok(Json(outcome))
}

async fn player_party(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Option<Party>>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    Ok(Json(state.lock_parties().party_of(Instant::now(), uuid)))
}

async fn party_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(action): Path<String>,
    Json(req): Json<PartyRequest>,
) -> Result<Json<PartyOutcome>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let now = Instant::now();
    let target = |req: &PartyRequest| {
        req.target
            .clone()
            .ok_or_else(|| ApiError::BadRequest("target manquant".to_owned()))
    };
    let outcome = match action.as_str() {
        "invite" => {
            let target = target(&req)?;
            state.lock_parties().invite(now, req.player, target)
        }
        "accept" => {
            let leader = target(&req)?;
            state.lock_parties().accept(now, req.player, leader.uuid)
        }
        "leave" => state.lock_parties().leave(now, req.player.uuid),
        "kick" => {
            let target = target(&req)?;
            state.lock_parties().kick(now, req.player.uuid, target.uuid)
        }
        other => return Err(ApiError::NotFound(format!("action inconnue : {other}"))),
    };
    Ok(Json(outcome))
}

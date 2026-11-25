use std::time::Duration;

use hypolia_protocol::social::{FriendsList, PlayerRef};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::{FriendStore, FriendStoreError};

#[derive(Debug, Clone)]
pub struct PgFriendStore {
    pool: PgPool,
}

fn unavailable(err: sqlx::Error) -> FriendStoreError {
    FriendStoreError::Unavailable(err.to_string())
}

fn pair(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    if a < b { (a, b) } else { (b, a) }
}

impl PgFriendStore {
    pub async fn connect(url: &str) -> Result<Self, FriendStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(3)
            .acquire_timeout(Duration::from_secs(5))
            .connect(url)
            .await
            .map_err(unavailable)?;
        Ok(Self { pool })
    }

    async fn references(
        &self,
        query: &str,
        player: Uuid,
    ) -> Result<Vec<PlayerRef>, FriendStoreError> {
        let rows = sqlx::query(query)
            .bind(player)
            .fetch_all(&self.pool)
            .await
            .map_err(unavailable)?;
        rows.iter()
            .map(|row| {
                Ok(PlayerRef {
                    uuid: row.try_get("uuid").map_err(unavailable)?,
                    name: row.try_get("name").map_err(unavailable)?,
                })
            })
            .collect()
    }
}

impl FriendStore for PgFriendStore {
    async fn find(&self, name: &str) -> Result<Option<PlayerRef>, FriendStoreError> {
        let row = sqlx::query(
            "SELECT uuid, name FROM players WHERE lower(name) = lower($1) ORDER BY rank_seen_at DESC LIMIT 1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(unavailable)?;
        row.map(|row| {
            Ok(PlayerRef {
                uuid: row.try_get("uuid").map_err(unavailable)?,
                name: row.try_get("name").map_err(unavailable)?,
            })
        })
        .transpose()
    }

    async fn list(&self, player: Uuid) -> Result<FriendsList, FriendStoreError> {
        Ok(FriendsList {
            friends: self
                .references(
                    "SELECT p.uuid, p.name FROM friendships f \
                     JOIN players p ON p.uuid = CASE WHEN f.player_a = $1 THEN f.player_b ELSE f.player_a END \
                     WHERE f.player_a = $1 OR f.player_b = $1 ORDER BY lower(p.name)",
                    player,
                )
                .await?,
            incoming: self
                .references(
                    "SELECT p.uuid, p.name FROM friend_requests r JOIN players p ON p.uuid = r.from_uuid \
                     WHERE r.to_uuid = $1 ORDER BY r.created_at",
                    player,
                )
                .await?,
            outgoing: self
                .references(
                    "SELECT p.uuid, p.name FROM friend_requests r JOIN players p ON p.uuid = r.to_uuid \
                     WHERE r.from_uuid = $1 ORDER BY r.created_at",
                    player,
                )
                .await?,
        })
    }

    async fn are_friends(&self, a: Uuid, b: Uuid) -> Result<bool, FriendStoreError> {
        let (a, b) = pair(a, b);
        sqlx::query("SELECT 1 FROM friendships WHERE player_a = $1 AND player_b = $2")
            .bind(a)
            .bind(b)
            .fetch_optional(&self.pool)
            .await
            .map(|row| row.is_some())
            .map_err(unavailable)
    }

    async fn has_request(&self, from: Uuid, to: Uuid) -> Result<bool, FriendStoreError> {
        sqlx::query("SELECT 1 FROM friend_requests WHERE from_uuid = $1 AND to_uuid = $2")
            .bind(from)
            .bind(to)
            .fetch_optional(&self.pool)
            .await
            .map(|row| row.is_some())
            .map_err(unavailable)
    }

    async fn add_request(&self, from: Uuid, to: Uuid) -> Result<(), FriendStoreError> {
        sqlx::query("INSERT INTO friend_requests (from_uuid, to_uuid) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(from)
            .bind(to)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }

    async fn delete_request(&self, from: Uuid, to: Uuid) -> Result<bool, FriendStoreError> {
        sqlx::query("DELETE FROM friend_requests WHERE from_uuid = $1 AND to_uuid = $2")
            .bind(from)
            .bind(to)
            .execute(&self.pool)
            .await
            .map(|done| done.rows_affected() > 0)
            .map_err(unavailable)
    }

    async fn befriend(&self, a: Uuid, b: Uuid) -> Result<(), FriendStoreError> {
        let (low, high) = pair(a, b);
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        sqlx::query(
            "INSERT INTO friendships (player_a, player_b) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(low)
        .bind(high)
        .execute(&mut *tx)
        .await
        .map_err(unavailable)?;
        sqlx::query(
            "DELETE FROM friend_requests WHERE (from_uuid = $1 AND to_uuid = $2) OR (from_uuid = $2 AND to_uuid = $1)",
        )
        .bind(a)
        .bind(b)
        .execute(&mut *tx)
        .await
        .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)
    }

    async fn unfriend(&self, a: Uuid, b: Uuid) -> Result<bool, FriendStoreError> {
        let (a, b) = pair(a, b);
        sqlx::query("DELETE FROM friendships WHERE player_a = $1 AND player_b = $2")
            .bind(a)
            .bind(b)
            .execute(&self.pool)
            .await
            .map(|done| done.rows_affected() > 0)
            .map_err(unavailable)
    }
}

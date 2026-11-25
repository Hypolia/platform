use std::future::Future;

use hypolia_protocol::social::{FriendOutcome, FriendRequest, FriendsList, MAX_FRIENDS, PlayerRef};
use uuid::Uuid;

use super::{MemoryFriendStore, PgFriendStore};

#[derive(Debug, thiserror::Error)]
pub enum FriendStoreError {
    #[error("base des amis indisponible : {0}")]
    Unavailable(String),
}

pub trait FriendStore: Send + Sync {
    fn find(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<Option<PlayerRef>, FriendStoreError>> + Send;
    fn list(
        &self,
        player: Uuid,
    ) -> impl Future<Output = Result<FriendsList, FriendStoreError>> + Send;
    fn are_friends(
        &self,
        a: Uuid,
        b: Uuid,
    ) -> impl Future<Output = Result<bool, FriendStoreError>> + Send;
    fn has_request(
        &self,
        from: Uuid,
        to: Uuid,
    ) -> impl Future<Output = Result<bool, FriendStoreError>> + Send;
    fn add_request(
        &self,
        from: Uuid,
        to: Uuid,
    ) -> impl Future<Output = Result<(), FriendStoreError>> + Send;
    fn delete_request(
        &self,
        from: Uuid,
        to: Uuid,
    ) -> impl Future<Output = Result<bool, FriendStoreError>> + Send;
    fn befriend(
        &self,
        a: Uuid,
        b: Uuid,
    ) -> impl Future<Output = Result<(), FriendStoreError>> + Send;
    fn unfriend(
        &self,
        a: Uuid,
        b: Uuid,
    ) -> impl Future<Output = Result<bool, FriendStoreError>> + Send;
}

#[derive(Clone)]
pub enum Friends {
    Memory(MemoryFriendStore),
    Postgres(PgFriendStore),
}

impl Default for Friends {
    fn default() -> Self {
        Self::Memory(MemoryFriendStore::default())
    }
}

macro_rules! delegate {
    ($self:ident, $method:ident ( $($arg:expr),* )) => {
        match $self {
            Self::Memory(store) => store.$method($($arg),*).await,
            Self::Postgres(store) => store.$method($($arg),*).await,
        }
    };
}

impl FriendStore for Friends {
    async fn find(&self, name: &str) -> Result<Option<PlayerRef>, FriendStoreError> {
        delegate!(self, find(name))
    }
    async fn list(&self, player: Uuid) -> Result<FriendsList, FriendStoreError> {
        delegate!(self, list(player))
    }
    async fn are_friends(&self, a: Uuid, b: Uuid) -> Result<bool, FriendStoreError> {
        delegate!(self, are_friends(a, b))
    }
    async fn has_request(&self, from: Uuid, to: Uuid) -> Result<bool, FriendStoreError> {
        delegate!(self, has_request(from, to))
    }
    async fn add_request(&self, from: Uuid, to: Uuid) -> Result<(), FriendStoreError> {
        delegate!(self, add_request(from, to))
    }
    async fn delete_request(&self, from: Uuid, to: Uuid) -> Result<bool, FriendStoreError> {
        delegate!(self, delete_request(from, to))
    }
    async fn befriend(&self, a: Uuid, b: Uuid) -> Result<(), FriendStoreError> {
        delegate!(self, befriend(a, b))
    }
    async fn unfriend(&self, a: Uuid, b: Uuid) -> Result<bool, FriendStoreError> {
        delegate!(self, unfriend(a, b))
    }
}

impl Friends {
    async fn target(
        &self,
        request: &FriendRequest,
    ) -> Result<Result<PlayerRef, FriendOutcome>, FriendStoreError> {
        let Some(target) = self.find(&request.target_name).await? else {
            return Ok(Err(FriendOutcome::UnknownPlayer));
        };
        if target.uuid == request.player.uuid {
            return Ok(Err(FriendOutcome::Yourself));
        }
        Ok(Ok(target))
    }

    pub async fn add(&self, request: &FriendRequest) -> Result<FriendOutcome, FriendStoreError> {
        let target = match self.target(request).await? {
            Ok(target) => target,
            Err(outcome) => return Ok(outcome),
        };
        let me = request.player.uuid;
        if self.are_friends(me, target.uuid).await? {
            return Ok(FriendOutcome::AlreadyFriends { target });
        }
        if self.delete_request(target.uuid, me).await? {
            self.befriend(me, target.uuid).await?;
            return Ok(FriendOutcome::Accepted { friend: target });
        }
        if self.has_request(me, target.uuid).await? {
            return Ok(FriendOutcome::AlreadyRequested { target });
        }
        if self.list(me).await?.friends.len() >= MAX_FRIENDS {
            return Ok(FriendOutcome::TooManyFriends);
        }
        self.add_request(me, target.uuid).await?;
        Ok(FriendOutcome::RequestSent { target })
    }

    pub async fn accept(&self, request: &FriendRequest) -> Result<FriendOutcome, FriendStoreError> {
        let target = match self.target(request).await? {
            Ok(target) => target,
            Err(outcome) => return Ok(outcome),
        };
        if !self
            .delete_request(target.uuid, request.player.uuid)
            .await?
        {
            return Ok(FriendOutcome::NoRequest);
        }
        if self.list(request.player.uuid).await?.friends.len() >= MAX_FRIENDS {
            return Ok(FriendOutcome::TooManyFriends);
        }
        self.befriend(request.player.uuid, target.uuid).await?;
        Ok(FriendOutcome::Accepted { friend: target })
    }

    pub async fn deny(&self, request: &FriendRequest) -> Result<FriendOutcome, FriendStoreError> {
        let target = match self.target(request).await? {
            Ok(target) => target,
            Err(outcome) => return Ok(outcome),
        };
        if self
            .delete_request(target.uuid, request.player.uuid)
            .await?
        {
            Ok(FriendOutcome::Denied { target })
        } else {
            Ok(FriendOutcome::NoRequest)
        }
    }

    pub async fn remove(&self, request: &FriendRequest) -> Result<FriendOutcome, FriendStoreError> {
        let target = match self.target(request).await? {
            Ok(target) => target,
            Err(outcome) => return Ok(outcome),
        };
        let me = request.player.uuid;
        let removed = self.unfriend(me, target.uuid).await?;
        let cancelled = self.delete_request(me, target.uuid).await?;
        if removed || cancelled {
            Ok(FriendOutcome::Removed { target })
        } else {
            Ok(FriendOutcome::NotFriends)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(id: u128, name: &str) -> PlayerRef {
        PlayerRef {
            uuid: Uuid::from_u128(id),
            name: name.to_owned(),
        }
    }

    fn friends() -> Friends {
        let store = MemoryFriendStore::default();
        store.know(player(1, "Steve"));
        store.know(player(2, "Alex"));
        store.know(player(3, "Notch"));
        Friends::Memory(store)
    }

    fn ask(from: u128, from_name: &str, target: &str) -> FriendRequest {
        FriendRequest {
            player: player(from, from_name),
            target_name: target.to_owned(),
        }
    }

    #[tokio::test]
    async fn a_request_then_an_accept_makes_friends_both_ways() {
        let friends = friends();
        assert_eq!(
            friends.add(&ask(1, "Steve", "alex")).await.unwrap(),
            FriendOutcome::RequestSent {
                target: player(2, "Alex")
            }
        );
        assert_eq!(
            friends.list(Uuid::from_u128(2)).await.unwrap().incoming,
            [player(1, "Steve")]
        );
        assert_eq!(
            friends.list(Uuid::from_u128(1)).await.unwrap().outgoing,
            [player(2, "Alex")]
        );
        assert_eq!(
            friends.accept(&ask(2, "Alex", "Steve")).await.unwrap(),
            FriendOutcome::Accepted {
                friend: player(1, "Steve")
            }
        );
        assert_eq!(
            friends.list(Uuid::from_u128(1)).await.unwrap().friends,
            [player(2, "Alex")]
        );
        assert_eq!(
            friends.list(Uuid::from_u128(2)).await.unwrap().friends,
            [player(1, "Steve")]
        );
        assert!(
            friends
                .list(Uuid::from_u128(2))
                .await
                .unwrap()
                .incoming
                .is_empty()
        );
    }

    #[tokio::test]
    async fn crossing_requests_become_a_friendship() {
        let friends = friends();
        friends.add(&ask(1, "Steve", "Alex")).await.unwrap();
        assert_eq!(
            friends.add(&ask(2, "Alex", "Steve")).await.unwrap(),
            FriendOutcome::Accepted {
                friend: player(1, "Steve")
            }
        );
    }

    #[tokio::test]
    async fn the_obvious_mistakes_get_their_own_answer() {
        let friends = friends();
        assert_eq!(
            friends.add(&ask(1, "Steve", "Herobrine")).await.unwrap(),
            FriendOutcome::UnknownPlayer
        );
        assert_eq!(
            friends.add(&ask(1, "Steve", "steve")).await.unwrap(),
            FriendOutcome::Yourself
        );
        friends.add(&ask(1, "Steve", "Alex")).await.unwrap();
        assert_eq!(
            friends.add(&ask(1, "Steve", "Alex")).await.unwrap(),
            FriendOutcome::AlreadyRequested {
                target: player(2, "Alex")
            }
        );
        assert_eq!(
            friends.accept(&ask(1, "Steve", "Notch")).await.unwrap(),
            FriendOutcome::NoRequest
        );
        assert_eq!(
            friends.remove(&ask(1, "Steve", "Notch")).await.unwrap(),
            FriendOutcome::NotFriends
        );
    }

    #[tokio::test]
    async fn deny_and_remove_clean_up() {
        let friends = friends();
        friends.add(&ask(1, "Steve", "Alex")).await.unwrap();
        assert_eq!(
            friends.deny(&ask(2, "Alex", "Steve")).await.unwrap(),
            FriendOutcome::Denied {
                target: player(1, "Steve")
            }
        );
        assert!(
            friends
                .list(Uuid::from_u128(1))
                .await
                .unwrap()
                .outgoing
                .is_empty()
        );
        friends.add(&ask(1, "Steve", "Alex")).await.unwrap();
        friends.accept(&ask(2, "Alex", "Steve")).await.unwrap();
        assert_eq!(
            friends.remove(&ask(2, "Alex", "Steve")).await.unwrap(),
            FriendOutcome::Removed {
                target: player(1, "Steve")
            }
        );
        assert!(
            friends
                .list(Uuid::from_u128(1))
                .await
                .unwrap()
                .friends
                .is_empty()
        );
    }
}

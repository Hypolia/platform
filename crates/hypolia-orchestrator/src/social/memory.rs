use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use hypolia_protocol::social::{FriendsList, PlayerRef};
use uuid::Uuid;

use super::{FriendStore, FriendStoreError};

#[derive(Default)]
struct Inner {
    names: BTreeMap<Uuid, String>,
    friendships: BTreeSet<(Uuid, Uuid)>,
    requests: BTreeSet<(Uuid, Uuid)>,
}

#[derive(Clone, Default)]
pub struct MemoryFriendStore {
    inner: Arc<Mutex<Inner>>,
}

fn pair(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    if a < b { (a, b) } else { (b, a) }
}

impl MemoryFriendStore {
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn know(&self, player: PlayerRef) {
        self.inner().names.insert(player.uuid, player.name);
    }
}

impl Inner {
    fn reference(&self, uuid: Uuid) -> PlayerRef {
        PlayerRef {
            uuid,
            name: self.names.get(&uuid).cloned().unwrap_or_default(),
        }
    }
}

impl FriendStore for MemoryFriendStore {
    async fn find(&self, name: &str) -> Result<Option<PlayerRef>, FriendStoreError> {
        let inner = self.inner();
        Ok(inner
            .names
            .iter()
            .find(|(_, known)| known.eq_ignore_ascii_case(name))
            .map(|(uuid, known)| PlayerRef {
                uuid: *uuid,
                name: known.clone(),
            }))
    }

    async fn list(&self, player: Uuid) -> Result<FriendsList, FriendStoreError> {
        let inner = self.inner();
        let mut list = FriendsList::default();
        for (a, b) in &inner.friendships {
            if *a == player {
                list.friends.push(inner.reference(*b));
            } else if *b == player {
                list.friends.push(inner.reference(*a));
            }
        }
        for (from, to) in &inner.requests {
            if *to == player {
                list.incoming.push(inner.reference(*from));
            } else if *from == player {
                list.outgoing.push(inner.reference(*to));
            }
        }
        Ok(list)
    }

    async fn are_friends(&self, a: Uuid, b: Uuid) -> Result<bool, FriendStoreError> {
        Ok(self.inner().friendships.contains(&pair(a, b)))
    }

    async fn has_request(&self, from: Uuid, to: Uuid) -> Result<bool, FriendStoreError> {
        Ok(self.inner().requests.contains(&(from, to)))
    }

    async fn add_request(&self, from: Uuid, to: Uuid) -> Result<(), FriendStoreError> {
        self.inner().requests.insert((from, to));
        Ok(())
    }

    async fn delete_request(&self, from: Uuid, to: Uuid) -> Result<bool, FriendStoreError> {
        Ok(self.inner().requests.remove(&(from, to)))
    }

    async fn befriend(&self, a: Uuid, b: Uuid) -> Result<(), FriendStoreError> {
        let mut inner = self.inner();
        inner.friendships.insert(pair(a, b));
        inner.requests.remove(&(a, b));
        inner.requests.remove(&(b, a));
        Ok(())
    }

    async fn unfriend(&self, a: Uuid, b: Uuid) -> Result<bool, FriendStoreError> {
        Ok(self.inner().friendships.remove(&pair(a, b)))
    }
}

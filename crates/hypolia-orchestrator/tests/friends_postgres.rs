#![cfg(feature = "postgres-tests")]

use hypolia_orchestrator::social::{FriendStore, Friends, PgFriendStore};
use hypolia_progression_pg::PgProgressionRepository;
use hypolia_protocol::social::{FriendOutcome, FriendRequest, PlayerRef};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use uuid::Uuid;

fn player(id: u128, name: &str) -> PlayerRef {
    PlayerRef {
        uuid: Uuid::from_u128(id),
        name: name.to_owned(),
    }
}

fn ask(from: PlayerRef, target: &str) -> FriendRequest {
    FriendRequest {
        player: from,
        target_name: target.to_owned(),
    }
}

#[tokio::test]
async fn friendships_live_in_postgres() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    PgProgressionRepository::connect(&url)
        .await
        .unwrap()
        .migrate()
        .await
        .unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    for who in [player(1, "Steve"), player(2, "Alex"), player(3, "Notch")] {
        sqlx::query("INSERT INTO players (uuid, name) VALUES ($1, $2)")
            .bind(who.uuid)
            .bind(&who.name)
            .execute(&pool)
            .await
            .unwrap();
    }
    let friends = Friends::Postgres(PgFriendStore::connect(&url).await.unwrap());

    assert_eq!(
        friends.add(&ask(player(1, "Steve"), "ALEX")).await.unwrap(),
        FriendOutcome::RequestSent {
            target: player(2, "Alex")
        }
    );
    assert_eq!(
        friends.list(Uuid::from_u128(2)).await.unwrap().incoming,
        [player(1, "Steve")]
    );
    assert_eq!(
        friends.add(&ask(player(1, "Steve"), "alex")).await.unwrap(),
        FriendOutcome::AlreadyRequested {
            target: player(2, "Alex")
        }
    );
    assert_eq!(
        friends
            .accept(&ask(player(2, "Alex"), "steve"))
            .await
            .unwrap(),
        FriendOutcome::Accepted {
            friend: player(1, "Steve")
        }
    );
    friends
        .add(&ask(player(3, "Notch"), "Steve"))
        .await
        .unwrap();
    let steve = friends.list(Uuid::from_u128(1)).await.unwrap();
    assert_eq!(steve.friends, [player(2, "Alex")]);
    assert_eq!(steve.incoming, [player(3, "Notch")]);
    assert!(steve.outgoing.is_empty());
    assert!(
        friends
            .are_friends(Uuid::from_u128(2), Uuid::from_u128(1))
            .await
            .unwrap()
    );
    assert_eq!(
        friends
            .remove(&ask(player(2, "Alex"), "Steve"))
            .await
            .unwrap(),
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
    assert_eq!(
        friends
            .deny(&ask(player(1, "Steve"), "Notch"))
            .await
            .unwrap(),
        FriendOutcome::Denied {
            target: player(3, "Notch")
        }
    );
    assert_eq!(
        friends
            .add(&ask(player(1, "Steve"), "Herobrine"))
            .await
            .unwrap(),
        FriendOutcome::UnknownPlayer
    );
}

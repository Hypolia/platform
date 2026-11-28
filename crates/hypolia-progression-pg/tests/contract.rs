mod common;

use hypolia_progression::MemoryProgressionRepository;

#[tokio::test]
async fn memory_passes_the_contract() {
    common::contract(MemoryProgressionRepository::default()).await;
}

#[tokio::test]
async fn memory_passes_the_leaderboard_contract() {
    common::leaderboard_contract(MemoryProgressionRepository::default()).await;
}

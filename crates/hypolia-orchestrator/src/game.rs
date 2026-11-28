use hypolia_protocol::GameKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameSpec {
    pub game: GameKind,
    pub component: &'static str,
    pub pod_prefix: &'static str,
    pub env_prefix: &'static str,
    pub display_name: &'static str,
    pub default_image: &'static str,
    pub default_max_players: u32,
    pub s3_maps: bool,
}

pub const SHEEPWARS: GameSpec = GameSpec {
    game: GameKind::SheepWars,
    component: "sheepwars",
    pod_prefix: "sheepwars",
    env_prefix: "SHEEPWARS",
    display_name: "SheepWars",
    default_image: "ghcr.io/hypolia/sheepwars:latest",
    default_max_players: 16,
    s3_maps: true,
};

pub const TOWERWARS: GameSpec = GameSpec {
    game: GameKind::TowerWars,
    component: "towerwars",
    pod_prefix: "towerwars",
    env_prefix: "TOWERWARS",
    display_name: "TowerWars",
    default_image: "ghcr.io/hypolia/towerwars:latest",
    default_max_players: 12,
    s3_maps: false,
};

pub const DEACOUDRE: GameSpec = GameSpec {
    game: GameKind::DeACoudre,
    component: "deacoudre",
    pod_prefix: "deacoudre",
    env_prefix: "DEACOUDRE",
    display_name: "Dé à coudre",
    default_image: "ghcr.io/hypolia/deacoudre:latest",
    default_max_players: 8,
    s3_maps: true,
};

impl GameSpec {
    pub fn maps_prefix(&self) -> String {
        format!("{}/", self.component)
    }

    pub const fn of(game: GameKind) -> &'static Self {
        match game {
            GameKind::SheepWars => &SHEEPWARS,
            GameKind::TowerWars => &TOWERWARS,
            GameKind::DeACoudre => &DEACOUDRE,
        }
    }

    pub fn pod_selector(&self) -> String {
        format!(
            "app.kubernetes.io/component={},app.kubernetes.io/managed-by=hypolia-orchestrator",
            self.component
        )
    }

    pub fn env_name(&self, suffix: &str) -> String {
        format!("{}_{suffix}", self.env_prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_game_has_a_distinct_spec() {
        for game in GameKind::ALL {
            assert_eq!(GameSpec::of(game).game, game);
            assert_eq!(GameSpec::of(game).component, game.slug());
        }
        assert_ne!(SHEEPWARS.pod_prefix, TOWERWARS.pod_prefix);
        assert_ne!(DEACOUDRE.pod_prefix, TOWERWARS.pod_prefix);
        assert_ne!(DEACOUDRE.pod_prefix, SHEEPWARS.pod_prefix);
    }

    #[test]
    fn deacoudre_spec_has_its_own_identity() {
        let spec = GameSpec::of(GameKind::DeACoudre);
        assert_eq!(spec.component, "deacoudre");
        assert_eq!(spec.env_name("MAX_PLAYERS"), "DEACOUDRE_MAX_PLAYERS");
        assert_eq!(spec.default_max_players, 8);
        assert_eq!(spec.default_image, "ghcr.io/hypolia/deacoudre:latest");
        assert!(spec.s3_maps);
        assert_eq!(spec.maps_prefix(), "deacoudre/");
        assert_eq!(
            spec.pod_selector(),
            "app.kubernetes.io/component=deacoudre,app.kubernetes.io/managed-by=hypolia-orchestrator"
        );
    }

    #[test]
    fn sheepwars_selector_is_the_historical_one() {
        assert_eq!(
            SHEEPWARS.pod_selector(),
            "app.kubernetes.io/component=sheepwars,app.kubernetes.io/managed-by=hypolia-orchestrator"
        );
        assert_eq!(SHEEPWARS.env_name("MAP"), "SHEEPWARS_MAP");
    }
}

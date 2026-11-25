CREATE TABLE friendships (
    player_a uuid NOT NULL,
    player_b uuid NOT NULL,
    since timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (player_a, player_b),
    CHECK (player_a < player_b)
);

CREATE INDEX friendships_player_b_idx ON friendships (player_b);

CREATE TABLE friend_requests (
    from_uuid uuid NOT NULL,
    to_uuid uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (from_uuid, to_uuid),
    CHECK (from_uuid <> to_uuid)
);

CREATE INDEX friend_requests_to_idx ON friend_requests (to_uuid);

CREATE INDEX players_lower_name_idx ON players (lower(name));

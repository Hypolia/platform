ALTER TABLE queue_entries
    DROP CONSTRAINT IF EXISTS queue_entries_game_check;

ALTER TABLE queue_entries
    ADD CONSTRAINT queue_entries_game_check
    CHECK (game IN ('sheepwars', 'towerwars', 'deacoudre'));

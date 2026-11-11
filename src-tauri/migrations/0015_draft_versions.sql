-- Dernière version déposée sur le serveur pour chaque brouillon : on ne peut pas la retrouver par une recherche
-- (certains serveurs n'indexent pas aussitôt un message déposé), ni toujours par la synchro, qui peut être en retard.
CREATE TABLE draft_versions (
    account_id   TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    draft_id     TEXT    NOT NULL,
    path         TEXT    NOT NULL,
    uid_validity INTEGER,
    uid          INTEGER NOT NULL,
    PRIMARY KEY (account_id, draft_id)
);

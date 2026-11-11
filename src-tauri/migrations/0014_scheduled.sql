-- Envois programmés : messages complets en attente de leur heure (l'application doit être ouverte pour les envoyer).
CREATE TABLE scheduled (
    id         INTEGER PRIMARY KEY,
    account_id TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    send_at    INTEGER NOT NULL,                 -- secondes Unix (UTC)
    message    TEXT    NOT NULL,                 -- OutgoingMessage en JSON
    status     TEXT    NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'sending', 'failed')),
    error      TEXT
);
CREATE INDEX idx_scheduled_due ON scheduled(send_at) WHERE status = 'pending';

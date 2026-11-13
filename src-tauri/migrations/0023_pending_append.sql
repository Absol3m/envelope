-- Copies dans « Envoyés » à déposer sur le serveur (APPEND IMAP) : file persistante, reprise à chaque synchro.
-- Le message brut est dans le magasin `raw` (jamais supprimé) ; la ligne disparaît quand la copie est sur le serveur
-- (ou, après un échec durable, rangée dans un dossier local). `message_id` : retrouver une copie déjà déposée.
CREATE TABLE pending_append (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    raw_hash    TEXT NOT NULL,
    message_id  TEXT,
    attempts    INTEGER NOT NULL DEFAULT 0,   -- échecs (délai croissant entre deux essais)
    refusals    INTEGER NOT NULL DEFAULT 0,   -- dont refus du serveur (NO) : au-delà de quelques-uns, copie locale
    next_try_at INTEGER NOT NULL,
    last_error  TEXT,
    created_at  INTEGER NOT NULL
);
CREATE INDEX idx_pending_append_account ON pending_append(account_id, next_try_at);

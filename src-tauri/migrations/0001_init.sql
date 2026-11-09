-- Schéma initial. Les secrets (mots de passe, tokens OAuth2) ne sont JAMAIS stockés ici :
-- ils vivent dans le trousseau de l'OS (crate `keyring`).

CREATE TABLE accounts (
    id            TEXT PRIMARY KEY,                -- UUID v4
    email         TEXT NOT NULL UNIQUE,
    display_name  TEXT,
    provider      TEXT NOT NULL DEFAULT 'imap',    -- imap | gmail | outlook | ...
    auth_kind     TEXT NOT NULL DEFAULT 'password' CHECK (auth_kind IN ('password','oauth2')),
    username      TEXT NOT NULL,
    imap_host     TEXT NOT NULL,
    imap_port     INTEGER NOT NULL DEFAULT 993,
    imap_security TEXT NOT NULL DEFAULT 'tls' CHECK (imap_security IN ('tls','starttls','none')),
    smtp_host     TEXT NOT NULL,
    smtp_port     INTEGER NOT NULL DEFAULT 465,
    smtp_security TEXT NOT NULL DEFAULT 'tls' CHECK (smtp_security IN ('tls','starttls','none')),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    last_sync_at  TEXT
);

CREATE TABLE folders (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id     TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    path           TEXT NOT NULL,                  -- chemin IMAP complet
    name           TEXT NOT NULL,
    role           TEXT CHECK (role IN ('inbox','sent','drafts','trash','junk','archive','all','flagged')),
    delimiter      TEXT,
    uid_validity   INTEGER,                        -- invalide le cache local si ça change
    uid_next       INTEGER,
    highest_modseq INTEGER,                        -- CONDSTORE
    unread_count   INTEGER NOT NULL DEFAULT 0,
    total_count    INTEGER NOT NULL DEFAULT 0,
    UNIQUE (account_id, path)
);

CREATE TABLE threads (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    subject         TEXT NOT NULL DEFAULT '',
    snippet         TEXT NOT NULL DEFAULT '',
    last_message_at TEXT NOT NULL,
    message_count   INTEGER NOT NULL DEFAULT 0,
    unread_count    INTEGER NOT NULL DEFAULT 0,
    is_starred      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_threads_account_date ON threads(account_id, last_message_at DESC);

CREATE TABLE messages (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id       INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    thread_id       INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    uid             INTEGER NOT NULL,
    message_id      TEXT,                          -- en-tête Message-ID
    in_reply_to     TEXT,
    references_ids  TEXT,                          -- JSON array
    subject         TEXT NOT NULL DEFAULT '',
    from_name       TEXT,
    from_addr       TEXT NOT NULL DEFAULT '',
    to_addrs        TEXT NOT NULL DEFAULT '[]',    -- JSON array
    cc_addrs        TEXT NOT NULL DEFAULT '[]',
    bcc_addrs       TEXT NOT NULL DEFAULT '[]',
    date            TEXT NOT NULL,
    snippet         TEXT NOT NULL DEFAULT '',
    body_text       TEXT,
    body_html       TEXT,                          -- brut : toujours sanitiser côté affichage
    is_read         INTEGER NOT NULL DEFAULT 0,
    is_starred      INTEGER NOT NULL DEFAULT 0,
    is_archived     INTEGER NOT NULL DEFAULT 0,
    has_attachments INTEGER NOT NULL DEFAULT 0,
    size            INTEGER,
    UNIQUE (folder_id, uid)
);
CREATE INDEX idx_messages_thread ON messages(thread_id);
CREATE INDEX idx_messages_folder_date ON messages(folder_id, date DESC);
CREATE INDEX idx_messages_message_id ON messages(message_id);

CREATE TABLE attachments (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id   INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    filename     TEXT,
    mime_type    TEXT NOT NULL,
    size         INTEGER NOT NULL DEFAULT 0,
    content_id   TEXT,                             -- images inline (cid:)
    is_inline    INTEGER NOT NULL DEFAULT 0,
    storage_path TEXT                              -- NULL tant que non téléchargée
);
CREATE INDEX idx_attachments_message ON attachments(message_id);

CREATE TABLE labels (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    color      TEXT,
    UNIQUE (account_id, name)
);
CREATE TABLE message_labels (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    label_id   INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, label_id)
);

-- Recherche plein texte (FTS5), synchronisée par triggers (table « external content »).
CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject, from_name, from_addr, body_text,
    content='messages', content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, subject, from_name, from_addr, body_text)
    VALUES (new.id, new.subject, new.from_name, new.from_addr, new.body_text);
END;
CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_addr, body_text)
    VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.body_text);
END;
CREATE TRIGGER messages_au AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_addr, body_text)
    VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.body_text);
    INSERT INTO messages_fts(rowid, subject, from_name, from_addr, body_text)
    VALUES (new.id, new.subject, new.from_name, new.from_addr, new.body_text);
END;

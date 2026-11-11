-- Identités d'envoi : adresses (alias) et noms sous lesquels un compte peut écrire. L'authentification reste celle du compte.
CREATE TABLE identities (
    id         INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name       TEXT NOT NULL DEFAULT '',
    email      TEXT NOT NULL COLLATE NOCASE,
    UNIQUE (account_id, email)
);

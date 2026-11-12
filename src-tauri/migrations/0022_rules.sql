-- Règles locales : appliquées aux nouveaux messages de la boîte de réception à la synchro.
-- `query` : la syntaxe de la recherche (de:, objet:, pj, texte libre…), évaluée sur les messages de la conversation.
-- Les règles s'appliquent dans l'ordre des identifiants ; un déplacement met fin à l'examen de la conversation.
CREATE TABLE rules (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL,
    enabled    INTEGER NOT NULL DEFAULT 1,
    account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,  -- NULL : tous les comptes
    query      TEXT NOT NULL,
    mark_read  INTEGER NOT NULL DEFAULT 0,
    star       INTEGER NOT NULL DEFAULT 0,
    move_to    TEXT CHECK (move_to IN ('archive','trash','junk'))
);

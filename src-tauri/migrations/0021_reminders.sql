-- Rappels locaux par conversation (un seul à la fois) :
--   snooze   : la conversation quitte les listes de dossiers jusqu'à `due_at`, puis y revient ;
--   followup : « relancer si pas de réponse » : à `due_at`, la conversation revient en haut seulement si
--              aucun message n'est arrivé depuis (`baseline_count` = nombre de messages à la création).
-- `fired_at` : échéance passée, conversation revenue (sert à la remonter dans la liste).
CREATE TABLE reminders (
    thread_id      INTEGER PRIMARY KEY REFERENCES threads(id) ON DELETE CASCADE,
    kind           TEXT NOT NULL CHECK (kind IN ('snooze','followup')),
    due_at         INTEGER NOT NULL,
    baseline_count INTEGER NOT NULL DEFAULT 0,
    fired_at       INTEGER
);
CREATE INDEX idx_reminders_due ON reminders(due_at) WHERE fired_at IS NULL;

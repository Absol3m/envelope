-- En-têtes List-Unsubscribe (RFC 2369) / List-Unsubscribe-Post (RFC 8058) des messages reçus.
-- Contenu non fiable : lien https et adresse mailto extraits, jamais exécutés sans action explicite.
ALTER TABLE messages ADD COLUMN unsub_url TEXT;
ALTER TABLE messages ADD COLUMN unsub_mailto TEXT;
ALTER TABLE messages ADD COLUMN unsub_one_click INTEGER NOT NULL DEFAULT 0;

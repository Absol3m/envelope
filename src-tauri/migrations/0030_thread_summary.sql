-- Résumé d'une conversation dénormalisé dans `threads` (déjà recalculé par `refresh_threads` pour les compteurs) :
-- la liste n'exécute plus quatre sous-requêtes corrélées par ligne. Remplissage identique à `refresh_threads`.
ALTER TABLE threads ADD COLUMN has_attachments INTEGER NOT NULL DEFAULT 0;
ALTER TABLE threads ADD COLUMN from_name TEXT;
ALTER TABLE threads ADD COLUMN from_addr TEXT;
ALTER TABLE threads ADD COLUMN participants TEXT;
ALTER TABLE threads ADD COLUMN folder_id INTEGER;

UPDATE threads SET
    has_attachments = EXISTS (SELECT 1 FROM messages m WHERE m.thread_id = threads.id AND m.has_attachments = 1),
    from_name = (SELECT from_name FROM messages WHERE thread_id = threads.id ORDER BY date DESC, id DESC LIMIT 1),
    from_addr = (SELECT from_addr FROM messages WHERE thread_id = threads.id ORDER BY date DESC, id DESC LIMIT 1),
    participants = (SELECT group_concat(n, ', ') FROM (SELECT DISTINCT COALESCE(NULLIF(from_name, ''), from_addr) AS n
        FROM messages WHERE thread_id = threads.id ORDER BY date LIMIT 4)),
    folder_id = (SELECT folder_id FROM messages WHERE thread_id = threads.id ORDER BY date DESC, id DESC LIMIT 1);

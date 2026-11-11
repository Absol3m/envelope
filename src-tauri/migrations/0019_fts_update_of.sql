-- L'index plein texte ne couvre que quatre colonnes. Le trigger d'origine se déclenchait à CHAQUE modification d'un
-- message (marquer lu, étoile, archiver, rattacher à une conversation, enregistrer l'empreinte du brut…) et
-- supprimait puis réindexait tout le texte : 30 à 50 µs par message, proportionnel à la taille du corps.
-- Désormais seule la modification d'une colonne indexée touche l'index.
DROP TRIGGER messages_au;
CREATE TRIGGER messages_au AFTER UPDATE OF subject, from_name, from_addr, body_text ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_addr, body_text)
    VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.body_text);
    INSERT INTO messages_fts(rowid, subject, from_name, from_addr, body_text)
    VALUES (new.id, new.subject, new.from_name, new.from_addr, new.body_text);
END;

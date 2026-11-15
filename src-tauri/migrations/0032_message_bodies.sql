-- À sens unique (une application plus ancienne ne sait plus lire cette base). Suppose SQLite >= 3.43 (3.46 embarqué par
-- sqlx : un test vérifie la version). Le fichier ne rétrécit qu'au prochain VACUUM (les pages libérées restent allouées).
-- Les corps (`body_text`, `body_html`) quittent `messages` : la table reste étroite (listes, jointures, VACUUM) et les
-- lignes larges vivent à part, lues seulement à l'ouverture d'un message.
CREATE TABLE message_bodies (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    body_text  TEXT,
    body_html  TEXT                                   -- brut : toujours sanitiser côté affichage
);
INSERT INTO message_bodies (message_id, body_text, body_html)
    SELECT id, body_text, body_html FROM messages WHERE body_text IS NOT NULL OR body_html IS NOT NULL;

-- Recherche plein texte : l'ancienne table FTS5 « external content » lisait les corps dans `messages`. Elle devient
-- « contentless » avec suppression par rowid (SQLite ≥ 3.43) : l'index garde les mots sans dupliquer le texte, et les
-- triggers n'ont plus à rejouer les anciennes valeurs (une valeur fausse corromprait un index à contenu externe).
DROP TRIGGER messages_ai;
DROP TRIGGER messages_ad;
DROP TRIGGER messages_au;
DROP TABLE messages_fts;
CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject, from_name, from_addr, body_text,
    content='', contentless_delete=1,
    tokenize='unicode61 remove_diacritics 2'
);
INSERT INTO messages_fts (rowid, subject, from_name, from_addr, body_text)
    SELECT m.id, m.subject, m.from_name, m.from_addr, COALESCE(b.body_text, '')
    FROM messages m LEFT JOIN message_bodies b ON b.message_id = m.id;

ALTER TABLE messages DROP COLUMN body_text;
ALTER TABLE messages DROP COLUMN body_html;

-- Un message arrive sans corps (et certains n'en ont jamais) : il est indexé tout de suite par son objet et son
-- expéditeur ; son corps, écrit juste après dans `message_bodies`, complète l'index (même mesure d'ingestion qu'avant).
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts (rowid, subject, from_name, from_addr, body_text)
    VALUES (new.id, new.subject, new.from_name, new.from_addr, '');
END;
CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
    DELETE FROM messages_fts WHERE rowid = old.id;   -- le corps part avec le message (ON DELETE CASCADE)
END;
-- Seules les colonnes indexées touchent l'index (marquer lu, étoile, archiver… ne le réécrivent pas).
CREATE TRIGGER messages_au AFTER UPDATE OF subject, from_name, from_addr ON messages BEGIN
    DELETE FROM messages_fts WHERE rowid = old.id;
    INSERT INTO messages_fts (rowid, subject, from_name, from_addr, body_text)
    VALUES (new.id, new.subject, new.from_name, new.from_addr,
            COALESCE((SELECT body_text FROM message_bodies WHERE message_id = new.id), ''));
END;
CREATE TRIGGER message_bodies_ai AFTER INSERT ON message_bodies BEGIN
    DELETE FROM messages_fts WHERE rowid = new.message_id;
    INSERT INTO messages_fts (rowid, subject, from_name, from_addr, body_text)
    SELECT id, subject, from_name, from_addr, COALESCE(new.body_text, '') FROM messages WHERE id = new.message_id;
END;
-- (Volontairement jumeau du précédent : SQLite n'a pas de trigger « INSERT OU UPDATE ».)
CREATE TRIGGER message_bodies_au AFTER UPDATE OF body_text ON message_bodies
WHEN old.body_text IS NOT new.body_text BEGIN
    DELETE FROM messages_fts WHERE rowid = new.message_id;
    INSERT INTO messages_fts (rowid, subject, from_name, from_addr, body_text)
    SELECT id, subject, from_name, from_addr, COALESCE(new.body_text, '') FROM messages WHERE id = new.message_id;
END;

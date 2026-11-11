-- Carnet d'adresses : adresses déjà vues (expéditeurs, destinataires, copies), pour l'autocomplétion des destinataires.
-- Avant : la suggestion relisait tous les messages à chaque frappe (38 ms à 20 000 messages, proportionnel à la base).
-- Maintenant : une ligne par adresse et par compte, tenue à jour par un trigger à l'insertion d'un message (donc par
-- tous les chemins d'entrée : synchro, import, archive locale). Les messages supprimés ne la décrémentent pas : une
-- adresse déjà vue reste proposée, comme dans un carnet d'adresses. Les brouillons n'y comptent pas : ils sont
-- ré-enregistrés à chaque frappe, un destinataire tapé puis effacé y resterait.
CREATE TABLE contacts (
    account_id TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    address    TEXT    NOT NULL,              -- en minuscules
    name       TEXT,                          -- dernier nom non vide vu
    uses       INTEGER NOT NULL DEFAULT 1,    -- apparitions (de / à / cc) : sert à classer les suggestions
    last_at    TEXT    NOT NULL,              -- date du message le plus récent
    PRIMARY KEY (account_id, address)
) WITHOUT ROWID;

-- Reprise de l'existant (une fois).
-- BACKFILL-BEGIN
INSERT INTO contacts (account_id, address, name, uses, last_at)
SELECT account_id, LOWER(TRIM(addr)), MAX(NULLIF(name, '')), COUNT(*), MAX(date)
FROM (
    SELECT account_id, from_addr AS addr, from_name AS name, date FROM messages
     WHERE folder_id NOT IN (SELECT id FROM folders WHERE role = 'drafts')
    UNION ALL
    SELECT m.account_id, json_extract(j.value, '$.address'), json_extract(j.value, '$.name'), m.date
      FROM messages m, json_each(CASE WHEN json_valid(m.to_addrs) THEN m.to_addrs ELSE '[]' END) j
     WHERE m.folder_id NOT IN (SELECT id FROM folders WHERE role = 'drafts')
    UNION ALL
    SELECT m.account_id, json_extract(j.value, '$.address'), json_extract(j.value, '$.name'), m.date
      FROM messages m, json_each(CASE WHEN json_valid(m.cc_addrs) THEN m.cc_addrs ELSE '[]' END) j
     WHERE m.folder_id NOT IN (SELECT id FROM folders WHERE role = 'drafts')
)
WHERE addr IS NOT NULL AND TRIM(addr) <> ''
GROUP BY account_id, LOWER(TRIM(addr));
-- BACKFILL-END

-- Un JSON d'adresses invalide est ignoré : il ne doit jamais faire échouer l'insertion d'un message.
CREATE TRIGGER contacts_ai AFTER INSERT ON messages
WHEN NOT EXISTS (SELECT 1 FROM folders WHERE id = NEW.folder_id AND role = 'drafts')
BEGIN
    INSERT INTO contacts (account_id, address, name, uses, last_at)
    SELECT NEW.account_id, LOWER(TRIM(addr)), NULLIF(name, ''), 1, NEW.date
    FROM (
        SELECT NEW.from_addr AS addr, NEW.from_name AS name
        UNION ALL
        SELECT json_extract(j.value, '$.address'), json_extract(j.value, '$.name')
          FROM json_each(CASE WHEN json_valid(NEW.to_addrs) THEN NEW.to_addrs ELSE '[]' END) j
        UNION ALL
        SELECT json_extract(j.value, '$.address'), json_extract(j.value, '$.name')
          FROM json_each(CASE WHEN json_valid(NEW.cc_addrs) THEN NEW.cc_addrs ELSE '[]' END) j
    )
    WHERE addr IS NOT NULL AND TRIM(addr) <> ''
    ON CONFLICT (account_id, address) DO UPDATE SET
        uses    = uses + 1,
        -- le nom du message le plus récent l'emporte (le rapatriement va du plus récent au plus ancien)
        name    = CASE WHEN excluded.name IS NULL THEN name
                       WHEN name IS NULL OR excluded.last_at >= last_at THEN excluded.name
                       ELSE name END,
        last_at = MAX(last_at, excluded.last_at);
END;

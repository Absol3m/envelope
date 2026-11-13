-- Les lignes provisoires de « Envoyés » (UID négatif, voir mail/sentcopy.rs) ne comptent pas dans le carnet d'adresses :
-- la vraie copie, rapatriée ensuite, le fait déjà ; sans cela chaque envoi compterait deux fois. `uid > 0` est la
-- condition de `server_row!()` (src/mail/mod.rs) : les deux doivent rester d'accord.
DROP TRIGGER contacts_ai;

CREATE TRIGGER contacts_ai AFTER INSERT ON messages
WHEN NEW.uid > 0
 AND NOT EXISTS (SELECT 1 FROM folders WHERE id = NEW.folder_id AND role = 'drafts')
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

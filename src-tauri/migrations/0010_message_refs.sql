-- Liaison des conversations : « ce message est cité par un message déjà reçu » devait balayer toute la
-- table (instr sur le JSON des références) pour chaque début de fil, ce qui rend l'import ou le
-- rapatriement d'une grosse boîte quadratique. Une table indexée des références le remplace.
CREATE TABLE message_refs (
    account_id TEXT    NOT NULL,
    ref_id     TEXT    NOT NULL,                       -- Message-ID cité dans References
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE
);
CREATE INDEX idx_message_refs ON message_refs(account_id, ref_id);
INSERT INTO message_refs (account_id, ref_id, message_id)
SELECT m.account_id, j.value, m.id FROM messages m, json_each(m.references_ids) j
WHERE m.references_ids IS NOT NULL AND json_valid(m.references_ids) AND j.type = 'text';

-- Messages dont le brut reste à récupérer (le rapatriement le demande à chaque réveil de la synchro) :
-- index partiel, quasi vide en régime normal, au lieu d'un balayage de la table.
CREATE INDEX idx_messages_raw_missing ON messages(account_id) WHERE raw_hash IS NULL;

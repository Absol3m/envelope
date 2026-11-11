-- Brouillons synchronisés : l'identifiant du brouillon (en-tête X-Envelope-Draft-Id) permet de retrouver
-- et de remplacer la version précédente sur le serveur (un message IMAP ne se modifie pas).
ALTER TABLE messages ADD COLUMN draft_id TEXT;
CREATE INDEX idx_messages_draft ON messages(account_id, draft_id) WHERE draft_id IS NOT NULL;

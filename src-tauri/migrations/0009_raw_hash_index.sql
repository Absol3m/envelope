-- Dédoublonnage par message brut (import, archivage local) : recherche par (dossier, empreinte).
CREATE INDEX IF NOT EXISTS idx_messages_folder_raw ON messages(folder_id, raw_hash);

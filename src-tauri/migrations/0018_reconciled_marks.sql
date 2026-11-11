-- État d'un dossier à sa dernière réconciliation COMPLÈTE des drapeaux (UIDNEXT, HIGHESTMODSEQ, nombre de messages,
-- date). Tant que le serveur annonce le même état, on ne relit pas les drapeaux de tout l'historique (voir `mail::sync`).
-- NULL = jamais réconcilié : la première passe complète s'en charge.
ALTER TABLE folders ADD COLUMN reconciled_uid_next INTEGER;
ALTER TABLE folders ADD COLUMN reconciled_modseq   INTEGER;
ALTER TABLE folders ADD COLUMN reconciled_count    INTEGER;
ALTER TABLE folders ADD COLUMN reconciled_at       INTEGER;  -- secondes Unix

-- État de synchro par compte (affichage UI + diagnostic).
ALTER TABLE accounts ADD COLUMN sync_status TEXT NOT NULL DEFAULT 'idle'; -- idle | syncing | error
ALTER TABLE accounts ADD COLUMN sync_error  TEXT;

-- Valeurs fermées de `accounts.provider` et `accounts.sync_status` (types `FolderRole`/`Provider`/`SyncStatus` de
-- envelope-core). SQLite ne sait pas ajouter une contrainte CHECK sans reconstruire la table (et ses ~10 tables
-- liées en cascade) : des déclencheurs tiennent lieu de CHECK. D'abord on ramène à une valeur connue ce qu'une
-- ancienne version aurait pu écrire, pour que la relecture typée ne puisse pas échouer. Un nouveau fournisseur demande
-- une nouvelle migration qui recrée ces déclencheurs (et l'union `Provider` de src/lib/api.ts). `auth_kind` et
-- `*_security` ont déjà leur CHECK (migration 0001).
UPDATE accounts SET provider = 'imap'
 WHERE provider NOT IN ('gmail','outlook','icloud','yahoo','fastmail','infomaniak','ovh','gandi','local','imap');
UPDATE accounts SET sync_status = 'idle' WHERE sync_status NOT IN ('idle','syncing','error');

CREATE TRIGGER accounts_provider_ins BEFORE INSERT ON accounts
WHEN NEW.provider NOT IN ('gmail','outlook','icloud','yahoo','fastmail','infomaniak','ovh','gandi','local','imap')
BEGIN SELECT RAISE(ABORT, 'accounts.provider: unknown value'); END;

CREATE TRIGGER accounts_provider_upd BEFORE UPDATE OF provider ON accounts
WHEN NEW.provider NOT IN ('gmail','outlook','icloud','yahoo','fastmail','infomaniak','ovh','gandi','local','imap')
BEGIN SELECT RAISE(ABORT, 'accounts.provider: unknown value'); END;

CREATE TRIGGER accounts_sync_status_ins BEFORE INSERT ON accounts
WHEN NEW.sync_status NOT IN ('idle','syncing','error')
BEGIN SELECT RAISE(ABORT, 'accounts.sync_status: unknown value'); END;

CREATE TRIGGER accounts_sync_status_upd BEFORE UPDATE OF sync_status ON accounts
WHEN NEW.sync_status NOT IN ('idle','syncing','error')
BEGIN SELECT RAISE(ABORT, 'accounts.sync_status: unknown value'); END;

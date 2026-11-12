//! Couche SQLite : ouverture du pool, pragmas et migrations embarquées.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

use crate::error::Result;

/// Chemin du fichier de base dans le dossier de données de l'application.
pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join(envelope_core::db::DB_FILE)
}

/// Ouvre (et crée si besoin) la base, active WAL + clés étrangères, applique les migrations.
pub async fn init(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent() {
        crate::private::create_dir_all_sync(parent)?;
    }
    crate::private::restrict_database(path)?; // avant l'ouverture : SQLite copie ces droits sur ses fichiers -wal et -shm
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal) // lecteurs concurrents pendant la synchro
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Base SQLite en mémoire (tests). Une seule connexion : chaque connexion `:memory:` est distincte.
#[cfg(test)]
pub async fn init_memory() -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

#[cfg(test)]
mod tests {
    /// Le serveur MCP retrouve la base grâce à cet identifiant : il doit suivre `tauri.conf.json`.
    #[test]
    fn app_id_matches_tauri_conf() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], envelope_core::db::APP_ID);
    }

    /// Les données de l'utilisateur ne sont lisibles que par lui : dossier, base, et fichiers `-wal`/`-shm` de SQLite
    /// (qui reprennent les droits de la base), y compris pour une installation antérieure restée en 755/644.
    #[cfg(unix)]
    #[tokio::test]
    async fn base_et_dossier_a_l_utilisateur_seul_meme_apres_une_installation_ouverte() {
        use super::init;
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let dir = std::env::temp_dir().join(format!("envelope-db-{}", uuid::Uuid::new_v4()));
        let path = dir.join("envelope.db");
        // Installation antérieure : dossier et base ouverts à tous.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(&path, b"").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let pool = init(&path).await.unwrap();
        sqlx::query("INSERT INTO settings (key, value) VALUES ('t', 'v')")
            .execute(&pool)
            .await
            .unwrap(); // crée -wal et -shm
        assert_eq!((mode(&dir), mode(&path)), (0o700, 0o600));
        for suffix in ["-wal", "-shm"] {
            let f = dir.join(format!("envelope.db{suffix}"));
            assert!(f.exists(), "{suffix} attendu");
            assert_eq!(mode(&f), 0o600, "{suffix}");
        }
        // Base neuve : créée directement avec les bons droits.
        let fresh = std::env::temp_dir()
            .join(format!("envelope-db-{}", uuid::Uuid::new_v4()))
            .join("envelope.db");
        init(&fresh).await.unwrap();
        assert_eq!(
            (mode(fresh.parent().unwrap()), mode(&fresh)),
            (0o700, 0o600)
        );
    }
}

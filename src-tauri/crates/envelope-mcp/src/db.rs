//! Ouverture de la base de l'application, en lecture seule.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

/// `--db`, sinon `ENVELOPE_DB`, sinon le dossier de données de l'application.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("ENVELOPE_DB")
        .map(PathBuf::from)
        .or_else(envelope_core::db::default_path)
}

/// Ouvre la base sans pouvoir y écrire (le mode WAL permet de lire pendant que l'application synchronise).
pub async fn open_readonly(path: &Path) -> Result<SqlitePool, String> {
    if !path.exists() {
        return Err(format!(
            "database not found: {} (start Envelope once, or pass --db)",
            path.display()
        ));
    }
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .busy_timeout(Duration::from_secs(5));
    SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(opts)
        .await
        .map_err(|e| format!("ouverture de {} : {e}", path.display()))
}

/// Le verrouillage de l'application est-il actif ? (réglage `lock.method` ≠ `off`). Le serveur MCP lit le fichier de
/// la base directement, sans passer par l'application : il ne connaît pas l'état « verrouillé » et ne peut pas
/// l'imposer. Il refuse donc de démarrer tant qu'un verrouillage est configuré, sauf demande explicite.
/// Une base sans table `settings` (ancienne version) n'a pas de verrouillage.
pub async fn lock_enabled(pool: &SqlitePool) -> bool {
    sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = 'lock.method'")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .is_some_and(|m| m != "off")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool(with_table: bool) -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        if with_table {
            sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
                .execute(&pool)
                .await
                .unwrap();
        }
        pool
    }

    #[tokio::test]
    async fn verrouillage_detecte_dans_les_reglages() {
        assert!(
            !lock_enabled(&pool(false).await).await,
            "ancienne base : pas de verrouillage"
        );
        let p = pool(true).await;
        assert!(!lock_enabled(&p).await, "aucun réglage");
        sqlx::query("INSERT INTO settings VALUES ('lock.method', 'off')")
            .execute(&p)
            .await
            .unwrap();
        assert!(!lock_enabled(&p).await);
        sqlx::query("UPDATE settings SET value = 'password'")
            .execute(&p)
            .await
            .unwrap();
        assert!(lock_enabled(&p).await);
        sqlx::query("UPDATE settings SET value = 'system'")
            .execute(&p)
            .await
            .unwrap();
        assert!(lock_enabled(&p).await);
    }
}

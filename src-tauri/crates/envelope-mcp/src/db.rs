//! Ouverture de la base de l'application, en lecture seule.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

/// Identifiant Tauri de l'application : le dossier de données en porte le nom.
const APP_ID: &str = "dev.envelope.app";

/// `--db`, sinon `ENVELOPE_DB`, sinon le dossier de données de l'application.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("ENVELOPE_DB")
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|d| d.join(APP_ID).join("envelope.db")))
}

/// Ouvre la base sans pouvoir y écrire (le mode WAL permet de lire pendant que l'application synchronise).
pub async fn open_readonly(path: &Path) -> Result<SqlitePool, String> {
    if !path.exists() {
        return Err(format!(
            "base introuvable : {} (lancez Envelope une première fois, ou indiquez --db)",
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

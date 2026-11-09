//! Couche SQLite : ouverture du pool, pragmas et migrations embarquées.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

use crate::error::Result;

/// Chemin du fichier de base dans le dossier de données de l'application.
pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("envelope.db")
}

/// Ouvre (et crée si besoin) la base, active WAL + clés étrangères, applique les migrations.
pub async fn init(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal) // lecteurs concurrents pendant la synchro
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new().max_connections(5).connect_with(opts).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

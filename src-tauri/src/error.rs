//! Erreur applicative unique. Sérialisable pour traverser la frontière IPC Tauri
//! (le frontend reçoit un simple message texte).

use serde::{Serialize, Serializer};

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("base de données : {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration : {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("E/S : {0}")]
    Io(#[from] std::io::Error),
    #[error("tauri : {0}")]
    Tauri(#[from] tauri::Error),
    #[error("{0}")]
    Other(String),
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

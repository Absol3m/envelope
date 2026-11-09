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
    #[error("IMAP : {0}")]
    Imap(#[from] async_imap::error::Error),
    #[error("SMTP : {0}")]
    Smtp(String),
    #[error("trousseau : {0}")]
    Keyring(#[from] keyring::Error),
    #[error("réseau : {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON : {0}")]
    Json(#[from] serde_json::Error),
    #[error("authentification : {0}")]
    Auth(String),
    #[error("introuvable : {0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

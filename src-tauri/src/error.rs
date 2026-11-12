//! Erreur applicative unique. Sérialisable pour traverser la frontière IPC Tauri.
//!
//! Le front reçoit `{ code, args, message }` : il traduit le **code** (espace `errors`, clé `rust.<code>`) avec ses
//! paramètres, et n'affiche `message` (texte anglais, ou détail technique d'une bibliothèque) qu'à défaut. Aucune phrase
//! destinée à l'utilisateur n'est donc écrite en dur côté Rust : voir [`msg!`].

use std::collections::BTreeMap;

use serde::{Serialize, Serializer};

pub type Result<T> = std::result::Result<T, AppError>;

/// Message d'erreur à traduire par le front : code stable + paramètres, avec un texte anglais de repli (journaux, tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Msg {
    /// Code stable (`lock.wrong_password`) ; vide pour un texte brut venu d'une bibliothèque ou du serveur.
    pub code: &'static str,
    pub args: Vec<(&'static str, String)>,
    /// Texte anglais de repli.
    pub text: String,
}

impl std::fmt::Display for Msg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

/// Texte brut (message d'une bibliothèque ou du serveur distant) : rien à traduire, affiché tel quel.
impl From<String> for Msg {
    fn from(text: String) -> Self {
        Msg {
            code: "",
            args: Vec::new(),
            text,
        }
    }
}
impl From<&str> for Msg {
    fn from(text: &str) -> Self {
        text.to_string().into()
    }
}

/// `msg!("lock.too_many_attempts", "too many attempts: try again in {seconds} s", seconds = n)` : code, texte anglais de
/// repli (les paramètres y sont lus comme dans `format!`) et paramètres transmis au front pour la traduction.
#[macro_export]
macro_rules! msg {
    ($code:literal, $text:literal $(, $k:ident = $v:expr)* $(,)?) => {{
        $(let $k = format!("{}", $v);)*
        $crate::error::Msg {
            code: $code,
            args: vec![$((stringify!($k), $k.clone())),*],
            text: format!($text),
        }
    }};
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("tauri: {0}")]
    Tauri(#[from] tauri::Error),
    #[error("IMAP: {0}")]
    Imap(#[from] async_imap::error::Error),
    #[error("SMTP: {0}")]
    Smtp(Msg),
    #[error("keychain: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("network: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Auth(Msg),
    #[error("{0}")]
    NotFound(Msg),
    #[error("{0}")]
    Invalid(Msg),
}

/// Forme envoyée au front.
#[derive(Serialize)]
struct Wire<'a> {
    code: &'a str,
    args: BTreeMap<&'a str, &'a str>,
    message: String,
}

impl AppError {
    /// Code et paramètres : ceux du message, ou le code de la catégorie avec le détail de la bibliothèque.
    fn parts(&self) -> (&'static str, Vec<(&'static str, String)>) {
        let detail = |code: &'static str, d: String| (code, vec![("detail", d)]);
        match self {
            AppError::Smtp(m)
            | AppError::Auth(m)
            | AppError::NotFound(m)
            | AppError::Invalid(m) => (m.code, m.args.clone()),
            AppError::Database(e) => detail("lib.database", e.to_string()),
            AppError::Migration(e) => detail("lib.migration", e.to_string()),
            AppError::Io(e) => detail("lib.io", e.to_string()),
            AppError::Tauri(e) => detail("lib.tauri", e.to_string()),
            AppError::Imap(e) => detail("lib.imap", e.to_string()),
            AppError::Keyring(e) => detail("lib.keychain", e.to_string()),
            AppError::Http(e) => detail("lib.network", e.to_string()),
            AppError::Json(e) => detail("lib.json", e.to_string()),
        }
    }

    /// Même forme que sur l'IPC, en JSON : pour ce qui est gardé en base ou émis dans un événement (état de synchro…),
    /// que le front traduit à l'affichage.
    pub fn to_wire(&self) -> String {
        let (code, args) = self.parts();
        let wire = Wire {
            code,
            args: args.iter().map(|(k, v)| (*k, v.as_str())).collect(),
            message: self.to_string(),
        };
        serde_json::to_string(&wire).unwrap_or_else(|_| self.to_string())
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let (code, args) = self.parts();
        Wire {
            code,
            args: args.iter().map(|(k, v)| (*k, v.as_str())).collect(),
            message: self.to_string(),
        }
        .serialize(s)
    }
}

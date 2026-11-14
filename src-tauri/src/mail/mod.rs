//! Moteur mail.
//!
//! ```text
//!  account ──▶ secrets (trousseau OS) / oauth (PKCE, refresh)
//!     │
//!  imap  (connexion TLS + login/XOAUTH2)      smtp (envoi lettre)
//!     │
//!  sync  (1 tâche tokio/compte : LIST, fetch incrémental, IDLE)
//!     │
//!  ingest (mail-parser → SQLite, threading, pièces jointes sur disque)
//!     │
//!  search (FTS5) + requêtes de lecture
//! ```

/// Condition SQL des messages qui existent sur le serveur (colonne `uid` de `messages`). Les lignes provisoires de
/// « Envoyés » (`sentcopy`) ont un UID négatif : toute requête qui lit ou pousse des UID vers un serveur IMAP
/// (rapatriement, réconciliation, actions) doit y ajouter ce filtre — `concat!("… WHERE ", server_row!())` — et jamais
/// écrire `uid > 0` à la main. Texte brut : il entre dans des requêtes `&'static str`.
macro_rules! server_row {
    () => {
        "uid > 0"
    };
}

/// Contraire de `server_row!` : lignes provisoires, dont l'UID est l'opposé de l'identifiant de la file `pending_append`.
macro_rules! provisional_row {
    () => {
        "uid < 0"
    };
}

pub mod account;
pub mod actions;
pub mod authres;
pub mod autoconfig;
pub mod drafts;
pub mod export;
pub mod imap;
pub mod ingest;
pub mod local;
pub mod mbox;
pub mod oauth;
pub mod rawstore;
pub mod secrets;
pub mod sentcopy;
pub mod smtp;
pub mod sync;
pub mod unsubscribe;

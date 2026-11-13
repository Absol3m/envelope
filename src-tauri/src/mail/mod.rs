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
pub mod sentcopy;
pub mod smtp;
pub mod sync;
pub mod unsubscribe;

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
pub mod imap;
pub mod ingest;
pub mod oauth;
pub mod search;
pub mod smtp;
pub mod sync;

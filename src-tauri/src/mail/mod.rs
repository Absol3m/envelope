//! Moteur mail (Étape 2) :
//! - `account`  : configuration IMAP/SMTP + OAuth2, secrets dans le trousseau OS
//! - `sync`     : daemon d'arrière-plan (IMAP IDLE, fetch incrémental via UIDVALIDITY/MODSEQ)
//! - `search`   : requêtes FTS5 sur `messages_fts`

pub mod account;
pub mod search;
pub mod sync;

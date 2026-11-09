//! Lecture de la base Envelope : recherche plein texte, dossiers, comptes, conversations.
//! Aucune écriture ici : le schéma et les migrations appartiennent à l'application.

pub mod search;
pub mod thread;

pub use sqlx::Error;
pub type Result<T> = std::result::Result<T, Error>;

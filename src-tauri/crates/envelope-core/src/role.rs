//! Rôle d'un dossier (colonne `folders.role`, contrainte CHECK de la migration 0001) : un seul type pour la base, la
//! détection IMAP, les filtres de la synchro et l'interface. Dans les requêtes, on lie la valeur
//! (`.bind(FolderRole::Sent)`) au lieu d'écrire `'sent'` : un renommage casse alors la compilation, pas la base.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
pub enum FolderRole {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
    All,
    Flagged,
}

impl FolderRole {
    /// Rang d'un dossier sans rôle (ou sans rang propre) dans la liste des dossiers.
    pub const UNRANKED: u8 = 6;

    /// Rang dans la liste des dossiers : boîte de réception, brouillons, envoyés, archives, indésirables, corbeille, puis
    /// les autres.
    pub fn rank(self) -> u8 {
        match self {
            FolderRole::Inbox => 0,
            FolderRole::Drafts => 1,
            FolderRole::Sent => 2,
            FolderRole::Archive => 3,
            FolderRole::Junk => 4,
            FolderRole::Trash => 5,
            FolderRole::All | FolderRole::Flagged => Self::UNRANKED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texte_stocke_et_rang() {
        assert_eq!(
            serde_json::to_string(&FolderRole::Inbox).unwrap(),
            "\"inbox\""
        );
        assert_eq!(
            serde_json::to_string(&FolderRole::Flagged).unwrap(),
            "\"flagged\""
        );
        assert!(FolderRole::Inbox.rank() < FolderRole::Trash.rank());
        assert!(FolderRole::Trash.rank() < FolderRole::UNRANKED);
    }
}

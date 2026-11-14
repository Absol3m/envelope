//! Valeurs fermées des colonnes de `accounts` : fournisseur, mode d'authentification, état de synchro. Un type par
//! colonne, partagé par l'application, la découverte des serveurs et le serveur MCP (mêmes textes en base, vers
//! l'interface et dans les sorties MCP).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Fournisseur d'un compte (`accounts.provider`). `Generic` : tout serveur IMAP/SMTP configuré à la main ou deviné.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
pub enum Provider {
    Gmail,
    Outlook,
    Icloud,
    Yahoo,
    Fastmail,
    Infomaniak,
    Ovh,
    Gandi,
    /// Compte spécial « Sur cet ordinateur » (dossiers locaux, sans serveur).
    Local,
    #[serde(rename = "imap")]
    #[sqlx(rename = "imap")]
    Generic,
}

impl Provider {
    /// Tous les fournisseurs, pour les essais : ce que la base (déclencheurs de la migration 0028) accepte doit
    /// concorder avec l'énumération ; chaque nouveau fournisseur demande une migration qui les recrée, et l'union
    /// `Provider` de `src/lib/api.ts`.
    pub const ALL: [Provider; 10] = [
        Provider::Gmail,
        Provider::Outlook,
        Provider::Icloud,
        Provider::Yahoo,
        Provider::Fastmail,
        Provider::Infomaniak,
        Provider::Ovh,
        Provider::Gandi,
        Provider::Local,
        Provider::Generic,
    ];

    /// Valeur stockée en base ; aussi le nom sous lequel les secrets propres au fournisseur sont rangés (trousseau).
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Gmail => "gmail",
            Provider::Outlook => "outlook",
            Provider::Icloud => "icloud",
            Provider::Yahoo => "yahoo",
            Provider::Fastmail => "fastmail",
            Provider::Infomaniak => "infomaniak",
            Provider::Ovh => "ovh",
            Provider::Gandi => "gandi",
            Provider::Local => "local",
            Provider::Generic => "imap",
        }
    }

    /// Le serveur range lui-même un message envoyé dans « Envoyés » : y déposer une copie ferait un doublon.
    pub fn files_sent_itself(self) -> bool {
        // Exhaustif : un nouveau fournisseur oblige à décider.
        match self {
            Provider::Gmail => true,
            Provider::Outlook
            | Provider::Icloud
            | Provider::Yahoo
            | Provider::Fastmail
            | Provider::Infomaniak
            | Provider::Ovh
            | Provider::Gandi
            | Provider::Local
            | Provider::Generic => false,
        }
    }
}

/// Mode d'authentification (`accounts.auth_kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
pub enum AuthKind {
    Password,
    OAuth2,
}

/// État de la synchro d'un compte (`accounts.sync_status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
pub enum SyncStatus {
    Idle,
    Syncing,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json<T: Serialize>(v: T) -> String {
        serde_json::to_string(&v).unwrap()
    }

    /// `as_str` (base, trousseau) et la forme sérialisée vers l'interface concordent pour chaque fournisseur ; le nom
    /// sous lequel les secrets d'un fournisseur sont rangés (trousseau) ne doit jamais changer.
    #[test]
    fn textes_stockes_inchanges() {
        for p in Provider::ALL {
            assert_eq!(json(p), format!("\"{}\"", p.as_str()));
        }
        assert_eq!(Provider::Generic.as_str(), "imap");
        assert_eq!(Provider::Gmail.as_str(), "gmail");
        assert_eq!(Provider::Outlook.as_str(), "outlook");
        assert_eq!(json(AuthKind::OAuth2), "\"oauth2\"");
        assert_eq!(json(SyncStatus::Syncing), "\"syncing\"");
        assert!(Provider::Gmail.files_sent_itself() && !Provider::Generic.files_sent_itself());
    }
}

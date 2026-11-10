//! Comptes IMAP/SMTP : modèle, découverte des serveurs, persistance et secrets.
//!
//! Les secrets (mot de passe ou jetons OAuth2) vivent dans le trousseau de l'OS,
//! sous le service `dev.envelope.app`, clés `<account_id>:password` / `<account_id>:oauth`.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::error::{AppError, Result};
use crate::mail::oauth::{self, OAuthProvider};

const KEYRING_SERVICE: &str = "dev.envelope.app";
const PASSWORD: &str = "password";
const OAUTH: &str = "oauth";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// TLS implicite (IMAP 993, SMTP 465).
    Tls,
    /// Connexion claire puis STARTTLS (SMTP 587).
    Starttls,
    None,
}

impl Security {
    pub fn as_str(self) -> &'static str {
        match self {
            Security::Tls => "tls",
            Security::Starttls => "starttls",
            Security::None => "none",
        }
    }
    pub fn parse(s: &str) -> Security {
        match s {
            "starttls" => Security::Starttls,
            "none" => Security::None,
            _ => Security::Tls,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
}

/// Compte tel que stocké en base (sans secret).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub provider: String,
    pub auth_kind: String,
    pub username: String,
    pub imap_host: String,
    pub imap_port: i64,
    pub imap_security: String,
    pub smtp_host: String,
    pub smtp_port: i64,
    pub smtp_security: String,
    pub last_sync_at: Option<String>,
    pub sync_status: String,
    pub sync_error: Option<String>,
}

pub const ACCOUNT_COLUMNS: &str = "id, email, display_name, provider, auth_kind, username, \
    imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, \
    last_sync_at, sync_status, sync_error";

/// Identifiants résolus, prêts à être présentés à IMAP/SMTP.
#[derive(Debug, Clone)]
pub enum Credential {
    Password {
        user: String,
        password: String,
    },
    /// Jeton d'accès OAuth2 valide (déjà rafraîchi si nécessaire) — SASL XOAUTH2.
    OAuth {
        user: String,
        access_token: String,
    },
}

/// Entrée de `add_account` (authentification par mot de passe / mot de passe d'application).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAccount {
    pub email: String,
    pub display_name: Option<String>,
    pub username: Option<String>,
    pub password: String,
    pub imap: Option<ServerConfig>,
    pub smtp: Option<ServerConfig>,
}

/// Serveurs détectés pour une adresse (voir `mail::autoconfig`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovered {
    pub provider: String,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
    /// Identifiant de connexion IMAP/SMTP (souvent l'adresse complète).
    pub username: String,
    /// Fournisseur OAuth2 à proposer plutôt qu'un mot de passe.
    pub oauth: Option<OAuthProvider>,
    /// Origine : `builtin`, `autoconfig`, `ispdb`, `mx` ou `guess`.
    pub source: &'static str,
}

fn srv(host: &str, port: u16, security: Security) -> ServerConfig {
    ServerConfig {
        host: host.into(),
        port,
        security,
    }
}

/// Table de fournisseurs connus : (fournisseur, IMAP, SMTP, OAuth2).
pub fn known_provider(
    domain: &str,
) -> Option<(
    &'static str,
    ServerConfig,
    ServerConfig,
    Option<OAuthProvider>,
)> {
    use Security::{Starttls, Tls};
    let g = Some(OAuthProvider::Google);
    let m = Some(OAuthProvider::Microsoft);
    Some(match domain {
        "gmail.com" | "googlemail.com" | "google.com" => (
            "gmail",
            srv("imap.gmail.com", 993, Tls),
            srv("smtp.gmail.com", 465, Tls),
            g,
        ),
        "outlook.com" | "hotmail.com" | "hotmail.fr" | "live.com" | "live.fr" | "msn.com" => (
            "outlook",
            srv("outlook.office365.com", 993, Tls),
            srv("smtp-mail.outlook.com", 587, Starttls),
            m,
        ),
        "icloud.com" | "me.com" | "mac.com" => (
            "icloud",
            srv("imap.mail.me.com", 993, Tls),
            srv("smtp.mail.me.com", 587, Starttls),
            None,
        ),
        "yahoo.com" | "yahoo.fr" => (
            "yahoo",
            srv("imap.mail.yahoo.com", 993, Tls),
            srv("smtp.mail.yahoo.com", 465, Tls),
            None,
        ),
        "fastmail.com" => (
            "fastmail",
            srv("imap.fastmail.com", 993, Tls),
            srv("smtp.fastmail.com", 465, Tls),
            None,
        ),
        "orange.fr" => (
            "imap",
            srv("imap.orange.fr", 993, Tls),
            srv("smtp.orange.fr", 465, Tls),
            None,
        ),
        "free.fr" => (
            "imap",
            srv("imap.free.fr", 993, Tls),
            srv("smtp.free.fr", 465, Tls),
            None,
        ),
        // Hébergeurs courants, absents de l'ISPDB : reconnus aussi via le MX d'un domaine perso.
        "infomaniak.com" | "infomaniak.ch" => (
            "infomaniak",
            srv("mail.infomaniak.com", 993, Tls),
            srv("mail.infomaniak.com", 465, Tls),
            None,
        ),
        "ovh.net" | "ovh.com" => (
            "ovh",
            srv("ssl0.ovh.net", 993, Tls),
            srv("ssl0.ovh.net", 465, Tls),
            None,
        ),
        "gandi.net" => (
            "gandi",
            srv("mail.gandi.net", 993, Tls),
            srv("mail.gandi.net", 465, Tls),
            None,
        ),
        _ => return None,
    })
}

/// Serveurs fixes d'un fournisseur OAuth2.
pub fn oauth_servers(provider: OAuthProvider) -> Result<(ServerConfig, ServerConfig)> {
    let domain = match provider {
        OAuthProvider::Google => "gmail.com",
        OAuthProvider::Microsoft => "outlook.com",
    };
    known_provider(domain)
        .map(|(_, imap, smtp, _)| (imap, smtp))
        .ok_or_else(|| AppError::Invalid(format!("fournisseur sans serveurs connus : {domain}")))
}

/// Dernier recours : `imap.<domaine>` / `smtp.<domaine>`.
pub fn guess(domain: &str, email: &str) -> Discovered {
    Discovered {
        provider: "imap".into(),
        imap: srv(&format!("imap.{domain}"), 993, Security::Tls),
        smtp: srv(&format!("smtp.{domain}"), 465, Security::Tls),
        username: email.into(),
        oauth: None,
        source: "guess",
    }
}

// ---------- Secrets ----------

fn entry(account_id: &str, kind: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(
        KEYRING_SERVICE,
        &format!("{account_id}:{kind}"),
    )?)
}
pub fn store_password(account_id: &str, password: &str) -> Result<()> {
    Ok(entry(account_id, PASSWORD)?.set_password(password)?)
}
fn load_password(account_id: &str) -> Result<String> {
    Ok(entry(account_id, PASSWORD)?.get_password()?)
}
pub fn store_oauth(account_id: &str, tokens: &oauth::OAuthTokens) -> Result<()> {
    Ok(entry(account_id, OAUTH)?.set_password(&serde_json::to_string(tokens)?)?)
}
pub fn load_oauth(account_id: &str) -> Result<oauth::OAuthTokens> {
    Ok(serde_json::from_str(
        &entry(account_id, OAUTH)?.get_password()?,
    )?)
}
/// Identifiants d'application OAuth2 (par fournisseur, partagés par tous les comptes).
fn client_entry(provider: OAuthProvider) -> Result<keyring::Entry> {
    entry("oauth-client", provider.as_str())
}
pub fn store_client(provider: OAuthProvider, creds: &oauth::ClientCreds) -> Result<()> {
    Ok(client_entry(provider)?.set_password(&serde_json::to_string(creds)?)?)
}
pub fn load_client(provider: OAuthProvider) -> Result<Option<oauth::ClientCreds>> {
    match client_entry(provider)?.get_password() {
        Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
pub fn delete_client(provider: OAuthProvider) {
    if let Ok(e) = client_entry(provider) {
        let _ = e.delete_credential();
    }
}
pub fn delete_secrets(account_id: &str) {
    for kind in [PASSWORD, OAUTH] {
        if let Ok(e) = entry(account_id, kind) {
            let _ = e.delete_credential();
        }
    }
}

/// Résout les identifiants d'un compte (rafraîchit le jeton OAuth2 si besoin).
pub async fn resolve_credential(account: &Account) -> Result<Credential> {
    if account.auth_kind == "oauth2" {
        let provider = OAuthProvider::parse(&account.provider).ok_or_else(|| {
            AppError::Auth(format!("fournisseur OAuth2 inconnu : {}", account.provider))
        })?;
        let access_token = oauth::fresh_access_token(provider, &account.id).await?;
        Ok(Credential::OAuth {
            user: account.username.clone(),
            access_token,
        })
    } else {
        let password = load_password(&account.id)?;
        Ok(Credential::Password {
            user: account.username.clone(),
            password,
        })
    }
}

// ---------- Persistance ----------

/// Ligne à insérer dans `accounts`.
pub struct NewAccountRow<'a> {
    pub id: &'a str,
    pub email: &'a str,
    pub display_name: Option<&'a str>,
    pub provider: &'a str,
    pub auth_kind: &'a str,
    pub username: &'a str,
    pub imap: &'a ServerConfig,
    pub smtp: &'a ServerConfig,
}

pub async fn insert(pool: &SqlitePool, r: &NewAccountRow<'_>) -> Result<()> {
    let (imap, smtp) = (r.imap, r.smtp);
    sqlx::query(
        "INSERT INTO accounts (id, email, display_name, provider, auth_kind, username, \
         imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(r.id)
    .bind(r.email)
    .bind(r.display_name)
    .bind(r.provider)
    .bind(r.auth_kind)
    .bind(r.username)
    .bind(&imap.host)
    .bind(imap.port as i64)
    .bind(imap.security.as_str())
    .bind(&smtp.host)
    .bind(smtp.port as i64)
    .bind(smtp.security.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Account>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {ACCOUNT_COLUMNS} FROM accounts ORDER BY created_at"
    ))
    .fetch_all(pool)
    .await?)
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Account> {
    sqlx::query_as(&format!(
        "SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("compte {id}")))
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM accounts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    delete_secrets(id);
    Ok(())
}

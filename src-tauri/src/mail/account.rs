//! Comptes IMAP/SMTP : modèle, découverte des serveurs, persistance et secrets.
//!
//! Les secrets (mot de passe ou jetons OAuth2) vivent dans le trousseau de l'OS,
//! sous le service `dev.envelope.app`, clés `<account_id>:password` / `<account_id>:oauth`.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::error::{AppError, Result};
use crate::mail::oauth::{self, OAuthProvider};

const KEYRING_SERVICE: &str = "dev.envelope.app";

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

#[derive(Debug, Clone, Deserialize)]
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
    Password { user: String, password: String },
    /// Jeton d'accès OAuth2 valide (déjà rafraîchi si nécessaire) — SASL XOAUTH2.
    OAuth { user: String, access_token: String },
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

pub struct Discovered {
    pub provider: &'static str,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
}

impl Discovered {
    pub fn into_servers(self) -> (ServerConfig, ServerConfig) {
        (self.imap, self.smtp)
    }
}

fn srv(host: &str, port: u16, security: Security) -> ServerConfig {
    ServerConfig { host: host.into(), port, security }
}

/// Table de serveurs connus, avec repli `imap.<domaine>` / `smtp.<domaine>`.
pub fn discover(email: &str) -> Result<Discovered> {
    let domain = email
        .rsplit_once('@')
        .map(|(_, d)| d.trim().to_lowercase())
        .filter(|d| d.contains('.'))
        .ok_or_else(|| AppError::Invalid(format!("adresse e-mail invalide : {email}")))?;
    use Security::*;
    let (provider, imap, smtp) = match domain.as_str() {
        "gmail.com" | "googlemail.com" => ("gmail", srv("imap.gmail.com", 993, Tls), srv("smtp.gmail.com", 465, Tls)),
        "outlook.com" | "hotmail.com" | "hotmail.fr" | "live.com" | "live.fr" | "msn.com" => {
            ("outlook", srv("outlook.office365.com", 993, Tls), srv("smtp-mail.outlook.com", 587, Starttls))
        }
        "icloud.com" | "me.com" | "mac.com" => ("icloud", srv("imap.mail.me.com", 993, Tls), srv("smtp.mail.me.com", 587, Starttls)),
        "yahoo.com" | "yahoo.fr" => ("yahoo", srv("imap.mail.yahoo.com", 993, Tls), srv("smtp.mail.yahoo.com", 465, Tls)),
        "fastmail.com" => ("fastmail", srv("imap.fastmail.com", 993, Tls), srv("smtp.fastmail.com", 465, Tls)),
        "orange.fr" => ("imap", srv("imap.orange.fr", 993, Tls), srv("smtp.orange.fr", 465, Tls)),
        "free.fr" => ("imap", srv("imap.free.fr", 993, Tls), srv("smtp.free.fr", 465, Tls)),
        d => ("imap", srv(&format!("imap.{d}"), 993, Tls), srv(&format!("smtp.{d}"), 465, Tls)),
    };
    Ok(Discovered { provider, imap, smtp })
}

// ---------- Secrets ----------

fn entry(account_id: &str, kind: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, &format!("{account_id}:{kind}"))?)
}
pub fn store_password(account_id: &str, password: &str) -> Result<()> {
    Ok(entry(account_id, "password")?.set_password(password)?)
}
pub fn store_oauth(account_id: &str, tokens: &oauth::OAuthTokens) -> Result<()> {
    Ok(entry(account_id, "oauth")?.set_password(&serde_json::to_string(tokens)?)?)
}
pub fn load_oauth(account_id: &str) -> Result<oauth::OAuthTokens> {
    Ok(serde_json::from_str(&entry(account_id, "oauth")?.get_password()?)?)
}
pub fn delete_secrets(account_id: &str) {
    for kind in ["password", "oauth"] {
        if let Ok(e) = entry(account_id, kind) {
            let _ = e.delete_credential();
        }
    }
}

/// Résout les identifiants d'un compte (rafraîchit le jeton OAuth2 si besoin).
pub async fn resolve_credential(account: &Account) -> Result<Credential> {
    if account.auth_kind == "oauth2" {
        let provider = OAuthProvider::parse(&account.provider)
            .ok_or_else(|| AppError::Auth(format!("fournisseur OAuth2 inconnu : {}", account.provider)))?;
        let access_token = oauth::fresh_access_token(provider, &account.id).await?;
        Ok(Credential::OAuth { user: account.username.clone(), access_token })
    } else {
        let password = entry(&account.id, "password")?.get_password()?;
        Ok(Credential::Password { user: account.username.clone(), password })
    }
}

// ---------- Persistance ----------

pub async fn insert(
    pool: &SqlitePool,
    id: &str,
    email: &str,
    display_name: Option<&str>,
    provider: &str,
    auth_kind: &str,
    username: &str,
    imap: &ServerConfig,
    smtp: &ServerConfig,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO accounts (id, email, display_name, provider, auth_kind, username, \
         imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(id)
    .bind(email)
    .bind(display_name)
    .bind(provider)
    .bind(auth_kind)
    .bind(username)
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
    Ok(sqlx::query_as(&format!("SELECT {ACCOUNT_COLUMNS} FROM accounts ORDER BY created_at"))
        .fetch_all(pool)
        .await?)
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Account> {
    sqlx::query_as(&format!("SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("compte {id}")))
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM accounts WHERE id = ?").bind(id).execute(pool).await?;
    delete_secrets(id);
    Ok(())
}

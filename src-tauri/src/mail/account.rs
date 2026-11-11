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

/// Adresse d'envoi supplémentaire d'un compte (alias), avec le nom affiché.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub id: i64,
    pub name: String,
    pub email: String,
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
    pub signature: String,
    /// Alias d'envoi (table `identities`), remplis par `list` et `get`.
    #[sqlx(skip)]
    pub identities: Vec<Identity>,
}

pub const ACCOUNT_COLUMNS: &str = "id, email, display_name, provider, auth_kind, username, \
    imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, \
    last_sync_at, sync_status, sync_error, signature";

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
/// Secret de l'application elle-même (hors compte) dans le trousseau du système ; `kind` en désigne l'usage.
pub fn store_app_secret(kind: &str, value: &str) -> Result<()> {
    Ok(entry("app", kind)?.set_password(value)?)
}
pub fn load_app_secret(kind: &str) -> Result<Option<String>> {
    match entry("app", kind)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
pub fn delete_app_secret(kind: &str) {
    if let Ok(e) = entry("app", kind) {
        let _ = e.delete_credential();
    }
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

/// Accès au trousseau du système depuis du code asynchrone : l'appel est bloquant (jusqu'à plusieurs secondes si le
/// trousseau est verrouillé ou demande une autorisation) et ne doit pas occuper un thread de l'exécuteur.
pub async fn blocking_keychain<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|e| AppError::Auth(format!("accès au trousseau interrompu : {e}")))?
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
        let password = blocking_keychain({
            let id = account.id.clone();
            move || load_password(&id)
        })
        .await?;
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

/// Identifiant du compte spécial « Sur cet ordinateur » (dossiers locaux, sans serveur).
pub const LOCAL_ID: &str = "local";

/// Comptes de messagerie (le compte local n'est pas un compte : ni synchro, ni envoi, ni suppression).
pub async fn list(pool: &SqlitePool) -> Result<Vec<Account>> {
    let mut accounts: Vec<Account> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE provider <> 'local' ORDER BY created_at"
    )))
    .fetch_all(pool)
    .await?;
    let identities: Vec<(String, i64, String, String)> =
        sqlx::query_as("SELECT account_id, id, name, email FROM identities ORDER BY id")
            .fetch_all(pool)
            .await?;
    for (account_id, id, name, email) in identities {
        if let Some(a) = accounts.iter_mut().find(|a| a.id == account_id) {
            a.identities.push(Identity { id, name, email });
        }
    }
    Ok(accounts)
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Account> {
    let mut account: Account = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("compte {id}")))?;
    account.identities =
        sqlx::query_as("SELECT id, name, email FROM identities WHERE account_id = ? ORDER BY id")
            .bind(id)
            .fetch_all(pool)
            .await?;
    Ok(account)
}

/// Ajoute un alias d'envoi ; la même adresse (sans tenir compte de la casse) n'existe qu'une fois par compte.
pub async fn add_identity(
    pool: &SqlitePool,
    account_id: &str,
    name: &str,
    email: &str,
) -> Result<()> {
    let email = email.trim();
    let name = name.trim();
    if name.chars().count() > 100 {
        return Err(AppError::Invalid("nom trop long (100 caractères)".into()));
    }
    // Analyse d'adresse par lettre (bibliothèque éprouvée), pas d'expression maison.
    let parsed: lettre::Address = email
        .parse()
        .map_err(|_| AppError::Invalid(format!("adresse invalide : {email}")))?;
    sqlx::query("INSERT INTO identities (account_id, name, email) VALUES (?, ?, ?)")
        .bind(account_id)
        .bind(name)
        .bind(parsed.to_string())
        .execute(pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => {
                AppError::Invalid("cette adresse existe déjà pour ce compte".into())
            }
            e => e.into(),
        })?;
    Ok(())
}

pub async fn remove_identity(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM identities WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Expéditeur d'un message : le compte lui-même, ou l'un de ses alias (jamais une adresse libre).
/// Renvoie une `Mailbox` : lettre encode correctement un nom contenant des caractères spéciaux.
pub fn sender(account: &Account, from_email: Option<&str>) -> Result<lettre::message::Mailbox> {
    let own = account.display_name.clone().filter(|n| !n.is_empty());
    let (name, email) = match from_email
        .map(str::trim)
        .filter(|e| !e.eq_ignore_ascii_case(&account.email))
    {
        None => (own, account.email.as_str()),
        Some(e) => {
            let id = account
                .identities
                .iter()
                .find(|i| i.email.eq_ignore_ascii_case(e))
                .ok_or_else(|| AppError::Invalid(format!("adresse d'expéditeur inconnue : {e}")))?;
            (
                Some(id.name.clone()).filter(|n| !n.is_empty()).or(own),
                id.email.as_str(),
            )
        }
    };
    let address = email
        .parse()
        .map_err(|_| AppError::Invalid(format!("adresse invalide : {email}")))?;
    Ok(lettre::message::Mailbox::new(name, address))
}

pub async fn set_signature(pool: &SqlitePool, id: &str, signature: &str) -> Result<()> {
    let done = sqlx::query("UPDATE accounts SET signature = ? WHERE id = ?")
        .bind(signature)
        .bind(id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("compte {id}")));
    }
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    if id == LOCAL_ID {
        return Err(AppError::Invalid(
            "le stockage local ne peut pas être supprimé : il contient des mails archivés".into(),
        ));
    }
    sqlx::query("DELETE FROM accounts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    let id = id.to_string();
    blocking_keychain(move || {
        delete_secrets(&id);
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool_with_account() -> SqlitePool {
        let pool = crate::db::init_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host,display_name) VALUES ('a','me@x.fr','a','i','s','Moi')")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn alias_ajoute_liste_refuse_en_double_et_supprime_avec_le_compte() {
        let pool = pool_with_account().await;
        add_identity(&pool, "a", " Pro ", "contact@x.fr")
            .await
            .unwrap();
        assert!(
            add_identity(&pool, "a", "", "CONTACT@x.fr").await.is_err(),
            "doublon (casse ignorée)"
        );
        assert!(add_identity(&pool, "a", "", "pas une adresse")
            .await
            .is_err());
        let acct = get(&pool, "a").await.unwrap();
        assert_eq!(acct.identities.len(), 1);
        assert_eq!(
            (
                acct.identities[0].name.as_str(),
                acct.identities[0].email.as_str()
            ),
            ("Pro", "contact@x.fr")
        );
        assert_eq!(list(&pool).await.unwrap()[0].identities.len(), 1);
        remove_identity(&pool, acct.identities[0].id).await.unwrap();
        assert!(get(&pool, "a").await.unwrap().identities.is_empty());
        add_identity(&pool, "a", "", "autre@x.fr").await.unwrap();
        sqlx::query("DELETE FROM accounts WHERE id = 'a'")
            .execute(&pool)
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM identities")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn expediteur_limite_au_compte_et_a_ses_alias() {
        let pool = pool_with_account().await;
        add_identity(&pool, "a", "Pro", "contact@x.fr")
            .await
            .unwrap();
        add_identity(&pool, "a", "", "sans-nom@x.fr").await.unwrap();
        let acct = get(&pool, "a").await.unwrap();
        let show = |m: lettre::message::Mailbox| (m.name, m.email.to_string());
        let moi = (Some("Moi".to_string()), "me@x.fr".to_string());
        assert_eq!(show(sender(&acct, None).unwrap()), moi);
        assert_eq!(show(sender(&acct, Some("ME@x.fr")).unwrap()), moi);
        assert_eq!(
            show(sender(&acct, Some("Contact@x.fr")).unwrap()),
            (Some("Pro".to_string()), "contact@x.fr".to_string())
        );
        // Alias sans nom : celui du compte.
        assert_eq!(
            sender(&acct, Some("sans-nom@x.fr")).unwrap().name,
            Some("Moi".to_string())
        );
        assert!(sender(&acct, Some("usurpe@autre.fr")).is_err());
    }

    /// Un accès bloquant au trousseau ne doit pas figer l'exécuteur : sur un exécuteur à UN seul fil, une tâche
    /// concurrente continue d'avancer pendant que « le trousseau » dort.
    #[tokio::test(flavor = "current_thread")]
    async fn acces_trousseau_ne_bloque_pas_l_executeur() {
        let ticks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = ticks.clone();
        let ticker = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });
        let value = blocking_keychain(|| {
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok(42)
        })
        .await
        .unwrap();
        ticker.abort();
        assert_eq!(value, 42);
        assert!(
            ticks.load(std::sync::atomic::Ordering::SeqCst) >= 10,
            "l'exécuteur a continué à tourner : {} battements",
            ticks.load(std::sync::atomic::Ordering::SeqCst)
        );
        // Une erreur du trousseau remonte telle quelle.
        let err = blocking_keychain::<()>(|| Err(AppError::Auth("refusé".into())))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("refusé"));
    }
}

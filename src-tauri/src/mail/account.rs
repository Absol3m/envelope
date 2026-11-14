//! Comptes IMAP/SMTP : modèle, découverte des serveurs, persistance et secrets.
//!
//! Les secrets (mot de passe ou jetons OAuth2) vivent dans le trousseau de l'OS,
//! sous le service `dev.envelope.app`, clés `<account_id>:password` / `<account_id>:oauth`.

use envelope_core::kinds::{AuthKind, Provider, SyncStatus};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use ts_rs::TS;

use crate::error::{AppError, Result};
use crate::mail::oauth::{self, OAuthProvider};
use crate::mail::secrets::{self, Secret};
use crate::mail::{imap, sessions, smtp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
pub enum Security {
    /// TLS implicite (IMAP 993, SMTP 465).
    Tls,
    /// Connexion claire puis STARTTLS (SMTP 587).
    Starttls,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
}

/// Adresse d'envoi supplémentaire d'un compte (alias), avec le nom affiché.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub id: i64,
    pub name: String,
    pub email: String,
    /// Vide : la signature du compte s'applique.
    pub signature: String,
}

/// Alias avec le compte auquel il appartient (lecture groupée de `list`).
#[derive(sqlx::FromRow)]
struct IdentityRow {
    account_id: String,
    #[sqlx(flatten)]
    identity: Identity,
}

/// Compte tel que stocké en base (sans secret).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub provider: Provider,
    pub auth_kind: AuthKind,
    pub username: String,
    pub imap_host: String,
    pub imap_port: i64,
    pub imap_security: Security,
    pub smtp_host: String,
    pub smtp_port: i64,
    pub smtp_security: Security,
    pub last_sync_at: Option<String>,
    pub sync_status: SyncStatus,
    pub sync_error: Option<String>,
    pub signature: String,
    /// Déposer une copie des messages envoyés dans « Envoyés » (réglage du compte ; voir `keeps_sent_copy`).
    pub copy_sent: bool,
    /// Copies d'envoyés en attente de dépôt sur le serveur (file `pending_append`), remplies par `list`.
    #[sqlx(skip)]
    pub pending_copies: i64,
    /// Dernière erreur d'une copie en attente (codée, voir `AppError::to_wire`).
    #[sqlx(skip)]
    pub pending_copy_error: Option<String>,
    /// Alias d'envoi (table `identities`), remplis par `list` et `get`.
    #[sqlx(skip)]
    pub identities: Vec<Identity>,
}

impl Account {
    /// Le serveur range lui-même un message envoyé dans « Envoyés » (Gmail) : y déposer une copie ferait un doublon.
    pub fn server_files_sent(&self) -> bool {
        self.provider.files_sent_itself()
    }

    /// Faut-il déposer nous-mêmes une copie de chaque message envoyé : voulu par l'utilisateur, et le serveur ne le fait pas.
    pub fn keeps_sent_copy(&self) -> bool {
        self.copy_sent && !self.server_files_sent()
    }
}

#[cfg(test)]
impl Account {
    /// Compte de test : mot de passe, serveurs `i` / `s` en TLS, aucun alias ; les essais ajustent ce qui compte
    /// (`Account { imap_port: 143, ..Account::fixture() }`).
    pub fn fixture() -> Account {
        Account {
            id: "a".into(),
            email: "me@x.fr".into(),
            display_name: Some("Moi".into()),
            provider: Provider::Generic,
            auth_kind: AuthKind::Password,
            username: "me".into(),
            imap_host: "i".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "s".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            last_sync_at: None,
            sync_status: SyncStatus::Idle,
            sync_error: None,
            signature: String::new(),
            copy_sent: true,
            pending_copies: 0,
            pending_copy_error: None,
            identities: vec![],
        }
    }
}

pub const ACCOUNT_COLUMNS: &str = "id, email, display_name, provider, auth_kind, username, \
    imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, \
    last_sync_at, sync_status, sync_error, signature, copy_sent";

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
#[derive(Debug, Deserialize, TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase")]
pub struct NewAccount {
    pub email: String,
    pub display_name: Option<String>,
    pub username: Option<String>,
    pub password: String,
    pub imap: Option<ServerConfig>,
    pub smtp: Option<ServerConfig>,
}

/// Origine d'une détection de serveurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum DiscoverySource {
    Builtin,
    Autoconfig,
    Ispdb,
    Mx,
    Guess,
}

/// Serveurs détectés pour une adresse (voir `mail::autoconfig`).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, rename = "Detected")]
#[serde(rename_all = "camelCase")]
pub struct Discovered {
    pub provider: Provider,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
    /// Identifiant de connexion IMAP/SMTP (souvent l'adresse complète).
    pub username: String,
    /// Fournisseur OAuth2 à proposer plutôt qu'un mot de passe.
    pub oauth: Option<OAuthProvider>,
    /// Origine : `builtin`, `autoconfig`, `ispdb`, `mx` ou `guess`.
    pub source: DiscoverySource,
}

fn srv(host: &str, port: u16, security: Security) -> ServerConfig {
    ServerConfig {
        host: host.into(),
        port,
        security,
    }
}

/// Table de fournisseurs connus : (fournisseur, IMAP, SMTP). L'OAuth2 à proposer se déduit du fournisseur
/// (`OAuthProvider::of`).
pub fn known_provider(domain: &str) -> Option<(Provider, ServerConfig, ServerConfig)> {
    use Security::{Starttls, Tls};
    Some(match domain {
        "gmail.com" | "googlemail.com" | "google.com" => (
            Provider::Gmail,
            srv("imap.gmail.com", 993, Tls),
            srv("smtp.gmail.com", 465, Tls),
        ),
        "outlook.com" | "hotmail.com" | "hotmail.fr" | "live.com" | "live.fr" | "msn.com" => (
            Provider::Outlook,
            srv("outlook.office365.com", 993, Tls),
            srv("smtp-mail.outlook.com", 587, Starttls),
        ),
        "icloud.com" | "me.com" | "mac.com" => (
            Provider::Icloud,
            srv("imap.mail.me.com", 993, Tls),
            srv("smtp.mail.me.com", 587, Starttls),
        ),
        "yahoo.com" | "yahoo.fr" => (
            Provider::Yahoo,
            srv("imap.mail.yahoo.com", 993, Tls),
            srv("smtp.mail.yahoo.com", 465, Tls),
        ),
        "fastmail.com" => (
            Provider::Fastmail,
            srv("imap.fastmail.com", 993, Tls),
            srv("smtp.fastmail.com", 465, Tls),
        ),
        "orange.fr" => (
            Provider::Generic,
            srv("imap.orange.fr", 993, Tls),
            srv("smtp.orange.fr", 465, Tls),
        ),
        "free.fr" => (
            Provider::Generic,
            srv("imap.free.fr", 993, Tls),
            srv("smtp.free.fr", 465, Tls),
        ),
        // Hébergeurs courants, absents de l'ISPDB : reconnus aussi via le MX d'un domaine perso.
        "infomaniak.com" | "infomaniak.ch" => (
            Provider::Infomaniak,
            srv("mail.infomaniak.com", 993, Tls),
            srv("mail.infomaniak.com", 465, Tls),
        ),
        "ovh.net" | "ovh.com" => (
            Provider::Ovh,
            srv("ssl0.ovh.net", 993, Tls),
            srv("ssl0.ovh.net", 465, Tls),
        ),
        "gandi.net" => (
            Provider::Gandi,
            srv("mail.gandi.net", 993, Tls),
            srv("mail.gandi.net", 465, Tls),
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
        .map(|(_, imap, smtp)| (imap, smtp))
        .ok_or_else(|| {
            AppError::Invalid(msg!(
                "account.no_known_servers",
                "no known servers for provider: {domain}",
                domain = domain
            ))
        })
}

/// Dernier recours : `imap.<domaine>` / `smtp.<domaine>`.
pub fn guess(domain: &str, email: &str) -> Discovered {
    Discovered {
        provider: Provider::Generic,
        imap: srv(&format!("imap.{domain}"), 993, Security::Tls),
        smtp: srv(&format!("smtp.{domain}"), 465, Security::Tls),
        username: email.into(),
        oauth: None,
        source: DiscoverySource::Guess,
    }
}

/// Résout les identifiants d'un compte (rafraîchit le jeton OAuth2 si besoin).
pub async fn resolve_credential(account: &Account) -> Result<Credential> {
    if account.auth_kind == AuthKind::OAuth2 {
        let provider = OAuthProvider::of(account.provider).ok_or_else(|| {
            AppError::Auth(msg!(
                "oauth.unknown_provider",
                "unknown OAuth2 provider: {provider}",
                provider = account.provider.as_str()
            ))
        })?;
        let access_token = oauth::fresh_access_token(provider, &account.id).await?;
        Ok(Credential::OAuth {
            user: account.username.clone(),
            access_token,
        })
    } else {
        let password = secrets::password(&account.id).await?;
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
    pub provider: Provider,
    pub auth_kind: AuthKind,
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
    .bind(imap.security)
    .bind(&smtp.host)
    .bind(smtp.port as i64)
    .bind(smtp.security)
    .execute(pool)
    .await?;
    Ok(())
}

/// État de synchro d'un compte (et dernière erreur, codée) ; `Idle` date aussi la dernière synchro réussie.
pub async fn set_sync_state(
    pool: &SqlitePool,
    id: &str,
    status: SyncStatus,
    error: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE accounts SET sync_status = ?1, sync_error = ?2, \
         last_sync_at = CASE WHEN ?1 = 'idle' THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE last_sync_at END \
         WHERE id = ?3",
    )
    .bind(status)
    .bind(error)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Compte à créer : identité et serveurs déjà résolus par l'appelant (formulaire, découverte, fournisseur OAuth2).
pub struct NewAccountSpec<'a> {
    pub email: &'a str,
    pub display_name: Option<&'a str>,
    pub provider: Provider,
    pub username: &'a str,
    pub imap: &'a ServerConfig,
    pub smtp: &'a ServerConfig,
    /// Mot de passe ou jetons OAuth2 ; le mode d'authentification en découle.
    pub secret: Secret,
}

/// Crée un compte : ligne en base, secret dans le trousseau, puis **essai IMAP + SMTP** avec les identifiants réels
/// (hôte, TLS, authentification) — pour les deux modes d'authentification. À la moindre erreur, tout est annulé (ni
/// compte orphelin ni secret oublié). La synchro, elle, est démarrée par l'appelant.
pub async fn create(pool: &SqlitePool, spec: NewAccountSpec<'_>) -> Result<Account> {
    let id = uuid::Uuid::new_v4().to_string();
    // Identifiants de l'essai : ceux qu'on vient de recevoir, sans relire le trousseau.
    let cred = match &spec.secret {
        Secret::Password(password) => Credential::Password {
            user: spec.username.to_string(),
            password: password.clone(),
        },
        Secret::OAuth(tokens) => Credential::OAuth {
            user: spec.username.to_string(),
            access_token: tokens.access_token.clone(),
        },
    };
    insert(
        pool,
        &NewAccountRow {
            id: &id,
            email: spec.email,
            display_name: spec.display_name,
            provider: spec.provider,
            auth_kind: spec.secret.auth_kind(),
            username: spec.username,
            imap: spec.imap,
            smtp: spec.smtp,
        },
    )
    .await?;
    let verified = async {
        secrets::store(&id, spec.secret).await?;
        let acct = get(pool, &id).await?;
        // Les deux serveurs sont indépendants : essais en parallèle.
        let imap_check = async {
            let mut session = imap::connect(&acct, &cred).await?; // hôte, TLS, authentification
            let _ = session.logout().await;
            Ok::<_, AppError>(())
        };
        tokio::try_join!(imap_check, smtp::test(&acct, &cred))?;
        Ok::<_, AppError>(acct)
    }
    .await;
    if verified.is_err() {
        let _ = remove(pool, &id).await; // l'erreur d'origine prime sur celle du nettoyage
    }
    verified
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
    let identities: Vec<IdentityRow> =
        sqlx::query_as("SELECT account_id, id, name, email, signature FROM identities ORDER BY id")
            .fetch_all(pool)
            .await?;
    for IdentityRow {
        account_id,
        identity,
    } in identities
    {
        if let Some(a) = accounts.iter_mut().find(|a| a.id == account_id) {
            a.identities.push(identity);
        }
    }
    let pending: Vec<(String, i64, Option<String>)> = sqlx::query_as(
        "SELECT account_id, COUNT(*), (SELECT last_error FROM pending_append q WHERE q.account_id = p.account_id \
         AND q.last_error IS NOT NULL ORDER BY q.id DESC LIMIT 1) FROM pending_append p GROUP BY account_id",
    )
    .fetch_all(pool)
    .await?;
    for (account_id, count, error) in pending {
        if let Some(a) = accounts.iter_mut().find(|a| a.id == account_id) {
            a.pending_copies = count;
            a.pending_copy_error = error;
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
    .ok_or_else(|| {
        AppError::NotFound(msg!(
            "not_found.account",
            "account not found: {id}",
            id = id
        ))
    })?;
    account.identities = sqlx::query_as(
        "SELECT id, name, email, signature FROM identities WHERE account_id = ? ORDER BY id",
    )
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
        return Err(AppError::Invalid(msg!(
            "identity.name_too_long",
            "name too long (100 characters)"
        )));
    }
    // Analyse d'adresse par lettre (bibliothèque éprouvée), pas d'expression maison.
    let parsed: lettre::Address = email.parse().map_err(|_| {
        AppError::Invalid(msg!(
            "address.invalid",
            "invalid address: {email}",
            email = email
        ))
    })?;
    sqlx::query("INSERT INTO identities (account_id, name, email) VALUES (?, ?, ?)")
        .bind(account_id)
        .bind(name)
        .bind(parsed.to_string())
        .execute(pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => AppError::Invalid(msg!(
                "identity.exists",
                "this address already exists for this account"
            )),
            e => e.into(),
        })?;
    Ok(())
}

/// Retire un alias du compte indiqué (l'identifiant seul ne suffit pas : un alias d'un autre compte n'est pas touché).
pub async fn remove_identity(pool: &SqlitePool, account_id: &str, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM identities WHERE id = ? AND account_id = ?")
        .bind(id)
        .bind(account_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Signature (texte brut) d'un alias ; vide, celle du compte s'applique.
pub async fn set_identity_signature(
    pool: &SqlitePool,
    account_id: &str,
    id: i64,
    signature: &str,
) -> Result<()> {
    let done = sqlx::query("UPDATE identities SET signature = ? WHERE id = ? AND account_id = ?")
        .bind(signature)
        .bind(id)
        .bind(account_id)
        .execute(pool)
        .await?;
    ensure_found(&done, account_id)
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
                .ok_or_else(|| {
                    AppError::Invalid(msg!(
                        "identity.unknown_sender",
                        "unknown sender address: {address}",
                        address = e
                    ))
                })?;
            (
                Some(id.name.clone()).filter(|n| !n.is_empty()).or(own),
                id.email.as_str(),
            )
        }
    };
    let address = email.parse().map_err(|_| {
        AppError::Invalid(msg!(
            "address.invalid",
            "invalid address: {email}",
            email = email
        ))
    })?;
    Ok(lettre::message::Mailbox::new(name, address))
}

/// Une mise à jour qui n'a touché aucun compte : le compte n'existe pas.
fn ensure_found(done: &sqlx::sqlite::SqliteQueryResult, id: &str) -> Result<()> {
    if done.rows_affected() == 0 {
        return Err(AppError::NotFound(msg!(
            "not_found.account",
            "account not found: {id}",
            id = id
        )));
    }
    Ok(())
}

pub async fn set_signature(pool: &SqlitePool, id: &str, signature: &str) -> Result<()> {
    let done = sqlx::query("UPDATE accounts SET signature = ? WHERE id = ?")
        .bind(signature)
        .bind(id)
        .execute(pool)
        .await?;
    ensure_found(&done, id)
}

/// Active ou désactive la copie dans « Envoyés » ; désactivée, les copies encore en attente sont abandonnées.
pub async fn set_copy_sent(pool: &SqlitePool, id: &str, on: bool) -> Result<()> {
    let done = sqlx::query("UPDATE accounts SET copy_sent = ? WHERE id = ?")
        .bind(on)
        .bind(id)
        .execute(pool)
        .await?;
    ensure_found(&done, id)?;
    if !on {
        sqlx::query("DELETE FROM pending_append WHERE account_id = ?")
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Supprime le compte de la base seulement (ses secrets partent par `secrets::delete` : voir `remove`).
pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    if id == LOCAL_ID {
        return Err(AppError::Invalid(msg!(
            "local.cannot_delete",
            "local storage can't be deleted: it contains archived mail"
        )));
    }
    sqlx::query("DELETE FROM accounts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Supprime un compte : sa ligne (et ce qui en dépend) puis ses secrets du trousseau.
pub async fn remove(pool: &SqlitePool, id: &str) -> Result<()> {
    delete(pool, id).await?;
    sessions::forget(id);
    secrets::delete(id).await
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

    /// Les colonnes fermées se relisent en types forts (compte local compris) ; une valeur inconnue est refusée à
    /// l'écriture (déclencheurs de la migration 0028) au lieu de casser la relecture.
    #[tokio::test]
    async fn colonnes_fermees_typees_et_protegees() {
        let pool = crate::db::init_memory().await.unwrap();
        let (imap, smtp) = oauth_servers(OAuthProvider::Google).unwrap();
        insert(
            &pool,
            &NewAccountRow {
                id: "g",
                email: "me@gmail.com",
                display_name: None,
                provider: Provider::Gmail,
                auth_kind: AuthKind::OAuth2,
                username: "me@gmail.com",
                imap: &imap,
                smtp: &smtp,
            },
        )
        .await
        .unwrap();
        let g = get(&pool, "g").await.unwrap();
        assert_eq!(
            (
                g.provider,
                g.auth_kind,
                g.sync_status,
                g.imap_security,
                g.smtp_security
            ),
            (
                Provider::Gmail,
                AuthKind::OAuth2,
                SyncStatus::Idle,
                Security::Tls,
                Security::Tls
            )
        );
        assert!(g.server_files_sent() && !g.keeps_sent_copy());
        assert_eq!(
            get(&pool, LOCAL_ID).await.unwrap().provider,
            Provider::Local
        );

        set_sync_state(&pool, "g", SyncStatus::Error, Some("boum"))
            .await
            .unwrap();
        let g = get(&pool, "g").await.unwrap();
        assert_eq!(
            (g.sync_status, g.sync_error.as_deref(), g.last_sync_at),
            (SyncStatus::Error, Some("boum"), None)
        );
        set_sync_state(&pool, "g", SyncStatus::Idle, None)
            .await
            .unwrap();
        assert!(
            get(&pool, "g").await.unwrap().last_sync_at.is_some(),
            "Idle date la synchro réussie"
        );

        // Tout fournisseur connu est accepté par la base (concordance énumération / déclencheurs).
        for p in Provider::ALL {
            sqlx::query("UPDATE accounts SET provider = ? WHERE id = 'g'")
                .bind(p)
                .execute(&pool)
                .await
                .unwrap_or_else(|e| panic!("{p:?}: {e}"));
        }
        assert_eq!(
            OAuthProvider::Google.provider().as_str(),
            "gmail",
            "nom d'entrée du trousseau"
        );
        assert_eq!(OAuthProvider::Microsoft.provider().as_str(), "outlook");
        for sql in [
            "UPDATE accounts SET provider = 'telepathie' WHERE id = 'g'",
            "UPDATE accounts SET sync_status = 'perdu' WHERE id = 'g'",
            "INSERT INTO accounts (id,email,username,imap_host,smtp_host,provider) VALUES ('x','x@x.fr','x','i','s','telepathie')",
        ] {
            assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{sql}");
        }
    }

    /// Une création qui échoue (ici : serveurs injoignables ; sans trousseau, elle échoue plus tôt, avec le même résultat)
    /// ne laisse aucun compte, pour les deux modes d'authentification.
    #[tokio::test]
    async fn creation_echouee_annulee_pour_les_deux_modes() {
        let pool = crate::db::init_memory().await.unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // plus personne n'écoute
        let server = ServerConfig {
            host: "127.0.0.1".into(),
            port,
            security: Security::Starttls,
        };
        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        for secret in [
            Secret::Password("x".into()),
            Secret::OAuth(oauth::OAuthTokens {
                access_token: "t".into(),
                refresh_token: None,
                expires_at: i64::MAX,
            }),
        ] {
            let spec = NewAccountSpec {
                email: "me@x.fr",
                display_name: None,
                provider: Provider::Generic,
                username: "me",
                imap: &server,
                smtp: &server,
                secret,
            };
            assert!(create(&pool, spec).await.is_err());
        }
        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(before, after, "aucun compte orphelin");
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
        // Un alias ne se retire que par son propre compte.
        remove_identity(&pool, "autre", acct.identities[0].id)
            .await
            .unwrap();
        assert_eq!(get(&pool, "a").await.unwrap().identities.len(), 1);
        set_identity_signature(&pool, "a", acct.identities[0].id, "Pro\nContact")
            .await
            .unwrap();
        assert_eq!(
            get(&pool, "a").await.unwrap().identities[0].signature,
            "Pro\nContact"
        );
        assert!(
            set_identity_signature(&pool, "autre", acct.identities[0].id, "x")
                .await
                .is_err()
        );
        remove_identity(&pool, "a", acct.identities[0].id)
            .await
            .unwrap();
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
}

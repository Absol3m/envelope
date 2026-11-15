//! Commandes Tauri exposées au frontend (`invoke`).

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;
use ts_rs::TS;

use crate::error::{AppError, Result};
use crate::mail::account::{self, Account, NewAccount};
use crate::mail::actions::{self, Action, ServerDestination};
use crate::mail::autoconfig;
use crate::mail::drafts;
use crate::mail::export;
use crate::mail::local;
use crate::mail::mbox;
use crate::mail::oauth::{self, OAuthProvider};
use crate::mail::secrets::{self, Secret};
use crate::mail::sentcopy;
use crate::mail::sessions;
use crate::mail::smtp::{self, OutgoingMessage};
use crate::mail::unsubscribe;
use crate::picks;
use crate::reminders;
use crate::rules;
use crate::scheduled;
use crate::templates::{self, Template};
use crate::AppState;
use envelope_core::addr::Addr;
use envelope_core::contacts;
use envelope_core::page::Page;
use envelope_core::search::{self, FolderRow, MessageSummary};
use envelope_core::thread::{self, ThreadDetail, ThreadQuery, ThreadSummary};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    name: &'static str,
    version: &'static str,
    db_path: String,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        db_path: state.db_path.display().to_string(),
    }
}

// ---------- Verrouillage de l'application ----------

#[tauri::command]
pub fn lock_status(lock: State<'_, crate::lock::Lock>) -> crate::lock::Status {
    lock.status()
}

#[tauri::command]
pub async fn unlock_password(lock: State<'_, crate::lock::Lock>, password: String) -> Result<()> {
    lock.unlock(crate::lock::Method::Password, Some(password))
        .await
}

/// Ouvre l'invite d'authentification du système (Touch ID, Windows Hello) ; rend la main quand elle est close.
#[tauri::command]
pub async fn unlock_system(lock: State<'_, crate::lock::Lock>) -> Result<()> {
    lock.unlock(crate::lock::Method::System, None).await
}

/// Secours quand le déverrouillage est impossible (mot de passe oublié, trousseau réinitialisé, Windows Hello retiré) :
/// efface **toutes** les données locales (comptes, messages bruts, pièces jointes, stockage local, modèles, réglages) et
/// le verrouillage. Les e-mails restent sur les serveurs. Réservé à l'état verrouillé ; l'interface demande confirmation.
#[tauri::command]
pub async fn reset_app(
    state: State<'_, AppState>,
    lock: State<'_, crate::lock::Lock>,
) -> Result<()> {
    if !lock.is_locked() {
        return Err(AppError::Invalid(msg!(
            "lock.not_locked",
            "the application is not locked"
        )));
    }
    for a in account::list(&state.db).await? {
        state.sync.stop(&a.id);
        sessions::forget(&a.id);
        secrets::delete(&a.id).await?;
    }
    // Les identifiants d'application OAuth2 (client_id / secret saisis par l'utilisateur) sont aussi des secrets.
    for p in [OAuthProvider::Google, OAuthProvider::Microsoft] {
        secrets::delete_client(p).await?;
    }
    crate::pgp_store::delete_all_secrets(&state.db).await?;
    lock.reset(&state.db).await?;
    for dir in [&state.raw_dir, &state.attachments_dir] {
        let _ = tokio::fs::remove_dir_all(dir).await;
        let _ = crate::private::create_dir_all(dir).await;
    }
    Ok(())
}

#[tauri::command]
pub fn lock_now(lock: State<'_, crate::lock::Lock>) -> Result<()> {
    lock.lock_now()
}

/// Configure le verrouillage. `current_password` : ancien mot de passe, exigé pour modifier un verrouillage
/// par mot de passe déjà actif (par le système, c'est l'invite native qui s'en charge).
#[tauri::command]
pub async fn set_lock(
    state: State<'_, AppState>,
    lock: State<'_, crate::lock::Lock>,
    method: crate::lock::Method,
    timeout_secs: u32,
    password: Option<String>,
    current_password: Option<String>,
) -> Result<crate::lock::Status> {
    if timeout_secs > 24 * 3600 {
        return Err(AppError::Invalid(msg!(
            "lock.timeout_too_long",
            "timeout too long (24 h at most)"
        )));
    }
    let next = crate::lock::Config {
        method,
        timeout_secs,
    };
    lock.configure(&state.db, next, password, current_password)
        .await?;
    Ok(lock.status())
}

/// Chiffrement du disque qui contient les données (FileVault, BitLocker, LUKS) : `on`, `off` ou `unknown`.
#[tauri::command]
pub async fn disk_encryption(
    state: State<'_, AppState>,
) -> Result<crate::diskcrypt::DiskEncryption> {
    let dir = state.db_path.parent().unwrap_or(&state.db_path);
    Ok(crate::diskcrypt::status(dir).await)
}

/// Couleurs du système (clair/sombre, accent, palette du bureau), relues à chaque retour au premier plan.
#[tauri::command]
pub async fn system_appearance() -> envelope_appearance::Appearance {
    envelope_appearance::read().await
}

#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>> {
    account::list(&state.db).await
}

/// Ajoute un compte (mot de passe / mot de passe d'application) : découverte des serveurs, création
/// (`account::create` : essai IMAP + SMTP, annulation en cas d'échec), démarrage de la synchro.
#[tauri::command]
pub async fn add_account(state: State<'_, AppState>, input: NewAccount) -> Result<Account> {
    let email = input.email.trim().to_string();
    let found = autoconfig::detect(&email).await?;
    let imap = input.imap.unwrap_or(found.imap);
    let smtp = input.smtp.unwrap_or(found.smtp);
    let username = input
        .username
        .filter(|u| !u.is_empty())
        .unwrap_or(found.username);
    let created = account::create(
        &state.db,
        account::NewAccountSpec {
            email: &email,
            display_name: input.display_name.as_deref(),
            provider: found.provider,
            username: &username,
            imap: &imap,
            smtp: &smtp,
            secret: Secret::Password(input.password),
        },
    )
    .await?;
    state.sync.start(created.id.clone());
    Ok(created)
}

/// Autorise dans le navigateur système et vérifie que le compte choisi est bien `email`.
async fn authorize_in_browser(
    app: &AppHandle,
    provider: OAuthProvider,
    email: &str,
) -> Result<oauth::OAuthTokens> {
    let auth = oauth::authorize(provider, |url| {
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|e| AppError::Auth((e.to_string()).into()))
    })
    .await?;
    oauth::check_authorized_email(auth.email.as_deref(), email)?;
    Ok(auth.tokens)
}

/// Renouvelle l'autorisation d'un compte OAuth2 dont le jeton a expiré ou été révoqué.
#[tauri::command]
pub async fn reconnect_oauth_account(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<()> {
    let acc = account::get(&state.db, &id).await?;
    let provider = OAuthProvider::of(acc.provider).ok_or_else(|| {
        AppError::Invalid(msg!("account.not_oauth", "this account doesn't use OAuth2"))
    })?;
    let tokens = authorize_in_browser(&app, provider, &acc.email).await?;
    secrets::store(&id, Secret::OAuth(tokens)).await?;
    sessions::forget(&id); // la session gardée date de l'ancienne autorisation
    state.sync.trigger(&id);
    Ok(())
}

/// Ajoute un compte Gmail / Outlook via OAuth2 (ouvre le navigateur système).
#[tauri::command]
pub async fn add_oauth_account(
    app: AppHandle,
    state: State<'_, AppState>,
    provider: OAuthProvider,
    email: String,
    display_name: Option<String>,
) -> Result<Account> {
    let email = email.trim().to_string();
    let tokens = authorize_in_browser(&app, provider, &email).await?;
    // Serveurs fixes du fournisseur choisi (le domaine de l'e-mail peut être personnalisé).
    let (imap, smtp) = account::oauth_servers(provider)?;
    let created = account::create(
        &state.db,
        account::NewAccountSpec {
            email: &email,
            display_name: display_name.as_deref(),
            provider: provider.provider(),
            username: &email,
            imap: &imap,
            smtp: &smtp,
            secret: Secret::OAuth(tokens),
        },
    )
    .await?;
    state.sync.start(created.id.clone());
    Ok(created)
}

/// Ouvre une adresse dans le navigateur ou le client mail du système (jamais dans l'application). C'est la seule voie :
/// la webview n'a pas la permission du plugin `opener`, dont les commandes ne passent pas par la barrière du
/// verrouillage (Tauri les route vers le plugin, pas vers ce gestionnaire).
#[tauri::command]
pub fn open_external(app: AppHandle, url: String) -> Result<()> {
    let parsed = url::Url::parse(&url)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https" | "mailto"));
    let Some(parsed) = parsed else {
        return Err(AppError::Invalid(msg!(
            "link.not_web",
            "only web and mail addresses can be opened"
        )));
    };
    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|e| AppError::Invalid((e.to_string()).into()))
}

/// Domaine enregistrable d'un hôte, d'après la liste des suffixes publics (rubriques privées comprises : `github.io`
/// ne désigne pas un seul site). Sert à comparer le site annoncé par un lien et sa destination.
#[tauri::command]
pub fn registrable_domain(host: String) -> String {
    autoconfig::registrable_domain(&host)
}

/// Détecte les serveurs d'une adresse (table intégrée, autoconfig, ISPDB, MX).
#[tauri::command]
pub async fn detect_servers(email: String) -> Result<account::Discovered> {
    autoconfig::detect(&email).await
}

/// Fournisseurs OAuth2 dont l'application (identifiant client) est configurée.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct OAuthStatus {
    google: bool,
    microsoft: bool,
}

// Les commandes qui touchent au trousseau sont asynchrones : une commande synchrone s'exécute sur le fil principal de
// l'interface, et le trousseau peut y bloquer longtemps (verrouillé, autorisation demandée).
#[tauri::command]
pub async fn oauth_status() -> Result<OAuthStatus> {
    Ok(OAuthStatus {
        google: OAuthProvider::Google.is_configured().await,
        microsoft: OAuthProvider::Microsoft.is_configured().await,
    })
}

#[tauri::command]
pub async fn set_oauth_client(
    provider: OAuthProvider,
    client_id: String,
    client_secret: Option<String>,
) -> Result<()> {
    let client_id = client_id.trim().to_string();
    if client_id.is_empty() {
        return Err(AppError::Invalid(msg!(
            "oauth.empty_client_id",
            "client ID is empty"
        )));
    }
    let client_secret = client_secret
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    secrets::store_client(
        provider,
        oauth::ClientCreds {
            client_id,
            client_secret,
        },
    )
    .await
}

#[tauri::command]
pub async fn clear_oauth_client(provider: OAuthProvider) -> Result<()> {
    secrets::delete_client(provider).await
}

#[tauri::command]
pub async fn remove_account(state: State<'_, AppState>, account_id: String) -> Result<()> {
    // L'identifiant vient de la webview : il doit être celui d'un compte connu (sinon `NotFound`, rien n'est
    // supprimé), et le dossier supprimé est calculé depuis l'identifiant lu en base, jamais depuis l'argument.
    let account = match account::get(&state.db, &account_id).await {
        Ok(account) => account,
        // Déjà supprimé (double clic, reprise après un échec du trousseau) : on termine le nettoyage des secrets.
        Err(AppError::NotFound(_)) => {
            sessions::forget(&account_id);
            return secrets::delete(&account_id).await;
        }
        Err(e) => return Err(e),
    };
    state.sync.stop(&account.id);
    account::remove(&state.db, &account.id).await?;
    if let Some(dir) = account_dir(&state.attachments_dir, &account.id) {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
    Ok(())
}

/// Dossier des pièces jointes d'un compte, seulement si l'identifiant est un nom simple : `..`, un chemin absolu ou un
/// séparateur ferait sortir de `base` (`Path::join` remplace la base par un chemin absolu).
fn account_dir(base: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
    let simple = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    simple.then(|| base.join(id))
}

/// Désabonne l'utilisateur de la liste d'un message (POST RFC 8058 ou message mailto).
#[tauri::command]
pub async fn unsubscribe(state: State<'_, AppState>, message_id: i64) -> Result<()> {
    let row: Option<(String, Option<String>, Option<String>, bool)> = sqlx::query_as(
        "SELECT account_id, unsub_url, unsub_mailto, unsub_one_click FROM messages WHERE id = ?",
    )
    .bind(message_id)
    .fetch_optional(&state.db)
    .await?;
    let (account_id, url, mailto, one_click) = row.ok_or_else(|| {
        AppError::NotFound(msg!(
            "not_found.message",
            "message not found: {id}",
            id = message_id
        ))
    })?;
    let acct = account::get(&state.db, &account_id).await?;
    let cred = account::resolve_credential(&acct).await?;
    let u = envelope_core::thread::Unsubscribe {
        url,
        mailto,
        one_click,
    };
    unsubscribe::run(&acct, &cred, &u).await
}

#[tauri::command]
pub async fn set_account_signature(
    state: State<'_, AppState>,
    account_id: String,
    signature: String,
) -> Result<()> {
    account::set_signature(&state.db, &account_id, checked_signature(&signature)?).await
}

#[tauri::command]
pub async fn set_identity_signature(
    state: State<'_, AppState>,
    account_id: String,
    id: i64,
    signature: String,
) -> Result<()> {
    account::set_identity_signature(&state.db, &account_id, id, checked_signature(&signature)?)
        .await
}

/// Texte brut, borné : la signature est insérée telle quelle dans le corps des messages.
fn checked_signature(signature: &str) -> Result<&str> {
    if signature.chars().count() > 2000 {
        return Err(AppError::Invalid(msg!(
            "account.signature_too_long",
            "signature too long (2000 characters)"
        )));
    }
    Ok(signature.trim_end())
}

#[tauri::command]
pub async fn set_account_copy_sent(
    state: State<'_, AppState>,
    account_id: String,
    on: bool,
) -> Result<()> {
    account::set_copy_sent(&state.db, &account_id, on).await
}

#[tauri::command]
pub async fn add_identity(
    state: State<'_, AppState>,
    account_id: String,
    name: String,
    email: String,
) -> Result<()> {
    account::add_identity(&state.db, &account_id, &name, &email).await
}

#[tauri::command]
pub async fn remove_identity(
    state: State<'_, AppState>,
    account_id: String,
    id: i64,
) -> Result<()> {
    account::remove_identity(&state.db, &account_id, id).await
}

#[tauri::command]
pub async fn list_templates(state: State<'_, AppState>) -> Result<Vec<Template>> {
    templates::list(&state.db).await
}

#[tauri::command]
pub async fn save_template(state: State<'_, AppState>, template: Template) -> Result<Template> {
    templates::save(&state.db, &template).await
}

#[tauri::command]
pub async fn delete_template(state: State<'_, AppState>, id: i64) -> Result<()> {
    templates::delete(&state.db, id).await
}

#[tauri::command]
pub fn sync_now(state: State<'_, AppState>, account_id: String) {
    state.sync.trigger(&account_id);
}

#[tauri::command]
pub async fn list_folders(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<FolderRow>> {
    Ok(search::list_folders(&state.db, account_id.as_deref()).await?)
}

#[tauri::command]
pub async fn list_messages(
    state: State<'_, AppState>,
    folder_id: i64,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<MessageSummary>> {
    Ok(search::list_messages(&state.db, folder_id, Page::new(limit, offset, 50)).await?)
}

#[tauri::command]
pub async fn search_emails(
    state: State<'_, AppState>,
    query: String,
    account_id: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<MessageSummary>> {
    Ok(search::search(
        &state.db,
        &query,
        account_id.as_deref(),
        Page::new(limit, None, 50),
    )
    .await?)
}

/// Copie non déposée dans « Envoyés » (le message, lui, est parti) : transmise à l'interface, qui ne peut pas l'attendre.
#[derive(Clone, Serialize)]
struct SentCopyFailed {
    subject: String,
    /// Erreur codée (`AppError::to_wire`), traduite à l'affichage.
    error: String,
}

/// Sort d'un envoi demandé par l'utilisateur.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SendOutcome {
    Sent,
    /// Serveur injoignable : le message est en boîte d'envoi et partira dès que la connexion revient.
    Queued,
}

#[tauri::command]
pub async fn send_email(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    message: OutgoingMessage,
) -> Result<SendOutcome> {
    match deliver(&app, &state, &account_id, &message).await {
        Ok(()) => Ok(SendOutcome::Sent),
        Err(e @ AppError::Unreachable(_)) => {
            scheduled::add_outbox(&state.db, &account_id, &message, &e.to_wire()).await?;
            Ok(SendOutcome::Queued)
        }
        Err(e) => Err(e),
    }
}

/// Envoi proprement dit (aussi utilisé par les envois programmés) : SMTP, mise en file de la copie dans « Envoyés »
/// (déposée ensuite par la synchro, sans retarder la réponse), retrait du brouillon.
pub(crate) async fn deliver(
    app: &AppHandle,
    state: &AppState,
    account_id: &str,
    message: &OutgoingMessage,
) -> Result<()> {
    let acct = account::get(&state.db, account_id).await?;
    let cred = account::resolve_credential(&acct).await?;
    let raw = smtp::send(&acct, &cred, message).await?;
    // Copie dans « Envoyés » : mise en file dès maintenant (avant le retrait du brouillon, voir `sentcopy::enqueue`),
    // affichée une fois le reste fait ; sauf si le serveur la range lui-même (Gmail).
    let queued = if acct.keeps_sent_copy() {
        queue_sent_copy(app, state, &acct.id, &message.subject, &raw).await
    } else {
        None
    };
    // Le brouillon a atteint son but : on le retire du serveur (sans jamais faire échouer l'envoi).
    if let Some(draft_id) = message.draft_id.as_deref() {
        let _guard = state.actions.lock().await;
        if let Err(e) = drafts::remove(&state.db, &acct, &cred, draft_id).await {
            tracing::warn!(account = %acct.id, "draft not deleted from the server after sending: {e}");
        }
    }
    if let Some(queued) = queued {
        match sentcopy::show_now(&state.db, &state.attachments_dir, &acct.id, &queued, &raw).await {
            Ok(()) => {
                let _ = app.emit("sent-copy:shown", &acct.id);
            }
            // La copie, elle, est en file : la synchro la dépose quand même.
            Err(e) => tracing::warn!(account = %acct.id, "sent message not shown at once: {e}"),
        }
    }
    state.sync.trigger(&acct.id); // la synchro dépose la copie
    Ok(())
}

/// Met la copie en file ; seul le disque peut l'en empêcher, signalé tout de suite à l'interface (`sent-copy:failed`).
async fn queue_sent_copy(
    app: &AppHandle,
    state: &AppState,
    account_id: &str,
    subject: &str,
    raw: &[u8],
) -> Option<sentcopy::Queued> {
    match sentcopy::enqueue(&state.db, &state.raw_dir, account_id, raw).await {
        Ok(queued) => Some(queued),
        Err(e) => {
            tracing::warn!(account = %account_id, "could not copy to Sent: {e}");
            let failed = SentCopyFailed {
                subject: subject.to_string(),
                error: e.to_wire(),
            };
            let _ = app.emit("sent-copy:failed", failed);
            None
        }
    }
}

/// Met un message en file pour l'envoyer à `send_at` (secondes Unix). L'application doit être ouverte à cette heure.
#[tauri::command]
pub async fn schedule_send(
    state: State<'_, AppState>,
    account_id: String,
    message: OutgoingMessage,
    send_at: i64,
) -> Result<i64> {
    // Mêmes contrôles qu'un envoi immédiat : expéditeur autorisé, au moins un destinataire.
    let acct = account::get(&state.db, &account_id).await?;
    smtp::check_sendable(&acct, &message)?;
    scheduled::add(&state.db, &account_id, send_at, &message).await
}

#[tauri::command]
pub async fn list_rules(state: State<'_, AppState>) -> Result<Vec<rules::Rule>> {
    rules::list(&state.db).await
}

#[tauri::command]
pub async fn save_rule(state: State<'_, AppState>, rule: rules::Rule) -> Result<i64> {
    rules::save(&state.db, &rule).await
}

#[tauri::command]
pub async fn delete_rule(state: State<'_, AppState>, id: i64) -> Result<()> {
    rules::delete(&state.db, id).await
}

/// Applique les règles aux conversations récentes de la boîte de réception ayant du courrier non lu ; rend le nombre
/// de conversations traitées.
#[tauri::command]
pub async fn run_rules_now(state: State<'_, AppState>) -> Result<usize> {
    rules::run_recent(&state.db, &state.actions).await
}

#[tauri::command]
pub async fn set_reminder(
    state: State<'_, AppState>,
    thread_id: i64,
    kind: reminders::Kind,
    due_at: i64,
) -> Result<()> {
    reminders::set(&state.db, thread_id, kind, due_at).await
}

#[tauri::command]
pub async fn clear_reminder(state: State<'_, AppState>, thread_id: i64) -> Result<()> {
    reminders::clear(&state.db, thread_id).await
}

#[tauri::command]
pub async fn list_scheduled(state: State<'_, AppState>) -> Result<Vec<scheduled::Scheduled>> {
    scheduled::list(&state.db).await
}

/// Retire un envoi de la file et rend le message, pour le rouvrir dans le composer.
#[tauri::command]
pub async fn cancel_scheduled(state: State<'_, AppState>, id: i64) -> Result<OutgoingMessage> {
    scheduled::cancel(&state.db, id).await
}

#[tauri::command]
pub async fn send_scheduled_now(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<()> {
    scheduled::deliver_now(&app, &state, id).await
}

/// Enregistre le brouillon sur le serveur (nouvelle version, puis suppression des précédentes).
#[tauri::command]
pub async fn save_draft(
    state: State<'_, AppState>,
    account_id: String,
    message: OutgoingMessage,
) -> Result<()> {
    let _guard = state.actions.lock().await; // une seule opération sur les brouillons à la fois
    let acct = account::get(&state.db, &account_id).await?;
    let cred = account::resolve_credential(&acct).await?;
    drafts::save(&state.db, &acct, &cred, &message).await?;
    state.sync.trigger(&account_id); // la nouvelle version apparaît aussitôt dans « Brouillons »
    Ok(())
}

/// Supprime un brouillon du serveur (abandon, ou changement de compte expéditeur).
#[tauri::command]
pub async fn delete_draft(
    state: State<'_, AppState>,
    account_id: String,
    draft_id: String,
) -> Result<()> {
    let _guard = state.actions.lock().await;
    let acct = account::get(&state.db, &account_id).await?;
    let cred = account::resolve_credential(&acct).await?;
    drafts::remove(&state.db, &acct, &cred, &draft_id).await?;
    state.sync.trigger(&account_id);
    Ok(())
}

/// Relit le brouillon d'une conversation pour le rouvrir dans le composer.
#[tauri::command]
pub async fn open_draft(state: State<'_, AppState>, thread_id: i64) -> Result<drafts::DraftData> {
    drafts::open(&state.db, &state.raw_dir, thread_id).await
}

/// Adresses déjà vues qui correspondent à la saisie (autocomplétion du destinataire).
#[tauri::command]
pub async fn suggest_contacts(
    state: State<'_, AppState>,
    account_id: Option<String>,
    query: String,
) -> Result<Vec<Addr>> {
    Ok(contacts::suggest(&state.db, account_id.as_deref(), &query, 8).await?)
}

// ---------- Conversations (interface) ----------

#[tauri::command]
pub async fn list_threads(
    state: State<'_, AppState>,
    query: ThreadQuery,
) -> Result<Vec<ThreadSummary>> {
    Ok(thread::list_threads(&state.db, &query).await?)
}

/// Télécharge le contenu d'un message volumineux, resté en en-tête seul à la synchro.
#[tauri::command]
pub async fn download_message(state: State<'_, AppState>, message_id: i64) -> Result<()> {
    crate::mail::sync::download_message(
        &state.db,
        &state.attachments_dir,
        &state.raw_dir,
        message_id,
    )
    .await
}

#[tauri::command]
pub async fn pgp_list_keys(
    state: State<'_, AppState>,
) -> Result<Vec<crate::pgp_store::PgpKeyInfo>> {
    crate::pgp_store::list_keys(&state.db).await
}

/// Importe une clé OpenPGP armurée (publique ou privée).
#[tauri::command]
pub async fn pgp_import_key(
    state: State<'_, AppState>,
    armored: String,
) -> Result<crate::pgp_store::PgpKeyInfo> {
    crate::pgp_store::import_key(&state.db, armored.as_bytes()).await
}

/// Ouvre le sélecteur de fichier natif et importe la clé OpenPGP choisie. `None` si l'utilisateur annule.
#[tauri::command]
pub async fn pgp_import_pick(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<crate::pgp_store::PgpKeyInfo>> {
    use tauri_plugin_dialog::DialogExt;
    let texts = crate::native_text::get();
    let Some(picked) = app
        .dialog()
        .file()
        .set_title(&texts.pgp_pick_title)
        .add_filter(&texts.pgp_filter, &["asc", "gpg", "pgp", "key"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = picks::dialog_path(picked)?;
    let data = tokio::fs::read(&path).await?;
    crate::pgp_store::import_key(&state.db, &data)
        .await
        .map(Some)
}

#[tauri::command]
pub async fn pgp_delete_key(state: State<'_, AppState>, fingerprint: String) -> Result<()> {
    crate::pgp_store::delete_key(&state.db, &fingerprint).await
}

/// Signature et contenu déchiffré d'un message PGP (`None` : pas du PGP). `passphrase` : phrase secrète saisie pour la
/// clé privée qui déchiffre ; elle reste en mémoire quelques minutes, jamais écrite.
#[tauri::command]
pub async fn pgp_inspect(
    state: State<'_, AppState>,
    message_id: i64,
    passphrase: Option<String>,
) -> Result<Option<crate::pgp_store::PgpInspection>> {
    crate::pgp_store::inspect(&state.db, &state.raw_dir, message_id, passphrase.as_deref()).await
}

#[tauri::command]
pub async fn get_thread_detail(state: State<'_, AppState>, thread_id: i64) -> Result<ThreadDetail> {
    thread::get_thread_detail(&state.db, thread_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(msg!(
                "not_found.thread",
                "conversation not found: {id}",
                id = thread_id
            ))
        })
}

/// Actions : le verrou sérialise les connexions IMAP (des raccourcis enchaînés ne doivent pas
/// ouvrir autant de connexions simultanées que de touches).
async fn run_action(state: &AppState, thread_id: i64, action: Action) -> Result<Vec<String>> {
    let _guard = state.actions.lock().await;
    actions::apply(&state.db, thread_id, action).await
}

#[tauri::command]
pub async fn set_thread_read(state: State<'_, AppState>, thread_id: i64, read: bool) -> Result<()> {
    run_action(&state, thread_id, Action::SetRead(read))
        .await
        .map(drop)
}

#[tauri::command]
pub async fn set_thread_starred(
    state: State<'_, AppState>,
    thread_id: i64,
    starred: bool,
) -> Result<()> {
    run_action(&state, thread_id, Action::SetStarred(starred))
        .await
        .map(drop)
}

#[tauri::command]
pub async fn move_thread(
    state: State<'_, AppState>,
    thread_id: i64,
    folder_id: i64,
    to: ServerDestination,
) -> Result<()> {
    let accounts = run_action(&state, thread_id, Action::Move { folder_id, to }).await?;
    // Le message a un nouvel UID dans le dossier cible : une synchro le fait apparaître aussitôt.
    // (Seule l'IDLE de la boîte de réception se réveille d'elle-même.)
    for account in &accounts {
        state.sync.trigger(account);
    }
    Ok(())
}

/// Suppression définitive : depuis la corbeille ou le spam d'un compte, ou depuis un dossier local.
#[tauri::command]
pub async fn delete_thread(
    state: State<'_, AppState>,
    thread_id: i64,
    folder_id: i64,
) -> Result<()> {
    let _guard = state.actions.lock().await;
    if is_local_folder(&state, folder_id).await? {
        return local::delete_messages(&state.db, thread_id, folder_id).await;
    }
    actions::apply(&state.db, thread_id, Action::Delete { folder_id })
        .await
        .map(drop)
}

async fn is_local_folder(state: &AppState, folder_id: i64) -> Result<bool> {
    Ok(
        sqlx::query_scalar::<_, String>("SELECT account_id FROM folders WHERE id = ?")
            .bind(folder_id)
            .fetch_optional(&state.db)
            .await?
            .is_some_and(|a| a == account::LOCAL_ID),
    )
}

/// Archive la conversation en local : copie vérifiée sur cet ordinateur, puis suppression sur le
/// serveur. Depuis un dossier local, la déplace simplement vers un autre dossier local.
#[tauri::command]
pub async fn archive_local(
    state: State<'_, AppState>,
    thread_id: i64,
    folder_id: i64,
    local_folder_id: i64,
) -> Result<()> {
    let _guard = state.actions.lock().await;
    if is_local_folder(&state, folder_id).await? {
        return local::move_within(&state.db, thread_id, folder_id, local_folder_id).await;
    }
    let accounts = local::archive(
        &state.db,
        &state.attachments_dir,
        &state.raw_dir,
        thread_id,
        folder_id,
        local_folder_id,
    )
    .await?;
    for account in &accounts {
        state.sync.trigger(account);
    }
    Ok(())
}

// ---------- Import des dossiers locaux d'un autre client (Thunderbird) ----------

/// Dossier des profils Thunderbird de cette machine, s'il existe : point de départ du sélecteur de
/// dossier natif (l'utilisateur choisit lui-même le profil, rien n'est exploré sans son geste).
fn thunderbird_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    let path = app.path();
    let home = path.home_dir().ok();
    let candidates: Vec<std::path::PathBuf> = if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Thunderbird/Profiles"))
            .into_iter()
            .collect()
    } else if cfg!(windows) {
        path.data_dir()
            .map(|d| d.join("Thunderbird").join("Profiles"))
            .into_iter()
            .collect()
    } else {
        home.map(|h| {
            vec![
                h.join(".thunderbird"),
                h.join(".var/app/org.mozilla.Thunderbird/.thunderbird"),
                h.join("snap/thunderbird/common/.thunderbird"),
            ]
        })
        .unwrap_or_default()
    };
    candidates.into_iter().find(|p| p.is_dir())
}

/// Source d'import choisie par l'utilisateur : jeton opaque, chemin à afficher, dossiers trouvés.
#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ImportSource {
    token: String,
    path: String,
    folders: Vec<mbox::Found>,
}

/// Ouvre le sélecteur de dossier natif, puis liste les dossiers importables de ce qui a été choisi.
/// `None` si l'utilisateur annule. L'interface ne fournit jamais de chemin : elle reçoit un jeton.
#[tauri::command]
pub async fn import_pick(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<ImportSource>> {
    use tauri_plugin_dialog::DialogExt;
    let mut dialog = app
        .dialog()
        .file()
        .set_title(&crate::native_text::get().import_pick_title);
    if let Some(dir) = thunderbird_dir(&app) {
        dialog = dialog.set_directory(dir);
    }
    let Some(picked) = dialog.blocking_pick_folder() else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| AppError::Invalid((e.to_string()).into()))?;
    let folders = mbox::discover(&path).await?;
    Ok(Some(ImportSource {
        token: state.picks.remember(path.clone()),
        path: path.display().to_string(),
        folders,
    }))
}

/// Importe les dossiers choisis (`keys`, issus de `import_pick`) dans « Sur cet ordinateur » ;
/// l'avancement part en événements `import:progress`.
#[tauri::command]
pub async fn import_run(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
    keys: Vec<String>,
) -> Result<mbox::Report> {
    use tauri::Emitter;
    let path = state.picks.get(&token).ok_or_else(|| {
        AppError::Invalid(msg!(
            "import.source_expired",
            "import source expired: choose it again"
        ))
    })?;
    // Pas de verrou global : un import long ne doit pas bloquer les autres actions.
    let keys: std::collections::HashSet<String> = keys.into_iter().collect();
    let emit = move |p: mbox::Progress| {
        let _ = app.emit("import:progress", p);
    };
    let report = mbox::import(
        &state.db,
        &state.attachments_dir,
        &state.raw_dir,
        &path,
        &keys,
        &emit,
    )
    .await;
    state.picks.forget(&token);
    report
}

/// Ouvre le sélecteur natif de la destination d'un export : « Enregistrer sous » pour un fichier mbox,
/// choix d'un dossier pour des fichiers .eml. Renvoie un jeton (`None` si l'utilisateur annule).
#[tauri::command]
pub async fn export_pick(
    app: AppHandle,
    state: State<'_, AppState>,
    format: export::Format,
    default_name: String,
) -> Result<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let texts = crate::native_text::get();
    let picked = match format {
        export::Format::Mbox => app
            .dialog()
            .file()
            .set_file_name(export::slug(&default_name) + ".mbox")
            .add_filter(&texts.mbox_filter, &["mbox"])
            .blocking_save_file(),
        export::Format::Eml => app
            .dialog()
            .file()
            .set_title(&texts.export_pick_title)
            .blocking_pick_folder(),
    };
    let Some(picked) = picked else {
        return Ok(None);
    };
    Ok(Some(state.picks.remember(picks::dialog_path(picked)?)))
}

/// Exporte un dossier vers la destination choisie (`token`, issu de `export_pick`) : un fichier mbox, ou
/// des `.eml` dans un sous-dossier nommé d'après le dossier. L'avancement part en événements `export:progress`.
#[tauri::command]
pub async fn export_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    folder_id: i64,
    format: export::Format,
    token: String,
) -> Result<export::Report> {
    use tauri::Emitter;
    let chosen = state.picks.get(&token).ok_or_else(|| {
        AppError::Invalid(msg!(
            "export.destination_expired",
            "export destination expired: choose it again"
        ))
    })?;
    let emit = move |p: export::Progress| {
        let _ = app.emit("export:progress", p);
    };
    let report = match format {
        export::Format::Mbox => {
            export::export_mbox(&state.db, &state.raw_dir, folder_id, &chosen, &emit).await
        }
        export::Format::Eml => {
            let name: String = sqlx::query_scalar("SELECT name FROM folders WHERE id = ?")
                .bind(folder_id)
                .fetch_optional(&state.db)
                .await?
                .ok_or_else(|| {
                    AppError::NotFound(msg!(
                        "not_found.folder",
                        "folder not found: {id}",
                        id = folder_id
                    ))
                })?;
            export::export_eml(
                &state.db,
                &state.raw_dir,
                folder_id,
                &chosen.join(export::slug(&name)),
                &emit,
            )
            .await
        }
    };
    state.picks.forget(&token);
    report
}

#[tauri::command]
pub async fn create_local_folder(state: State<'_, AppState>, name: String) -> Result<i64> {
    local::create_folder(&state.db, &name).await
}

#[tauri::command]
pub async fn rename_local_folder(
    state: State<'_, AppState>,
    folder_id: i64,
    name: String,
) -> Result<()> {
    local::rename_folder(&state.db, folder_id, &name).await
}

#[tauri::command]
pub async fn delete_local_folder(state: State<'_, AppState>, folder_id: i64) -> Result<()> {
    local::delete_folder(&state.db, folder_id).await
}

#[tauri::command]
pub async fn empty_folder(state: State<'_, AppState>, folder_id: i64) -> Result<()> {
    let _guard = state.actions.lock().await;
    actions::empty_folder(&state.db, folder_id).await
}

// ---------- Serveur MCP (panneau de réglages) ----------

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct McpInfo {
    /// Binaire `envelope-mcp` à côté de l'application, s'il existe.
    binary_path: Option<String>,
    db_path: String,
}

#[tauri::command]
pub fn mcp_info(state: State<'_, AppState>) -> McpInfo {
    let name = if cfg!(windows) {
        "envelope-mcp.exe"
    } else {
        "envelope-mcp"
    };
    let binary_path = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(name)))
        .filter(|p| p.exists())
        .map(|p| p.display().to_string());
    McpInfo {
        binary_path,
        db_path: state.db_path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Un identifiant de compte venu de la webview ne fait jamais sortir du dossier des pièces jointes.
    #[test]
    fn dossier_de_compte_refuse_les_identifiants_qui_sortent_de_la_base() {
        let base = Path::new("/data/attachments");
        assert_eq!(
            account_dir(base, "8f14e45f-ceea-4e6e"),
            Some(base.join("8f14e45f-ceea-4e6e"))
        );
        for bad in ["", "..", ".", "/etc", "a/b", "a\\b", "../x", "C:\\x", "a.b"] {
            assert_eq!(account_dir(base, bad), None, "{bad:?}");
        }
    }
}

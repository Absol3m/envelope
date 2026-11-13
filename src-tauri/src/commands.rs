//! Commandes Tauri exposées au frontend (`invoke`).

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::mail::account::{self, Account, NewAccount};
use crate::mail::actions::{self, Action, Destination};
use crate::mail::autoconfig;
use crate::mail::drafts;
use crate::mail::export;
use crate::mail::imap;
use crate::mail::local;
use crate::mail::mbox;
use crate::mail::oauth::{self, OAuthProvider};
use crate::mail::sentcopy;
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
        let id = a.id.clone();
        account::blocking_keychain(move || {
            account::delete_secrets(&id);
            Ok(())
        })
        .await?;
    }
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

/// Ajoute un compte (mot de passe / mot de passe d'application) : découverte des serveurs,
/// test IMAP + SMTP, enregistrement, secret dans le trousseau, démarrage de la synchro.
#[tauri::command]
pub async fn add_account(state: State<'_, AppState>, input: NewAccount) -> Result<Account> {
    let email = input.email.trim().to_string();
    let found = autoconfig::detect(&email).await?;
    let imap_cfg = input.imap.unwrap_or(found.imap);
    let smtp_cfg = input.smtp.unwrap_or(found.smtp);
    let username = input
        .username
        .filter(|u| !u.is_empty())
        .unwrap_or(found.username);
    let id = Uuid::new_v4().to_string();

    account::insert(
        &state.db,
        &account::NewAccountRow {
            id: &id,
            email: &email,
            display_name: input.display_name.as_deref(),
            provider: &found.provider,
            auth_kind: "password",
            username: &username,
            imap: &imap_cfg,
            smtp: &smtp_cfg,
        },
    )
    .await?;
    let finish = async {
        let (secret_id, password) = (id.clone(), input.password.clone());
        account::blocking_keychain(move || account::store_password(&secret_id, &password)).await?;
        let acct = account::get(&state.db, &id).await?;
        let cred = account::resolve_credential(&acct).await?;
        let mut session = imap::connect(&acct, &cred).await?; // valide hôte, TLS et identifiants
        let _ = session.logout().await;
        smtp::test(&acct, &cred).await?;
        Ok::<_, AppError>(acct)
    }
    .await;
    match finish {
        Ok(acct) => {
            state.sync.start(id);
            Ok(acct)
        }
        Err(e) => {
            account::delete(&state.db, &id).await?; // annule : ni compte orphelin ni secret
            Err(e)
        }
    }
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
    let provider = OAuthProvider::parse(&acc.provider).ok_or_else(|| {
        AppError::Invalid(msg!("account.not_oauth", "this account doesn't use OAuth2"))
    })?;
    let tokens = authorize_in_browser(&app, provider, &acc.email).await?;
    let secret_id = id.clone();
    account::blocking_keychain(move || account::store_oauth(&secret_id, &tokens)).await?;
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
    let (imap_cfg, smtp_cfg) = account::oauth_servers(provider)?;

    let id = Uuid::new_v4().to_string();
    account::insert(
        &state.db,
        &account::NewAccountRow {
            id: &id,
            email: &email,
            display_name: display_name.as_deref(),
            provider: provider.as_str(),
            auth_kind: "oauth2",
            username: &email,
            imap: &imap_cfg,
            smtp: &smtp_cfg,
        },
    )
    .await?;
    let (secret_id, secret) = (id.clone(), tokens.clone());
    if let Err(e) =
        account::blocking_keychain(move || account::store_oauth(&secret_id, &secret)).await
    {
        account::delete(&state.db, &id).await?;
        return Err(e);
    }
    state.sync.start(id.clone());
    account::get(&state.db, &id).await
}

/// Détecte les serveurs d'une adresse (table intégrée, autoconfig, ISPDB, MX).
#[tauri::command]
pub async fn detect_servers(email: String) -> Result<account::Discovered> {
    autoconfig::detect(&email).await
}

/// Fournisseurs OAuth2 dont l'application (identifiant client) est configurée.
#[derive(Serialize)]
pub struct OAuthStatus {
    google: bool,
    microsoft: bool,
}

// Les commandes qui touchent au trousseau sont asynchrones : une commande synchrone s'exécute sur le fil principal de
// l'interface, et le trousseau peut y bloquer longtemps (verrouillé, autorisation demandée).
#[tauri::command]
pub async fn oauth_status() -> Result<OAuthStatus> {
    account::blocking_keychain(|| {
        Ok(OAuthStatus {
            google: OAuthProvider::Google.is_configured(),
            microsoft: OAuthProvider::Microsoft.is_configured(),
        })
    })
    .await
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
    account::blocking_keychain(move || {
        account::store_client(
            provider,
            &oauth::ClientCreds {
                client_id,
                client_secret,
            },
        )
    })
    .await
}

#[tauri::command]
pub async fn clear_oauth_client(provider: OAuthProvider) -> Result<()> {
    account::blocking_keychain(move || {
        account::delete_client(provider);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn remove_account(state: State<'_, AppState>, account_id: String) -> Result<()> {
    state.sync.stop(&account_id);
    account::delete(&state.db, &account_id).await?;
    let _ = tokio::fs::remove_dir_all(state.attachments_dir.join(&account_id)).await;
    Ok(())
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
    // Texte brut, borné : la signature est insérée telle quelle dans le corps des messages.
    if signature.chars().count() > 2000 {
        return Err(AppError::Invalid(msg!(
            "account.signature_too_long",
            "signature too long (2000 characters)"
        )));
    }
    account::set_signature(&state.db, &account_id, signature.trim_end()).await
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
pub async fn remove_identity(state: State<'_, AppState>, id: i64) -> Result<()> {
    account::remove_identity(&state.db, id).await
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
    Ok(search::list_messages(
        &state.db,
        folder_id,
        limit.unwrap_or(50),
        offset.unwrap_or(0),
    )
    .await?)
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
        limit.unwrap_or(50),
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

/// Envoi proprement dit (aussi utilisé par les envois programmés) : SMTP, retrait du brouillon ; la copie dans
/// « Envoyés » se fait ensuite en tâche de fond (`after_send`), sans retarder la réponse.
pub(crate) async fn deliver(
    app: &AppHandle,
    state: &AppState,
    account_id: &str,
    message: &OutgoingMessage,
) -> Result<()> {
    let acct = account::get(&state.db, account_id).await?;
    let cred = account::resolve_credential(&acct).await?;
    let raw = smtp::send(&acct, &cred, message).await?;
    // Le brouillon a atteint son but : on le retire du serveur (sans jamais faire échouer l'envoi).
    if let Some(draft_id) = message.draft_id.as_deref() {
        let _guard = state.actions.lock().await;
        if let Err(e) = drafts::remove(&state.db, &acct, &cred, draft_id).await {
            tracing::warn!(account = %acct.id, "draft not deleted from the server after sending: {e}");
        }
    }
    let app = app.clone();
    let subject = message.subject.clone();
    tauri::async_runtime::spawn(async move { after_send(&app, acct, subject, raw).await });
    Ok(())
}

/// Après l'envoi : met la copie dans « Envoyés » en file (`mail::sentcopy`, reprise à chaque synchro, jamais perdue), sauf
/// si le serveur la range lui-même (Gmail), puis réveille la synchro qui la dépose. Seule l'impossibilité de la mettre en
/// file (disque) est signalée tout de suite à l'interface (`sent-copy:failed`).
async fn after_send(app: &AppHandle, acct: Account, subject: String, raw: Vec<u8>) {
    let state = app.state::<AppState>();
    if acct.keeps_sent_copy() {
        match enqueue_and_show(&state, &acct.id, &raw).await {
            Ok(()) => {
                let _ = app.emit("sent-copy:shown", &acct.id);
            }
            Err(e) => {
                tracing::warn!(account = %acct.id, "could not copy to Sent: {e}");
                let _ = app.emit(
                    "sent-copy:failed",
                    SentCopyFailed {
                        subject,
                        error: e.to_wire(),
                    },
                );
            }
        }
    }
    state.sync.trigger(&acct.id);
}

/// Met la copie en file, puis la montre aussitôt dans « Envoyés » : seule la mise en file est une erreur ; si
/// l'affichage échoue, la synchro déposera quand même la copie.
async fn enqueue_and_show(state: &AppState, account_id: &str, raw: &[u8]) -> Result<()> {
    let queued = sentcopy::enqueue(&state.db, &state.raw_dir, account_id, raw).await?;
    if let Err(e) =
        sentcopy::show_now(&state.db, &state.attachments_dir, account_id, &queued, raw).await
    {
        tracing::warn!(account = %account_id, "sent message not shown at once: {e}");
    }
    Ok(())
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
    smtp::build_message(&acct, &message)?;
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
    to: Destination,
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
#[derive(Serialize)]
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

#[derive(Serialize)]
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

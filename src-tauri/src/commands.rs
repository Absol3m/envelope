//! Commandes Tauri exposées au frontend (`invoke`).

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::mail::account::{self, Account, NewAccount};
use crate::mail::actions::{self, Action, Destination};
use crate::mail::autoconfig;
use crate::mail::export;
use crate::mail::imap;
use crate::mail::ingest;
use crate::mail::local;
use crate::mail::mbox;
use crate::mail::oauth::{self, OAuthProvider};
use crate::mail::smtp::{self, OutgoingMessage};
use crate::mail::unsubscribe;
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
        account::store_password(&id, &input.password)?;
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
    let tokens = oauth::authorize(provider, |url| {
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|e| AppError::Auth(e.to_string()))
    })
    .await?;

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
    if let Err(e) = account::store_oauth(&id, &tokens) {
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

#[tauri::command]
pub fn oauth_status() -> OAuthStatus {
    OAuthStatus {
        google: OAuthProvider::Google.is_configured(),
        microsoft: OAuthProvider::Microsoft.is_configured(),
    }
}

#[tauri::command]
pub fn set_oauth_client(
    provider: OAuthProvider,
    client_id: String,
    client_secret: Option<String>,
) -> Result<()> {
    let client_id = client_id.trim().to_string();
    if client_id.is_empty() {
        return Err(AppError::Invalid("identifiant client vide".into()));
    }
    let client_secret = client_secret
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    account::store_client(
        provider,
        &oauth::ClientCreds {
            client_id,
            client_secret,
        },
    )
}

#[tauri::command]
pub fn clear_oauth_client(provider: OAuthProvider) {
    account::delete_client(provider);
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
    let (account_id, url, mailto, one_click) =
        row.ok_or_else(|| AppError::NotFound(format!("message {message_id}")))?;
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
        return Err(AppError::Invalid(
            "signature trop longue (2000 caractères)".into(),
        ));
    }
    account::set_signature(&state.db, &account_id, signature.trim_end()).await
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

/// Sort de la copie du message envoyé (le message lui-même est parti dans tous les cas).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SentCopy {
    /// Copie déposée dans « Envoyés ».
    Saved,
    /// Le serveur range lui-même le message (Gmail).
    Server,
    /// Copie impossible, avec la cause.
    Failed(String),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendReport {
    sent_copy: SentCopy,
}

#[tauri::command]
pub async fn send_email(
    state: State<'_, AppState>,
    account_id: String,
    message: OutgoingMessage,
) -> Result<SendReport> {
    let acct = account::get(&state.db, &account_id).await?;
    let cred = account::resolve_credential(&acct).await?;
    let raw = smtp::send(&acct, &cred, &message).await?;

    let sent_copy = if acct.provider == "gmail" {
        SentCopy::Server
    } else {
        match file_in_sent(&state, &acct, &cred, &raw).await {
            Ok(()) => SentCopy::Saved,
            Err(e) => {
                tracing::warn!(account = %acct.id, "copie dans Envoyés impossible : {e}");
                SentCopy::Failed(e.to_string())
            }
        }
    };
    state.sync.trigger(&account_id); // la copie (ou le nouveau dossier) apparaît à la synchro
    Ok(SendReport { sent_copy })
}

async fn file_in_sent(
    state: &AppState,
    acct: &Account,
    cred: &account::Credential,
    raw: &[u8],
) -> Result<()> {
    // Dossier déjà connu de la synchro ; sinon `append_sent` le cherche sur le serveur, voire le crée.
    let known: Option<String> =
        sqlx::query_scalar("SELECT path FROM folders WHERE account_id = ? AND role = 'sent'")
            .bind(&acct.id)
            .fetch_optional(&state.db)
            .await?;
    let mut session = imap::connect(acct, cred).await?;
    let res = imap::append_sent(&mut session, known.as_deref(), raw).await;
    let _ = session.logout().await;
    res
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
        .ok_or_else(|| AppError::NotFound(format!("conversation {thread_id}")))
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
#[tauri::command]
pub fn import_default_dir(app: AppHandle) -> Option<String> {
    use tauri::Manager;
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
    candidates
        .into_iter()
        .find(|p| p.is_dir())
        .map(|p| p.display().to_string())
}

/// Liste les dossiers importables de la source choisie (profil Thunderbird, dossier de boîtes…).
#[tauri::command]
pub async fn import_scan(path: String) -> Result<Vec<mbox::Found>> {
    mbox::discover(std::path::Path::new(&path)).await
}

/// Importe les dossiers choisis dans « Sur cet ordinateur » ; l'avancement part en événements `import:progress`.
#[tauri::command]
pub async fn import_run(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    keys: Vec<String>,
) -> Result<mbox::Report> {
    use tauri::Emitter;
    // Pas de verrou global : un import long ne doit pas bloquer les autres actions.
    let keys: std::collections::HashSet<String> = keys.into_iter().collect();
    let emit = move |p: mbox::Progress| {
        let _ = app.emit("import:progress", p);
    };
    mbox::import(
        &state.db,
        &state.attachments_dir,
        &state.raw_dir,
        std::path::Path::new(&path),
        &keys,
        &emit,
    )
    .await
}

/// Exporte un dossier : un fichier mbox (`path` : fichier choisi) ou des `.eml` (`path` : dossier choisi).
/// L'avancement part en événements `export:progress`.
#[tauri::command]
pub async fn export_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    folder_id: i64,
    format: export::Format,
    path: String,
) -> Result<export::Report> {
    use tauri::Emitter;
    let emit = move |p: export::Progress| {
        let _ = app.emit("export:progress", p);
    };
    let dest = std::path::Path::new(&path);
    match format {
        export::Format::Mbox => {
            export::export_mbox(&state.db, &state.raw_dir, folder_id, dest, &emit).await
        }
        export::Format::Eml => {
            export::export_eml(&state.db, &state.raw_dir, folder_id, dest, &emit).await
        }
    }
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

// ---------- Pièces jointes ----------

/// Chemin d'une pièce jointe, uniquement si elle est bien dans le dossier des pièces jointes
/// (une base altérée ne doit pas permettre d'ouvrir un fichier arbitraire).
async fn attachment_path(state: &AppState, attachment_id: i64) -> Result<std::path::PathBuf> {
    let stored: Option<String> =
        sqlx::query_scalar("SELECT storage_path FROM attachments WHERE id = ?")
            .bind(attachment_id)
            .fetch_optional(&state.db)
            .await?
            .flatten();
    let path = match stored {
        Some(p) => std::path::PathBuf::from(p),
        // Historique rapatrié : le contenu est extrait du message brut à la première ouverture.
        None => {
            ingest::materialize_attachment(
                &state.db,
                &state.attachments_dir,
                &state.raw_dir,
                attachment_id,
            )
            .await?
        }
    };
    let path = tokio::fs::canonicalize(path).await?;
    let root = tokio::fs::canonicalize(&state.attachments_dir).await?;
    if !path.starts_with(&root) {
        return Err(AppError::Invalid(
            "pièce jointe hors du dossier autorisé".into(),
        ));
    }
    // Sous Windows, `canonicalize` produit un chemin `\\?\C:\…` que certaines applications
    // associées refusent : on le rend sous sa forme habituelle (sans effet ailleurs).
    Ok(dunce::simplified(&path).to_path_buf())
}

#[tauri::command]
pub async fn open_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    attachment_id: i64,
) -> Result<()> {
    let path = attachment_path(&state, attachment_id).await?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| AppError::Invalid(e.to_string()))
}

/// Images « inline » d'un message (`cid:`), en data URLs, pour l'affichage hors-ligne.
/// Bornées en taille : une image géante ne doit pas gonfler la réponse.
#[tauri::command]
pub async fn inline_images(
    state: State<'_, AppState>,
    message_id: i64,
) -> Result<std::collections::HashMap<String, String>> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    const MAX_INLINE_BYTES: i64 = 2 * 1024 * 1024;
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT id, content_id, mime_type FROM attachments \
         WHERE message_id = ? AND content_id IS NOT NULL AND mime_type LIKE 'image/%' AND size <= ?",
    )
    .bind(message_id)
    .bind(MAX_INLINE_BYTES)
    .fetch_all(&state.db)
    .await?;
    let mut out = std::collections::HashMap::new();
    for (id, cid, mime) in rows {
        let bytes = tokio::fs::read(attachment_path(&state, id).await?).await?;
        out.insert(
            cid,
            format!("data:{mime};base64,{}", STANDARD.encode(bytes)),
        );
    }
    Ok(out)
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

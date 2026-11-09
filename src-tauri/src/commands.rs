//! Commandes Tauri exposées au frontend (`invoke`).

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::mail::account::{self, Account, NewAccount};
use crate::mail::oauth::{self, OAuthProvider};
use crate::mail::search::{self, FolderRow, MessageSummary};
use crate::mail::smtp::{self, OutgoingMessage};
use crate::mail::imap;
use crate::AppState;

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

#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>> {
    account::list(&state.db).await
}

/// Ajoute un compte (mot de passe / mot de passe d'application) : découverte des serveurs,
/// test IMAP + SMTP, enregistrement, secret dans le trousseau, démarrage de la synchro.
#[tauri::command]
pub async fn add_account(state: State<'_, AppState>, input: NewAccount) -> Result<Account> {
    let email = input.email.trim().to_string();
    let found = account::discover(&email)?;
    let imap_cfg = input.imap.unwrap_or(found.imap);
    let smtp_cfg = input.smtp.unwrap_or(found.smtp);
    let username = input.username.filter(|u| !u.is_empty()).unwrap_or_else(|| email.clone());
    let id = Uuid::new_v4().to_string();

    account::insert(&state.db, &id, &email, input.display_name.as_deref(), found.provider, "password", &username, &imap_cfg, &smtp_cfg).await?;
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
pub async fn add_oauth_account(app: AppHandle, state: State<'_, AppState>, provider: OAuthProvider, email: String, display_name: Option<String>) -> Result<Account> {
    let email = email.trim().to_string();
    let found = account::discover(&email)?;
    let tokens = oauth::authorize(provider, |url| {
        app.opener().open_url(url, None::<&str>).map_err(|e| AppError::Auth(e.to_string()))
    })
    .await?;

    // Serveurs fixes du fournisseur choisi (le domaine de l'e-mail peut être personnalisé).
    let (imap_cfg, smtp_cfg) = match provider {
        OAuthProvider::Google => account::discover("x@gmail.com")?,
        OAuthProvider::Microsoft => account::discover("x@outlook.com")?,
    }
    .into_servers();
    let _ = found;

    let id = Uuid::new_v4().to_string();
    account::insert(&state.db, &id, &email, display_name.as_deref(), provider.as_str(), "oauth2", &email, &imap_cfg, &smtp_cfg).await?;
    if let Err(e) = account::store_oauth(&id, &tokens) {
        account::delete(&state.db, &id).await?;
        return Err(e);
    }
    state.sync.start(id.clone());
    account::get(&state.db, &id).await
}

#[tauri::command]
pub async fn remove_account(state: State<'_, AppState>, account_id: String) -> Result<()> {
    state.sync.stop(&account_id);
    account::delete(&state.db, &account_id).await?;
    let _ = tokio::fs::remove_dir_all(state.attachments_dir.join(&account_id)).await;
    Ok(())
}

#[tauri::command]
pub fn sync_now(state: State<'_, AppState>, account_id: String) {
    state.sync.trigger(&account_id);
}

#[tauri::command]
pub async fn list_folders(state: State<'_, AppState>, account_id: Option<String>) -> Result<Vec<FolderRow>> {
    search::list_folders(&state.db, account_id.as_deref()).await
}

#[tauri::command]
pub async fn list_messages(state: State<'_, AppState>, folder_id: i64, limit: Option<i64>, offset: Option<i64>) -> Result<Vec<MessageSummary>> {
    search::list_messages(&state.db, folder_id, limit.unwrap_or(50), offset.unwrap_or(0)).await
}

#[tauri::command]
pub async fn search_emails(state: State<'_, AppState>, query: String, account_id: Option<String>, limit: Option<i64>) -> Result<Vec<MessageSummary>> {
    search::search(&state.db, &query, account_id.as_deref(), limit.unwrap_or(50)).await
}

#[tauri::command]
pub async fn send_email(state: State<'_, AppState>, account_id: String, message: OutgoingMessage) -> Result<()> {
    let acct = account::get(&state.db, &account_id).await?;
    let cred = account::resolve_credential(&acct).await?;
    smtp::send(&acct, &cred, &message).await
}

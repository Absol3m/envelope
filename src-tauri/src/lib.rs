//! Point d'entrée de la bibliothèque : construit l'application Tauri.
//!
//! Architecture :
//!   frontend (React) ──invoke──▶ commands ──▶ db / mail ◀── mcp (agents IA)

mod commands;
mod db;
mod error;
mod mail;
mod mcp;

use std::path::PathBuf;
use std::sync::Arc;

use sqlx::SqlitePool;
use tauri::Manager;

/// État partagé injecté dans les commandes via `State<AppState>`.
pub struct AppState {
    pub db: SqlitePool,
    pub db_path: PathBuf,
    pub attachments_dir: PathBuf,
    pub sync: Arc<mail::sync::SyncManager>,
    /// Sérialise les actions IMAP (archiver, marquer lu…).
    pub actions: tokio::sync::Mutex<()>,
}

pub fn run() {
    // Un seul fournisseur crypto rustls pour tout le process (IMAP, SMTP, HTTP).
    let _ = rustls::crypto::ring::default_provider().install_default();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let db_path = db::db_path(&data_dir);
            let pool = tauri::async_runtime::block_on(db::init(&db_path))?;
            tracing::info!("base SQLite prête : {}", db_path.display());

            let attachments_dir = data_dir.join("attachments");
            let sync = mail::sync::SyncManager::new(
                pool.clone(),
                app.handle().clone(),
                attachments_dir.clone(),
            );
            app.manage(AppState {
                db: pool,
                db_path,
                attachments_dir,
                sync: sync.clone(),
                actions: tokio::sync::Mutex::new(()),
            });
            tauri::async_runtime::spawn(async move {
                if let Err(e) = sync.start_all().await {
                    tracing::error!("démarrage de la synchro : {e}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_accounts,
            commands::add_account,
            commands::add_oauth_account,
            commands::detect_servers,
            commands::oauth_status,
            commands::set_oauth_client,
            commands::clear_oauth_client,
            commands::remove_account,
            commands::sync_now,
            commands::list_folders,
            commands::list_messages,
            commands::search_emails,
            commands::send_email,
            commands::suggest_contacts,
            commands::list_threads,
            commands::get_thread_detail,
            commands::set_thread_read,
            commands::set_thread_starred,
            commands::archive_thread,
            commands::open_attachment,
            commands::inline_images,
            commands::mcp_info,
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de l'application Tauri");
}

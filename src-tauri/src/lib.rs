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

use sqlx::SqlitePool;
use tauri::Manager;

/// État partagé injecté dans les commandes via `State<AppState>`.
pub struct AppState {
    pub db: SqlitePool,
    pub db_path: PathBuf,
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let db_path = db::db_path(&app.path().app_data_dir()?);
            let pool = tauri::async_runtime::block_on(db::init(&db_path))?;
            tracing::info!("base SQLite prête : {}", db_path.display());
            app.manage(AppState { db: pool, db_path });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::app_info, commands::list_accounts])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de l'application Tauri");
}

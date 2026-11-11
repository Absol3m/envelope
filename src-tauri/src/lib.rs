//! Point d'entrée de la bibliothèque : construit l'application Tauri.
//!
//! Architecture :
//!   frontend (React) ──invoke──▶ commands ──▶ db / mail ◀── mcp (agents IA)

mod commands;
mod db;
#[cfg(debug_assertions)]
#[doc(hidden)]
pub mod dev;
mod error;
mod mail;
mod mcp;
mod picks;
mod tls;

use std::path::PathBuf;
use std::sync::Arc;

use sqlx::SqlitePool;
use tauri::Manager;

/// État partagé injecté dans les commandes via `State<AppState>`.
pub struct AppState {
    pub db: SqlitePool,
    pub db_path: PathBuf,
    pub attachments_dir: PathBuf,
    /// Magasin des messages bruts (voir `mail::rawstore`).
    pub raw_dir: PathBuf,
    pub sync: Arc<mail::sync::SyncManager>,
    /// Sérialise les actions IMAP (archiver, marquer lu…).
    pub actions: tokio::sync::Mutex<()>,
    /// Chemins choisis dans les sélecteurs natifs (import, export) : l'interface n'en voit que des jetons.
    pub picks: picks::Picks,
}

/// Fenêtre principale, aux conventions de chaque système :
/// - macOS : barre de titre fusionnée avec l'interface, barre latérale en « vibrancy » ;
/// - Windows 11 : matière Mica sous l'interface (Windows 10 : fenêtre opaque) ;
/// - Linux : fenêtre opaque et décorations du bureau.
fn main_window(app: &tauri::App) -> tauri::Result<()> {
    use envelope_appearance::Material;
    use tauri::window::{Effect, EffectState, EffectsBuilder};

    let host = envelope_appearance::host();
    // Palette et matière connues de la page avant son premier affichage (voir lib/theme.ts).
    let host_json = serde_json::to_string(&host).map_err(|e| tauri::Error::Anyhow(e.into()))?;
    let mut builder = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
        .title("Envelope")
        .inner_size(1280.0, 800.0)
        .min_inner_size(900.0, 560.0)
        .disable_drag_drop_handler()
        .initialization_script(format!("window.__ENVELOPE_HOST__ = {host_json};"));
    #[cfg(target_os = "macos")]
    {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true);
    }
    if let Some(material) = host.material {
        let effect = match material {
            Material::Mica => Effect::Mica,
            Material::Vibrancy => Effect::Sidebar,
        };
        // La matière n'apparaît que là où l'interface laisse le fond transparent (voir index.css).
        builder = builder.transparent(true).effects(
            EffectsBuilder::new()
                .effect(effect)
                .state(EffectState::FollowsWindowActiveState)
                .build(),
        );
    }
    builder.build()?;
    Ok(())
}

pub fn run() {
    // Un seul fournisseur crypto rustls pour tout le process (IMAP, SMTP, HTTP).
    let _ = rustls::crypto::ring::default_provider().install_default();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let builder = tauri::Builder::default();
    // Une seule instance : deux Envelope sur la même base (synchro, actions IMAP) se gêneraient.
    // Un second lancement ramène au premier plan la fenêtre déjà ouverte. Ce plugin passe en premier.
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        // Taille, position et état (agrandie…) de la fenêtre retrouvés d'une session à l'autre.
        .plugin(tauri_plugin_window_state::Builder::default().build());
    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            main_window(app)?;
            let data_dir = app.path().app_data_dir()?;
            let db_path = db::db_path(&data_dir);
            let pool = tauri::async_runtime::block_on(db::init(&db_path))?;
            tracing::info!("base SQLite prête : {}", db_path.display());

            let attachments_dir = data_dir.join("attachments");
            let raw_dir = data_dir.join("raw");
            let sync = mail::sync::SyncManager::new(
                pool.clone(),
                app.handle().clone(),
                attachments_dir.clone(),
                raw_dir.clone(),
            );
            app.manage(AppState {
                db: pool,
                db_path,
                attachments_dir,
                raw_dir,
                sync: sync.clone(),
                actions: tokio::sync::Mutex::new(()),
                picks: picks::Picks::default(),
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
            commands::system_appearance,
            commands::list_accounts,
            commands::add_account,
            commands::add_oauth_account,
            commands::detect_servers,
            commands::oauth_status,
            commands::set_oauth_client,
            commands::clear_oauth_client,
            commands::remove_account,
            commands::set_account_signature,
            commands::unsubscribe,
            commands::sync_now,
            commands::list_folders,
            commands::list_messages,
            commands::search_emails,
            commands::send_email,
            commands::save_draft,
            commands::delete_draft,
            commands::open_draft,
            commands::suggest_contacts,
            commands::list_threads,
            commands::get_thread_detail,
            commands::set_thread_read,
            commands::set_thread_starred,
            commands::move_thread,
            commands::delete_thread,
            commands::archive_local,
            commands::export_folder,
            commands::import_pick,
            commands::import_run,
            commands::export_pick,
            commands::create_local_folder,
            commands::rename_local_folder,
            commands::delete_local_folder,
            commands::empty_folder,
            commands::open_attachment,
            commands::inline_images,
            commands::mcp_info,
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de l'application Tauri");
}

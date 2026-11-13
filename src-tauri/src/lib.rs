//! Point d'entrée de la bibliothèque : construit l'application Tauri.
//!
//! Architecture :
//!   frontend (React) ──invoke──▶ commands ──▶ db / mail ◀── mcp (agents IA)

// Déclaré en premier : la macro `msg!` n'est visible que des modules qui suivent.
#[macro_use]
mod error;
mod attachment_protocol;
mod attachments;
mod commands;
mod db;
#[cfg(debug_assertions)]
#[doc(hidden)]
pub mod dev;
mod diskcrypt;
mod lock;
mod mail;
mod mcp;
mod picks;
mod private;
mod reminders;
mod rules;
mod scheduled;
mod templates;
mod tls;
#[cfg(desktop)]
mod update;

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

/// Barrière du verrouillage : tant que l'application est verrouillée, toute commande hors celles du
/// déverrouillage (`lock::ALLOWED_WHEN_LOCKED`) est refusée **ici**, quoi que fasse l'interface.
fn gated<R: tauri::Runtime>(
    inner: impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        let locked = lock::is_locked_or_missing(invoke.message.webview_ref());
        if lock::blocks(locked, invoke.message.command()) {
            invoke.resolver.reject(error::AppError::Auth(msg!(
                "lock.locked",
                "the application is locked"
            )));
            return true;
        }
        inner(invoke)
    }
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
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build());
    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .register_asynchronous_uri_scheme_protocol(
            attachment_protocol::SCHEME,
            |ctx, request, responder| {
                let app = ctx.app_handle().clone();
                let path = request.uri().path().to_string();
                let webview = ctx.webview_label().to_string();
                tauri::async_runtime::spawn(async move {
                    responder.respond(attachment_protocol::serve(&app, &webview, &path).await);
                });
            },
        )
        .setup(|app| {
            main_window(app)?;
            let data_dir = app.path().app_data_dir()?;
            // Développement : base d'essai ailleurs (ex. `seed_photos`), sans toucher à `HOME` (le trousseau du système
            // en dépend). `ENVELOPE_NO_SYNC` : pas de synchro (comptes fictifs).
            #[cfg(debug_assertions)]
            let data_dir = std::env::var_os("ENVELOPE_DATA_DIR").map_or(data_dir, PathBuf::from);
            let db_path = db::db_path(&data_dir);
            let pool = tauri::async_runtime::block_on(db::init(&db_path))?;
            let lock_config = tauri::async_runtime::block_on(lock::load_config(&pool))?;
            tracing::info!("base SQLite prête : {}", db_path.display());

            let attachments_dir = data_dir.join("attachments");
            let raw_dir = data_dir.join("raw");
            let sync = mail::sync::SyncManager::new(
                pool.clone(),
                app.handle().clone(),
                attachments_dir.clone(),
                raw_dir.clone(),
            );
            app.manage(lock::Lock::new(lock_config));
            app.manage(AppState {
                db: pool,
                db_path,
                attachments_dir,
                raw_dir,
                sync: sync.clone(),
                actions: tokio::sync::Mutex::new(()),
                picks: picks::Picks::default(),
            });
            tauri::async_runtime::spawn(scheduled::run(app.handle().clone()));
            tauri::async_runtime::spawn(reminders::run(app.handle().clone()));
            #[cfg(debug_assertions)]
            let no_sync = std::env::var_os("ENVELOPE_NO_SYNC").is_some();
            #[cfg(not(debug_assertions))]
            let no_sync = false;
            tauri::async_runtime::spawn(async move {
                if no_sync {
                    return;
                }
                if let Err(e) = sync.start_all().await {
                    tracing::error!("démarrage de la synchro : {e}");
                }
            });
            Ok(())
        })
        .invoke_handler(gated(tauri::generate_handler![
            commands::app_info,
            commands::lock_status,
            commands::unlock_password,
            commands::unlock_system,
            commands::reset_app,
            commands::lock_now,
            commands::set_lock,
            commands::disk_encryption,
            commands::system_appearance,
            commands::list_accounts,
            commands::add_account,
            commands::add_oauth_account,
            commands::reconnect_oauth_account,
            commands::detect_servers,
            commands::oauth_status,
            commands::set_oauth_client,
            commands::clear_oauth_client,
            commands::remove_account,
            commands::set_account_signature,
            commands::schedule_send,
            commands::list_rules,
            commands::save_rule,
            commands::delete_rule,
            commands::run_rules_now,
            commands::set_reminder,
            commands::clear_reminder,
            commands::list_scheduled,
            commands::cancel_scheduled,
            commands::send_scheduled_now,
            commands::add_identity,
            commands::remove_identity,
            commands::list_templates,
            commands::save_template,
            commands::delete_template,
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
            attachments::open_attachment,
            attachments::inline_images,
            attachments::save_attachments,
            commands::mcp_info,
            #[cfg(desktop)]
            update::check_update,
            #[cfg(desktop)]
            update::install_update,
        ]))
        .run(tauri::generate_context!())
        .expect("erreur au lancement de l'application Tauri");
}

#[cfg(test)]
mod gate_tests {
    use super::*;
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{
        get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY,
    };

    #[tauri::command]
    fn secret() -> &'static str {
        "contenu des e-mails"
    }

    /// Même nom que la vraie commande : celle-ci doit rester joignable verrouillé.
    #[tauri::command]
    fn lock_status(lock: tauri::State<'_, lock::Lock>) -> bool {
        lock.is_locked()
    }

    fn call(
        webview: &tauri::WebviewWindow<MockRuntime>,
        cmd: &str,
    ) -> std::result::Result<tauri::ipc::InvokeResponseBody, serde_json::Value> {
        get_ipc_response(
            webview,
            tauri::webview::InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: if cfg!(windows) {
                    "http://tauri.localhost"
                } else {
                    "tauri://localhost"
                }
                .parse()
                .unwrap(),
                body: InvokeBody::default(),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
    }

    fn webview(
        method: lock::Method,
    ) -> (tauri::App<MockRuntime>, tauri::WebviewWindow<MockRuntime>) {
        let app = mock_builder()
            .manage(lock::Lock::new(lock::Config {
                method,
                timeout_secs: 0,
            }))
            .invoke_handler(gated(tauri::generate_handler![secret, lock_status]))
            .build(mock_context(noop_assets()))
            .unwrap();
        let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        (app, w)
    }

    #[test]
    fn verrouille_seules_les_commandes_de_deverrouillage_passent() {
        let (_app, w) = webview(lock::Method::Password); // verrouillé dès le lancement
        let refused = call(&w, "secret").expect_err("refusée");
        assert_eq!(refused["code"], "lock.locked", "{refused}");
        let status = call(&w, "lock_status").expect("autorisée");
        assert!(status.deserialize::<bool>().unwrap());
    }

    #[test]
    fn deverrouille_ou_sans_verrouillage_tout_passe() {
        let (_app, w) = webview(lock::Method::Off);
        assert_eq!(
            call(&w, "secret").unwrap().deserialize::<String>().unwrap(),
            "contenu des e-mails"
        );
    }
}

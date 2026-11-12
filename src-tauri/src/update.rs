//! Mises à jour de l'application : plugin officiel `tauri-plugin-updater` (natif d'abord).
//!
//! Le plugin lit `latest.json` (adresse dans `tauri.conf.json`), compare les versions et vérifie la signature de
//! l'archive avec la clé publique de la configuration avant d'installer : une archive non signée par la clé du
//! mainteneur est refusée, que les binaires soient signés par Apple/Microsoft ou non. Le webview ne pilote le
//! plugin qu'à travers ces deux commandes (pas de permission `updater:*` côté JS).

use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

use crate::error::{AppError, Result};

fn failed(e: tauri_plugin_updater::Error) -> AppError {
    AppError::Invalid(msg!("update.failed", "update: {detail}", detail = e))
}

async fn find(app: &AppHandle) -> Result<Option<tauri_plugin_updater::Update>> {
    app.updater().map_err(failed)?.check().await.map_err(failed)
}

/// Numéro de la version plus récente que celle installée, s'il y en a une.
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<String>> {
    Ok(find(&app).await?.map(|u| u.version))
}

/// Télécharge, vérifie et installe la dernière version, puis relance l'application. Le canal est relu à dessein :
/// la vérification a pu avoir lieu des heures plus tôt, une version plus récente a pu paraître depuis.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<()> {
    let update = find(&app)
        .await?
        .ok_or_else(|| AppError::NotFound(msg!("not_found.no_update", "no update available")))?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(failed)?;
    app.restart()
}

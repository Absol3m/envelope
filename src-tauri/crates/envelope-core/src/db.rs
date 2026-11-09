//! Emplacement de la base : connaissance partagée entre l'application et le serveur MCP.

use std::path::PathBuf;

/// Identifiant Tauri (`tauri.conf.json`) : le dossier de données de l'application en porte le nom.
/// Un test de l'application vérifie qu'il reste identique.
pub const APP_ID: &str = "dev.envelope.app";
pub const DB_FILE: &str = "envelope.db";

/// Chemin par défaut de la base de l'application (`<dossier de données>/<APP_ID>/envelope.db`).
pub fn default_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(APP_ID).join(DB_FILE))
}

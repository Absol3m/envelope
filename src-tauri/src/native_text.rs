//! Textes affichés hors du webview : invite d'authentification du système, page de fin d'autorisation OAuth dans le
//! navigateur, titres des sélecteurs de fichiers. Ils sont traduits par l'interface (espace `native`), qui envoie le lot
//! au démarrage et à chaque changement de langue ; rien n'est donc rédigé ici. Avant le premier envoi (aucune de ces
//! fenêtres ne peut s'ouvrir avant que l'interface ait démarré), repli sur le fichier anglais lui-même, embarqué.

use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use serde::Deserialize;

/// Textes anglais de l'interface (langue source) : seule source, pas de copie ici.
const ENGLISH: &str = include_str!("../../src/locales/en/native.json");

/// Chaque champ est une clé de `native.json` (l'interface envoie toutes les clés anglaises, traduites ou non) ;
/// clé et champ ne peuvent pas diverger sans faire échouer le test du fichier anglais.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeTexts {
    /// Raison affichée dans l'invite Touch ID / Windows Hello.
    pub unlock_reason: String,
    /// Pages rendues dans le navigateur à la fin de l'autorisation OAuth.
    pub oauth_granted: String,
    pub oauth_denied: String,
    pub oauth_bad_state: String,
    /// Sélecteurs de fichiers natifs.
    pub import_pick_title: String,
    pub export_pick_title: String,
    pub mbox_filter: String,
    pub save_folder_title: String,
}

/// Statique plutôt qu'`AppState` : l'invite du système et le port de rappel OAuth n'ont pas d'`AppHandle` sous la main.
static CURRENT: LazyLock<RwLock<Arc<NativeTexts>>> = LazyLock::new(|| {
    // Fichier du dépôt embarqué à la compilation et vérifié par le test ci-dessous : ne peut pas échouer en production.
    RwLock::new(Arc::new(
        serde_json::from_str(ENGLISH).expect("src/locales/en/native.json lisible"),
    ))
});

/// Textes de la langue active (copie partagée : rien n'est tenu verrouillé pendant l'affichage).
pub fn get() -> Arc<NativeTexts> {
    // Empoisonnement sans conséquence : la valeur n'est jamais laissée à moitié écrite (simple remplacement).
    CURRENT
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Reçoit les textes de la langue choisie dans l'interface.
#[tauri::command]
pub fn set_native_texts(texts: NativeTexts) {
    *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(texts);
}

#[cfg(test)]
mod tests {
    #[test]
    fn english_file_covers_every_field() {
        let texts: super::NativeTexts = serde_json::from_str(super::ENGLISH).unwrap();
        assert_eq!(texts.unlock_reason, "Unlock Envelope");
    }
}

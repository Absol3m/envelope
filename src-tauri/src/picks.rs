//! Chemins choisis par l'utilisateur dans un sélecteur natif **ouvert par l'application**.
//!
//! L'interface (la webview) n'a jamais à fournir un chemin pour importer ou exporter : elle reçoit un
//! jeton opaque et le rend. Ainsi, même une page compromise ne peut pas faire lire ou écrire un chemin
//! que l'utilisateur n'a pas désigné lui-même. Les jetons sont à usage limité : peu nombreux, oubliés
//! après usage, les plus anciens écartés.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_PICKS: usize = 8;

#[derive(Default)]
pub struct Picks(Mutex<VecDeque<(String, PathBuf)>>);

impl Picks {
    /// Mémorise un chemin choisi et renvoie son jeton.
    pub fn remember(&self, path: PathBuf) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        let mut picks = self.0.lock().unwrap_or_else(|e| e.into_inner());
        picks.push_back((token.clone(), path));
        while picks.len() > MAX_PICKS {
            picks.pop_front();
        }
        token
    }

    /// Le chemin désigné par ce jeton, s'il existe encore.
    pub fn get(&self, token: &str) -> Option<PathBuf> {
        let picks = self.0.lock().unwrap_or_else(|e| e.into_inner());
        picks
            .iter()
            .find(|(t, _)| t == token)
            .map(|(_, p)| p.clone())
    }

    pub fn forget(&self, token: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(t, _)| t != token);
    }
}

/// Chemin désigné dans un sélecteur natif.
pub fn dialog_path(p: tauri_plugin_dialog::FilePath) -> crate::error::Result<PathBuf> {
    p.into_path()
        .map_err(|e| crate::error::AppError::Invalid((e.to_string()).into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jetons_opaques_limites_et_oublies() {
        let picks = Picks::default();
        let a = picks.remember(PathBuf::from("/a"));
        assert_eq!(picks.get(&a), Some(PathBuf::from("/a")));
        assert_eq!(
            picks.get("/etc/passwd"),
            None,
            "un chemin n'est pas un jeton"
        );
        picks.forget(&a);
        assert_eq!(picks.get(&a), None);
        // Au-delà de la limite, les plus anciens sont écartés.
        let tokens: Vec<String> = (0..MAX_PICKS + 3)
            .map(|i| picks.remember(PathBuf::from(format!("/p{i}"))))
            .collect();
        assert_eq!(picks.get(&tokens[0]), None);
        assert_eq!(
            picks.get(&tokens[MAX_PICKS + 2]),
            Some(PathBuf::from(format!("/p{}", MAX_PICKS + 2)))
        );
    }
}

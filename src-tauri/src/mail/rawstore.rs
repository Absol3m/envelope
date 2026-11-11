//! Magasin des messages bruts (RFC 5322) : un fichier `.eml` par message, adressé par son
//! empreinte SHA-256 (déduplication gratuite), réparti en sous-dossiers `xx/`.
//!
//! Le message brut est la source de vérité : la base ne garde que l'index (champs analysés, FTS).
//! On peut ainsi ré-analyser, exporter ou remettre un message sur un serveur sans perte.
//! Écriture atomique (fichier temporaire puis renommage) : un arrêt en cours d'écriture ne laisse
//! jamais un `.eml` tronqué sous son vrai nom.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{AppError, Result};

/// Empreinte hexadécimale (64 caractères) : seule forme acceptée dans un chemin, contre toute traversée.
fn valid_hash(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn path_of(dir: &Path, hash: &str) -> Result<PathBuf> {
    if !valid_hash(hash) {
        return Err(AppError::Invalid("empreinte de message invalide".into()));
    }
    Ok(dir.join(&hash[..2]).join(format!("{hash}.eml")))
}

/// Range le message brut ; retourne son empreinte. Sans effet s'il est déjà présent.
pub async fn put(dir: &Path, raw: &[u8]) -> Result<String> {
    let hash = format!("{:x}", Sha256::digest(raw));
    let path = path_of(dir, &hash)?;
    if tokio::fs::try_exists(&path).await? {
        return Ok(hash);
    }
    let parent = path.parent().unwrap_or(dir);
    tokio::fs::create_dir_all(parent).await?;
    let tmp = parent.join(format!(".{hash}.{}.tmp", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, raw).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(hash)
}

/// Empreinte exploitable : présente et non vide (`NULL` = jamais récupéré, `''` = introuvable sur le serveur).
pub fn usable(hash: Option<&str>) -> Option<&str> {
    hash.filter(|h| !h.is_empty())
}

/// Le message brut est-il sur le disque (et non vide) ? Contrôle sans le relire en entier.
pub async fn exists(dir: &Path, hash: &str) -> bool {
    match path_of(dir, hash) {
        Ok(p) => tokio::fs::metadata(p).await.is_ok_and(|m| m.len() > 0),
        Err(_) => false,
    }
}

/// Relit un message brut.
pub async fn get(dir: &Path, hash: &str) -> Result<Vec<u8>> {
    Ok(tokio::fs::read(path_of(dir, hash)?).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("envelope-raw-{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn range_relit_et_deduplique() {
        let dir = tmp();
        let raw = b"From: a@b.fr\r\nSubject: x\r\n\r\ncorps".to_vec();
        let h1 = put(&dir, &raw).await.unwrap();
        let h2 = put(&dir, &raw).await.unwrap();
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
        assert_eq!(get(&dir, &h1).await.unwrap(), raw);
        assert!(dir.join(&h1[..2]).join(format!("{h1}.eml")).exists());
        // Pas de fichier temporaire oublié.
        let stray = std::fs::read_dir(dir.join(&h1[..2]))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            })
            .count();
        assert_eq!(stray, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn refuse_les_empreintes_douteuses() {
        let dir = tmp();
        for bad in [
            "",
            "../../etc/passwd",
            "ABCDEF",
            &"g".repeat(64),
            &"a".repeat(63),
        ] {
            assert!(get(&dir, bad).await.is_err(), "{bad}");
        }
    }
}

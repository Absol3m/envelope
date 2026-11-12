//! Droits d'accès aux données : à l'utilisateur seul (dossiers `0700`, fichiers `0600`) sous Unix.
//!
//! Défense principale : le dossier de données en `0700` (personne d'autre ne peut y entrer) ; les fichiers en `0600`
//! sont une défense en profondeur. Les messages sont stockés en clair (la protection repose sur le chiffrement de disque du système, voir SECURITY.md) :
//! sans cela, un autre compte local pourrait les lire dès que le dossier personnel est traversable (certaines distributions
//! Linux le laissent en `755`). Ni groupe ni « autres », et `root` passe de toute façon outre les droits.
//! Sous Windows le dossier hérite des listes de contrôle d'accès du profil de l'utilisateur : rien à faire ici.

use std::io;
use std::path::Path;

#[cfg(unix)]
const DIR_MODE: u32 = 0o700;
#[cfg(unix)]
const FILE_MODE: u32 = 0o600;

/// Crée le dossier (et ses parents manquants) avec les droits de l'utilisateur seul, et resserre ceux du dossier
/// lui-même s'il existait déjà plus ouvert (installation antérieure).
pub fn create_dir_all_sync(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(DIR_MODE)
            .create(path)?;
        // `mode` ne s'applique qu'aux dossiers créés (et subit l'umask) : on resserre celui-ci s'il existait déjà.
        if std::fs::metadata(path)?.permissions().mode() & 0o777 != DIR_MODE {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(DIR_MODE))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(path)
}

/// Version pour le code asynchrone (même appel bloquant, hors de l'exécuteur).
pub async fn create_dir_all(path: &Path) -> io::Result<()> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || create_dir_all_sync(&path))
        .await
        .map_err(io::Error::other)?
}

/// Écrit un fichier créé avec les droits de l'utilisateur seul.
pub async fn write(path: &Path, data: &[u8]) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(FILE_MODE);
    let mut file = options.open(path).await?;
    file.write_all(data).await?;
    file.flush().await
}

/// Crée le fichier de base de données (vide) avec les droits de l'utilisateur seul s'il n'existe pas, et resserre ceux
/// d'un fichier existant. SQLite donne ensuite ces droits à ses fichiers `-wal` et `-shm`.
pub fn restrict_database(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(FILE_MODE)
            .open(path)?;
        for suffix in ["", "-wal", "-shm"] {
            let mut file = path.as_os_str().to_owned();
            file.push(suffix);
            match std::fs::set_permissions(&file, std::fs::Permissions::from_mode(FILE_MODE)) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                other => other?,
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }
    fn tmp() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("envelope-perm-{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn dossiers_et_fichiers_a_l_utilisateur_seul() {
        let base = tmp();
        let deep = base.join("a").join("b");
        create_dir_all(&deep).await.unwrap();
        assert_eq!(
            (mode(&base), mode(&base.join("a")), mode(&deep)),
            (0o700, 0o700, 0o700),
            "tous les dossiers créés"
        );
        write(&deep.join("f.eml"), b"contenu").await.unwrap();
        assert_eq!(mode(&deep.join("f.eml")), 0o600);
        assert_eq!(std::fs::read(deep.join("f.eml")).unwrap(), b"contenu");
        write(&deep.join("f.eml"), b"x").await.unwrap(); // réécriture : tronque, droits inchangés
        assert_eq!(
            (
                std::fs::read(deep.join("f.eml")).unwrap(),
                mode(&deep.join("f.eml"))
            ),
            (b"x".to_vec(), 0o600)
        );
    }

    #[tokio::test]
    async fn dossier_existant_trop_ouvert_est_resserre() {
        let base = tmp();
        std::fs::create_dir_all(&base).unwrap();
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
        create_dir_all(&base).await.unwrap();
        assert_eq!(mode(&base), 0o700);
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
        create_dir_all_sync(&base).unwrap();
        assert_eq!(mode(&base), 0o700);
    }

    #[test]
    fn fichiers_de_la_base_resserres_y_compris_wal_et_shm() {
        let base = tmp();
        std::fs::create_dir_all(&base).unwrap();
        let db = base.join("envelope.db");
        restrict_database(&db).unwrap(); // absent : créé vide, droits restreints
        assert_eq!(mode(&db), 0o600);
        for name in ["envelope.db-wal", "envelope.db-shm"] {
            std::fs::write(base.join(name), b"x").unwrap();
            std::fs::set_permissions(base.join(name), std::fs::Permissions::from_mode(0o644))
                .unwrap();
        }
        std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();
        restrict_database(&db).unwrap();
        assert_eq!(
            (
                mode(&db),
                mode(&base.join("envelope.db-wal")),
                mode(&base.join("envelope.db-shm"))
            ),
            (0o600, 0o600, 0o600)
        );
    }
}

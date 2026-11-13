//! Pièces jointes : fichier sur le disque (jamais hors du dossier des pièces jointes), ouverture dans l'application du
//! système, enregistrement par le sélecteur natif, images `cid:` du corps. Le protocole `attachment://` sert les octets
//! (`attachment_protocol`).

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, Result};
use crate::mail::ingest;
use crate::picks::dialog_path;
use crate::AppState;
use envelope_core::attachment::{is_image, previewable};

/// Une pièce jointe sur le disque : son fichier, son type et son nom.
pub struct AttachmentFile {
    pub path: std::path::PathBuf,
    pub mime: String,
    pub filename: Option<String>,
}

async fn attachment_path(state: &AppState, attachment_id: i64) -> Result<std::path::PathBuf> {
    Ok(attachment_file(
        &state.db,
        &state.attachments_dir,
        &state.raw_dir,
        attachment_id,
    )
    .await?
    .path)
}

/// Fichier d'une pièce jointe, uniquement s'il est bien dans le dossier des pièces jointes (une base altérée ne
/// doit pas permettre d'ouvrir un fichier arbitraire).
pub async fn attachment_file(
    db: &sqlx::SqlitePool,
    attachments_dir: &std::path::Path,
    raw_dir: &std::path::Path,
    attachment_id: i64,
) -> Result<AttachmentFile> {
    let row: Option<(Option<String>, String, Option<String>)> =
        sqlx::query_as("SELECT storage_path, mime_type, filename FROM attachments WHERE id = ?")
            .bind(attachment_id)
            .fetch_optional(db)
            .await?;
    let (stored, mime, filename) = row
        .ok_or_else(|| AppError::NotFound(msg!("attachment.not_found", "attachment not found")))?;
    let path = match stored {
        Some(p) => std::path::PathBuf::from(p),
        // Historique rapatrié : le contenu est extrait du message brut à la première ouverture.
        None => ingest::materialize_attachment(db, attachments_dir, raw_dir, attachment_id).await?,
    };
    let path = tokio::fs::canonicalize(path).await?;
    let root = tokio::fs::canonicalize(attachments_dir).await?;
    if !path.starts_with(&root) {
        return Err(AppError::Invalid(msg!(
            "attachment.outside_folder",
            "attachment outside the allowed folder"
        )));
    }
    // Sous Windows, `canonicalize` produit un chemin `\\?\C:\…` que certaines applications
    // associées refusent : on le rend sous sa forme habituelle (sans effet ailleurs).
    Ok(AttachmentFile {
        path: dunce::simplified(&path).to_path_buf(),
        mime,
        filename,
    })
}

#[tauri::command]
pub async fn open_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    attachment_id: i64,
) -> Result<()> {
    let path = attachment_path(&state, attachment_id).await?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| AppError::Invalid((e.to_string()).into()))
}

/// Nom de fichier proposé pour une pièce jointe : celui du message, réduit à son dernier élément (jamais de chemin).
fn attachment_file_name(name: Option<&str>) -> String {
    let leaf = name
        .and_then(|n| n.rsplit(['/', '\\']).next())
        .map(|n| n.trim().trim_start_matches('.'))
        .unwrap_or("");
    let clean: String = leaf.chars().filter(|c| !c.is_control()).collect();
    if clean.is_empty() {
        "attachment".into()
    } else {
        clean
    }
}

/// `name`, ou `name (2)`, `name (3)`… si le fichier existe déjà dans `dir`.
async fn free_name(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let taken = |p: &std::path::Path| {
        let p = p.to_path_buf();
        async move { tokio::fs::try_exists(&p).await.unwrap_or(true) }
    };
    let candidate = dir.join(name);
    if !taken(&candidate).await {
        return candidate;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 2.. {
        let p = dir.join(format!("{stem} ({n}){ext}"));
        if !taken(&p).await {
            return p;
        }
    }
    candidate
}

/// Enregistre des pièces jointes : « Enregistrer sous » pour une seule, choix d'un dossier pour plusieurs (les noms
/// déjà pris sont suivis de « (2) »). Le sélecteur est ouvert ici : l'interface ne fournit jamais de chemin.
/// Rend le nombre de fichiers enregistrés (0 si l'utilisateur annule).
#[tauri::command]
pub async fn save_attachments(
    app: AppHandle,
    state: State<'_, AppState>,
    attachment_ids: Vec<i64>,
    title: String, // titre du sélecteur de dossier, traduit par l'interface
) -> Result<usize> {
    use tauri_plugin_dialog::DialogExt;
    let mut files = Vec::new();
    for id in attachment_ids {
        let file = attachment_file(&state.db, &state.attachments_dir, &state.raw_dir, id).await?;
        files.push((file.path, attachment_file_name(file.filename.as_deref())));
    }
    match files.as_slice() {
        [] => Ok(0),
        [(src, name)] => {
            let dialog = app.dialog().file().set_file_name(name);
            let Some(dest) = dialog.blocking_save_file() else {
                return Ok(0);
            };
            tokio::fs::copy(src, dialog_path(dest)?).await?;
            Ok(1)
        }
        _ => {
            let Some(dir) = app.dialog().file().set_title(title).blocking_pick_folder() else {
                return Ok(0);
            };
            let dir = dialog_path(dir)?;
            for (src, name) in &files {
                tokio::fs::copy(src, free_name(&dir, name).await).await?;
            }
            Ok(files.len())
        }
    }
}

/// Images « inline » d'un message (`cid:`) : identifiant de contenu → identifiant de pièce jointe, que l'interface
/// sert par le protocole `attachment://` (même règle d'affichage que l'aperçu : `previewable`, images seulement).
#[tauri::command]
pub async fn inline_images(
    state: State<'_, AppState>,
    message_id: i64,
) -> Result<std::collections::HashMap<String, i64>> {
    let rows: Vec<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT id, content_id, mime_type, size FROM attachments WHERE message_id = ? AND content_id IS NOT NULL",
    )
    .bind(message_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|(_, _, mime, size)| is_image(mime) && previewable(mime, (*size).max(0) as u64))
        .map(|(id, cid, ..)| (cid, id))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nom_propose_sans_chemin_ni_point_initial() {
        assert_eq!(attachment_file_name(Some("../../etc/passwd")), "passwd");
        assert_eq!(attachment_file_name(Some("C:\\Users\\x\\a.pdf")), "a.pdf");
        assert_eq!(attachment_file_name(Some(".bashrc")), "bashrc");
        assert_eq!(attachment_file_name(Some("a\nb.txt")), "ab.txt");
        assert_eq!(attachment_file_name(Some("")), "attachment");
        assert_eq!(attachment_file_name(None), "attachment");
    }

    #[tokio::test]
    async fn nom_libre_dans_le_dossier() {
        let dir = std::env::temp_dir().join(format!("envelope-free-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_name(&dir, "a.pdf").await, dir.join("a.pdf"));
        std::fs::write(dir.join("a.pdf"), b"x").unwrap();
        assert_eq!(free_name(&dir, "a.pdf").await, dir.join("a (2).pdf"));
        std::fs::write(dir.join("a (2).pdf"), b"x").unwrap();
        assert_eq!(free_name(&dir, "a.pdf").await, dir.join("a (3).pdf"));
        std::fs::write(dir.join("noext"), b"x").unwrap();
        assert_eq!(free_name(&dir, "noext").await, dir.join("noext (2)"));
    }
}

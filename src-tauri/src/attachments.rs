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
    let file = attachment_file(
        &state.db,
        &state.attachments_dir,
        &state.raw_dir,
        attachment_id,
    )
    .await?;
    // Le nom vient de l'expéditeur : seuls les types de la liste blanche s'ouvrent d'un clic. On vérifie le nom de
    // l'expéditeur et celui du fichier réellement lancé (`{rang}_{nom assaini}`), puis on pose la quarantaine : le
    // système avertit comme pour tout fichier téléchargé.
    if !is_openable_name(file.filename.as_deref()) || !is_openable_name(file.path.to_str()) {
        let name = attachment_file_name(file.filename.as_deref());
        return Err(AppError::Invalid(msg!(
            "attachment.not_openable",
            "“{name}” is not a type that opens from a message. Save it first if you trust the sender.",
            name = name
        )));
    }
    let path = file.path;
    mark_downloaded(&path);
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| AppError::Invalid((e.to_string()).into()))
}

/// Extensions qui s'ouvrent d'un clic : documents sans code actif, images, audio, vidéo, archives. Liste blanche : une
/// liste noire (exe, scripts, macros, pages web…) est toujours incomplète, et le système exécuterait le reste. La dernière
/// extension du nom décide (« facture.pdf.exe » est un `.exe`) ; sans extension, rien ne s'ouvre. Les macros Office
/// (`docm`, `xlsm`…), les anciens formats Office (`doc`, `xls`, `ppt`), `html` et `svg` n'en font pas partie : ils exécutent du code.
const OPENABLE_EXTENSIONS: &[&str] = &[
    "pdf", "txt", "md", "csv", "rtf", "docx", "xlsx", "pptx", "odt", "ods", "odp", "pages",
    "numbers", "key", "epub", "png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "avif", "bmp",
    "tif", "tiff", "mp3", "m4a", "wav", "aac", "flac", "ogg", "mp4", "mov", "m4v", "webm", "zip",
    "ics", "vcf", "eml",
];

/// Le nom d'une pièce jointe désigne-t-il un type qu'on peut ouvrir sans risque d'exécution ?
fn is_openable_name(name: Option<&str>) -> bool {
    // Nom brut de l'expéditeur : Windows ignore les points et espaces finaux (« a.exe. » est un `.exe`).
    let leaf = name
        .and_then(|n| n.rsplit(['/', '\\']).next())
        .unwrap_or("");
    leaf.trim_end_matches([' ', '.'])
        .rsplit_once('.')
        .is_some_and(|(_, ext)| {
            OPENABLE_EXTENSIONS
                .iter()
                .any(|e| ext.eq_ignore_ascii_case(e))
        })
}

/// Marque le fichier comme venu d'Internet pour que le système l'avertisse à l'ouverture (Gatekeeper sous macOS,
/// Mark of the Web sous Windows). Au mieux : un système de fichiers sans attributs étendus n'empêche pas d'enregistrer.
/// Pas de bibliothèque inutile : `xattr` (macOS) enveloppe l'appel système, le flux NTFS (Windows) s'écrit en std.
fn mark_downloaded(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    {
        // Drapeau « téléchargé, pas encore approuvé » (0081) : même forme que Safari et Mail.
        let value = format!("0081;{:x};Envelope;", crate::clock::now());
        if let Err(e) = xattr::set(path, "com.apple.quarantine", value.as_bytes()) {
            tracing::debug!("quarantaine non posée : {e}");
        }
    }
    #[cfg(windows)]
    {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        // ZoneId 3 = Internet.
        if let Err(e) = std::fs::write(stream, "[ZoneTransfer]\r\nZoneId=3\r\n") {
            tracing::debug!("marque du Web non posée : {e}");
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = path;
}

/// Nom de fichier proposé pour une pièce jointe : celui du message, réduit à son dernier élément (jamais de chemin).
fn attachment_file_name(name: Option<&str>) -> String {
    let leaf = name
        .and_then(|n| n.rsplit(['/', '\\']).next())
        .map(|n| n.trim().trim_start_matches('.'))
        .unwrap_or("");
    // Caractères interdits dans un nom de fichier Windows (`:` crée aussi un flux de données alternatif) : remplacés.
    let clean: String = leaf
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let stem = clean.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    if clean.is_empty() {
        "attachment".into()
    } else if reserved {
        format!("_{clean}")
    } else {
        clean
    }
}

/// Copie une pièce jointe vers `dest` et la marque comme téléchargée (les deux ensemble, pour toute écriture).
async fn copy_marked(src: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    tokio::fs::copy(src, dest).await?;
    mark_downloaded(dest);
    Ok(())
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
            copy_marked(src, &dialog_path(dest)?).await?;
            Ok(1)
        }
        _ => {
            let Some(dir) = app
                .dialog()
                .file()
                .set_title(&crate::native_text::get().save_folder_title)
                .blocking_pick_folder()
            else {
                return Ok(0);
            };
            let dir = dialog_path(dir)?;
            for (src, name) in &files {
                copy_marked(src, &free_name(&dir, name).await).await?;
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
    fn seuls_les_types_surs_s_ouvrent() {
        for name in [
            "facture.pdf",
            "photo.JPG",
            "notes.txt",
            "a.docx",
            "archive.zip",
            "b.pdf ",
        ] {
            assert!(is_openable_name(Some(name)), "{name}");
        }
        for name in [
            "Facture.exe",
            "facture.pdf.exe",
            "a.exe.",
            "x.command",
            "page.html",
            "m.docm",
            "inconnu.xyz",
            "sans-extension",
            ".bashrc",
            "a.pdf/b.sh",
        ] {
            assert!(!is_openable_name(Some(name)), "{name}");
        }
        assert!(!is_openable_name(None));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn quarantaine_posee() {
        let f = std::env::temp_dir().join(format!("envelope-quar-{}.pdf", uuid::Uuid::new_v4()));
        std::fs::write(&f, b"x").unwrap();
        mark_downloaded(&f);
        let v = xattr::get(&f, "com.apple.quarantine").unwrap().unwrap();
        assert!(v.starts_with(b"0081;"));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn nom_propose_sans_chemin_ni_point_initial() {
        assert_eq!(attachment_file_name(Some("../../etc/passwd")), "passwd");
        assert_eq!(attachment_file_name(Some("C:\\Users\\x\\a.pdf")), "a.pdf");
        assert_eq!(attachment_file_name(Some(".bashrc")), "bashrc");
        assert_eq!(attachment_file_name(Some("a\nb.txt")), "ab.txt");
        assert_eq!(attachment_file_name(Some("")), "attachment");
        assert_eq!(attachment_file_name(None), "attachment");
        // Windows : caractères interdits, flux alternatif (`nom:flux`), noms réservés.
        assert_eq!(attachment_file_name(Some("a:b<c>.txt")), "a_b_c_.txt");
        assert_eq!(attachment_file_name(Some("C:x.pdf")), "C_x.pdf");
        assert_eq!(attachment_file_name(Some("NUL.txt")), "_NUL.txt");
        assert_eq!(attachment_file_name(Some("com1")), "_com1");
        assert_eq!(attachment_file_name(Some("console.txt")), "console.txt");
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

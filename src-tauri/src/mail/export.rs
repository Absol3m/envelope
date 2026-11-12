//! Export d'un dossier : un fichier mbox, ou un dossier de fichiers `.eml` (un par message).
//!
//! La source est le message brut (`rawstore`), conservé tel qu'il est arrivé : l'export est sans perte,
//! à deux exceptions près, propres au format mbox : les lignes du corps qui commencent par `From `
//! sont précédées de `>` (sinon un lecteur les prendrait pour le début d'un message), et les
//! drapeaux (lu, suivi) ne sont pas écrits (ils ne font pas partie du message).
//! Les fichiers sont écrits sous un nom temporaire puis renommés : un échec ou un arrêt en cours
//! de route ne laisse jamais un export tronqué sous le nom demandé.

use std::path::{Path, PathBuf};

use chrono::DateTime;
use futures::TryStreamExt;
use serde::Serialize;
use sqlx::SqlitePool;
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::error::{AppError, Result};
use crate::mail::rawstore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Mbox,
    Eml,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub exported: usize,
    /// Messages dont le message brut n'a pas pu être relu (jamais récupéré, ou fichier absent).
    pub missing: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

#[derive(sqlx::FromRow)]
struct Row {
    raw_hash: Option<String>,
    date: String,
    subject: String,
}

async fn count(pool: &SqlitePool, folder_id: i64) -> Result<usize> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE folder_id = ?")
        .bind(folder_id)
        .fetch_one(pool)
        .await?;
    if n == 0 {
        // Un dossier vide ou inexistant : rien à exporter, ce n'est pas une réussite silencieuse.
        let known: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM folders WHERE id = ?")
            .bind(folder_id)
            .fetch_one(pool)
            .await?;
        if known == 0 {
            return Err(AppError::NotFound(msg!(
                "not_found.folder",
                "folder not found: {id}",
                id = folder_id
            )));
        }
    }
    Ok(n as usize)
}

/// Nom de fichier : lettres (accentuées comprises), chiffres et `-`, 40 caractères au plus (jamais de chemin).
pub fn slug(subject: &str) -> String {
    let s: String = subject
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let s: String = s.chars().take(40).collect();
    if s.is_empty() {
        "sans-objet".into()
    } else {
        s
    }
}

/// Date de la ligne « From » d'un mbox (`Mon Mar  2 10:00:00 2026`, en UTC).
fn from_line_date(iso: &str) -> String {
    DateTime::parse_from_rfc3339(iso)
        .map(|d| d.format("%a %b %e %H:%M:%S %Y").to_string())
        .unwrap_or_else(|_| "Thu Jan  1 00:00:00 1970".into())
}

/// Écrit un message dans un mbox : ligne « From », corps (lignes `From ` échappées), ligne vide.
async fn write_mbox_message<W: tokio::io::AsyncWrite + Unpin>(
    out: &mut W,
    date: &str,
    raw: &[u8],
) -> Result<()> {
    out.write_all(format!("From - {}\n", from_line_date(date)).as_bytes())
        .await?;
    for line in raw.split_inclusive(|&b| b == b'\n') {
        if line.starts_with(b"From ") {
            out.write_all(b">").await?;
        }
        out.write_all(line).await?;
    }
    if !raw.ends_with(b"\n") {
        out.write_all(b"\n").await?;
    }
    out.write_all(b"\n").await?; // ligne vide : sépare du message suivant
    Ok(())
}

fn partial(dest: &Path) -> PathBuf {
    let mut name = dest
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".{}.partial", uuid::Uuid::new_v4()));
    dest.with_file_name(name)
}

/// Exporte le dossier dans un fichier mbox (remplacé s'il existe : le sélecteur natif l'a confirmé).
pub async fn export_mbox(
    pool: &SqlitePool,
    raw_dir: &Path,
    folder_id: i64,
    dest: &Path,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<Report> {
    let total = count(pool, folder_id).await?;
    let tmp = partial(dest);
    let result = async {
        let mut out = BufWriter::new(tokio::fs::File::create(&tmp).await?);
        let mut report = Report::default();
        let mut rows = sqlx::query_as::<_, Row>(
            "SELECT raw_hash, date, subject FROM messages WHERE folder_id = ? ORDER BY date, id",
        )
        .bind(folder_id)
        .fetch(pool);
        while let Some(row) = rows.try_next().await? {
            match read_raw(raw_dir, &row).await {
                Some(raw) => {
                    write_mbox_message(&mut out, &row.date, &raw).await?;
                    report.exported += 1;
                }
                None => report.missing += 1,
            }
            if (report.exported + report.missing) % 200 == 0 {
                progress(Progress {
                    done: report.exported + report.missing,
                    total,
                });
            }
        }
        out.flush().await?;
        out.get_mut().sync_all().await?;
        tokio::fs::rename(&tmp, dest).await?;
        progress(Progress { done: total, total });
        Ok::<_, AppError>(report)
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

/// Exporte le dossier en fichiers `.eml` dans `dest_dir` (créé au besoin). Ne remplace jamais un fichier existant.
pub async fn export_eml(
    pool: &SqlitePool,
    raw_dir: &Path,
    folder_id: i64,
    dest_dir: &Path,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<Report> {
    let total = count(pool, folder_id).await?;
    tokio::fs::create_dir_all(dest_dir).await?;
    let mut report = Report::default();
    let mut rows = sqlx::query_as::<_, Row>(
        "SELECT raw_hash, date, subject FROM messages WHERE folder_id = ? ORDER BY date, id",
    )
    .bind(folder_id)
    .fetch(pool);
    let mut n = 0usize;
    while let Some(row) = rows.try_next().await? {
        n += 1;
        let Some(raw) = read_raw(raw_dir, &row).await else {
            report.missing += 1;
            continue;
        };
        let day = row.date.get(..10).unwrap_or("0000-00-00");
        let path = dest_dir.join(format!("{day}-{n:06}-{}.eml", slug(&row.subject)));
        let tmp = partial(&path);
        // Jamais d'écrasement silencieux (et pas de lien physique : absents des clés USB en FAT/exFAT).
        if tokio::fs::try_exists(&path).await? {
            return Err(AppError::Invalid(msg!(
                "export.exists",
                "{path} already exists",
                path = path.display()
            )));
        }
        tokio::fs::write(&tmp, &raw).await?;
        if let Err(e) = tokio::fs::rename(&tmp, &path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e.into());
        }
        report.exported += 1;
        if n % 200 == 0 {
            progress(Progress { done: n, total });
        }
    }
    progress(Progress { done: total, total });
    Ok(report)
}

async fn read_raw(raw_dir: &Path, row: &Row) -> Option<Vec<u8>> {
    let hash = rawstore::usable(row.raw_hash.as_deref())?;
    rawstore::get(raw_dir, hash).await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::ingest::{parse, store_with, Flags, MessageLoc, StoreOpts};
    use crate::mail::mbox::MboxReader;

    const A: &str = "From: a@x.fr\r\nTo: me@x.fr\r\nSubject: Contrat été\r\nDate: Mon, 2 Mar 2026 10:00:00 +0000\r\nMessage-ID: <a@x>\r\n\r\nbonjour\r\n";
    const B: &str = "From: b@x.fr\nTo: me@x.fr\nSubject: Avec From\nDate: Tue, 3 Mar 2026 11:00:00 +0000\nMessage-ID: <b@x>\n\nligne\n\nFrom ici une ligne qui ressemble à un séparateur\nfin sans saut de ligne";

    async fn setup(raws: &[&str]) -> (SqlitePool, PathBuf, i64) {
        let pool = crate::db::init_memory().await.unwrap();
        let base = std::env::temp_dir().join(format!("envelope-export-{}", uuid::Uuid::new_v4()));
        let folder: i64 = sqlx::query_scalar("SELECT id FROM folders WHERE account_id = 'local'")
            .fetch_one(&pool)
            .await
            .unwrap();
        for (i, raw) in raws.iter().enumerate() {
            let hash = rawstore::put(&base.join("raw"), raw.as_bytes())
                .await
                .unwrap();
            let loc = MessageLoc {
                account_id: "local",
                folder_id: folder,
                uid: i as i64 + 1,
                size: None,
            };
            let opts = StoreOpts {
                raw_hash: Some(&hash),
                lazy_attachments: true,
            };
            store_with(
                &pool,
                &base.join("att"),
                loc,
                Flags::default(),
                parse(raw.as_bytes(), None).unwrap(),
                opts,
            )
            .await
            .unwrap();
        }
        (pool, base, folder)
    }

    #[tokio::test]
    async fn mbox_relisible_et_sans_perte() {
        let (pool, base, folder) = setup(&[A, B]).await;
        let dest = base.join("out.mbox");
        let r = export_mbox(&pool, &base.join("raw"), folder, &dest, &|_| {})
            .await
            .unwrap();
        assert_eq!((r.exported, r.missing), (2, 0));
        let text = std::fs::read_to_string(&dest).unwrap();
        assert!(text.starts_with("From - Mon Mar  2 10:00:00 2026\n"));
        // La ligne du corps qui ressemble à un séparateur est échappée ; aucune autre ligne « From » en tête de ligne.
        assert!(text.contains("\n>From ici une ligne"));
        assert_eq!(text.lines().filter(|l| l.starts_with("From ")).count(), 2);
        // Relu par notre lecteur : 2 messages, identiques (hors échappement) à l'original.
        let mut reader = MboxReader::new(tokio::fs::File::open(&dest).await.unwrap());
        let first = reader.next().await.unwrap().unwrap();
        let second = reader.next().await.unwrap().unwrap();
        assert!(reader.next().await.unwrap().is_none());
        assert_eq!(first.raw, A.as_bytes());
        assert_eq!(
            String::from_utf8(second.raw)
                .unwrap()
                .replace("\n>From ici", "\nFrom ici"),
            format!("{B}\n")
        );
        // Pas de fichier temporaire oublié.
        assert!(std::fs::read_dir(&base).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".partial")));
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn eml_un_fichier_par_message_sans_ecrasement() {
        let (pool, base, folder) = setup(&[A, B]).await;
        let dir = base.join("eml");
        let r = export_eml(&pool, &base.join("raw"), folder, &dir, &|_| {})
            .await
            .unwrap();
        assert_eq!((r.exported, r.missing), (2, 0));
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "2026-03-02-000001-Contrat-été.eml",
                "2026-03-03-000002-Avec-From.eml"
            ]
        );
        assert_eq!(std::fs::read(dir.join(&names[0])).unwrap(), A.as_bytes());
        // Relancer ne remplace rien : erreur nette, fichiers intacts.
        assert!(export_eml(&pool, &base.join("raw"), folder, &dir, &|_| {})
            .await
            .is_err());
        assert_eq!(std::fs::read(dir.join(&names[0])).unwrap(), A.as_bytes());
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn messages_sans_brut_comptes_et_dossier_inconnu_refuse() {
        let (pool, base, folder) = setup(&[A]).await;
        sqlx::query("UPDATE messages SET raw_hash = ''")
            .execute(&pool)
            .await
            .unwrap();
        let r = export_mbox(
            &pool,
            &base.join("raw"),
            folder,
            &base.join("o.mbox"),
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!((r.exported, r.missing), (0, 1));
        assert!(export_mbox(
            &pool,
            &base.join("raw"),
            99_999,
            &base.join("x.mbox"),
            &|_| {}
        )
        .await
        .is_err());
        assert!(!base.join("x.mbox").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn noms_de_fichier() {
        assert_eq!(slug("Re: Contrat — été (v2)"), "Re-Contrat-été-v2");
        assert_eq!(slug("../../etc/passwd"), "etc-passwd");
        assert_eq!(slug("   "), "sans-objet");
        assert_eq!(slug(&"a".repeat(100)).len(), 40);
    }
}

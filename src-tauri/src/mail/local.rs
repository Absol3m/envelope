//! Dossiers locaux (« Sur cet ordinateur ») et archivage en local.
//!
//! Archiver en local déplace des messages d'un compte IMAP vers un dossier qui n'existe que sur
//! cette machine, comme les « Dossiers locaux » de Thunderbird mais sans passer par un export :
//! le message brut est conservé tel quel (`rawstore`), l'index est reconstruit à partir de lui.
//!
//! Sécurité des données : le serveur n'est touché qu'**après** que chaque copie locale est écrite
//! et relue. Un échec avant cela ne supprime rien ; un échec de suppression côté serveur laisse
//! une copie des deux côtés (jamais l'inverse).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use chrono::DateTime;
use futures::TryStreamExt;
use sqlx::SqlitePool;

use crate::error::{AppError, Result};
use crate::mail::account::{self, LOCAL_ID};
use crate::mail::actions;
use crate::mail::{imap, ingest, rawstore};

const MAX_NAME: usize = 80;

fn clean_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME
        || name.chars().any(char::is_control)
        || name.contains('/')
    {
        return Err(AppError::Invalid(format!(
            "nom de dossier invalide (1 à {MAX_NAME} caractères, sans « / »)"
        )));
    }
    Ok(name.to_string())
}

async fn ensure_local_folder(pool: &SqlitePool, folder_id: i64) -> Result<()> {
    let account: Option<String> = sqlx::query_scalar("SELECT account_id FROM folders WHERE id = ?")
        .bind(folder_id)
        .fetch_optional(pool)
        .await?;
    match account.as_deref() {
        Some(LOCAL_ID) => Ok(()),
        Some(_) => Err(AppError::Invalid("ce n'est pas un dossier local".into())),
        None => Err(AppError::NotFound(format!("dossier {folder_id}"))),
    }
}

async fn name_taken(pool: &SqlitePool, name: &str, except: Option<i64>) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM folders WHERE account_id = ? AND lower(name) = lower(?) AND id <> ?",
    )
    .bind(LOCAL_ID)
    .bind(name)
    .bind(except.unwrap_or(-1))
    .fetch_one(pool)
    .await?
        > 0)
}

pub async fn create_folder(pool: &SqlitePool, name: &str) -> Result<i64> {
    let name = clean_name(name)?;
    if name_taken(pool, &name, None).await? {
        return Err(AppError::Invalid(format!(
            "le dossier « {name} » existe déjà"
        )));
    }
    Ok(sqlx::query("INSERT INTO folders (account_id, path, name, role, delimiter) VALUES (?1, ?2, ?2, NULL, '/')")
        .bind(LOCAL_ID)
        .bind(&name)
        .execute(pool)
        .await?
        .last_insert_rowid())
}

/// Nom de dossier local valide tiré d'un texte quelconque (nom d'un dossier d'un autre client) :
/// caractères de contrôle et « / » remplacés, 80 caractères au plus ; « Sans nom » s'il ne reste rien.
pub fn sanitize_name(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| if c.is_control() || c == '/' { '-' } else { c })
        .take(MAX_NAME)
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        "Sans nom".into()
    } else {
        s
    }
}

/// Dossier local de ce nom (sans tenir compte de la casse), créé s'il n'existe pas. Sert à l'import.
pub async fn get_or_create_folder(pool: &SqlitePool, name: &str) -> Result<i64> {
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM folders WHERE account_id = ? AND lower(name) = lower(?)",
    )
    .bind(LOCAL_ID)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    match existing {
        Some(id) => Ok(id),
        None => create_folder(pool, name).await,
    }
}

pub async fn rename_folder(pool: &SqlitePool, folder_id: i64, name: &str) -> Result<()> {
    ensure_local_folder(pool, folder_id).await?;
    let name = clean_name(name)?;
    if name_taken(pool, &name, Some(folder_id)).await? {
        return Err(AppError::Invalid(format!(
            "le dossier « {name} » existe déjà"
        )));
    }
    sqlx::query("UPDATE folders SET name = ?1, path = ?1 WHERE id = ?2")
        .bind(&name)
        .bind(folder_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Un dossier local ne se supprime que vide : il contient peut-être la seule copie de ces mails.
pub async fn delete_folder(pool: &SqlitePool, folder_id: i64) -> Result<()> {
    ensure_local_folder(pool, folder_id).await?;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE folder_id = ?")
        .bind(folder_id)
        .fetch_one(pool)
        .await?;
    if n > 0 {
        return Err(AppError::Invalid(format!(
            "ce dossier contient {n} message{} : videz-le d'abord",
            if n > 1 { "s" } else { "" }
        )));
    }
    sqlx::query("DELETE FROM folders WHERE id = ?")
        .bind(folder_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
struct Source {
    id: i64,
    uid: i64,
    account_id: String,
    path: String,
    is_read: bool,
    is_starred: bool,
    raw_hash: Option<String>,
    size: Option<i64>,
    date: String,
    thread_id: Option<i64>,
}

/// Complète le brut des messages qui n'en ont pas encore (ancienne synchro) depuis le serveur.
async fn fetch_missing_raw(pool: &SqlitePool, raw_dir: &Path, rows: &mut [Source]) -> Result<()> {
    let mut wanted: BTreeMap<(String, String), Vec<i64>> = BTreeMap::new();
    for r in rows.iter().filter(|r| no_raw(r)) {
        wanted
            .entry((r.account_id.clone(), r.path.clone()))
            .or_default()
            .push(r.uid);
    }
    for ((account_id, path), uids) in wanted {
        let acct = account::get(pool, &account_id).await?;
        let cred = account::resolve_credential(&acct).await?;
        let mut session = imap::connect(&acct, &cred).await?;
        session.select(&path).await?;
        let set = imap::uid_set(&uids);
        let mut hashes: HashMap<i64, String> = HashMap::new();
        {
            let mut stream = session.uid_fetch(set, "(UID BODY.PEEK[])").await?;
            while let Some(item) = stream.try_next().await? {
                if let (Some(uid), Some(raw)) = (item.uid, item.body()) {
                    hashes.insert(i64::from(uid), rawstore::put(raw_dir, raw).await?);
                }
            }
        }
        let _ = session.logout().await;
        for r in rows
            .iter_mut()
            .filter(|r| r.account_id == account_id && r.path == path)
        {
            if let Some(h) = hashes.get(&r.uid) {
                r.raw_hash = Some(h.clone());
            }
        }
    }
    // Un message introuvable sur le serveur n'est pas archivable : on s'arrête avant d'écrire.
    if let Some(r) = rows.iter().find(|r| no_raw(r)) {
        return Err(AppError::NotFound(format!(
            "message {} introuvable sur le serveur",
            r.uid
        )));
    }
    Ok(())
}

/// Pas de message brut connu (jamais récupéré, ou introuvable sur le serveur).
fn no_raw(s: &Source) -> bool {
    rawstore::usable(s.raw_hash.as_deref()).is_none()
}

/// Résultat de la phase de copie : ce qu'il reste à retirer côté source.
pub struct Copied {
    sources: Vec<Source>,
    new_threads: HashSet<i64>,
}

/// Phase 1 : copie les messages de la conversation (présents dans `from_folder`) dans le dossier
/// local, depuis leur message brut, et relit chaque copie. N'écrit rien côté serveur.
pub async fn copy_to_local(
    pool: &SqlitePool,
    attachments_dir: &Path,
    raw_dir: &Path,
    thread_id: i64,
    from_folder: i64,
    to_folder: i64,
) -> Result<Copied> {
    ensure_local_folder(pool, to_folder).await?;
    let mut sources: Vec<Source> = sqlx::query_as(
        "SELECT m.id, m.uid, m.account_id, f.path, m.is_read, m.is_starred, m.raw_hash, m.size, m.date, m.thread_id \
         FROM messages m JOIN folders f ON f.id = m.folder_id WHERE m.thread_id = ? AND m.folder_id = ? ORDER BY m.uid",
    )
    .bind(thread_id)
    .bind(from_folder)
    .fetch_all(pool)
    .await?;
    if sources.is_empty() {
        return Err(AppError::NotFound(
            "conversation introuvable dans ce dossier".into(),
        ));
    }
    if sources.iter().any(|s| s.account_id == LOCAL_ID) {
        return Err(AppError::Invalid(
            "cette conversation est déjà stockée en local".into(),
        ));
    }
    fetch_missing_raw(pool, raw_dir, &mut sources).await?;

    let mut new_threads = HashSet::new();
    for s in &sources {
        let hash = rawstore::usable(s.raw_hash.as_deref())
            .ok_or_else(|| AppError::NotFound("message brut indisponible".into()))?;
        // Rien à lire ni à analyser si la copie existe déjà (rejeu idempotent).
        if !ingest::has_raw(pool, to_folder, hash).await? {
            let raw = rawstore::get(raw_dir, hash).await?;
            let fallback = DateTime::parse_from_rfc3339(&s.date)
                .ok()
                .map(|d| d.timestamp());
            // Pièces jointes extraites à la demande : inutile de les copier en mémoire.
            let parsed = ingest::parse_with(&raw, fallback, false)
                .ok_or_else(|| AppError::Invalid("message illisible : archivage annulé".into()))?;
            let tid = ingest::store_with(
                pool,
                attachments_dir,
                ingest::MessageLoc {
                    account_id: LOCAL_ID,
                    folder_id: to_folder,
                    uid: ingest::next_uid(pool, to_folder).await?,
                    size: s.size.or(Some(raw.len() as i64)),
                },
                ingest::Flags {
                    seen: s.is_read,
                    flagged: s.is_starred,
                },
                parsed,
                ingest::StoreOpts {
                    raw_hash: Some(hash),
                    lazy_attachments: true,
                },
            )
            .await?;
            new_threads.insert(tid);
        }
        // Relecture : la copie est en base et le brut est bien sur le disque (sans le relire en entier).
        if !ingest::has_raw(pool, to_folder, hash).await? || !rawstore::exists(raw_dir, hash).await
        {
            return Err(AppError::Invalid(
                "copie locale non vérifiée : archivage annulé".into(),
            ));
        }
    }
    Ok(Copied {
        sources,
        new_threads,
    })
}

/// Archive la conversation en local : copie vérifiée, puis suppression côté serveur, puis nettoyage
/// du cache. Retourne les comptes touchés (à resynchroniser).
pub async fn archive(
    pool: &SqlitePool,
    attachments_dir: &Path,
    raw_dir: &Path,
    thread_id: i64,
    from_folder: i64,
    to_folder: i64,
) -> Result<Vec<String>> {
    let Copied {
        sources,
        new_threads,
    } = copy_to_local(
        pool,
        attachments_dir,
        raw_dir,
        thread_id,
        from_folder,
        to_folder,
    )
    .await?;

    // Phase 2 : le serveur, seulement maintenant.
    let mut by_account: BTreeMap<&str, BTreeMap<&str, Vec<i64>>> = BTreeMap::new();
    for s in &sources {
        by_account
            .entry(&s.account_id)
            .or_default()
            .entry(&s.path)
            .or_default()
            .push(s.uid);
    }
    for (account_id, folders) in &by_account {
        let acct = account::get(pool, account_id).await?;
        let cred = account::resolve_credential(&acct).await?;
        let mut session = imap::connect(&acct, &cred).await?;
        for (path, uids) in folders {
            actions::delete_on_server(&mut session, path, uids)
                .await
                .map_err(|e| {
                    AppError::Invalid(format!("copié en local mais pas supprimé du serveur : {e}"))
                })?;
        }
        let _ = session.logout().await;
    }

    // Phase 3 : le cache.
    let mut tx = pool.begin().await?;
    for s in &sources {
        sqlx::query("DELETE FROM messages WHERE id = ?")
            .bind(s.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    let mut touched: HashSet<i64> = new_threads;
    touched.extend(sources.iter().filter_map(|s| s.thread_id));
    ingest::refresh_threads(pool, &touched).await?;
    ingest::refresh_folder_unread(pool, from_folder).await?;
    ingest::refresh_folder_unread(pool, to_folder).await?;
    Ok(by_account.keys().map(|a| a.to_string()).collect())
}

/// Déplace la conversation d'un dossier local à un autre (base seulement).
pub async fn move_within(
    pool: &SqlitePool,
    thread_id: i64,
    from_folder: i64,
    to_folder: i64,
) -> Result<()> {
    ensure_local_folder(pool, from_folder).await?;
    ensure_local_folder(pool, to_folder).await?;
    if from_folder == to_folder {
        return Err(AppError::Invalid(
            "la conversation est déjà dans ce dossier".into(),
        ));
    }
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM messages WHERE thread_id = ? AND folder_id = ? ORDER BY uid",
    )
    .bind(thread_id)
    .bind(from_folder)
    .fetch_all(pool)
    .await?;
    for id in ids {
        let uid = ingest::next_uid(pool, to_folder).await?;
        sqlx::query("UPDATE messages SET folder_id = ?, uid = ? WHERE id = ?")
            .bind(to_folder)
            .bind(uid)
            .bind(id)
            .execute(pool)
            .await?;
    }
    ingest::refresh_threads(pool, &HashSet::from([thread_id])).await?;
    ingest::refresh_folder_unread(pool, from_folder).await?;
    ingest::refresh_folder_unread(pool, to_folder).await
}

/// Supprime définitivement les messages de la conversation présents dans ce dossier local.
/// (Le message brut reste dans le magasin jusqu'au prochain nettoyage.)
pub async fn delete_messages(pool: &SqlitePool, thread_id: i64, folder_id: i64) -> Result<()> {
    ensure_local_folder(pool, folder_id).await?;
    let done = sqlx::query("DELETE FROM messages WHERE thread_id = ? AND folder_id = ?")
        .bind(thread_id)
        .bind(folder_id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(AppError::NotFound(
            "rien à supprimer dans ce dossier".into(),
        ));
    }
    ingest::refresh_threads(pool, &HashSet::from([thread_id])).await?;
    ingest::refresh_folder_unread(pool, folder_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::ingest::{parse, store_with, Flags, MessageLoc, StoreOpts};

    const RAW: &str = "From: Zoé <z@x.fr>\r\nTo: a@x.fr\r\nSubject: Contrat\r\nDate: Mon, 2 Mar 2026 10:00:00 +0000\r\nMessage-ID: <c1@x>\r\n\r\nclauses du contrat\r\n";

    async fn setup() -> (SqlitePool, std::path::PathBuf, i64) {
        let pool = crate::db::init_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','a@x.fr','a','i','s')")
            .execute(&pool).await.unwrap();
        let inbox = sqlx::query(
            "INSERT INTO folders (account_id,path,name,role) VALUES ('a','INBOX','INBOX','inbox')",
        )
        .execute(&pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let base = std::env::temp_dir().join(format!("envelope-local-{}", uuid::Uuid::new_v4()));
        (pool, base, inbox)
    }

    async fn local_folder(pool: &SqlitePool, name: &str) -> i64 {
        sqlx::query_scalar("SELECT id FROM folders WHERE account_id = 'local' AND name = ?")
            .bind(name)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// Un message déjà synchronisé (brut rangé), tel que l'archivage le trouve.
    async fn synced(pool: &SqlitePool, base: &Path, folder: i64) -> i64 {
        let hash = rawstore::put(&base.join("raw"), RAW.as_bytes())
            .await
            .unwrap();
        let loc = MessageLoc {
            account_id: "a",
            folder_id: folder,
            uid: 7,
            size: None,
        };
        let opts = StoreOpts {
            raw_hash: Some(&hash),
            lazy_attachments: false,
        };
        store_with(
            pool,
            &base.join("att"),
            loc,
            Flags {
                seen: false,
                flagged: true,
            },
            parse(RAW.as_bytes(), None).unwrap(),
            opts,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn le_compte_local_existe_avec_son_dossier_par_defaut() {
        let (pool, _, _) = setup().await;
        assert!(local_folder(&pool, "Archives").await > 0);
        // Il n'est ni un compte de messagerie ni supprimable.
        assert!(account::list(&pool)
            .await
            .unwrap()
            .iter()
            .all(|a| a.id != LOCAL_ID));
        assert!(account::delete(&pool, LOCAL_ID).await.is_err());
    }

    #[tokio::test]
    async fn dossiers_noms_et_suppression() {
        let (pool, base, _) = setup().await;
        let id = create_folder(&pool, "  Clients 2024 ").await.unwrap();
        assert!(
            create_folder(&pool, "clients 2024").await.is_err(),
            "unicité sans tenir compte de la casse"
        );
        for bad in ["", "   ", "a/b", &"x".repeat(81), "ligne\nbrisée"] {
            assert!(create_folder(&pool, bad).await.is_err(), "{bad:?}");
        }
        rename_folder(&pool, id, "Clients").await.unwrap();
        assert!(rename_folder(&pool, id, "Archives").await.is_err());
        // Un dossier de compte n'est pas un dossier local.
        let inbox: i64 = sqlx::query_scalar("SELECT id FROM folders WHERE role = 'inbox'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(rename_folder(&pool, inbox, "X").await.is_err());
        assert!(delete_folder(&pool, inbox).await.is_err());
        // Un dossier non vide ne se supprime pas.
        let t = synced(&pool, &base, inbox).await;
        copy_to_local(&pool, &base.join("att"), &base.join("raw"), t, inbox, id)
            .await
            .unwrap();
        assert!(delete_folder(&pool, id)
            .await
            .unwrap_err()
            .to_string()
            .contains("videz-le"));
        let local_thread: i64 =
            sqlx::query_scalar("SELECT thread_id FROM messages WHERE folder_id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        delete_messages(&pool, local_thread, id).await.unwrap();
        delete_folder(&pool, id).await.unwrap();
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn la_copie_locale_garde_le_brut_les_drapeaux_et_est_idempotente() {
        let (pool, base, inbox) = setup().await;
        let archives = local_folder(&pool, "Archives").await;
        let t = synced(&pool, &base, inbox).await;
        let (att, raw) = (base.join("att"), base.join("raw"));

        let c = copy_to_local(&pool, &att, &raw, t, inbox, archives)
            .await
            .unwrap();
        assert_eq!(c.sources.len(), 1);
        let (hash, seen, flagged, account, subject): (String, bool, bool, String, String) = sqlx::query_as(
            "SELECT raw_hash, is_read, is_starred, account_id, subject FROM messages WHERE folder_id = ?")
            .bind(archives).fetch_one(&pool).await.unwrap();
        assert_eq!(
            (seen, flagged, account.as_str(), subject.as_str()),
            (false, true, "local", "Contrat")
        );
        assert_eq!(rawstore::get(&raw, &hash).await.unwrap(), RAW.as_bytes());
        // La source n'a pas bougé : la suppression est l'étape suivante, côté serveur.
        let src: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE folder_id = ?")
            .bind(inbox)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(src, 1);
        // Rejouer ne crée pas de doublon.
        copy_to_local(&pool, &att, &raw, t, inbox, archives)
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE folder_id = ?")
            .bind(archives)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1);
        // Le dossier de destination doit être local ; une conversation déjà locale n'est pas réarchivée.
        assert!(copy_to_local(&pool, &att, &raw, t, inbox, inbox)
            .await
            .is_err());
        let lt: i64 = sqlx::query_scalar("SELECT thread_id FROM messages WHERE folder_id = ?")
            .bind(archives)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(copy_to_local(&pool, &att, &raw, lt, archives, archives)
            .await
            .is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn deplacement_entre_dossiers_locaux() {
        let (pool, base, inbox) = setup().await;
        let archives = local_folder(&pool, "Archives").await;
        let clients = create_folder(&pool, "Clients").await.unwrap();
        let t = synced(&pool, &base, inbox).await;
        copy_to_local(
            &pool,
            &base.join("att"),
            &base.join("raw"),
            t,
            inbox,
            archives,
        )
        .await
        .unwrap();
        let lt: i64 = sqlx::query_scalar("SELECT thread_id FROM messages WHERE folder_id = ?")
            .bind(archives)
            .fetch_one(&pool)
            .await
            .unwrap();
        move_within(&pool, lt, archives, clients).await.unwrap();
        let (n_old, n_new): (i64, i64) = (
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE folder_id = ?")
                .bind(archives)
                .fetch_one(&pool)
                .await
                .unwrap(),
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE folder_id = ?")
                .bind(clients)
                .fetch_one(&pool)
                .await
                .unwrap(),
        );
        assert_eq!((n_old, n_new), (0, 1));
        assert!(move_within(&pool, lt, clients, clients).await.is_err());
        let _ = std::fs::remove_dir_all(base);
    }
}

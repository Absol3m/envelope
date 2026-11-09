//! Actions de l'utilisateur sur une conversation : lu / non lu, suivi, archivage.
//!
//! Ordre : **serveur d'abord, base ensuite**. Si IMAP refuse, la base locale n'a pas bougé et
//! l'erreur remonte à l'interface (qui annule son affichage optimiste) : aucune divergence à
//! réconcilier. Une connexion IMAP est ouverte par action ; l'interface masque la latence.

use std::collections::{BTreeMap, HashSet};

use futures::TryStreamExt;
use sqlx::SqlitePool;

use crate::error::{AppError, Result};
use crate::mail::imap::ImapSession;
use crate::mail::{account, imap, ingest};

#[derive(Debug, Clone, Copy)]
pub enum Action {
    SetRead(bool),
    SetStarred(bool),
    /// Archive les messages de la conversation présents dans ce dossier.
    Archive {
        folder_id: i64,
    },
}

/// Un message à modifier.
#[derive(Debug, sqlx::FromRow)]
struct Target {
    id: i64,
    account_id: String,
    folder_id: i64,
    folder_path: String,
    uid: i64,
}

/// Messages touchés par l'action (ceux dont l'état est déjà la cible sont ignorés).
async fn targets(pool: &SqlitePool, thread_id: i64, action: Action) -> Result<Vec<Target>> {
    let filter = match action {
        Action::SetRead(read) => format!("AND m.is_read <> {}", i32::from(read)),
        Action::SetStarred(on) => format!("AND m.is_starred <> {}", i32::from(on)),
        Action::Archive { folder_id } => format!("AND m.folder_id = {folder_id}"),
    };
    Ok(sqlx::query_as(&format!(
        "SELECT m.id, m.account_id, m.folder_id, f.path AS folder_path, m.uid \
         FROM messages m JOIN folders f ON f.id = m.folder_id \
         WHERE m.thread_id = ? {filter} ORDER BY m.account_id, f.path, m.uid"
    ))
    .bind(thread_id)
    .fetch_all(pool)
    .await?)
}

/// Applique l'action sur le serveur puis en base.
pub async fn apply(pool: &SqlitePool, thread_id: i64, action: Action) -> Result<()> {
    let targets = targets(pool, thread_id, action).await?;
    if targets.is_empty() {
        return Ok(());
    }
    // Une connexion par compte, un SELECT par dossier.
    let mut by_account: BTreeMap<&str, BTreeMap<&str, Vec<i64>>> = BTreeMap::new();
    for t in &targets {
        by_account
            .entry(&t.account_id)
            .or_default()
            .entry(&t.folder_path)
            .or_default()
            .push(t.uid);
    }
    for (account_id, folders) in by_account {
        let acct = account::get(pool, account_id).await?;
        let cred = account::resolve_credential(&acct).await?;
        let mut session = imap::connect(&acct, &cred).await?;
        for (path, uids) in folders {
            let set = uids
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            propagate(&mut session, pool, account_id, path, &set, action).await?;
        }
        let _ = session.logout().await;
    }
    apply_local(pool, thread_id, &targets, action).await
}

async fn propagate(
    session: &mut ImapSession,
    pool: &SqlitePool,
    account_id: &str,
    path: &str,
    uids: &str,
    action: Action,
) -> Result<()> {
    session.select(path).await?;
    match action {
        Action::SetRead(on) => store_flag(session, uids, "\\Seen", on).await,
        Action::SetStarred(on) => store_flag(session, uids, "\\Flagged", on).await,
        Action::Archive { .. } => {
            let dest = archive_path(pool, account_id).await?;
            move_to(session, uids, &dest).await
        }
    }
}

async fn store_flag(session: &mut ImapSession, uids: &str, flag: &str, on: bool) -> Result<()> {
    let op = if on { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
    session
        .uid_store(uids, format!("{op} ({flag})"))
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    Ok(())
}

/// `UID MOVE` (RFC 6851) si le serveur le supporte, sinon COPY + \Deleted + EXPUNGE.
async fn move_to(session: &mut ImapSession, uids: &str, dest: &str) -> Result<()> {
    if session.capabilities().await?.has_str("MOVE") {
        session.uid_mv(uids, dest).await?;
    } else {
        session.uid_copy(uids, dest).await?;
        store_flag(session, uids, "\\Deleted", true).await?;
        session
            .uid_expunge(uids)
            .await?
            .try_collect::<Vec<_>>()
            .await?;
    }
    Ok(())
}

/// Dossier d'archive du compte ; Gmail n'en a pas, « Tous les messages » en tient lieu.
async fn archive_path(pool: &SqlitePool, account_id: &str) -> Result<String> {
    sqlx::query_scalar("SELECT path FROM folders WHERE account_id = ? AND role IN ('archive','all') ORDER BY role = 'archive' DESC LIMIT 1")
        .bind(account_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::Invalid("aucun dossier d'archive sur ce compte".into()))
}

/// Aligne la base une fois le serveur à jour.
async fn apply_local(
    pool: &SqlitePool,
    thread_id: i64,
    targets: &[Target],
    action: Action,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    for t in targets {
        match action {
            Action::SetRead(read) => sqlx::query("UPDATE messages SET is_read = ? WHERE id = ?")
                .bind(read)
                .bind(t.id),
            Action::SetStarred(on) => {
                sqlx::query("UPDATE messages SET is_starred = ? WHERE id = ?")
                    .bind(on)
                    .bind(t.id)
            }
            Action::Archive { .. } => sqlx::query("DELETE FROM messages WHERE id = ?").bind(t.id),
        }
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    ingest::refresh_threads(pool, &HashSet::from([thread_id])).await?;
    for folder_id in targets.iter().map(|t| t.folder_id).collect::<HashSet<_>>() {
        ingest::refresh_folder_unread(pool, folder_id).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 conversation de 2 messages dans INBOX (1 non lu) ; retourne (pool, folder_id).
    async fn setup() -> (SqlitePool, i64) {
        let pool = crate::db::init_memory().await.unwrap();
        for sql in [
            "INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','a@x.fr','a','i','s')",
            "INSERT INTO folders (id,account_id,path,name,role) VALUES (1,'a','INBOX','INBOX','inbox')",
            "INSERT INTO threads (id,account_id,subject,last_message_at) VALUES (1,'a','S','2026-03-02T10:00:00Z')",
            "INSERT INTO messages (id,account_id,folder_id,thread_id,uid,message_id,subject,from_addr,date,is_read) VALUES (1,'a',1,1,1,'m1','S','x@y.fr','2026-03-02T10:00:00Z',1)",
            "INSERT INTO messages (id,account_id,folder_id,thread_id,uid,message_id,subject,from_addr,date,is_read) VALUES (2,'a',1,1,2,'m2','S','x@y.fr','2026-03-02T11:00:00Z',0)",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        (pool, 1)
    }

    #[tokio::test]
    async fn only_messages_that_differ_are_targeted() {
        let (pool, folder) = setup().await;
        assert_eq!(
            targets(&pool, 1, Action::SetRead(true))
                .await
                .unwrap()
                .len(),
            1,
            "seul le non lu change"
        );
        assert_eq!(
            targets(&pool, 1, Action::SetRead(false))
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            targets(&pool, 1, Action::Archive { folder_id: folder })
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            targets(&pool, 1, Action::Archive { folder_id: 99 })
                .await
                .unwrap()
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn local_read_star_and_archive_refresh_aggregates() {
        let (pool, folder) = setup().await;
        let t = targets(&pool, 1, Action::SetRead(true)).await.unwrap();
        apply_local(&pool, 1, &t, Action::SetRead(true))
            .await
            .unwrap();
        let (unread, folder_unread): (i64, i64) =
            sqlx::query_as("SELECT (SELECT unread_count FROM threads WHERE id=1), (SELECT unread_count FROM folders WHERE id=1)").fetch_one(&pool).await.unwrap();
        assert_eq!((unread, folder_unread), (0, 0));

        let t = targets(&pool, 1, Action::SetStarred(true)).await.unwrap();
        apply_local(&pool, 1, &t, Action::SetStarred(true))
            .await
            .unwrap();
        assert!(
            sqlx::query_scalar::<_, bool>("SELECT is_starred FROM threads WHERE id=1")
                .fetch_one(&pool)
                .await
                .unwrap()
        );

        let a = Action::Archive { folder_id: folder };
        let t = targets(&pool, 1, a).await.unwrap();
        apply_local(&pool, 1, &t, a).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM threads")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0,
            "conversation vide supprimée"
        );
    }

    #[tokio::test]
    async fn archive_needs_an_archive_folder() {
        let (pool, _) = setup().await;
        assert!(archive_path(&pool, "a").await.is_err());
        sqlx::query("INSERT INTO folders (account_id,path,name,role) VALUES ('a','[Gmail]/All Mail','All Mail','all')").execute(&pool).await.unwrap();
        assert_eq!(archive_path(&pool, "a").await.unwrap(), "[Gmail]/All Mail");
        sqlx::query("INSERT INTO folders (account_id,path,name,role) VALUES ('a','Archive','Archive','archive')").execute(&pool).await.unwrap();
        assert_eq!(
            archive_path(&pool, "a").await.unwrap(),
            "Archive",
            "le dossier d'archive prime sur « Tous les messages »"
        );
    }
}

//! Actions de l'utilisateur sur une conversation : lu / non lu, suivi, archivage.
//!
//! Ordre : **serveur d'abord, base ensuite**. Si IMAP refuse, la base locale n'a pas bougé et
//! l'erreur remonte à l'interface (qui annule son affichage optimiste) : aucune divergence à
//! réconcilier. Une connexion IMAP est ouverte par action ; l'interface masque la latence.

use std::collections::{BTreeMap, HashSet};

use futures::TryStreamExt;
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use crate::error::{AppError, Result};
use crate::mail::imap::ImapSession;
use crate::mail::{account, imap, ingest};

#[derive(Debug, Clone, Copy)]
pub enum Action {
    SetRead(bool),
    SetStarred(bool),
    /// Déplace les messages de la conversation présents dans ce dossier.
    Move {
        folder_id: i64,
        to: Destination,
    },
    /// Supprime définitivement les messages de la conversation présents dans ce dossier, qui doit
    /// être la corbeille ou le spam (aucune autre boîte n'est supprimable d'un geste).
    Delete {
        folder_id: i64,
    },
}

/// Dossier de destination d'un déplacement, désigné par son rôle (les chemins varient selon le
/// serveur et la langue).
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Destination {
    Inbox,
    Archive,
    Trash,
    Junk,
}

impl Destination {
    /// Rôles acceptés, par ordre de préférence ; Gmail n'a pas d'archive : « Tous les messages »
    /// en tient lieu.
    fn roles(self) -> &'static [&'static str] {
        match self {
            Self::Inbox => &["inbox"],
            Self::Archive => &["archive", "all"],
            Self::Trash => &["trash"],
            Self::Junk => &["junk"],
        }
    }

    /// Nom du dossier à créer sur le serveur s'il n'en existe aucun (boîte neuve) ; son rôle est
    /// le premier de `roles()`.
    fn default_name(self) -> &'static str {
        match self {
            Self::Inbox => "INBOX",
            Self::Archive => "Archive",
            Self::Trash => "Trash",
            Self::Junk => "Junk",
        }
    }
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
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT m.id, m.account_id, m.folder_id, f.path AS folder_path, m.uid \
         FROM messages m JOIN folders f ON f.id = m.folder_id WHERE m.thread_id = ",
    );
    qb.push_bind(thread_id);
    match action {
        Action::SetRead(read) => qb.push(" AND m.is_read <> ").push_bind(read),
        Action::SetStarred(on) => qb.push(" AND m.is_starred <> ").push_bind(on),
        Action::Move { folder_id, .. } => qb.push(" AND m.folder_id = ").push_bind(folder_id),
        Action::Delete { folder_id } => qb
            .push(" AND f.role IN ('trash','junk') AND m.folder_id = ")
            .push_bind(folder_id),
    };
    qb.push(" ORDER BY m.account_id, f.path, m.uid");
    Ok(qb.build_query_as().fetch_all(pool).await?)
}

/// Applique l'action sur le serveur puis en base ; retourne les comptes touchés (la conversation,
/// elle, peut avoir disparu de la base si elle est devenue vide).
pub async fn apply(pool: &SqlitePool, thread_id: i64, action: Action) -> Result<Vec<String>> {
    let targets = targets(pool, thread_id, action).await?;
    if targets.is_empty() {
        // Les autres actions sont idempotentes ; supprimer pour de bon ne doit jamais « réussir »
        // à vide (dossier qui n'est ni la corbeille ni le spam, ou conversation déjà supprimée).
        if matches!(action, Action::Delete { .. }) {
            return Err(AppError::Invalid(
                "rien à supprimer : seuls les messages de la corbeille ou du spam le peuvent"
                    .into(),
            ));
        }
        return Ok(Vec::new());
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
    let accounts: Vec<String> = by_account.keys().map(|a| a.to_string()).collect();
    for (account_id, folders) in by_account {
        if account_id == account::LOCAL_ID {
            continue; // dossiers locaux : pas de serveur, la base suffit
        }
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
    apply_local(pool, thread_id, &targets, action).await?;
    Ok(accounts)
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
        Action::SetRead(on) => store_flag(session, uids, "\\Seen", on).await?,
        Action::SetStarred(on) => store_flag(session, uids, "\\Flagged", on).await?,
        Action::Delete { .. } => {
            store_flag(session, uids, "\\Deleted", true).await?;
            expunge(session, uids).await?;
        }
        Action::Move { to, .. } => {
            let dest = match destination_path(pool, account_id, to).await? {
                Some(path) => path,
                None => imap::find_or_create(session, to.roles()[0], to.default_name()).await?,
            };
            // Gmail : « Archives » est « Tous les messages » ; déplacer un message vers son propre
            // dossier le copierait puis l'effacerait.
            if dest == path {
                return Err(AppError::Invalid(
                    "la conversation est déjà dans ce dossier".into(),
                ));
            }
            move_to(session, uids, &dest).await?;
        }
    }
    Ok(())
}

pub(crate) async fn store_flag(
    session: &mut ImapSession,
    uids: &str,
    flag: &str,
    on: bool,
) -> Result<()> {
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
        expunge(session, uids).await?;
    }
    Ok(())
}

/// Efface les messages marqués `\Deleted` de cet ensemble d'UID.
pub(crate) async fn expunge(session: &mut ImapSession, uids: &str) -> Result<()> {
    session
        .uid_expunge(uids)
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    Ok(())
}

/// Chemin du dossier de destination connu de la synchro, s'il y en a un.
async fn destination_path(
    pool: &SqlitePool,
    account_id: &str,
    to: Destination,
) -> Result<Option<String>> {
    for role in to.roles() {
        let path = sqlx::query_scalar("SELECT path FROM folders WHERE account_id = ? AND role = ?")
            .bind(account_id)
            .bind(role)
            .fetch_optional(pool)
            .await?;
        if let Some(path) = path {
            return Ok(Some(path));
        }
    }
    Ok(None)
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
            Action::Move { .. } | Action::Delete { .. } => {
                sqlx::query("DELETE FROM messages WHERE id = ?").bind(t.id)
            }
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

/// Vide la corbeille ou le spam d'un compte : serveur d'abord, base ensuite.
pub async fn empty_folder(pool: &SqlitePool, folder_id: i64) -> Result<()> {
    let (account_id, path, role): (String, String, Option<String>) =
        sqlx::query_as("SELECT account_id, path, role FROM folders WHERE id = ?")
            .bind(folder_id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("dossier {folder_id}")))?;
    if !matches!(role.as_deref(), Some("trash" | "junk")) {
        return Err(AppError::Invalid(
            "seuls la corbeille et le spam peuvent être vidés".into(),
        ));
    }
    let threads: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT thread_id FROM messages WHERE folder_id = ? AND thread_id IS NOT NULL",
    )
    .bind(folder_id)
    .fetch_all(pool)
    .await?;

    let acct = account::get(pool, &account_id).await?;
    let cred = account::resolve_credential(&acct).await?;
    let mut session = imap::connect(&acct, &cred).await?;
    let status = session.select(&path).await?;
    if status.exists > 0 {
        store_flag(&mut session, "1:*", "\\Deleted", true).await?;
        expunge(&mut session, "1:*").await?;
    }
    let _ = session.logout().await;

    sqlx::query("DELETE FROM messages WHERE folder_id = ?")
        .bind(folder_id)
        .execute(pool)
        .await?;
    ingest::refresh_threads(pool, &threads.into_iter().collect()).await?;
    ingest::refresh_folder_unread(pool, folder_id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 conversation de 2 messages dans INBOX (1 non lu) ; retourne (pool, folder_id).
    async fn setup() -> (SqlitePool, i64) {
        let pool = crate::db::init_memory().await.unwrap();
        // Ces tests fixent leurs identifiants : on écarte le compte local créé par la migration.
        sqlx::query("DELETE FROM accounts WHERE id = 'local'")
            .execute(&pool)
            .await
            .unwrap();
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
            targets(
                &pool,
                1,
                Action::Move {
                    folder_id: folder,
                    to: Destination::Archive
                }
            )
            .await
            .unwrap()
            .len(),
            2
        );
        assert_eq!(
            targets(
                &pool,
                1,
                Action::Move {
                    folder_id: 99,
                    to: Destination::Archive
                }
            )
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

        let a = Action::Move {
            folder_id: folder,
            to: Destination::Archive,
        };
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
    async fn destination_is_found_by_role() {
        use Destination::*;
        let (pool, _) = setup().await;
        assert!(destination_path(&pool, "a", Archive)
            .await
            .unwrap()
            .is_none());
        sqlx::query("INSERT INTO folders (account_id,path,name,role) VALUES ('a','[Gmail]/All Mail','All Mail','all')").execute(&pool).await.unwrap();
        assert_eq!(
            destination_path(&pool, "a", Archive)
                .await
                .unwrap()
                .as_deref(),
            Some("[Gmail]/All Mail")
        );
        sqlx::query("INSERT INTO folders (account_id,path,name,role) VALUES ('a','Archive','Archive','archive')").execute(&pool).await.unwrap();
        assert_eq!(
            destination_path(&pool, "a", Archive)
                .await
                .unwrap()
                .as_deref(),
            Some("Archive"),
            "le dossier d'archive prime sur « Tous les messages »"
        );
        assert!(destination_path(&pool, "a", Trash).await.unwrap().is_none());
        sqlx::query("INSERT INTO folders (account_id,path,name,role) VALUES ('a','Corbeille','Corbeille','trash')").execute(&pool).await.unwrap();
        assert_eq!(
            destination_path(&pool, "a", Trash)
                .await
                .unwrap()
                .as_deref(),
            Some("Corbeille")
        );
        assert!(destination_path(&pool, "a", Junk).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_only_targets_trash_and_junk() {
        let (pool, inbox) = setup().await;
        assert!(
            targets(&pool, 1, Action::Delete { folder_id: inbox })
                .await
                .unwrap()
                .is_empty(),
            "la boîte de réception n'est jamais supprimable d'un geste"
        );
        sqlx::query("INSERT INTO folders (id,account_id,path,name,role) VALUES (7,'a','Trash','Trash','trash')").execute(&pool).await.unwrap();
        sqlx::query("UPDATE messages SET folder_id = 7 WHERE id = 2")
            .execute(&pool)
            .await
            .unwrap();
        let t = targets(&pool, 1, Action::Delete { folder_id: 7 })
            .await
            .unwrap();
        assert_eq!(t.iter().map(|m| m.id).collect::<Vec<_>>(), vec![2]);
    }

    #[tokio::test]
    async fn empty_folder_refuses_other_boxes() {
        let (pool, inbox) = setup().await;
        assert!(empty_folder(&pool, inbox).await.is_err());
        assert!(empty_folder(&pool, 999).await.is_err());
    }
}

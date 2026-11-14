//! Règles locales : une condition (la syntaxe de la recherche, voir `envelope_core::search::parse_search`) et des
//! actions (marquer lu, suivre, archiver, corbeille, spam), appliquées aux nouveaux messages de la boîte de réception.
//!
//! Tout est local : rien n'est installé sur le serveur (Sieve : hors périmètre), donc une règle ne s'applique que
//! application ouverte, à l'arrivée du message vue par la synchro. La condition porte sur la conversation : elle est
//! remplie si l'un de ses messages **non lus** la satisfait (les actions portent ensuite sur la conversation).

use std::collections::HashSet;
use ts_rs::TS;

use envelope_core::role::FolderRole;
use envelope_core::search::{parse_search, push_message_filters, SearchSpec};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use crate::error::{AppError, Result};
use crate::mail::actions::{self, Action, Destination};

const MAX_RULES: i64 = 100;
const MAX_NAME: usize = 100; // caractères (le formulaire borne pareil)
const MAX_QUERY: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, rename = "RuleMove")]
#[serde(rename_all = "lowercase")]
pub enum MoveTo {
    Archive,
    Trash,
    Junk,
}

impl MoveTo {
    fn as_str(self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Trash => "trash",
            Self::Junk => "junk",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        [Self::Archive, Self::Trash, Self::Junk]
            .into_iter()
            .find(|m| m.as_str() == s)
    }
    fn destination(self) -> Destination {
        match self {
            Self::Archive => Destination::Archive,
            Self::Trash => Destination::Trash,
            Self::Junk => Destination::Junk,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    /// Ignoré à la création.
    #[serde(default)]
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    /// `None` : tous les comptes.
    pub account_id: Option<String>,
    pub query: String,
    pub mark_read: bool,
    pub star: bool,
    pub move_to: Option<MoveTo>,
}

type Row = (
    i64,
    String,
    bool,
    Option<String>,
    String,
    bool,
    bool,
    Option<String>,
);

const COLUMNS: &str = "id, name, enabled, account_id, query, mark_read, star, move_to";

fn rule(r: Row) -> Rule {
    Rule {
        id: r.0,
        name: r.1,
        enabled: r.2,
        account_id: r.3,
        query: r.4,
        mark_read: r.5,
        star: r.6,
        move_to: r.7.as_deref().and_then(MoveTo::parse),
    }
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Rule>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM rules ORDER BY id"
    )))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(rule).collect())
}

fn validate(r: &Rule) -> Result<()> {
    let bad = |m: crate::error::Msg| Err(AppError::Invalid(m));
    if r.name.trim().is_empty() || r.name.chars().count() > MAX_NAME {
        return bad(msg!("rules.name_invalid", "rule name empty or too long"));
    }
    if r.query.trim().is_empty() || r.query.chars().count() > MAX_QUERY {
        return bad(msg!(
            "rules.condition_invalid",
            "condition empty or too long"
        ));
    }
    if !r.mark_read && !r.star && r.move_to.is_none() {
        return bad(msg!(
            "rules.action_required",
            "a rule must have at least one action"
        ));
    }
    Ok(())
}

/// Crée la règle (`id` = 0) ou remplace celle qui porte cet identifiant ; rend l'identifiant.
pub async fn save(pool: &SqlitePool, r: &Rule) -> Result<i64> {
    validate(r)?;
    let name = r.name.trim();
    let move_to = r.move_to.map(MoveTo::as_str);
    if r.id == 0 {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rules")
            .fetch_one(pool)
            .await?;
        if n >= MAX_RULES {
            return Err(AppError::Invalid(msg!(
                "rules.too_many",
                "too many rules ({max} at most)",
                max = MAX_RULES
            )));
        }
        return Ok(sqlx::query(
            "INSERT INTO rules (name, enabled, account_id, query, mark_read, star, move_to) VALUES (?,?,?,?,?,?,?)",
        )
        .bind(name)
        .bind(r.enabled)
        .bind(&r.account_id)
        .bind(r.query.trim())
        .bind(r.mark_read)
        .bind(r.star)
        .bind(move_to)
        .execute(pool)
        .await?
        .last_insert_rowid());
    }
    let done = sqlx::query(
        "UPDATE rules SET name=?, enabled=?, account_id=?, query=?, mark_read=?, star=?, move_to=? WHERE id=?",
    )
    .bind(name)
    .bind(r.enabled)
    .bind(&r.account_id)
    .bind(r.query.trim())
    .bind(r.mark_read)
    .bind(r.star)
    .bind(move_to)
    .bind(r.id)
    .execute(pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(AppError::Invalid(msg!("not_found.rule", "rule not found")));
    }
    Ok(r.id)
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM rules WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// La conversation a-t-elle un message **non lu** qui satisfait la condition ? Une règle vise l'arrivée de courrier :
/// un nouveau message d'un autre expéditeur dans un vieux fil ne doit pas faire jouer une règle sur un message ancien,
/// et rejouer les règles ne doit pas remettre « lu » ou l'étoile que l'utilisateur a retirés.
async fn matches(pool: &SqlitePool, thread_id: i64, spec: &SearchSpec) -> Result<bool> {
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT EXISTS (SELECT 1 FROM messages m WHERE m.is_read = 0 AND m.thread_id = ",
    );
    qb.push_bind(thread_id);
    if !push_message_filters(&mut qb, spec) {
        return Ok(false);
    }
    qb.push(")");
    Ok(qb.build_query_scalar().fetch_one(pool).await?)
}

/// Dossier de réception de la conversation (source d'un déplacement).
async fn inbox_folder(pool: &SqlitePool, thread_id: i64) -> Result<Option<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT m.folder_id FROM messages m JOIN folders f ON f.id = m.folder_id \
         WHERE m.thread_id = ? AND f.role = ? LIMIT 1",
    )
    .bind(thread_id)
    .bind(FolderRole::Inbox)
    .fetch_optional(pool)
    .await?)
}

/// Applique les règles actives du compte aux conversations données (de la boîte de réception, avec du courrier non
/// lu) ; rend celles qu'une règle a traitées **avec succès**. Toutes les conditions d'une conversation sont évaluées
/// avant la première action (marquer lu ne doit pas masquer une règle suivante). `lock` sérialise les actions IMAP
/// avec celles de l'interface. Une action en échec (serveur injoignable) est journalisée ; la conversation n'est pas
/// comptée traitée (le message reste notifié) et les autres conversations continuent.
pub async fn apply(
    pool: &SqlitePool,
    lock: &tokio::sync::Mutex<()>,
    account_id: &str,
    thread_ids: &[i64],
) -> Result<HashSet<i64>> {
    let rules: Vec<(Rule, SearchSpec)> = list(pool)
        .await?
        .into_iter()
        .filter(|r| r.enabled && r.account_id.as_deref().map_or(true, |a| a == account_id))
        .map(|r| {
            let spec = parse_search(&r.query);
            (r, spec)
        })
        .collect();
    let mut handled = HashSet::new();
    for &tid in thread_ids {
        let mut hits = Vec::new();
        for (r, spec) in &rules {
            if matches(pool, tid, spec).await? {
                hits.push(r);
            }
        }
        let mut ok = !hits.is_empty();
        for r in hits {
            let mut steps = Vec::new();
            if r.mark_read {
                steps.push(Action::SetRead(true));
            }
            if r.star {
                steps.push(Action::SetStarred(true));
            }
            if let Some(to) = r.move_to {
                if let Some(folder_id) = inbox_folder(pool, tid).await? {
                    steps.push(Action::Move {
                        folder_id,
                        to: to.destination(),
                    });
                }
            }
            let _guard = lock.lock().await;
            for step in steps {
                if let Err(e) = actions::apply(pool, tid, step).await {
                    tracing::warn!("rule \"{}\" on thread {tid}: {e}", r.name);
                    ok = false;
                }
            }
            if r.move_to.is_some() {
                break; // la conversation a quitté la boîte de réception
            }
        }
        if ok {
            handled.insert(tid);
        }
    }
    Ok(handled)
}

/// Applique les règles aux conversations de la boîte de réception ayant du courrier non lu (les 200 plus récentes par
/// compte, hors dossiers locaux) ; rend le nombre de conversations traitées.
pub async fn run_recent(pool: &SqlitePool, lock: &tokio::sync::Mutex<()>) -> Result<usize> {
    let accounts: Vec<String> = sqlx::query_scalar("SELECT id FROM accounts WHERE id <> 'local'")
        .fetch_all(pool)
        .await?;
    let mut total = 0;
    for account in accounts {
        let threads: Vec<i64> = sqlx::query_scalar(
            "SELECT t.id FROM threads t WHERE t.account_id = ? AND t.id IN \
             (SELECT m.thread_id FROM messages m JOIN folders f ON f.id = m.folder_id \
              WHERE f.role = ? AND m.is_read = 0) \
             ORDER BY t.last_message_at DESC LIMIT 200",
        )
        .bind(&account)
        .bind(FolderRole::Inbox)
        .fetch_all(pool)
        .await?;
        total += apply(pool, lock, &account, &threads).await?.len();
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let pool = crate::db::init_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','me@x.fr','a','i','s')")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    fn new_rule(query: &str) -> Rule {
        Rule {
            id: 0,
            name: "R".into(),
            enabled: true,
            account_id: None,
            query: query.into(),
            mark_read: true,
            star: false,
            move_to: None,
        }
    }

    #[tokio::test]
    async fn creation_modification_suppression_et_validation() {
        let pool = pool().await;
        assert!(
            save(
                &pool,
                &Rule {
                    query: " ".into(),
                    ..new_rule("x")
                }
            )
            .await
            .is_err(),
            "condition vide"
        );
        assert!(
            save(
                &pool,
                &Rule {
                    mark_read: false,
                    ..new_rule("de:a")
                }
            )
            .await
            .is_err(),
            "sans action"
        );
        assert!(save(
            &pool,
            &Rule {
                name: "".into(),
                ..new_rule("de:a")
            }
        )
        .await
        .is_err());
        let id = save(
            &pool,
            &Rule {
                move_to: Some(MoveTo::Archive),
                ..new_rule("de:news")
            },
        )
        .await
        .unwrap();
        let mut got = list(&pool).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].id, got[0].move_to), (id, Some(MoveTo::Archive)));
        got[0].enabled = false;
        got[0].move_to = Some(MoveTo::Junk);
        save(&pool, &got[0]).await.unwrap();
        assert_eq!(list(&pool).await.unwrap()[0].move_to, Some(MoveTo::Junk));
        assert!(save(
            &pool,
            &Rule {
                id: 999,
                ..new_rule("de:a")
            }
        )
        .await
        .is_err());
        delete(&pool, id).await.unwrap();
        assert!(list(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn condition_selon_la_syntaxe_de_recherche_et_actions_locales() {
        let pool = pool().await;
        sqlx::query("INSERT INTO folders (id, account_id, path, name, role) VALUES (50,'local','INBOX','INBOX','inbox')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO threads (id, account_id, last_message_at, message_count, unread_count) VALUES (1,'local','2026-01-01',1,1),(2,'local','2026-01-02',1,1)").execute(&pool).await.unwrap();
        for (id, from, subj) in [(1, "promo@shop.fr", "Soldes"), (2, "ami@x.fr", "Salut")] {
            sqlx::query("INSERT INTO messages (id, account_id, folder_id, thread_id, uid, subject, from_addr, date, is_read) VALUES (?, 'local', 50, ?, ?, ?, ?, '2026-01-01T00:00:00Z', 0)")
                .bind(id).bind(id).bind(id).bind(subj).bind(from).execute(&pool).await.unwrap();
        }
        assert!(matches(&pool, 1, &parse_search("de:promo")).await.unwrap());
        assert!(!matches(&pool, 2, &parse_search("de:promo")).await.unwrap());
        assert!(matches(&pool, 2, &parse_search("objet:salut"))
            .await
            .unwrap());
        save(
            &pool,
            &Rule {
                star: true,
                ..new_rule("de:promo")
            },
        )
        .await
        .unwrap();
        let lock = tokio::sync::Mutex::new(());
        let done = apply(&pool, &lock, "local", &[1, 2]).await.unwrap();
        assert_eq!(done, HashSet::from([1]));
        let read: Vec<(i64, bool, bool)> =
            sqlx::query_as("SELECT id, is_read, is_starred FROM messages ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(read, vec![(1, true, true), (2, false, false)]);
        // Rejouer : le message est lu, la règle ne le reprend pas (l'étoile retirée par l'utilisateur reste retirée).
        sqlx::query("UPDATE messages SET is_starred = 0 WHERE id = 1")
            .execute(&pool)
            .await
            .unwrap();
        assert!(apply(&pool, &lock, "local", &[1]).await.unwrap().is_empty());
        let starred: bool = sqlx::query_scalar("SELECT is_starred FROM messages WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!starred);
    }

    #[tokio::test]
    async fn regle_d_un_autre_compte_ou_desactivee_ne_s_applique_pas() {
        let pool = pool().await;
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('b','b@x.fr','b','i','s')").execute(&pool).await.unwrap();
        save(
            &pool,
            &Rule {
                account_id: Some("b".into()),
                ..new_rule("de:a")
            },
        )
        .await
        .unwrap();
        save(
            &pool,
            &Rule {
                enabled: false,
                ..new_rule("de:a")
            },
        )
        .await
        .unwrap();
        let lock = tokio::sync::Mutex::new(());
        assert!(apply(&pool, &lock, "a", &[1]).await.unwrap().is_empty());
    }
}

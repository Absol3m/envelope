//! Suggestions de destinataires : adresses déjà vues (expéditeurs et destinataires), tenues dans la table `contacts`.

use sqlx::SqlitePool;

use crate::addr::Addr;
use crate::page::Page;
use crate::Result;

#[derive(Debug, sqlx::FromRow)]
struct Row {
    address: String,
    name: Option<String>,
}

/// Adresses dont le nom ou l'adresse contient `query`, les plus fréquentes d'abord.
/// Les adresses des comptes de l'application ne sont jamais proposées.
pub async fn suggest(
    pool: &SqlitePool,
    account_id: Option<&str>,
    query: &str,
    limit: i64,
) -> Result<Vec<Addr>> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(vec![]);
    }
    let like = crate::search::like_contains(q);
    // Table `contacts` (migration 0020) : quelques milliers de lignes, jamais les messages.
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT address, MAX(name) AS name FROM contacts \
         WHERE (?1 IS NULL OR account_id = ?1) AND address NOT IN (SELECT LOWER(email) FROM accounts) \
           AND (address LIKE ?2 ESCAPE '\\' OR name LIKE ?2 ESCAPE '\\') \
         GROUP BY address ORDER BY SUM(uses) DESC, address LIMIT ?3",
    )
    .bind(account_id)
    .bind(like)
    .bind(Page::first(limit).limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Addr {
            name: r.name.filter(|n| !n.is_empty()),
            address: r.address,
        })
        .collect())
}

//! Suggestions de destinataires : adresses déjà vues dans la base locale (expéditeurs et destinataires).

use serde::Serialize;
use sqlx::SqlitePool;

use crate::addr::Addr;
use crate::Result;

#[derive(Debug, Serialize, sqlx::FromRow)]
struct Row {
    address: String,
    name: Option<String>,
}

/// Adresses dont le nom ou l'adresse contient `query`, les plus fréquentes d'abord.
/// `exclude` (adresse du compte) n'est jamais proposée.
pub async fn suggest(
    pool: &SqlitePool,
    account_id: Option<&str>,
    query: &str,
    exclude: &[&str],
    limit: i64,
) -> Result<Vec<Addr>> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(vec![]);
    }
    // `\` échappe % et _ saisis par l'utilisateur.
    let like = format!(
        "%{}%",
        q.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT LOWER(address) AS address, MAX(name) AS name FROM ( \
           SELECT from_addr AS address, from_name AS name FROM messages \
             WHERE (?1 IS NULL OR account_id = ?1) \
           UNION ALL \
           SELECT json_extract(j.value, '$.address'), json_extract(j.value, '$.name') \
             FROM messages m, json_each(m.to_addrs) j WHERE (?1 IS NULL OR m.account_id = ?1) \
           UNION ALL \
           SELECT json_extract(j.value, '$.address'), json_extract(j.value, '$.name') \
             FROM messages m, json_each(m.cc_addrs) j WHERE (?1 IS NULL OR m.account_id = ?1) \
         ) WHERE address != '' AND (address LIKE ?2 ESCAPE '\\' OR name LIKE ?2 ESCAPE '\\') \
         GROUP BY LOWER(address) ORDER BY COUNT(*) DESC, address LIMIT ?3",
    )
    .bind(account_id)
    .bind(like)
    .bind(limit + exclude.len() as i64)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|r| !exclude.iter().any(|e| e.eq_ignore_ascii_case(&r.address)))
        .take(limit as usize)
        .map(|r| Addr {
            name: r.name.filter(|n| !n.is_empty()),
            address: r.address,
        })
        .collect())
}

//! Recherche plein texte (FTS5) et requêtes de lecture pour l'UI et le serveur MCP.

use serde::Serialize;
use sqlx::SqlitePool;

use crate::Result;

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MessageSummary {
    pub id: i64,
    pub account_id: String,
    pub folder_id: i64,
    pub thread_id: Option<i64>,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_addr: String,
    pub date: String,
    pub snippet: String,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
}

const SUMMARY_COLS: &str =
    "m.id, m.account_id, m.folder_id, m.thread_id, m.subject, m.from_name, m.from_addr, \
    m.date, m.snippet, m.is_read, m.is_starred, m.has_attachments";

/// Transforme une saisie utilisateur en requête FTS5 sûre : chaque mot devient un préfixe entre
/// guillemets (`"mot"*`), combinés en ET. Neutralise les opérateurs FTS5 (`OR`, `NEAR`, `-`, `:`…).
pub fn build_fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|t| t.replace('"', ""))
        .filter(|t| t.chars().any(char::is_alphanumeric))
        .map(|t| format!("\"{t}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

pub async fn search(
    pool: &SqlitePool,
    query: &str,
    account_id: Option<&str>,
    limit: i64,
) -> Result<Vec<MessageSummary>> {
    let Some(fts) = build_fts_query(query) else {
        return Ok(vec![]);
    };
    sqlx::query_as(&format!(
        "SELECT {SUMMARY_COLS} FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid \
         WHERE messages_fts MATCH ?1 AND (?2 IS NULL OR m.account_id = ?2) \
         ORDER BY rank LIMIT ?3"
    ))
    .bind(fts)
    .bind(account_id)
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await
}

pub async fn list_messages(
    pool: &SqlitePool,
    folder_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<MessageSummary>> {
    sqlx::query_as(&format!(
        "SELECT {SUMMARY_COLS} FROM messages m WHERE m.folder_id = ? ORDER BY m.date DESC LIMIT ? OFFSET ?"
    ))
    .bind(folder_id)
    .bind(limit.clamp(1, 500))
    .bind(offset.max(0))
    .fetch_all(pool)
    .await
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct FolderRow {
    pub id: i64,
    pub account_id: String,
    pub path: String,
    pub name: String,
    pub role: Option<String>,
    pub unread_count: i64,
    pub total_count: i64,
}

pub async fn list_folders(pool: &SqlitePool, account_id: Option<&str>) -> Result<Vec<FolderRow>> {
    sqlx::query_as(
        "SELECT id, account_id, path, name, role, unread_count, total_count FROM folders \
         WHERE (?1 IS NULL OR account_id = ?1) \
         ORDER BY account_id, CASE role WHEN 'inbox' THEN 0 WHEN 'drafts' THEN 1 WHEN 'sent' THEN 2 \
         WHEN 'archive' THEN 3 WHEN 'junk' THEN 4 WHEN 'trash' THEN 5 ELSE 6 END, name",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
}

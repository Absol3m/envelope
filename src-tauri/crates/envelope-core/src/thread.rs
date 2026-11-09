//! Conversations (threads) et comptes, vus en lecture seule.

use serde::Serialize;
use sqlx::SqlitePool;

use crate::Result;

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub provider: String,
    pub sync_status: String,
    pub last_sync_at: Option<String>,
}

/// Comptes, sans configuration serveur ni secret.
pub async fn list_accounts(pool: &SqlitePool) -> Result<Vec<AccountSummary>> {
    sqlx::query_as("SELECT id, email, display_name, provider, sync_status, last_sync_at FROM accounts ORDER BY created_at")
        .fetch_all(pool)
        .await
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMessage {
    pub id: i64,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_addr: String,
    /// JSON `[{"name":..,"address":..}]`, tel que stocké.
    pub to_addrs: String,
    pub cc_addrs: String,
    pub date: String,
    pub snippet: String,
    pub body_text: Option<String>,
    pub is_read: bool,
}

/// Métadonnées d'une pièce jointe. Jamais son chemin sur disque ni son contenu.
#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInfo {
    pub message_id: i64,
    pub filename: Option<String>,
    pub mime_type: String,
    pub size: i64,
    pub is_inline: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: i64,
    pub account_id: String,
    pub subject: String,
    pub messages: Vec<ThreadMessage>,
    pub attachments: Vec<AttachmentInfo>,
}

/// Une conversation, messages du plus ancien au plus récent. Une même copie présente dans deux
/// dossiers (ex. Envoyés + Reçus) n'apparaît qu'une fois (dédoublonnage sur Message-ID).
pub async fn get_thread(pool: &SqlitePool, thread_id: i64) -> Result<Option<Thread>> {
    let head: Option<(String, String)> =
        sqlx::query_as("SELECT account_id, subject FROM threads WHERE id = ?")
            .bind(thread_id)
            .fetch_optional(pool)
            .await?;
    let Some((account_id, subject)) = head else {
        return Ok(None);
    };

    let messages = sqlx::query_as(
        "SELECT id, subject, from_name, from_addr, to_addrs, cc_addrs, date, snippet, body_text, is_read \
         FROM messages WHERE id IN ( \
             SELECT MIN(id) FROM messages WHERE thread_id = ?1 GROUP BY COALESCE(message_id, 'id:' || id)) \
         ORDER BY date, id",
    )
    .bind(thread_id)
    .fetch_all(pool)
    .await?;
    let attachments = sqlx::query_as(
        "SELECT message_id, filename, mime_type, size, is_inline FROM attachments \
         WHERE message_id IN (SELECT id FROM messages WHERE thread_id = ?) ORDER BY message_id, id",
    )
    .bind(thread_id)
    .fetch_all(pool)
    .await?;
    Ok(Some(Thread {
        id: thread_id,
        account_id,
        subject,
        messages,
        attachments,
    }))
}

//! Conversations (threads) et comptes, vus en lecture seule.

use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use crate::addr::{self, Addr};
use crate::kinds::{Provider, SyncStatus};
use crate::page::Page;
use crate::role::FolderRole;
use crate::search::{parse_search, push_message_filters};
use crate::Result;

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub provider: Provider,
    pub sync_status: SyncStatus,
    pub last_sync_at: Option<String>,
}

/// Comptes, sans configuration serveur ni secret. Projection réduite voulue, distincte de `mail::account::list` de l'application
/// (serveurs, identités, état des copies) : le serveur MCP n'a pas à voir ce qui n'aide pas un agent.
pub async fn list_accounts(
    pool: &SqlitePool,
    account_id: Option<&str>,
) -> Result<Vec<AccountSummary>> {
    sqlx::query_as("SELECT id, email, display_name, provider, sync_status, last_sync_at FROM accounts WHERE (?1 IS NULL OR id = ?1) ORDER BY created_at")
        .bind(account_id)
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
/// `body_chars` borne chaque corps côté SQL : un long fil ne charge pas des Mo en mémoire.
/// `account_id` : ne rend que la conversation de ce compte (une autre est « introuvable »).
pub async fn get_thread(
    pool: &SqlitePool,
    thread_id: i64,
    body_chars: i64,
    account_id: Option<&str>,
) -> Result<Option<Thread>> {
    let head: Option<(String, String)> = sqlx::query_as(
        "SELECT account_id, subject FROM threads WHERE id = ?1 AND (?2 IS NULL OR account_id = ?2)",
    )
    .bind(thread_id)
    .bind(account_id)
    .fetch_optional(pool)
    .await?;
    let Some((account_id, subject)) = head else {
        return Ok(None);
    };

    let messages = sqlx::query_as(
        "SELECT id, subject, from_name, from_addr, to_addrs, cc_addrs, date, snippet, substr(body_text, 1, ?2) AS body_text, is_read \
         FROM messages WHERE id IN ( \
             SELECT MIN(id) FROM messages WHERE thread_id = ?1 GROUP BY COALESCE(message_id, 'id:' || id)) \
         ORDER BY date, id",
    )
    .bind(thread_id)
    .bind(body_chars)
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

// ---------- Liste de conversations (UI) ----------

/// Une conversation dans la liste : agrégats + dernier expéditeur + participants.
#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: i64,
    pub account_id: String,
    pub subject: String,
    pub snippet: String,
    pub last_message_at: String,
    pub message_count: i64,
    pub unread_count: i64,
    pub is_starred: bool,
    pub has_attachments: bool,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    pub participants: Option<String>,
    /// Dossier du dernier message : les résultats de recherche couvrent tous les dossiers, l'interface
    /// en déduit le nom, le rôle et le compte à partir de sa liste de dossiers.
    pub folder_id: Option<i64>,
    /// Rappel de la conversation : `snooze` | `followup`, échéance (secondes Unix) et, s'il est échu, son heure de retour.
    pub reminder_kind: Option<String>,
    pub reminder_due: Option<i64>,
    pub reminder_fired: Option<i64>,
}

/// Filtres de la liste. `folder_id` (un dossier) et `role` (ex. `inbox`, tous comptes confondus)
/// sont exclusifs ; les autres se combinent.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadQuery {
    pub folder_id: Option<i64>,
    pub role: Option<FolderRole>,
    pub account_id: Option<String>,
    pub starred_only: bool,
    pub unread_only: bool,
    pub with_attachments: bool,
    /// Conversations ayant un rappel en attente (vue « Reportés »).
    pub reminders_only: bool,
    /// Recherche plein texte (même syntaxe que `search_emails`).
    pub query: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

const SUMMARY_SELECT: &str = "SELECT t.id, t.account_id, t.subject, t.snippet, t.last_message_at, t.message_count, \
    t.unread_count, t.is_starred, \
    EXISTS (SELECT 1 FROM messages m WHERE m.thread_id = t.id AND m.has_attachments = 1) AS has_attachments, \
    (SELECT from_name FROM messages WHERE thread_id = t.id ORDER BY date DESC, id DESC LIMIT 1) AS from_name, \
    (SELECT from_addr FROM messages WHERE thread_id = t.id ORDER BY date DESC, id DESC LIMIT 1) AS from_addr, \
    (SELECT group_concat(n, ', ') FROM (SELECT DISTINCT COALESCE(NULLIF(from_name, ''), from_addr) AS n \
        FROM messages WHERE thread_id = t.id ORDER BY date LIMIT 4)) AS participants, \
    (SELECT m.folder_id FROM messages m WHERE m.thread_id = t.id ORDER BY m.date DESC, m.id DESC LIMIT 1) AS folder_id, \
    r.kind AS reminder_kind, r.due_at AS reminder_due, r.fired_at AS reminder_fired \
    FROM threads t LEFT JOIN reminders r ON r.thread_id = t.id WHERE 1 = 1";

pub async fn list_threads(pool: &SqlitePool, q: &ThreadQuery) -> Result<Vec<ThreadSummary>> {
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(SUMMARY_SELECT);
    if let Some(id) = q.folder_id {
        qb.push(" AND t.id IN (SELECT thread_id FROM messages WHERE folder_id = ")
            .push_bind(id)
            .push(")");
    } else if let Some(role) = &q.role {
        qb.push(" AND t.id IN (SELECT m.thread_id FROM messages m JOIN folders f ON f.id = m.folder_id WHERE f.role = ")
            .push_bind(role)
            .push(")");
    }
    if let Some(account) = &q.account_id {
        // Une recherche couvre aussi les archives locales (« Sur cet ordinateur »), quel que soit le compte choisi.
        if q.query.as_deref().is_some_and(|t| !t.trim().is_empty()) {
            qb.push(" AND t.account_id IN (")
                .push_bind(account)
                .push(", 'local')");
        } else {
            qb.push(" AND t.account_id = ").push_bind(account);
        }
    }
    if q.reminders_only {
        qb.push(" AND r.kind IS NOT NULL AND r.fired_at IS NULL");
    } else if q.query.as_deref().is_none_or(|t| t.trim().is_empty()) {
        // Un report en cours masque la conversation des listes ; la recherche, elle, la retrouve toujours.
        qb.push(" AND (r.kind IS NOT 'snooze' OR r.fired_at IS NOT NULL)");
    }
    if q.starred_only {
        qb.push(" AND t.is_starred = 1");
    }
    if q.unread_only {
        qb.push(" AND t.unread_count > 0");
    }
    if q.with_attachments {
        qb.push(" AND EXISTS (SELECT 1 FROM messages m WHERE m.thread_id = t.id AND m.has_attachments = 1)");
    }
    if let Some(input) = q.query.as_deref().filter(|t| !t.trim().is_empty()) {
        let spec = parse_search(input);
        // Une conversation correspond si l'un de ses messages satisfait tous les critères.
        qb.push(" AND t.id IN (SELECT m.thread_id FROM messages m WHERE m.thread_id IS NOT NULL");
        if !push_message_filters(&mut qb, &spec) {
            return Ok(vec![]);
        }
        qb.push(")");
    }
    // Les conversations revenues (rappel échu) passent en tête, puis ordre chronologique.
    let page = Page::new(q.limit, q.offset, 100);
    qb.push(" ORDER BY r.fired_at IS NOT NULL DESC, t.last_message_at DESC, t.id DESC LIMIT ")
        .push_bind(page.limit)
        .push(" OFFSET ")
        .push_bind(page.offset);
    qb.build_query_as().fetch_all(pool).await
}

// ---------- Détail d'une conversation (UI) ----------

#[derive(Debug, sqlx::FromRow)]
struct RawDetailMessage {
    id: i64,
    message_id: Option<String>,
    references_ids: Option<String>,
    folder_id: i64,
    subject: String,
    from_name: Option<String>,
    from_addr: String,
    to_addrs: String,
    cc_addrs: String,
    date: String,
    body_text: Option<String>,
    body_html: Option<String>,
    is_read: bool,
    is_starred: bool,
    unsub_url: Option<String>,
    unsub_mailto: Option<String>,
    unsub_one_click: bool,
    auth_fail: Option<String>,
}

/// Moyens de désabonnement d'une liste de diffusion, tirés des en-têtes List-Unsubscribe.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unsubscribe {
    /// Lien https vers un domaine public (page de désabonnement ou point d'appel du POST).
    pub url: Option<String>,
    pub mailto: Option<String>,
    /// Désabonnement par simple POST (RFC 8058), sans page intermédiaire.
    pub one_click: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailMessage {
    pub id: i64,
    /// En-tête Message-ID (sert au threading des réponses).
    pub message_id: Option<String>,
    /// Message-ID des ancêtres, du plus ancien au plus récent.
    pub references: Vec<String>,
    pub folder_id: i64,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_addr: String,
    pub to: Vec<Addr>,
    pub cc: Vec<Addr>,
    pub date: String,
    pub body_text: Option<String>,
    /// HTML brut de l'expéditeur : toujours à assainir côté affichage.
    pub body_html: Option<String>,
    pub is_read: bool,
    pub is_starred: bool,
    pub unsubscribe: Option<Unsubscribe>,
    /// Vérification d'identité de l'expéditeur échouée (« dmarc », « spf », « dkim ») : possible usurpation.
    pub auth_fail: Option<String>,
}

impl From<RawDetailMessage> for DetailMessage {
    fn from(m: RawDetailMessage) -> Self {
        Self {
            id: m.id,
            message_id: m.message_id,
            references: m
                .references_ids
                .and_then(|j| serde_json::from_str(&j).ok())
                .unwrap_or_default(),
            folder_id: m.folder_id,
            subject: m.subject,
            from_name: m.from_name,
            from_addr: m.from_addr,
            to: addr::decode(&m.to_addrs),
            cc: addr::decode(&m.cc_addrs),
            date: m.date,
            body_text: m.body_text,
            body_html: m.body_html,
            is_read: m.is_read,
            is_starred: m.is_starred,
            auth_fail: m.auth_fail,
            unsubscribe: (m.unsub_url.is_some() || m.unsub_mailto.is_some()).then_some(
                Unsubscribe {
                    url: m.unsub_url,
                    mailto: m.unsub_mailto,
                    one_click: m.unsub_one_click,
                },
            ),
        }
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DetailAttachment {
    pub id: i64,
    pub message_id: i64,
    pub filename: Option<String>,
    pub mime_type: String,
    pub size: i64,
    pub content_id: Option<String>,
    pub is_inline: bool,
    /// L'interface peut l'afficher (image de taille raisonnable) ; calculé après la requête.
    #[sqlx(skip)]
    pub previewable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDetail {
    pub id: i64,
    pub account_id: String,
    pub subject: String,
    pub is_starred: bool,
    pub messages: Vec<DetailMessage>,
    pub attachments: Vec<DetailAttachment>,
}

/// Conversation complète pour l'affichage : corps texte et HTML, pièces jointes avec identifiants.
pub async fn get_thread_detail(pool: &SqlitePool, thread_id: i64) -> Result<Option<ThreadDetail>> {
    let head: Option<(String, String, bool)> =
        sqlx::query_as("SELECT account_id, subject, is_starred FROM threads WHERE id = ?")
            .bind(thread_id)
            .fetch_optional(pool)
            .await?;
    let Some((account_id, subject, is_starred)) = head else {
        return Ok(None);
    };

    let raw: Vec<RawDetailMessage> = sqlx::query_as(
        "SELECT id, message_id, references_ids, folder_id, subject, from_name, from_addr, to_addrs, cc_addrs, date, body_text, body_html, is_read, is_starred, unsub_url, unsub_mailto, unsub_one_click, auth_fail \
         FROM messages WHERE id IN (SELECT MIN(id) FROM messages WHERE thread_id = ?1 GROUP BY COALESCE(message_id, 'id:' || id)) \
         ORDER BY date, id",
    )
    .bind(thread_id)
    .fetch_all(pool)
    .await?;
    let mut attachments: Vec<DetailAttachment> = sqlx::query_as(
        "SELECT id, message_id, filename, mime_type, size, content_id, is_inline FROM attachments \
         WHERE message_id IN (SELECT id FROM messages WHERE thread_id = ?) ORDER BY message_id, id",
    )
    .bind(thread_id)
    .fetch_all(pool)
    .await?;
    for a in &mut attachments {
        a.previewable = crate::attachment::previewable(&a.mime_type, a.size.max(0) as u64);
    }
    Ok(Some(ThreadDetail {
        id: thread_id,
        account_id,
        subject,
        is_starred,
        messages: raw.into_iter().map(Into::into).collect(),
        attachments,
    }))
}

//! Parsing RFC 5322 (mail-parser) → SQLite : message, threading, pièces jointes.
//!
//! `parse` est synchrone et produit des données possédées (le `Message` emprunté ne traverse
//! jamais un `.await`) ; `store` fait ensuite les écritures async.

use std::collections::HashSet;
use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use envelope_core::addr::{self, Addr};
use mail_parser::{Address, HeaderValue, MessageParser, MimeHeaders};
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use crate::error::{AppError, Result};
use crate::mail::{authres, unsubscribe};
use envelope_core::thread::Unsubscribe;

#[derive(Debug, Clone, Copy, Default)]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
}

#[derive(Debug)]
pub struct ParsedAttachment {
    pub filename: Option<String>,
    pub mime_type: String,
    pub content_id: Option<String>,
    pub is_inline: bool,
    pub size: usize,
    /// Contenu décodé. **Vide avec `size > 0`** = à extraire du message brut à la première ouverture (grosse pièce
    /// jointe, ou analyse sans copie) : `store_with` exige alors `raw_hash`, sans quoi le contenu serait perdu.
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub struct ParsedMessage {
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_addr: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub date: String,
    pub snippet: String,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub attachments: Vec<ParsedAttachment>,
    pub unsubscribe: Option<Unsubscribe>,
    /// `X-Envelope-Draft-Id` : ce message est un brouillon d'Envelope (voir `mail::drafts`).
    pub draft_id: Option<String>,
    /// Mécanisme (« dmarc », « spf », « dkim ») dont la vérification a échoué d'après `Authentication-Results`.
    pub auth_fail: Option<&'static str>,
}

fn addrs(a: Option<&Address<'_>>) -> String {
    let list: Vec<Addr> = a
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    x.address().map(|ad| Addr {
                        name: x.name().map(str::to_string),
                        address: ad.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    addr::encode(&list)
}

fn header_ids(v: &HeaderValue<'_>) -> Vec<String> {
    match v {
        HeaderValue::Text(t) => vec![t.to_string()],
        HeaderValue::TextList(l) => l.iter().map(|s| s.to_string()).collect(),
        _ => vec![],
    }
}

fn snippet(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect()
}

fn iso(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Sujet de base de la conversation (RFC 5256 §2.1, par mail-parser) : sans préfixes de réponse ou de transfert dans la
/// plupart des langues (« Re: », « Fwd: », « AW: », « SV: », « Re[2]: »…), ni balises de liste (« [liste] ») ni « (fwd) ».
/// Un sujet réduit à ces éléments reste tel quel : une conversation n'a jamais de titre vide.
pub fn normalize_subject(s: &str) -> String {
    let s = s.trim();
    match mail_parser::parsers::fields::thread::thread_name(s) {
        "" => s.to_string(),
        base => base.to_string(),
    }
}

/// `fallback_ts` : INTERNALDATE du serveur, utilisée si l'en-tête Date est absent/invalide.
pub fn parse(raw: &[u8], fallback_ts: Option<i64>) -> Option<ParsedMessage> {
    parse_with(raw, fallback_ts, true)
}

/// Pièce jointe copiée et écrite sur disque dès l'arrivée du message : petits documents (≤ 1 Mio) et images « inline »
/// (≤ `INLINE_IMAGE_MAX`, affichées dans le message : lecture immédiate). Les autres gardent leurs métadonnées et leur
/// taille ; leur contenu est extrait du message brut, déjà sur disque, à la première ouverture (`materialize_attachment`) :
/// ni copie en mémoire, ni doublon sur disque.
const EAGER_ATTACHMENT_MAX: usize = 1024 * 1024;
/// Taille maximale d'une image « inline » (`cid:`) affichée dans le message (voir `inline_images`).
pub const INLINE_IMAGE_MAX: usize = 2 * 1024 * 1024;

/// Comme [`parse`] ; `keep_attachments` : copier les pièces jointes qui doivent l'être (voir `EAGER_ATTACHMENT_MAX`).
pub fn parse_with(
    raw: &[u8],
    fallback_ts: Option<i64>,
    keep_attachments: bool,
) -> Option<ParsedMessage> {
    let msg = MessageParser::default().parse(raw)?;
    let from = msg.from().and_then(|a| a.first());
    // Première occurrence = celle de notre serveur de réception (chaque relais ajoute la sienne en tête) ;
    // `header_raw` renverrait la dernière, forgeable par l'expéditeur.
    let auth_fail = msg
        .headers_raw()
        .find(|(name, _)| name.eq_ignore_ascii_case("Authentication-Results"))
        .and_then(|(_, value)| authres::failure(value));
    let body_text = msg.body_text(0).map(|c| c.into_owned());
    let body_html = msg.body_html(0).map(|c| c.into_owned());
    let ts = msg
        .date()
        .map(|d| d.to_timestamp())
        .or(fallback_ts)
        .unwrap_or_else(|| Utc::now().timestamp());

    let attachments = msg
        .attachments()
        .map(|p| {
            let mime = p
                .content_type()
                .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")))
                .unwrap_or_else(|| "application/octet-stream".into());
            let size = p.contents().len();
            let content_id = p
                .content_id()
                .map(|c| c.trim_matches(|ch| ch == '<' || ch == '>').to_string());
            let inline_image = content_id.is_some() && mime.starts_with("image/");
            let eager = size <= EAGER_ATTACHMENT_MAX || (inline_image && size <= INLINE_IMAGE_MAX);
            ParsedAttachment {
                filename: p.attachment_name().map(str::to_string),
                mime_type: mime,
                content_id,
                is_inline: p
                    .content_disposition()
                    .is_some_and(|d| d.ctype().eq_ignore_ascii_case("inline")),
                size,
                data: if keep_attachments && eager {
                    p.contents().to_vec()
                } else {
                    Vec::new()
                },
            }
        })
        .collect();

    Some(ParsedMessage {
        message_id: msg.message_id().map(str::to_string),
        in_reply_to: header_ids(msg.in_reply_to()).into_iter().next(),
        references: header_ids(msg.references()),
        subject: msg.subject().unwrap_or("").to_string(),
        from_name: from.and_then(|f| f.name()).map(str::to_string),
        from_addr: from.and_then(|f| f.address()).unwrap_or("").to_string(),
        to: addrs(msg.to()),
        cc: addrs(msg.cc()),
        bcc: addrs(msg.bcc()),
        date: iso(ts),
        snippet: snippet(body_text.as_deref().unwrap_or("")),
        body_text,
        body_html,
        attachments,
        unsubscribe: msg
            .header_raw("List-Unsubscribe")
            .and_then(|l| unsubscribe::parse(l, msg.header_raw("List-Unsubscribe-Post"))),
        auth_fail,
        draft_id: msg
            .header_raw(super::smtp::DRAFT_ID_HEADER)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty() && v.len() <= super::smtp::MAX_DRAFT_ID),
    })
}

/// Début de la recherche de la conversation d'un message par les `Message-ID` qu'il cite (suivi de `account_id`).
/// `INDEXED BY` : sans `ANALYZE` (jamais lancé pendant un premier import), SQLite préfère `idx_messages_in_reply_to`,
/// ne s'en sert que pour `account_id` et balaie tout le compte à chaque message : coût quadratique (20 000 messages :
/// 141 s au lieu de 7,6 s). Le test `recherche_de_conversation_utilise_l_index_message_id` garde ce choix.
const FIND_THREAD_BY_IDS: &str =
    "SELECT thread_id FROM messages INDEXED BY idx_messages_message_id \
     WHERE thread_id IS NOT NULL AND account_id = ";

/// Cherche une conversation existante via Message-ID / In-Reply-To / References (dans les deux sens).
async fn find_thread(
    pool: &SqlitePool,
    account_id: &str,
    m: &ParsedMessage,
) -> Result<Option<i64>> {
    let mut ids: HashSet<&str> = m.references.iter().map(String::as_str).collect();
    if let Some(r) = &m.in_reply_to {
        ids.insert(r);
    }
    if let Some(id) = &m.message_id {
        ids.insert(id);
    }
    if !ids.is_empty() {
        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(FIND_THREAD_BY_IDS);
        qb.push_bind(account_id).push(" AND message_id IN (");
        let mut sep = qb.separated(", ");
        for id in &ids {
            sep.push_bind(*id);
        }
        qb.push(") ORDER BY date DESC LIMIT 1");
        if let Some(t) = qb.build_query_scalar::<i64>().fetch_optional(pool).await? {
            return Ok(Some(t));
        }
    }
    // Réponses arrivées avant leur parent (cas courant pendant le rapatriement : du plus récent au plus ancien) :
    // par In-Reply-To, puis par References. Deux recherches indexées, jamais un balayage de la table.
    if let Some(id) = &m.message_id {
        let direct = sqlx::query_scalar::<_, i64>(
            "SELECT thread_id FROM messages WHERE thread_id IS NOT NULL AND account_id = ?1 AND in_reply_to = ?2 LIMIT 1",
        )
        .bind(account_id)
        .bind(id)
        .fetch_optional(pool)
        .await?;
        if direct.is_some() {
            return Ok(direct);
        }
        return Ok(sqlx::query_scalar::<_, i64>(
            "SELECT m.thread_id FROM message_refs r JOIN messages m ON m.id = r.message_id \
             WHERE r.account_id = ?1 AND r.ref_id = ?2 AND m.thread_id IS NOT NULL LIMIT 1",
        )
        .bind(account_id)
        .bind(id)
        .fetch_optional(pool)
        .await?);
    }
    Ok(None)
}

/// Identifiant (UID) du prochain message d'un dossier. Pour un dossier local, le UID n'est qu'un numéro d'ordre.
pub(crate) async fn next_uid(pool: &SqlitePool, folder_id: i64) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT COALESCE(MAX(uid), 0) + 1 FROM messages WHERE folder_id = ?")
            .bind(folder_id)
            .fetch_one(pool)
            .await?,
    )
}

/// Ce dossier contient-il déjà un message de ce message brut (même empreinte) ?
pub(crate) async fn has_raw(pool: &SqlitePool, folder_id: i64, raw_hash: &str) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM messages WHERE folder_id = ? AND raw_hash = ?)",
    )
    .bind(folder_id)
    .bind(raw_hash)
    .fetch_one(pool)
    .await?)
}

fn safe_filename(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    s.trim_start_matches('.').to_string()
}

/// Insère le message (idempotent sur `(folder_id, uid)`), rattache la conversation, écrit les
/// pièces jointes sur disque. Retourne l'id de conversation à rafraîchir.
/// Position du message sur le serveur.
pub struct MessageLoc<'a> {
    pub account_id: &'a str,
    pub folder_id: i64,
    pub uid: i64,
    pub size: Option<i64>,
}

/// Options de stockage d'un message.
#[derive(Default)]
pub struct StoreOpts<'a> {
    /// Empreinte du message brut déjà rangé dans le magasin (`rawstore`).
    pub raw_hash: Option<&'a str>,
    /// `false` : métadonnées des pièces jointes seulement ; leur contenu est extrait du message
    /// brut à la première ouverture (historique rapatrié : pas de copie disque systématique).
    pub lazy_attachments: bool,
}

/// Sans options : utilisé par les essais et les outils de développement (le moteur appelle `store_with`).
#[cfg(any(test, debug_assertions))]
pub async fn store(
    pool: &SqlitePool,
    attachments_dir: &Path,
    loc: MessageLoc<'_>,
    flags: Flags,
    m: ParsedMessage,
) -> Result<i64> {
    store_with(pool, attachments_dir, loc, flags, m, StoreOpts::default()).await
}

pub async fn store_with(
    pool: &SqlitePool,
    attachments_dir: &Path,
    loc: MessageLoc<'_>,
    flags: Flags,
    m: ParsedMessage,
    opts: StoreOpts<'_>,
) -> Result<i64> {
    let MessageLoc {
        account_id,
        folder_id,
        uid,
        size,
    } = loc;
    if !opts.lazy_attachments
        && opts.raw_hash.is_none()
        && m.attachments
            .iter()
            .any(|a| a.data.is_empty() && a.size > 0)
    {
        // Sans brut, une pièce jointe non copiée serait irrécupérable : on refuse plutôt que de la perdre en silence.
        return Err(AppError::Invalid(msg!(
            "ingest.raw_hash_required",
            "large attachment without a raw message: `raw_hash` is required"
        )));
    }
    let existing_thread = find_thread(pool, account_id, &m).await?;
    // Un message est écrit en entier ou pas du tout : conversation, message, références et pièces jointes.
    let mut tx = pool.begin().await?;
    let thread_id = match existing_thread {
        Some(t) => t,
        None => sqlx::query(
            "INSERT INTO threads (account_id, subject, snippet, last_message_at) VALUES (?,?,?,?)",
        )
        .bind(account_id)
        .bind(normalize_subject(&m.subject))
        .bind(&m.snippet)
        .bind(&m.date)
        .execute(&mut *tx)
        .await?
        .last_insert_rowid(),
    };

    let has_att = m
        .attachments
        .iter()
        .any(|a| !a.is_inline || a.filename.is_some());
    let res = sqlx::query(
        "INSERT INTO messages (account_id, folder_id, thread_id, uid, message_id, in_reply_to, references_ids, \
         subject, from_name, from_addr, to_addrs, cc_addrs, bcc_addrs, date, snippet, body_text, body_html, \
         is_read, is_starred, has_attachments, size, unsub_url, unsub_mailto, unsub_one_click, raw_hash, draft_id, auth_fail) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) \
         ON CONFLICT(folder_id, uid) DO NOTHING",
    )
    .bind(account_id)
    .bind(folder_id)
    .bind(thread_id)
    .bind(uid)
    .bind(&m.message_id)
    .bind(&m.in_reply_to)
    .bind(serde_json::to_string(&m.references)?)
    .bind(&m.subject)
    .bind(&m.from_name)
    .bind(&m.from_addr)
    .bind(&m.to)
    .bind(&m.cc)
    .bind(&m.bcc)
    .bind(&m.date)
    .bind(&m.snippet)
    .bind(&m.body_text)
    .bind(&m.body_html)
    .bind(flags.seen)
    .bind(flags.flagged)
    .bind(has_att)
    .bind(size)
    .bind(m.unsubscribe.as_ref().and_then(|u| u.url.as_deref()))
    .bind(m.unsubscribe.as_ref().and_then(|u| u.mailto.as_deref()))
    .bind(m.unsubscribe.as_ref().is_some_and(|u| u.one_click))
    .bind(opts.raw_hash)
    .bind(&m.draft_id)
    .bind(m.auth_fail)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() == 0 {
        // Déjà présent : rien à écrire (la conversation créée pour rien est annulée avec la transaction).
        tx.rollback().await?;
        let existing: Option<i64> =
            sqlx::query_scalar("SELECT thread_id FROM messages WHERE folder_id = ? AND uid = ?")
                .bind(folder_id)
                .bind(uid)
                .fetch_optional(pool)
                .await?
                .flatten();
        return Ok(existing.unwrap_or(thread_id));
    }
    let message_row = res.last_insert_rowid();
    for r in m.references.iter().collect::<HashSet<_>>() {
        sqlx::query("INSERT INTO message_refs (account_id, ref_id, message_id) VALUES (?, ?, ?)")
            .bind(account_id)
            .bind(r)
            .bind(message_row)
            .execute(&mut *tx)
            .await?;
    }

    if !m.attachments.is_empty() {
        let dir = attachments_dir
            .join(account_id)
            .join(message_row.to_string());
        if !opts.lazy_attachments && m.attachments.iter().any(|a| !a.data.is_empty()) {
            crate::private::create_dir_all(&dir).await?;
        }
        for (i, a) in m.attachments.iter().enumerate() {
            // Chemin NULL tant que le contenu n'est pas extrait (voir `materialize_attachment`).
            let path = if opts.lazy_attachments || (a.data.is_empty() && a.size > 0) {
                None
            } else {
                let fname = safe_filename(a.filename.as_deref().unwrap_or("attachment"));
                let path = dir.join(format!("{i}_{fname}"));
                crate::private::write(&path, &a.data).await?;
                Some(path.to_string_lossy().into_owned())
            };
            sqlx::query(
                "INSERT INTO attachments (message_id, filename, mime_type, size, content_id, is_inline, storage_path) \
                 VALUES (?,?,?,?,?,?,?)",
            )
            .bind(message_row)
            .bind(&a.filename)
            .bind(&a.mime_type)
            .bind(a.size as i64)
            .bind(&a.content_id)
            .bind(a.is_inline)
            .bind(path)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(thread_id)
}

/// Contenu de la pièce jointe de rang `rank` d'un message brut (fonction synchrone : le `Message`
/// de mail-parser ne traverse aucune attente).
fn extract_attachment(raw: &[u8], rank: usize) -> Result<Vec<u8>> {
    let msg = MessageParser::default().parse(raw).ok_or_else(|| {
        AppError::Invalid(msg!("ingest.raw_unreadable", "unreadable raw message"))
    })?;
    let part = msg.attachments().nth(rank).ok_or_else(|| {
        AppError::NotFound(msg!(
            "not_found.attachment_missing",
            "attachment missing from the message"
        ))
    })?;
    Ok(part.contents().to_vec())
}

/// Extrait une pièce jointe du message brut (historique rapatrié sans copie disque) et retourne
/// son chemin. L'ordre est celui de l'ingestion : rang parmi les pièces jointes du message.
pub async fn materialize_attachment(
    pool: &SqlitePool,
    attachments_dir: &Path,
    raw_dir: &Path,
    attachment_id: i64,
) -> Result<std::path::PathBuf> {
    let (message_id, filename, rank): (i64, Option<String>, i64) = sqlx::query_as(
        "SELECT message_id, filename, (SELECT COUNT(*) FROM attachments o WHERE o.message_id = a.message_id AND o.id < a.id) \
         FROM attachments a WHERE id = ?",
    )
    .bind(attachment_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(msg!("not_found.attachment", "attachment not found: {id}", id = attachment_id)))?;
    let (account_id, hash): (String, Option<String>) =
        sqlx::query_as("SELECT account_id, raw_hash FROM messages WHERE id = ?")
            .bind(message_id)
            .fetch_one(pool)
            .await?;
    let hash = hash.filter(|h| !h.is_empty()).ok_or_else(|| {
        AppError::NotFound(msg!("not_found.raw_unavailable", "raw message unavailable"))
    })?;
    let raw = crate::mail::rawstore::get(raw_dir, &hash).await?;
    // Analyse hors de tout `.await` : un `Message` mail-parser ne traverse pas d'attente.
    let data = extract_attachment(&raw, rank as usize)?;
    let dir = attachments_dir
        .join(&account_id)
        .join(message_id.to_string());
    crate::private::create_dir_all(&dir).await?;
    let path = dir.join(format!(
        "{rank}_{}",
        safe_filename(filename.as_deref().unwrap_or("attachment"))
    ));
    crate::private::write(&path, &data).await?;
    sqlx::query("UPDATE attachments SET storage_path = ? WHERE id = ?")
        .bind(path.to_string_lossy().as_ref())
        .bind(attachment_id)
        .execute(pool)
        .await?;
    Ok(path)
}

/// Recalcule le compteur de non lus d'un dossier.
pub async fn refresh_folder_unread(pool: &SqlitePool, folder_id: i64) -> Result<()> {
    sqlx::query("UPDATE folders SET unread_count = (SELECT COUNT(*) FROM messages WHERE folder_id = ?1 AND is_read = 0) WHERE id = ?1")
        .bind(folder_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Recalcule les agrégats des conversations touchées ; supprime celles devenues vides.
pub async fn refresh_threads(pool: &SqlitePool, ids: &HashSet<i64>) -> Result<()> {
    let mut tx = pool.begin().await?;
    for id in ids {
        sqlx::query(
            "UPDATE threads SET \
               message_count   = (SELECT COUNT(*) FROM messages WHERE thread_id = ?1), \
               unread_count    = (SELECT COUNT(*) FROM messages WHERE thread_id = ?1 AND is_read = 0), \
               is_starred      = COALESCE((SELECT MAX(is_starred) FROM messages WHERE thread_id = ?1), 0), \
               last_message_at = COALESCE((SELECT MAX(date) FROM messages WHERE thread_id = ?1), last_message_at), \
               snippet         = COALESCE((SELECT snippet FROM messages WHERE thread_id = ?1 ORDER BY date DESC LIMIT 1), snippet) \
             WHERE id = ?1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        crate::reminders::on_new_messages(&mut tx, *id).await?;
        sqlx::query("DELETE FROM threads WHERE id = ? AND message_count = 0")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use envelope_core::page::Page;
    use envelope_core::role::FolderRole;
    use envelope_core::search;

    const MSG1: &str = "From: Alice <alice@example.com>\r\nTo: bob@example.com\r\nSubject: Réunion budget\r\nMessage-ID: <m1@example.com>\r\nDate: Mon, 02 Mar 2026 10:00:00 +0000\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBonjour Bob, peut-on parler du budget trimestriel ?\r\n";
    const MSG2: &str = "From: Bob <bob@example.com>\r\nTo: alice@example.com\r\nSubject: Re: Réunion budget\r\nMessage-ID: <m2@example.com>\r\nIn-Reply-To: <m1@example.com>\r\nReferences: <m1@example.com>\r\nDate: Mon, 02 Mar 2026 11:00:00 +0000\r\n\r\nOui, demain.\r\n";
    const MSG_ATT: &str = "From: carol@example.com\r\nSubject: Facture\r\nMessage-ID: <m3@example.com>\r\nDate: Tue, 03 Mar 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nVoir pièce jointe.\r\n--b\r\nContent-Type: application/pdf; name=\"facture.pdf\"\r\nContent-Disposition: attachment; filename=\"facture.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nSGVsbG8=\r\n--b--\r\n";

    async fn setup() -> (SqlitePool, i64) {
        let pool = crate::db::init_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','a@x.fr','a','i','s')")
            .execute(&pool).await.unwrap();
        let f = sqlx::query(
            "INSERT INTO folders (account_id,path,name,role) VALUES ('a','INBOX','INBOX','inbox')",
        )
        .execute(&pool)
        .await
        .unwrap()
        .last_insert_rowid();
        (pool, f)
    }

    /// Dossiers triés par compte, puis rang du rôle (boîte de réception d'abord), puis nom.
    #[tokio::test]
    async fn dossiers_tries_par_role_puis_nom() {
        let pool = crate::db::init_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','a@x.fr','a','i','s')")
            .execute(&pool)
            .await
            .unwrap();
        for (path, role) in [
            ("Zèbre", None),
            ("Trash", Some(FolderRole::Trash)),
            ("Clients", None),
            ("Sent", Some(FolderRole::Sent)),
            ("INBOX", Some(FolderRole::Inbox)),
        ] {
            sqlx::query("INSERT INTO folders (account_id,path,name,role) VALUES ('a',?1,?1,?2)")
                .bind(path)
                .bind(role)
                .execute(&pool)
                .await
                .unwrap();
        }
        let folders = search::list_folders(&pool, Some("a")).await.unwrap();
        assert_eq!(
            folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["INBOX", "Sent", "Trash", "Clients", "Zèbre"]
        );
        assert_eq!(folders[0].role, Some(FolderRole::Inbox));
        assert_eq!(folders[4].role, None);
    }

    #[tokio::test]
    async fn threading_fts_and_attachments() {
        let (pool, folder) = setup().await;
        let dir = std::env::temp_dir().join(format!("envelope-test-{}", uuid::Uuid::new_v4()));
        let mut touched = HashSet::new();

        let t1 = store(
            &pool,
            &dir,
            MessageLoc {
                account_id: "a",
                folder_id: folder,
                uid: 1,
                size: None,
            },
            Flags::default(),
            parse(MSG1.as_bytes(), None).unwrap(),
        )
        .await
        .unwrap();
        let t2 = store(
            &pool,
            &dir,
            MessageLoc {
                account_id: "a",
                folder_id: folder,
                uid: 2,
                size: None,
            },
            Flags {
                seen: true,
                flagged: false,
            },
            parse(MSG2.as_bytes(), None).unwrap(),
        )
        .await
        .unwrap();
        let t3 = store(
            &pool,
            &dir,
            MessageLoc {
                account_id: "a",
                folder_id: folder,
                uid: 3,
                size: None,
            },
            Flags::default(),
            parse(MSG_ATT.as_bytes(), None).unwrap(),
        )
        .await
        .unwrap();
        // Ré-ingestion du même UID : idempotent.
        store(
            &pool,
            &dir,
            MessageLoc {
                account_id: "a",
                folder_id: folder,
                uid: 1,
                size: None,
            },
            Flags::default(),
            parse(MSG1.as_bytes(), None).unwrap(),
        )
        .await
        .unwrap();
        touched.extend([t1, t2, t3]);
        refresh_threads(&pool, &touched).await.unwrap();

        assert_eq!(t1, t2, "la réponse doit rejoindre la conversation");
        assert_ne!(t1, t3);
        let (count, unread): (i64, i64) =
            sqlx::query_as("SELECT message_count, unread_count FROM threads WHERE id=?")
                .bind(t1)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!((count, unread), (2, 1));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap(),
            3
        );

        // Plein texte : accents ignorés, préfixes.
        assert_eq!(
            search::search(&pool, "reunion", None, Page::first(10))
                .await
                .unwrap()
                .len(),
            2
        ); // sujet des 2 messages
        assert_eq!(
            search::search(&pool, "budg", None, Page::first(10))
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            search::search(&pool, "carol", None, Page::first(10))
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            search::search(&pool, "\"; DROP TABLE messages; --", None, Page::first(10))
                .await
                .unwrap()
                .len(),
            0
        );

        // Pièce jointe sur disque.
        let (name, path): (String, String) =
            sqlx::query_as("SELECT filename, storage_path FROM attachments")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(name, "facture.pdf");
        assert_eq!(std::fs::read(path).unwrap(), b"Hello");
        let has: bool = sqlx::query_scalar("SELECT has_attachments FROM messages WHERE uid=3")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(has);

        // Liste de conversations : tri, filtres, participants.
        use envelope_core::thread::{list_threads, ThreadQuery};
        let q = |f: fn(&mut ThreadQuery)| {
            let mut q = ThreadQuery {
                folder_id: Some(folder),
                ..Default::default()
            };
            f(&mut q);
            q
        };
        let all = list_threads(&pool, &q(|_| {})).await.unwrap();
        assert_eq!(
            all.iter().map(|t| t.subject.as_str()).collect::<Vec<_>>(),
            ["Facture", "Réunion budget"],
            "récent d'abord"
        );
        assert_eq!(all[1].participants.as_deref(), Some("Alice, Bob"));
        assert_eq!(
            all[1].from_addr.as_deref(),
            Some("bob@example.com"),
            "dernier expéditeur"
        );
        assert!(all[0].has_attachments && !all[1].has_attachments);
        assert_eq!(
            list_threads(&pool, &q(|q| q.with_attachments = true))
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            list_threads(&pool, &q(|q| q.unread_only = true))
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            list_threads(&pool, &q(|q| q.query = Some("budg".into())))
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            list_threads(&pool, &q(|q| q.query = Some("\"; DROP".into())))
                .await
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            list_threads(
                &pool,
                &ThreadQuery {
                    role: Some(FolderRole::Inbox),
                    ..Default::default()
                }
            )
            .await
            .unwrap()
            .len(),
            2
        );
        assert_eq!(
            list_threads(
                &pool,
                &ThreadQuery {
                    role: Some(FolderRole::Sent),
                    ..Default::default()
                }
            )
            .await
            .unwrap()
            .len(),
            0
        );
        let detail = envelope_core::thread::get_thread_detail(&pool, t1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(detail.messages.len(), 2);
        assert_eq!(detail.messages[0].to[0].address, "bob@example.com");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn search_operators_filter_threads() {
        use envelope_core::thread::{list_threads, ThreadQuery};
        let (pool, folder) = setup().await;
        let dir = std::env::temp_dir().join(format!("envelope-test-{}", uuid::Uuid::new_v4()));
        let mails = [
            (
                "Alice Martin <alice@x.fr>",
                "Budget 2026",
                "Mon, 2 Mar 2026 10:00:00 +0000",
                "chiffres du budget",
                false,
            ),
            (
                "Bob Durand <bob@y.fr>",
                "Photos",
                "Mon, 5 Jan 2026 10:00:00 +0000",
                "voir la pièce jointe",
                true,
            ),
            (
                "Alice Martin <alice@x.fr>",
                "Rappel 100% sûr",
                "Fri, 6 Feb 2026 10:00:00 +0000",
                "à bientôt",
                false,
            ),
        ];
        for (i, (from, subject, date, body, attach)) in mails.iter().enumerate() {
            let mut raw = format!("From: {from}\r\nTo: me@x.fr\r\nSubject: {subject}\r\nDate: {date}\r\nMessage-ID: <s{i}@x>\r\n");
            if *attach {
                raw += "MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=B\r\n\r\n--B\r\nContent-Type: text/plain\r\n\r\n";
                raw += body;
                raw += "\r\n--B\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"a.pdf\"\r\n\r\nJVBERg==\r\n--B--\r\n";
            } else {
                raw += &format!("\r\n{body}\r\n");
            }
            let loc = MessageLoc {
                account_id: "a",
                folder_id: folder,
                uid: i as i64 + 1,
                size: None,
            };
            store(
                &pool,
                &dir,
                loc,
                Flags::default(),
                parse(raw.as_bytes(), None).unwrap(),
            )
            .await
            .unwrap();
        }
        let find = |q: &str| {
            let pool = pool.clone();
            let q = q.to_string();
            async move {
                let mut subjects: Vec<String> = list_threads(
                    &pool,
                    &ThreadQuery {
                        query: Some(q),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .into_iter()
                .map(|t| t.subject)
                .collect();
                subjects.sort();
                subjects
            }
        };
        assert_eq!(find("de:alice").await, ["Budget 2026", "Rappel 100% sûr"]);
        assert_eq!(find("de:\"alice martin\" budget").await, ["Budget 2026"]);
        assert_eq!(find("de:durand").await, ["Photos"]);
        assert_eq!(find("pj").await, ["Photos"]);
        assert_eq!(find("avant:2026-02").await, ["Photos"]);
        assert_eq!(
            find("apres:2026-02").await,
            ["Budget 2026", "Rappel 100% sûr"]
        );
        assert_eq!(
            find("apres:2026-02 avant:2026-03").await,
            ["Rappel 100% sûr"]
        );
        assert_eq!(find("objet:100%").await, ["Rappel 100% sûr"]); // % littéral, pas un joker
        assert_eq!(find("objet:%").await, ["Rappel 100% sûr"]);
        assert_eq!(find("à:me@x.fr de:bob").await, ["Photos"]);
        assert_eq!(find("non-lu").await.len(), 3);
        assert_eq!(find("suivi").await.len(), 0);
        assert!(find("de:zzz").await.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn search_covers_every_folder_except_trash_and_spam() {
        use envelope_core::thread::{list_threads, ThreadQuery};
        let (pool, inbox) = setup().await;
        let dir = std::env::temp_dir().join(format!("envelope-test-{}", uuid::Uuid::new_v4()));
        let mut folders = vec![("INBOX", inbox)];
        for (path, role) in [
            ("Archive", Some("archive")),
            ("Trash", Some("trash")),
            ("Junk", Some("junk")),
            ("Clients", None),
        ] {
            let id = sqlx::query(
                "INSERT INTO folders (account_id,path,name,role) VALUES ('a',?1,?1,?2)",
            )
            .bind(path)
            .bind(role)
            .execute(&pool)
            .await
            .unwrap()
            .last_insert_rowid();
            folders.push((path, id));
        }
        for (i, (path, id)) in folders.iter().enumerate() {
            let raw = format!("From: Zoé <z@x.fr>\r\nTo: a@x.fr\r\nSubject: Rapport {path}\r\nDate: Mon, 2 Mar 2026 10:00:00 +0000\r\nMessage-ID: <f{i}@x>\r\n\r\nbudget annuel\r\n");
            let loc = MessageLoc {
                account_id: "a",
                folder_id: *id,
                uid: 1,
                size: None,
            };
            store(
                &pool,
                &dir,
                loc,
                Flags::default(),
                parse(raw.as_bytes(), None).unwrap(),
            )
            .await
            .unwrap();
        }
        let find = |q: &str| {
            let (pool, q) = (pool.clone(), q.to_string());
            async move {
                let rows = list_threads(
                    &pool,
                    &ThreadQuery {
                        query: Some(q),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
                // Le moteur renvoie l'identifiant du dossier ; le nom se lit dans la table des dossiers.
                let mut out = Vec::new();
                for t in rows {
                    let name: String = sqlx::query_scalar("SELECT name FROM folders WHERE id = ?")
                        .bind(t.folder_id.expect("folder_id renseigné"))
                        .fetch_one(&pool)
                        .await
                        .unwrap();
                    out.push((t.subject, Some(name)));
                }
                out.sort();
                out
            }
        };
        let names = |r: Vec<(String, Option<String>)>| {
            r.into_iter().filter_map(|(_, f)| f).collect::<Vec<_>>()
        };
        // Par défaut : tout sauf corbeille et spam, quel que soit le dossier d'où l'on cherche.
        assert_eq!(names(find("budget").await), ["Archive", "Clients", "INBOX"]);
        assert_eq!(names(find("dans:corbeille budget").await), ["Trash"]);
        assert_eq!(names(find("dans:spam").await), ["Junk"]);
        assert_eq!(names(find("dans:archives").await), ["Archive"]);
        assert_eq!(names(find("dans:clients").await), ["Clients"]);
        assert_eq!(names(find("dans:tout budget").await).len(), 5);
        assert_eq!(
            names(find("dans:archives dans:clients").await),
            ["Archive", "Clients"]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Une réponse reçue avant son parent (cas courant pendant le rapatriement, du plus récent au plus ancien)
    /// rejoint la conversation du parent : par In-Reply-To, ou par References quand l'intermédiaire manque.
    #[tokio::test]
    async fn replies_arriving_before_their_parent_join_its_thread() {
        let (pool, folder) = setup().await;
        let dir = std::env::temp_dir().join(format!("envelope-test-{}", uuid::Uuid::new_v4()));
        let mail = |id: &str, extra: &str| {
            format!("From: a@x.fr\r\nTo: me@x.fr\r\nSubject: Fil\r\nDate: Mon, 2 Mar 2026 10:00:00 +0000\r\nMessage-ID: <{id}@x>\r\n{extra}\r\ncorps\r\n")
        };
        // Petit-enfant (cite le parent p et l'enfant c ; répond à c), puis enfant (répond à p), puis le parent.
        let order = [
            ("g", "In-Reply-To: <c@x>\r\nReferences: <p@x> <c@x>\r\n"),
            ("c", "In-Reply-To: <p@x>\r\nReferences: <p@x>\r\n"),
            ("p", ""),
            ("autre", ""),
        ];
        let mut threads = Vec::new();
        for (uid, (id, extra)) in order.iter().enumerate() {
            let raw = mail(id, extra);
            let loc = MessageLoc {
                account_id: "a",
                folder_id: folder,
                uid: uid as i64 + 1,
                size: None,
            };
            threads.push(
                store(
                    &pool,
                    &dir,
                    loc,
                    Flags::default(),
                    parse(raw.as_bytes(), None).unwrap(),
                )
                .await
                .unwrap(),
            );
        }
        assert_eq!(
            threads[0], threads[1],
            "l'enfant rejoint le petit-enfant (il est cité par lui)"
        );
        assert_eq!(
            threads[1], threads[2],
            "le parent rejoint la conversation de ses descendants"
        );
        assert_ne!(threads[2], threads[3], "un message sans lien reste seul");
        let refs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM message_refs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(refs, 3, "références indexées : g cite p et c, c cite p");
        // Supprimer un message supprime ses références.
        sqlx::query("DELETE FROM messages WHERE message_id = 'g@x'")
            .execute(&pool)
            .await
            .unwrap();
        let refs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM message_refs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(refs, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn lazy_attachments_are_extracted_from_the_raw_message() {
        let (pool, folder) = setup().await;
        let base = std::env::temp_dir().join(format!("envelope-test-{}", uuid::Uuid::new_v4()));
        let (att_dir, raw_dir) = (base.join("att"), base.join("raw"));
        let raw = "From: a@x.fr\r\nTo: me@x.fr\r\nSubject: Devis\r\nDate: Mon, 2 Mar 2026 10:00:00 +0000\r\nMessage-ID: <lazy@x>\r\n\
MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=B\r\n\r\n--B\r\nContent-Type: text/plain\r\n\r\nvoir pièce jointe\r\n\
--B\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"devis.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nSGVsbG8gUERG\r\n--B--\r\n";
        let hash = crate::mail::rawstore::put(&raw_dir, raw.as_bytes())
            .await
            .unwrap();
        let loc = MessageLoc {
            account_id: "a",
            folder_id: folder,
            uid: 1,
            size: None,
        };
        let opts = StoreOpts {
            raw_hash: Some(&hash),
            lazy_attachments: true,
        };
        store_with(
            &pool,
            &att_dir,
            loc,
            Flags::default(),
            parse(raw.as_bytes(), None).unwrap(),
            opts,
        )
        .await
        .unwrap();

        let (id, path): (i64, Option<String>) =
            sqlx::query_as("SELECT id, storage_path FROM attachments")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(path.is_none(), "rien n'est écrit sur disque à l'ingestion");
        assert!(!att_dir.exists());
        let p = materialize_attachment(&pool, &att_dir, &raw_dir, id)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"Hello PDF");
        assert!(p.starts_with(&att_dir) && p.to_string_lossy().ends_with("devis.pdf"));
        let stored: Option<String> =
            sqlx::query_scalar("SELECT storage_path FROM attachments WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored.as_deref(), Some(p.to_string_lossy().as_ref()));
        // Message sans brut : pas d'extraction possible.
        sqlx::query("UPDATE messages SET raw_hash = ''")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE attachments SET storage_path = NULL")
            .execute(&pool)
            .await
            .unwrap();
        assert!(materialize_attachment(&pool, &att_dir, &raw_dir, id)
            .await
            .is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn list_unsubscribe_headers_are_parsed() {
        let raw = b"From: News <n@lists.example.com>\r\nTo: me@example.com\r\nSubject: Hi\r\n\
list-unsubscribe: <mailto:u@lists.example.com>,\r\n <https://lists.example.com/u/1>\r\n\
List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n\r\nbody";
        let u = parse(raw, None).unwrap().unsubscribe.unwrap();
        assert_eq!(u.url.as_deref(), Some("https://lists.example.com/u/1"));
        assert_eq!(u.mailto.as_deref(), Some("mailto:u@lists.example.com"));
        assert!(u.one_click);
        assert!(parse(b"From: a@b.fr\r\nSubject: x\r\n\r\nhi", None)
            .unwrap()
            .unsubscribe
            .is_none());
    }

    #[test]
    fn authentication_results_lit_en_tete_seulement() {
        let mail =
            |headers: &str| format!("{headers}From: a@b.fr\r\nSubject: x\r\n\r\nhi").into_bytes();
        // Notre serveur (en tête) signale l'échec ; un « pass » forgé par l'expéditeur plus bas ne l'efface pas.
        let raw = mail("Authentication-Results: mx.mon-serveur.fr; dmarc=fail header.from=b.fr\r\nAuthentication-Results: faux.test; dmarc=pass\r\n");
        assert_eq!(parse(&raw, None).unwrap().auth_fail, Some("dmarc"));
        assert_eq!(parse(&mail(""), None).unwrap().auth_fail, None);
        assert_eq!(
            parse(&mail("Authentication-Results: mx; dmarc=pass\r\n"), None)
                .unwrap()
                .auth_fail,
            None
        );
    }

    /// Insère un message minimal (le trigger du carnet d'adresses lit ses colonnes d'adresses).
    async fn raw_message(
        pool: &SqlitePool,
        (account, folder, uid): (&str, i64, i64),
        from: (&str, &str),
        to: &str,
        cc: &str,
        date: &str,
    ) {
        sqlx::query("INSERT INTO messages (account_id, folder_id, uid, from_name, from_addr, to_addrs, cc_addrs, date) VALUES (?,?,?,?,?,?,?,?)")
            .bind(account).bind(folder).bind(uid).bind(from.0).bind(from.1).bind(to).bind(cc).bind(date)
            .execute(pool).await.unwrap();
    }

    async fn contacts_of(pool: &SqlitePool) -> Vec<(String, String, Option<String>, i64, String)> {
        sqlx::query_as("SELECT account_id, address, name, uses, last_at FROM contacts ORDER BY account_id, address").fetch_all(pool).await.unwrap()
    }

    #[tokio::test]
    async fn carnet_d_adresses_tenu_par_trigger() {
        let (pool, folder) = setup().await;
        let bob = r#"[{"name":"Bob","address":"BOB@x.fr"}]"#;
        raw_message(
            &pool,
            ("a", folder, 1),
            ("Zoe", "Zoe@x.fr"),
            bob,
            r#"[{"name":"","address":"cc@x.fr"}]"#,
            "2026-01-01T00:00:00Z",
        )
        .await;
        // Plus récent : nouveau nom pour Bob. Plus ancien (rapatriement à rebours) : ne l'écrase pas.
        raw_message(
            &pool,
            ("a", folder, 2),
            ("Zoe", "zoe@x.fr"),
            r#"[{"name":"Robert","address":"bob@x.fr"}]"#,
            "[]",
            "2026-03-01T00:00:00Z",
        )
        .await;
        raw_message(
            &pool,
            ("a", folder, 3),
            ("", "zoe@x.fr"),
            r#"[{"name":"Bobby","address":"bob@x.fr"}]"#,
            "[]",
            "2025-06-01T00:00:00Z",
        )
        .await;
        let c = contacts_of(&pool).await;
        let get = |addr: &str| {
            c.iter()
                .find(|r| r.1 == addr)
                .unwrap_or_else(|| panic!("{addr} absent de {c:?}"))
                .clone()
        };
        // Adresses en minuscules, comptées une fois par apparition.
        assert_eq!(
            (get("bob@x.fr").3, get("zoe@x.fr").3, get("cc@x.fr").3),
            (3, 3, 1)
        );
        assert_eq!(
            get("bob@x.fr").2.as_deref(),
            Some("Robert"),
            "le nom du message le plus récent l'emporte"
        );
        assert_eq!(get("bob@x.fr").4, "2026-03-01T00:00:00Z");
        assert_eq!(get("cc@x.fr").2, None, "un nom vide n'est pas un nom");
        assert_eq!(
            get("zoe@x.fr").2.as_deref(),
            Some("Zoe"),
            "un nom vide ne remplace pas le nom connu"
        );

        // La suggestion s'appuie sur la table.
        let found = envelope_core::contacts::suggest(&pool, Some("a"), "bo", 8)
            .await
            .unwrap();
        assert_eq!(
            found.iter().map(|a| a.address.as_str()).collect::<Vec<_>>(),
            ["bob@x.fr"]
        );

        // Les messages supprimés ne retirent pas l'adresse du carnet.
        sqlx::query("DELETE FROM messages")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(contacts_of(&pool).await.len(), c.len());
    }

    #[tokio::test]
    async fn un_json_d_adresses_invalide_ne_bloque_pas_l_insertion() {
        let (pool, folder) = setup().await;
        raw_message(
            &pool,
            ("a", folder, 1),
            ("Zoe", "zoe@x.fr"),
            "pas du json",
            "{{",
            "2026-01-01T00:00:00Z",
        )
        .await;
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1, "le message est bien enregistré");
        assert_eq!(
            contacts_of(&pool)
                .await
                .iter()
                .map(|r| r.1.as_str())
                .collect::<Vec<_>>(),
            ["zoe@x.fr"],
            "seul l'expéditeur est retenu"
        );
    }

    #[tokio::test]
    async fn carnet_cloisonne_par_compte_et_supprime_avec_lui() {
        let (pool, folder) = setup().await;
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('b','b@x.fr','b','i','s')").execute(&pool).await.unwrap();
        let f2 = sqlx::query(
            "INSERT INTO folders (account_id,path,name,role) VALUES ('b','INBOX','INBOX','inbox')",
        )
        .execute(&pool)
        .await
        .unwrap()
        .last_insert_rowid();
        raw_message(
            &pool,
            ("a", folder, 1),
            ("Zoe", "zoe@x.fr"),
            "[]",
            "[]",
            "2026-01-01T00:00:00Z",
        )
        .await;
        raw_message(
            &pool,
            ("b", f2, 1),
            ("Yan", "yan@x.fr"),
            "[]",
            "[]",
            "2026-01-01T00:00:00Z",
        )
        .await;
        let names = |a: Option<&'static str>| {
            let pool = pool.clone();
            async move {
                envelope_core::contacts::suggest(&pool, a, "@x.fr", 8)
                    .await
                    .unwrap()
                    .len()
            }
        };
        assert_eq!(
            (
                names(Some("a")).await,
                names(Some("b")).await,
                names(None).await
            ),
            (1, 1, 2)
        );
        sqlx::query("DELETE FROM accounts WHERE id = 'b'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            names(None).await,
            1,
            "le carnet d'un compte supprimé disparaît avec lui"
        );
    }

    /// Un brouillon est ré-enregistré à chaque frappe : ses destinataires n'entrent pas dans le carnet, ni par le trigger
    /// ni par la reprise de l'existant.
    #[tokio::test]
    async fn les_brouillons_ne_comptent_pas_dans_le_carnet() {
        let (pool, inbox) = setup().await;
        let drafts = sqlx::query("INSERT INTO folders (account_id,path,name,role) VALUES ('a','Drafts','Drafts','drafts')").execute(&pool).await.unwrap().last_insert_rowid();
        let to = r#"[{"name":"Tapé","address":"efface@x.fr"}]"#;
        raw_message(
            &pool,
            ("a", drafts, 1),
            ("Moi", "a@x.fr"),
            to,
            "[]",
            "2026-01-01T00:00:00Z",
        )
        .await;
        raw_message(
            &pool,
            ("a", inbox, 1),
            ("Zoe", "zoe@x.fr"),
            "[]",
            "[]",
            "2026-01-02T00:00:00Z",
        )
        .await;
        let addresses = |c: Vec<(String, String, Option<String>, i64, String)>| {
            c.into_iter().map(|r| r.1).collect::<Vec<_>>()
        };
        assert_eq!(addresses(contacts_of(&pool).await), ["zoe@x.fr"]);
        let migration = include_str!("../../migrations/0020_contacts.sql");
        let backfill = migration
            .split("-- BACKFILL-BEGIN")
            .nth(1)
            .and_then(|b| b.split("-- BACKFILL-END").next())
            .expect("marqueurs BACKFILL absents");
        sqlx::query("DELETE FROM contacts")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(sqlx::AssertSqlSafe(backfill.to_string()))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(addresses(contacts_of(&pool).await), ["zoe@x.fr"]);
    }

    /// La reprise de l'existant (migration 0020) donne le même carnet que le trigger.
    #[tokio::test]
    async fn reprise_de_l_existant_equivalente_au_trigger() {
        let (pool, folder) = setup().await;
        let bob = r#"[{"name":"Bob","address":"bob@x.fr"},{"name":"","address":"Al@x.fr"}]"#;
        for (uid, date) in [
            (1, "2026-01-01T00:00:00Z"),
            (2, "2026-02-01T00:00:00Z"),
            (3, "2026-03-01T00:00:00Z"),
        ] {
            raw_message(
                &pool,
                ("a", folder, uid),
                ("Zoe", "zoe@x.fr"),
                bob,
                r#"[{"address":"cc@x.fr"}]"#,
                date,
            )
            .await;
        }
        let by_trigger = contacts_of(&pool).await;
        let migration = include_str!("../../migrations/0020_contacts.sql");
        let backfill = migration
            .split("-- BACKFILL-BEGIN")
            .nth(1)
            .unwrap()
            .split("-- BACKFILL-END")
            .next()
            .unwrap();
        sqlx::query("DELETE FROM contacts")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(sqlx::AssertSqlSafe(backfill.to_string()))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(contacts_of(&pool).await, by_trigger);
    }

    /// Un message est écrit en entier ou pas du tout : si l'écriture d'une pièce jointe échoue, ni le message, ni sa
    /// conversation, ni ses références ne restent en base (sinon `ON CONFLICT DO NOTHING` interdirait de réessayer).
    #[tokio::test]
    async fn message_ecrit_en_entier_ou_pas_du_tout() {
        let (pool, folder) = setup().await;
        let base = std::env::temp_dir().join(format!("envelope-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let fichier = base.join("pas-un-dossier");
        std::fs::write(&fichier, b"x").unwrap();
        let loc = MessageLoc {
            account_id: "a",
            folder_id: folder,
            uid: 1,
            size: None,
        };
        // Le dossier des pièces jointes est impossible à créer (son parent est un fichier) : l'écriture échoue.
        let mut parsed = parse(MSG_ATT.as_bytes(), None).unwrap();
        parsed.references = vec!["<parent@x>".into()];
        let echec = store_with(
            &pool,
            &fichier,
            loc,
            Flags::default(),
            parsed,
            StoreOpts::default(),
        )
        .await;
        assert!(echec.is_err());
        let compte = |table: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
                    "SELECT COUNT(*) FROM {table} WHERE account_id = 'a'"
                )))
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        assert_eq!(
            (
                compte("messages").await,
                compte("threads").await,
                compte("message_refs").await
            ),
            (0, 0, 0),
            "rien ne reste"
        );
        // Réessayer avec un dossier valide réussit (le message n'a pas été « consommé » par l'échec).
        let loc = MessageLoc {
            account_id: "a",
            folder_id: folder,
            uid: 1,
            size: None,
        };
        store_with(
            &pool,
            &base.join("ok"),
            loc,
            Flags::default(),
            parse(MSG_ATT.as_bytes(), None).unwrap(),
            StoreOpts::default(),
        )
        .await
        .unwrap();
        assert_eq!(compte("messages").await, 1);
    }

    /// Un message déjà présent n'est pas réécrit, et l'identifiant de conversation rendu est celui du message existant.
    #[tokio::test]
    async fn message_deja_present_rend_sa_conversation() {
        let (pool, folder) = setup().await;
        let dir = std::env::temp_dir().join(format!("envelope-test-{}", uuid::Uuid::new_v4()));
        let loc = || MessageLoc {
            account_id: "a",
            folder_id: folder,
            uid: 7,
            size: None,
        };
        let first = store(
            &pool,
            &dir,
            loc(),
            Flags::default(),
            parse(MSG1.as_bytes(), None).unwrap(),
        )
        .await
        .unwrap();
        let again = store(
            &pool,
            &dir,
            loc(),
            Flags::default(),
            parse(MSG1.as_bytes(), None).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(first, again);
        let threads: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM threads")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(threads, 1, "pas de conversation fantôme");
    }

    /// Petites pièces jointes et images « inline » : écrites tout de suite. Grosses pièces à ouvrir : métadonnées
    /// seulement à l'arrivée, contenu extrait du brut à la première ouverture ; même contenu dans les deux cas.
    #[tokio::test]
    async fn grosses_pieces_jointes_extraites_du_brut_seulement_a_l_ouverture() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let (pool, folder) = setup().await;
        let base = std::env::temp_dir().join(format!("envelope-test-{}", uuid::Uuid::new_v4()));
        let (att_dir, raw_dir) = (base.join("att"), base.join("raw"));
        let bytes = |n: usize| (0..n).map(|i| (i % 251) as u8).collect::<Vec<u8>>();
        let big = bytes(EAGER_ATTACHMENT_MAX + 1);
        // (nom, type, Content-ID, contenu)
        let parts: [(&str, &str, Option<&str>, Vec<u8>); 4] = [
            (
                "petit.bin",
                "application/octet-stream",
                None,
                b"petit".to_vec(),
            ),
            ("gros.bin", "application/octet-stream", None, big.clone()),
            (
                "logo.png",
                "image/png",
                Some("<logo@x>"),
                bytes(INLINE_IMAGE_MAX),
            ),
            (
                "photo.png",
                "image/png",
                Some("<photo@x>"),
                bytes(INLINE_IMAGE_MAX + 1),
            ),
        ];
        let mut raw = String::from("From: a@x.fr\r\nTo: me@x.fr\r\nSubject: Gros\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=B\r\n\r\n--B\r\nContent-Type: text/plain\r\n\r\nbonjour\r\n");
        for (name, mime, cid, data) in &parts {
            let b64 = STANDARD
                .encode(data)
                .as_bytes()
                .chunks(76)
                .map(|c| std::str::from_utf8(c).unwrap())
                .collect::<Vec<_>>()
                .join("\r\n");
            let cid = cid
                .map(|c| format!("Content-ID: {c}\r\n"))
                .unwrap_or_default();
            raw += &format!("--B\r\nContent-Type: {mime}\r\n{cid}Content-Disposition: attachment; filename=\"{name}\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{b64}\r\n");
        }
        raw += "--B--\r\n";
        let parsed = parse(raw.as_bytes(), None).unwrap();
        let copied: Vec<bool> = parsed
            .attachments
            .iter()
            .map(|a| !a.data.is_empty())
            .collect();
        assert_eq!(
            copied,
            [true, false, true, false],
            "petit et image inline ≤ 2 Mio copiés ; gros document et image géante non"
        );
        assert!(
            parsed
                .attachments
                .iter()
                .zip(&parts)
                .all(|(a, p)| a.size == p.3.len()),
            "la taille est toujours connue"
        );

        let hash = crate::mail::rawstore::put(&raw_dir, raw.as_bytes())
            .await
            .unwrap();
        let loc = MessageLoc {
            account_id: "a",
            folder_id: folder,
            uid: 1,
            size: None,
        };
        store_with(
            &pool,
            &att_dir,
            loc,
            Flags::default(),
            parsed,
            StoreOpts {
                raw_hash: Some(&hash),
                lazy_attachments: false,
            },
        )
        .await
        .unwrap();
        let rows: Vec<(i64, Option<String>)> =
            sqlx::query_as("SELECT id, storage_path FROM attachments ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            rows.iter().map(|r| r.1.is_some()).collect::<Vec<_>>(),
            copied
        );
        // Sans message brut, une grosse pièce jointe serait perdue : `store_with` refuse.
        let sans_brut = parse(raw.as_bytes(), None).unwrap();
        let loc = MessageLoc {
            account_id: "a",
            folder_id: folder,
            uid: 2,
            size: None,
        };
        let refus = store_with(
            &pool,
            &att_dir,
            loc,
            Flags::default(),
            sans_brut,
            StoreOpts::default(),
        )
        .await;
        assert!(refus.is_err(), "grosse pièce jointe sans raw_hash : refusé");
        for (row, part) in rows.iter().zip(&parts) {
            let path = materialize_attachment(&pool, &att_dir, &raw_dir, row.0)
                .await
                .unwrap();
            assert_eq!(
                std::fs::read(path).unwrap(),
                part.3,
                "{} : extraite du brut, octet pour octet",
                part.0
            );
        }
    }

    /// L'index plein texte suit les colonnes qu'il couvre (objet, expéditeur, texte) et seulement elles : un changement
    /// de drapeau, de dossier ou d'empreinte ne le touche pas, mais ne doit jamais le laisser incohérent.
    #[tokio::test]
    async fn index_plein_texte_suit_les_colonnes_indexees_seulement() {
        let (pool, folder) = setup().await;
        let dir = std::env::temp_dir().join(format!("envelope-test-{}", uuid::Uuid::new_v4()));
        let loc = |uid| MessageLoc {
            account_id: "a",
            folder_id: folder,
            uid,
            size: None,
        };
        store(
            &pool,
            &dir,
            loc(1),
            Flags::default(),
            parse(
                b"From: Zoe <z@x.fr>\r\nSubject: Alpha\r\nMessage-ID: <1@x>\r\n\r\nlicorne",
                None,
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let found = |q: &'static str| {
            let pool = pool.clone();
            async move {
                envelope_core::search::search(&pool, q, None, Page::first(10))
                    .await
                    .unwrap()
                    .len()
            }
        };
        assert_eq!((found("licorne").await, found("alpha").await), (1, 1));

        // Drapeaux, dossier, empreinte : l'index reste exact.
        sqlx::query("UPDATE messages SET is_read = 1, is_starred = 1, raw_hash = 'abc', thread_id = thread_id").execute(&pool).await.unwrap();
        assert_eq!((found("licorne").await, found("alpha").await), (1, 1));

        // Corps et objet : l'ancien texte disparaît, le nouveau est trouvé.
        sqlx::query("UPDATE messages SET body_text = 'dragon', subject = 'Beta'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!((found("licorne").await, found("alpha").await), (0, 0));
        assert_eq!((found("dragon").await, found("beta").await), (1, 1));

        // Suppression : plus rien dans l'index.
        sqlx::query("DELETE FROM messages")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(found("dragon").await, 0);
    }

    /// La recherche par `Message-ID` passe par l'index sur `message_id`, jamais par un balayage des messages du compte
    /// (régression constatée : coût par message proportionnel à la taille de la base).
    #[tokio::test]
    async fn recherche_de_conversation_utilise_l_index_message_id() {
        let (pool, _) = setup().await;
        let sql = format!("EXPLAIN QUERY PLAN {FIND_THREAD_BY_IDS}'a' AND message_id IN ('x','y') ORDER BY date DESC LIMIT 1");
        let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .fetch_all(&pool)
            .await
            .unwrap();
        let plan = plan
            .into_iter()
            .map(|(.., detail)| detail)
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(plan.contains("idx_messages_message_id"), "{plan}");
    }

    #[test]
    fn subject_normalization() {
        assert_eq!(normalize_subject("Re: RE: Fwd: Hello"), "Hello");
        // Préfixes d'autres langues et formes numérotées, qu'une liste maison oubliait.
        assert_eq!(normalize_subject("AW: Angebot"), "Angebot");
        assert_eq!(normalize_subject("SV: Re: Möte"), "Möte");
        assert_eq!(normalize_subject("Re[2]: devis"), "devis");
        assert_eq!(normalize_subject("  TR: facture (fwd)"), "facture");
        // Un « : » du vrai sujet n'est pas un préfixe ; sujet vide ou réduit à un préfixe : jamais de titre vide.
        assert_eq!(normalize_subject("Réunion: demain"), "Réunion: demain");
        assert_eq!(normalize_subject("Re:"), "Re:");
        assert_eq!(normalize_subject(""), "");
    }
    #[tokio::test]
    async fn reply_headers_and_contact_suggestions() {
        let (pool, folder) = setup().await;
        let dir = std::env::temp_dir().join(format!("envelope-test-{}", uuid::Uuid::new_v4()));
        let mut thread = 0;
        for (uid, raw) in [(1, MSG1), (2, MSG2)] {
            let loc = MessageLoc {
                account_id: "a",
                folder_id: folder,
                uid,
                size: None,
            };
            thread = store(
                &pool,
                &dir,
                loc,
                Flags::default(),
                parse(raw.as_bytes(), None).unwrap(),
            )
            .await
            .unwrap();
        }
        let detail = envelope_core::thread::get_thread_detail(&pool, thread)
            .await
            .unwrap()
            .unwrap();
        let last = detail.messages.last().unwrap();
        assert_eq!(last.message_id.as_deref(), Some("m2@example.com"));
        assert_eq!(last.references, vec!["m1@example.com".to_string()]);

        let found = envelope_core::contacts::suggest(&pool, Some("a"), "ali", 8)
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].address, "alice@example.com");
        // L'adresse d'un compte n'est jamais proposée ; % et _ saisis sont littéraux.
        sqlx::query("UPDATE accounts SET email = 'alice@example.com' WHERE id = 'a'")
            .execute(&pool)
            .await
            .unwrap();
        let others = envelope_core::contacts::suggest(&pool, None, "example", 8)
            .await
            .unwrap();
        assert_eq!(
            others
                .iter()
                .map(|a| a.address.as_str())
                .collect::<Vec<_>>(),
            vec!["bob@example.com"]
        );
        assert!(envelope_core::contacts::suggest(&pool, None, "%", 8)
            .await
            .unwrap()
            .is_empty());
    }
}

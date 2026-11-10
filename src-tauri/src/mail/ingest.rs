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

use crate::error::Result;

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

/// Retire les préfixes « Re: », « Fwd: »… pour le sujet de conversation.
pub fn normalize_subject(s: &str) -> String {
    let mut s = s.trim();
    loop {
        let lower = s.to_lowercase();
        let stripped = ["re:", "fwd:", "fw:", "tr:", "rv:"]
            .iter()
            .find(|p| lower.starts_with(**p));
        match stripped {
            Some(p) => s = s[p.len()..].trim_start(),
            None => return s.to_string(),
        }
    }
}

/// `fallback_ts` : INTERNALDATE du serveur, utilisée si l'en-tête Date est absent/invalide.
pub fn parse(raw: &[u8], fallback_ts: Option<i64>) -> Option<ParsedMessage> {
    let msg = MessageParser::default().parse(raw)?;
    let from = msg.from().and_then(|a| a.first());
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
            ParsedAttachment {
                filename: p.attachment_name().map(str::to_string),
                mime_type: mime,
                content_id: p
                    .content_id()
                    .map(|c| c.trim_matches(|ch| ch == '<' || ch == '>').to_string()),
                is_inline: p
                    .content_disposition()
                    .is_some_and(|d| d.ctype().eq_ignore_ascii_case("inline")),
                data: p.contents().to_vec(),
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
    })
}

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
        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT thread_id FROM messages WHERE thread_id IS NOT NULL AND account_id = ",
        );
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
    // Réponses arrivées avant leur parent.
    if let Some(id) = &m.message_id {
        let t = sqlx::query_scalar::<_, i64>(
            "SELECT thread_id FROM messages WHERE thread_id IS NOT NULL AND account_id = ?1 \
             AND (in_reply_to = ?2 OR instr(references_ids, ?3) > 0) LIMIT 1",
        )
        .bind(account_id)
        .bind(id)
        .bind(serde_json::to_string(id)?) // "<id>" entre guillemets JSON
        .fetch_optional(pool)
        .await?;
        return Ok(t);
    }
    Ok(None)
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

pub async fn store(
    pool: &SqlitePool,
    attachments_dir: &Path,
    loc: MessageLoc<'_>,
    flags: Flags,
    m: ParsedMessage,
) -> Result<i64> {
    let MessageLoc {
        account_id,
        folder_id,
        uid,
        size,
    } = loc;
    let thread_id = match find_thread(pool, account_id, &m).await? {
        Some(t) => t,
        None => sqlx::query(
            "INSERT INTO threads (account_id, subject, snippet, last_message_at) VALUES (?,?,?,?)",
        )
        .bind(account_id)
        .bind(normalize_subject(&m.subject))
        .bind(&m.snippet)
        .bind(&m.date)
        .execute(pool)
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
         is_read, is_starred, has_attachments, size) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) \
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
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Ok(thread_id); // déjà présent
    }
    let message_row = res.last_insert_rowid();

    if !m.attachments.is_empty() {
        let dir = attachments_dir
            .join(account_id)
            .join(message_row.to_string());
        tokio::fs::create_dir_all(&dir).await?;
        for (i, a) in m.attachments.iter().enumerate() {
            let fname = safe_filename(a.filename.as_deref().unwrap_or("attachment"));
            let path = dir.join(format!("{i}_{fname}"));
            tokio::fs::write(&path, &a.data).await?;
            sqlx::query(
                "INSERT INTO attachments (message_id, filename, mime_type, size, content_id, is_inline, storage_path) \
                 VALUES (?,?,?,?,?,?,?)",
            )
            .bind(message_row)
            .bind(&a.filename)
            .bind(&a.mime_type)
            .bind(a.data.len() as i64)
            .bind(&a.content_id)
            .bind(a.is_inline)
            .bind(path.to_string_lossy().as_ref())
            .execute(pool)
            .await?;
        }
    }
    Ok(thread_id)
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
            search::search(&pool, "reunion", None, 10)
                .await
                .unwrap()
                .len(),
            2
        ); // sujet des 2 messages
        assert_eq!(
            search::search(&pool, "budg", None, 10).await.unwrap().len(),
            2
        );
        assert_eq!(
            search::search(&pool, "carol", None, 10)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            search::search(&pool, "\"; DROP TABLE messages; --", None, 10)
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
                    role: Some("inbox".into()),
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
                    role: Some("sent".into()),
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

    #[test]
    fn subject_normalization() {
        assert_eq!(normalize_subject("Re: RE: Fwd: Hello"), "Hello");
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
        sqlx::query("UPDATE accounts SET email = 'alice@example.com'")
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

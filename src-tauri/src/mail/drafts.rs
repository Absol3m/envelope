//! Brouillons synchronisés : enregistrés dans le dossier « Brouillons » du serveur (IMAP `APPEND` avec
//! l'indicateur `\Draft`), donc retrouvés sur un autre appareil ou dans un autre client.
//!
//! Un message IMAP ne se modifie pas : chaque enregistrement dépose la **nouvelle version**, puis
//! supprime les anciennes. Les versions sont reliées par l'en-tête `X-Envelope-Draft-Id`, que la
//! synchro lit à l'arrivée (colonne `messages.draft_id`) : on retrouve ainsi ce qu'il faut remplacer,
//! même après une resynchronisation ou un redémarrage.
//!
//! L'ordre protège le brouillon : la nouvelle version est déposée **avant** de supprimer l'ancienne.
//! Un échec entre les deux laisse deux versions (la plus récente gagne à la réouverture), jamais zéro.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use base64::{engine::general_purpose::STANDARD, Engine};
use envelope_core::addr;
use serde::Serialize;
use sqlx::SqlitePool;

use crate::error::{AppError, Result};
use crate::mail::account::{Account, Credential};
use crate::mail::smtp::{self, OutgoingMessage};
use crate::mail::{actions, imap, ingest, rawstore};

/// Version d'un brouillon connue de la base (donc déjà synchronisée depuis le serveur).
#[derive(Debug, sqlx::FromRow)]
struct Version {
    id: i64,
    uid: i64,
    path: String,
    folder_id: i64,
    thread_id: Option<i64>,
}

async fn copies(pool: &SqlitePool, account_id: &str, draft_id: &str) -> Result<Vec<Version>> {
    Ok(sqlx::query_as(
        "SELECT m.id, m.uid, f.path, m.folder_id, m.thread_id FROM messages m JOIN folders f ON f.id = m.folder_id \
         WHERE m.account_id = ? AND m.draft_id = ? ORDER BY m.uid",
    )
    .bind(account_id)
    .bind(draft_id)
    .fetch_all(pool)
    .await?)
}

/// Dernière version déposée sur le serveur (mémorisée à chaque enregistrement).
#[derive(Debug, sqlx::FromRow)]
struct Recorded {
    path: String,
    uid_validity: Option<i64>,
    uid: i64,
}

async fn recorded(pool: &SqlitePool, account_id: &str, draft_id: &str) -> Result<Option<Recorded>> {
    Ok(sqlx::query_as(
        "SELECT path, uid_validity, uid FROM draft_versions WHERE account_id = ? AND draft_id = ?",
    )
    .bind(account_id)
    .bind(draft_id)
    .fetch_optional(pool)
    .await?)
}

/// Les UID ne valent que dans une « validité » donnée du dossier : si elle a changé, l'UID mémorisé ne désigne plus rien.
fn same_mailbox(r: &Recorded, path: &str, uid_validity: Option<i64>) -> bool {
    r.path == path && r.uid_validity == uid_validity
}

/// Supprime des versions du serveur (un EXPUNGE ciblé par dossier), puis de la base celles qu'elle connaît.
async fn remove_versions(
    pool: &SqlitePool,
    session: &mut imap::ImapSession,
    server: &BTreeMap<String, BTreeSet<i64>>,
    known: &[Version],
) -> Result<()> {
    for (path, uids) in server {
        if !uids.is_empty() {
            actions::delete_on_server(session, path, &uids.iter().copied().collect::<Vec<_>>())
                .await?;
        }
    }
    for c in known {
        sqlx::query("DELETE FROM messages WHERE id = ?")
            .bind(c.id)
            .execute(pool)
            .await?;
    }
    let threads: HashSet<i64> = known.iter().filter_map(|c| c.thread_id).collect();
    ingest::refresh_threads(pool, &threads).await?;
    for f in known.iter().map(|c| c.folder_id).collect::<HashSet<_>>() {
        ingest::refresh_folder_unread(pool, f).await?;
    }
    Ok(())
}

/// UID des versions à supprimer : celle mémorisée (si le dossier est le même) et celles que la synchro connaît.
fn targets(
    known: &[Version],
    last: Option<&Recorded>,
    path: &str,
    uid_validity: Option<i64>,
) -> BTreeMap<String, BTreeSet<i64>> {
    let mut server: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
    for c in known {
        server.entry(c.path.clone()).or_default().insert(c.uid);
    }
    if let Some(r) = last.filter(|r| same_mailbox(r, path, uid_validity)) {
        server.entry(r.path.clone()).or_default().insert(r.uid);
    }
    server
}

/// Enregistre le brouillon sur le serveur : dépose la nouvelle version, puis supprime les précédentes.
pub async fn save(
    pool: &SqlitePool,
    account: &Account,
    cred: &Credential,
    out: &OutgoingMessage,
) -> Result<()> {
    let draft_id = out
        .draft_id
        .as_deref()
        .ok_or_else(|| AppError::Invalid("identifiant de brouillon manquant".into()))?;
    let raw = smtp::build_draft(account, out)?.formatted();
    // Versions connues avant le dépôt, pour ne jamais supprimer la nouvelle.
    let known = copies(pool, &account.id, draft_id).await?;
    let last = recorded(pool, &account.id, draft_id).await?;
    let mut session = imap::connect(account, cred).await?;
    let result = async {
        let path = imap::find_or_create(&mut session, "drafts", "Drafts").await?;
        let uid_validity = session.select(&path).await?.uid_validity.map(i64::from);
        session
            .append(&path, Some("(\\Draft \\Seen)"), None, &raw)
            .await?;
        // La nouvelle version a le plus grand UID du dossier (liste complète, fiable : la recherche, elle, ne l'est pas aussitôt).
        let new_uid = session
            .uid_search("ALL")
            .await?
            .into_iter()
            .max()
            .map(i64::from)
            .ok_or_else(|| {
                AppError::Invalid("brouillon déposé mais introuvable dans le dossier".into())
            })?;
        let mut server = targets(&known, last.as_ref(), &path, uid_validity);
        if let Some(uids) = server.get_mut(&path) {
            uids.remove(&new_uid);
        }
        let known: Vec<Version> = known
            .into_iter()
            .filter(|c| !(c.path == path && c.uid == new_uid))
            .collect();
        record(pool, &account.id, draft_id, &path, uid_validity, new_uid).await?;
        remove_versions(pool, &mut session, &server, &known).await
    }
    .await;
    let _ = session.logout().await;
    result
}

async fn record(
    pool: &SqlitePool,
    account_id: &str,
    draft_id: &str,
    path: &str,
    uid_validity: Option<i64>,
    uid: i64,
) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO draft_versions (account_id, draft_id, path, uid_validity, uid) VALUES (?, ?, ?, ?, ?)")
        .bind(account_id)
        .bind(draft_id)
        .bind(path)
        .bind(uid_validity)
        .bind(uid)
        .execute(pool)
        .await?;
    Ok(())
}

/// Supprime toutes les versions de ce brouillon (envoi réussi, ou brouillon abandonné).
pub async fn remove(
    pool: &SqlitePool,
    account: &Account,
    cred: &Credential,
    draft_id: &str,
) -> Result<()> {
    let known = copies(pool, &account.id, draft_id).await?;
    let last = recorded(pool, &account.id, draft_id).await?;
    if known.is_empty() && last.is_none() {
        return Ok(());
    }
    let mut session = imap::connect(account, cred).await?;
    let result = async {
        // La validité du dossier se lit en le sélectionnant : un UID mémorisé d'avant un changement ne vaut plus rien.
        let (path, uid_validity) = match &last {
            Some(r) => (
                r.path.clone(),
                session.select(&r.path).await?.uid_validity.map(i64::from),
            ),
            None => (String::new(), None),
        };
        let server = targets(&known, last.as_ref(), &path, uid_validity);
        remove_versions(pool, &mut session, &server, &known).await?;
        sqlx::query("DELETE FROM draft_versions WHERE account_id = ? AND draft_id = ?")
            .bind(&account.id)
            .bind(draft_id)
            .execute(pool)
            .await?;
        Ok(())
    }
    .await;
    let _ = session.logout().await;
    result
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DraftAttachment {
    pub filename: String,
    pub mime_type: String,
    pub data_base64: String,
}

/// Brouillon relu depuis le serveur, prêt à rouvrir dans le composer.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftData {
    pub id: String,
    pub account_id: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_html: Option<String>,
    pub body_text: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub attachments: Vec<DraftAttachment>,
    /// Adresse d'expéditeur écrite dans le brouillon (celle du compte, ou un alias).
    pub from_email: String,
}

/// Lit un brouillon dans son message brut.
pub fn parse_draft(raw: &[u8], id: String, account_id: String) -> Result<DraftData> {
    let parsed =
        ingest::parse(raw, None).ok_or_else(|| AppError::Invalid("brouillon illisible".into()))?;
    let list = |json: &str| {
        addr::decode(json)
            .iter()
            .map(addr::Addr::display)
            .collect::<Vec<_>>()
    };
    Ok(DraftData {
        id,
        account_id,
        to: list(&parsed.to),
        cc: list(&parsed.cc),
        bcc: list(&parsed.bcc),
        subject: parsed.subject,
        body_html: parsed.body_html,
        body_text: parsed.body_text.unwrap_or_default(),
        in_reply_to: parsed.in_reply_to,
        references: parsed.references,
        from_email: parsed.from_addr,
        attachments: parsed
            .attachments
            .into_iter()
            .filter(|a| !a.is_inline || a.filename.is_some())
            .map(|a| DraftAttachment {
                filename: a.filename.unwrap_or_else(|| "pièce-jointe".into()),
                mime_type: a.mime_type,
                data_base64: STANDARD.encode(&a.data),
            })
            .collect(),
    })
}

/// Rouvre le brouillon d'une conversation (sa version la plus récente dans un dossier Brouillons).
/// Un brouillon créé par un autre client n'a pas d'identifiant : on lui en donne un, pour que
/// l'enregistrement suivant le remplace au lieu de le dupliquer.
pub async fn open(pool: &SqlitePool, raw_dir: &Path, thread_id: i64) -> Result<DraftData> {
    let row: Option<(i64, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT m.id, m.account_id, m.raw_hash, m.draft_id FROM messages m JOIN folders f ON f.id = m.folder_id \
         WHERE m.thread_id = ? AND f.role = 'drafts' ORDER BY m.date DESC, m.id DESC LIMIT 1",
    )
    .bind(thread_id)
    .fetch_optional(pool)
    .await?;
    let (message_id, account_id, raw_hash, draft_id) =
        row.ok_or_else(|| AppError::NotFound("aucun brouillon dans cette conversation".into()))?;
    let hash = rawstore::usable(raw_hash.as_deref()).ok_or_else(|| {
        AppError::NotFound(
            "le brouillon n'est pas encore téléchargé : réessayez dans un instant".into(),
        )
    })?;
    let raw = rawstore::get(raw_dir, hash).await?;
    let id = match draft_id {
        Some(id) => id,
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            sqlx::query("UPDATE messages SET draft_id = ? WHERE id = ?")
                .bind(&id)
                .bind(message_id)
                .execute(pool)
                .await?;
            id
        }
    };
    parse_draft(&raw, id, account_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::ingest::{parse, store_with, Flags, MessageLoc, StoreOpts};

    fn account() -> Account {
        Account {
            id: "a".into(),
            email: "me@x.fr".into(),
            display_name: Some("Moi".into()),
            provider: "imap".into(),
            auth_kind: "password".into(),
            username: "me".into(),
            imap_host: "i".into(),
            imap_port: 993,
            imap_security: "tls".into(),
            smtp_host: "s".into(),
            smtp_port: 465,
            smtp_security: "tls".into(),
            last_sync_at: None,
            sync_status: "idle".into(),
            sync_error: None,
            signature: String::new(),
            identities: vec![],
        }
    }

    fn outgoing(draft_id: &str) -> OutgoingMessage {
        OutgoingMessage {
            to: vec!["Bob Durand <bob@y.fr>".into()],
            cc: vec!["carol@y.fr".into()],
            bcc: vec!["secret@y.fr".into()],
            subject: "Devis été".into(),
            body_text: "Bonjour Bob,\n\nVoici le devis.".into(),
            body_html: Some("<p>Bonjour <b>Bob</b>,</p><p>Voici le devis.</p>".into()),
            in_reply_to: Some("parent@y.fr".into()),
            references: vec!["racine@y.fr".into(), "parent@y.fr".into()],
            attachments: vec![smtp::OutgoingAttachment {
                filename: "devis.pdf".into(),
                mime_type: "application/pdf".into(),
                data_base64: STANDARD.encode(b"%PDF-1.4 contenu"),
            }],
            draft_id: Some(draft_id.into()),
            from_email: None,
        }
    }

    /// Le brouillon survit à l'aller-retour : message MIME puis relecture, avec Bcc, pièce jointe et fil.
    #[test]
    fn aller_retour_complet() {
        let raw = smtp::build_draft(&account(), &outgoing("d-1"))
            .unwrap()
            .formatted();
        let text = String::from_utf8_lossy(&raw);
        assert!(
            text.contains("X-Envelope-Draft-Id: d-1"),
            "identifiant inscrit dans l'en-tête"
        );
        assert!(text.contains("Bcc:"), "Bcc conservé dans un brouillon");
        let d = parse_draft(&raw, "d-1".into(), "a".into()).unwrap();
        assert_eq!(d.to, ["Bob Durand <bob@y.fr>"]);
        assert_eq!(
            (d.cc.as_slice(), d.bcc.as_slice()),
            (
                &["carol@y.fr".to_string()][..],
                &["secret@y.fr".to_string()][..]
            )
        );
        assert_eq!(d.subject, "Devis été");
        assert!(d.body_html.as_deref().unwrap().contains("<b>Bob</b>"));
        assert!(d.body_text.contains("Voici le devis."));
        assert_eq!(d.in_reply_to.as_deref(), Some("parent@y.fr"));
        assert_eq!(d.references, ["racine@y.fr", "parent@y.fr"]);
        assert_eq!(d.attachments.len(), 1);
        assert_eq!(d.attachments[0].filename, "devis.pdf");
        assert_eq!(
            STANDARD.decode(&d.attachments[0].data_base64).unwrap(),
            b"%PDF-1.4 contenu"
        );
    }

    /// Un brouillon sans destinataire (cas courant) et sans objet se construit ; un message à envoyer, non.
    #[test]
    fn brouillon_sans_destinataire() {
        let mut out = outgoing("d-2");
        (out.to, out.cc, out.bcc, out.subject, out.attachments) =
            (vec![], vec![], vec![], String::new(), vec![]);
        let raw = smtp::build_draft(&account(), &out).unwrap().formatted();
        let d = parse_draft(&raw, "d-2".into(), "a".into()).unwrap();
        assert!(d.to.is_empty() && d.subject.is_empty() && d.attachments.is_empty());
        assert!(
            smtp::build_message(&account(), &out).is_err(),
            "à l'envoi, un destinataire reste obligatoire"
        );
        // Sans identifiant, ce n'est pas un brouillon valide.
        out.draft_id = None;
        assert!(smtp::build_draft(&account(), &out).is_err());
        // Et un message envoyé ne porte jamais l'en-tête d'identité.
        let sent = smtp::build_message(&account(), &outgoing("d-3"))
            .unwrap()
            .formatted();
        assert!(!String::from_utf8_lossy(&sent).contains("X-Envelope-Draft-Id"));
    }

    /// Les versions à supprimer : celle mémorisée (même dossier, même validité) et celles de la synchro.
    #[test]
    fn versions_a_supprimer() {
        let known = [Version {
            id: 1,
            uid: 10,
            path: "Drafts".into(),
            folder_id: 1,
            thread_id: None,
        }];
        let last = Recorded {
            path: "Drafts".into(),
            uid_validity: Some(7),
            uid: 12,
        };
        let t = targets(&known, Some(&last), "Drafts", Some(7));
        assert_eq!(t["Drafts"].iter().copied().collect::<Vec<_>>(), [10, 12]);
        // Dossier recréé (validité différente) ou autre dossier : l'UID mémorisé ne désigne plus rien.
        assert_eq!(
            targets(&known, Some(&last), "Drafts", Some(8))["Drafts"].len(),
            1
        );
        assert_eq!(
            targets(&known, Some(&last), "Brouillons", Some(7)).get("Brouillons"),
            None
        );
        assert!(targets(&[], None, "Drafts", None).is_empty());
    }

    #[tokio::test]
    async fn derniere_version_memorisee_par_compte_et_brouillon() {
        let pool = crate::db::init_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','me@x.fr','a','i','s')").execute(&pool).await.unwrap();
        assert!(recorded(&pool, "a", "d").await.unwrap().is_none());
        record(&pool, "a", "d", "Drafts", Some(7), 5).await.unwrap();
        record(&pool, "a", "d", "Drafts", Some(7), 9).await.unwrap(); // une version plus récente remplace la précédente
        let r = recorded(&pool, "a", "d").await.unwrap().unwrap();
        assert_eq!((r.uid, r.uid_validity), (9, Some(7)));
        assert!(recorded(&pool, "a", "autre").await.unwrap().is_none());
        // Supprimer le compte efface ses mémorisations.
        sqlx::query("DELETE FROM accounts WHERE id = 'a'")
            .execute(&pool)
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM draft_versions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    /// La synchro retient l'identifiant ; on retrouve les versions et on rouvre la plus récente.
    #[tokio::test]
    async fn identifiant_retenu_et_brouillon_rouvert() {
        let pool = crate::db::init_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','me@x.fr','a','i','s')").execute(&pool).await.unwrap();
        let folder = sqlx::query("INSERT INTO folders (account_id,path,name,role) VALUES ('a','Drafts','Drafts','drafts')")
            .execute(&pool).await.unwrap().last_insert_rowid();
        let base = std::env::temp_dir().join(format!("envelope-drafts-{}", uuid::Uuid::new_v4()));
        let mut last_thread = 0;
        for (uid, subject) in [(1, "v1"), (2, "v2")] {
            let mut out = outgoing("d-9");
            out.subject = subject.into();
            let raw = smtp::build_draft(&account(), &out).unwrap().formatted();
            let hash = rawstore::put(&base.join("raw"), &raw).await.unwrap();
            let loc = MessageLoc {
                account_id: "a",
                folder_id: folder,
                uid,
                size: None,
            };
            let opts = StoreOpts {
                raw_hash: Some(&hash),
                lazy_attachments: true,
            };
            last_thread = store_with(
                &pool,
                &base.join("att"),
                loc,
                Flags {
                    seen: true,
                    flagged: false,
                },
                parse(&raw, None).unwrap(),
                opts,
            )
            .await
            .unwrap();
        }
        let found = copies(&pool, "a", "d-9").await.unwrap();
        assert_eq!(
            found.iter().map(|c| c.uid).collect::<Vec<_>>(),
            [1, 2],
            "deux versions reliées par l'identifiant"
        );
        assert!(copies(&pool, "a", "inconnu").await.unwrap().is_empty());
        assert!(
            copies(&pool, "autre", "d-9").await.unwrap().is_empty(),
            "jamais d'un autre compte"
        );
        let d = open(&pool, &base.join("raw"), last_thread).await.unwrap();
        assert_eq!(
            (d.id.as_str(), d.subject.as_str()),
            ("d-9", "v2"),
            "la version la plus récente"
        );
        // Brouillon d'un autre client (sans identifiant) : on lui en donne un, mémorisé en base.
        sqlx::query("UPDATE messages SET draft_id = NULL")
            .execute(&pool)
            .await
            .unwrap();
        let adopted = open(&pool, &base.join("raw"), last_thread).await.unwrap();
        assert!(!adopted.id.is_empty() && adopted.id != "d-9");
        assert_eq!(copies(&pool, "a", &adopted.id).await.unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(base);
    }
}

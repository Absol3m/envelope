//! Copie des messages envoyés dans « Envoyés » : file persistante (`pending_append`) vidée à chaque synchro.
//!
//! Thunderbird tente l'`APPEND` une fois, puis demande quoi faire (réessayer, dossier local) tant que la fenêtre de
//! composition est ouverte : rien n'est repris plus tard. Ici le message brut est rangé dans `rawstore` dès l'envoi et la
//! copie est reprise tant qu'il le faut :
//! - panne réseau : on réessaie sans limite (à chaque synchro, avec un délai croissant) ;
//! - refus du serveur (`NO` : quota, droits…) : après quelques refus, ou au bout de [`MAX_AGE_SECS`], le message est
//!   rangé dans un dossier local « Sent-<adresse> » et l'utilisateur est prévenu : jamais perdu ;
//! - déjà déposé (réponse perdue) : le Message-ID est cherché dans « Envoyés » avant l'`APPEND`, pas de doublon.
//!
//! Affichage immédiat : dès la mise en file, le message apparaît dans « Envoyés » sous forme d'une ligne provisoire
//! (UID négatif `-id`, jamais envoyé au serveur : synchro et actions l'ignorent). Elle est retirée à la synchro du
//! dossier dès que sa ligne de file a disparu — la vraie copie, rapatriée par la même passe, la remplace (même
//! Message-ID), ou le message est dans le dossier local de repli.
//!
//! Limite : un arrêt entre la fin du SMTP et l'écriture de la ligne perd la copie (quelques microsecondes) ; renvoyer
//! le message serait pire.

use std::path::Path;

use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

use crate::error::{AppError, Result};
use crate::mail::account::{Account, LOCAL_ID};
use crate::mail::{imap, ingest, local, rawstore};

/// Refus du serveur (`NO`) avant de ranger le message en local.
const MAX_REFUSALS: i64 = 3;
/// Au-delà (sept jours), on range en local même si le serveur reste injoignable.
const MAX_AGE_SECS: i64 = 7 * 24 * 3600;
const BASE_DELAY_SECS: i64 = 30;
const MAX_DELAY_SECS: i64 = 3600;

#[derive(Debug, sqlx::FromRow)]
pub struct Pending {
    pub id: i64,
    pub raw_hash: String,
    pub message_id: Option<String>,
    pub attempts: i64,
    pub refusals: i64,
    pub created_at: i64,
}

/// Ce qui suit un échec.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    /// Réessayer après ce délai (secondes) ; `refused` : le serveur a dit non.
    Retry { delay: i64, refused: bool },
    /// Ranger dans un dossier local.
    Local,
}

/// Un refus du serveur (`NO`) est durable ; tout le reste (réseau, TLS, coupure) est passager.
fn is_refusal(e: &AppError) -> bool {
    matches!(e, AppError::Imap(async_imap::error::Error::No(_)))
}

/// Décision après un échec : `attempts` et `refusals` = échecs et refus déjà comptés, `age` = secondes depuis l'envoi.
pub fn after_failure(refusal: bool, attempts: i64, refusals: i64, age: i64) -> Next {
    if refusals + i64::from(refusal) >= MAX_REFUSALS || age >= MAX_AGE_SECS {
        return Next::Local;
    }
    let delay = (BASE_DELAY_SECS << attempts.clamp(0, 7)).min(MAX_DELAY_SECS);
    Next::Retry {
        delay,
        refused: refusal,
    }
}

/// Critère `UID SEARCH` d'un Message-ID, seulement s'il ne contient que des caractères sûrs (jamais d'injection IMAP).
pub fn search_criteria(message_id: &str) -> Option<String> {
    let ok = !message_id.is_empty()
        && message_id.len() <= 200
        && message_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-/=?^_`{|}~.@".contains(&b));
    ok.then(|| format!("HEADER Message-ID \"<{message_id}>\""))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Copie mise en file : son identifiant et l'empreinte du message brut rangé.
pub struct Queued {
    pub id: i64,
    pub raw_hash: String,
}

/// Range le message envoyé et le met en file ; l'envoi est déjà fait, on ne refait rien d'autre.
pub async fn enqueue(
    pool: &SqlitePool,
    raw_dir: &Path,
    account_id: &str,
    raw: &[u8],
) -> Result<Queued> {
    let hash = rawstore::put(raw_dir, raw).await?;
    let message_id = mail_parser::MessageParser::default()
        .parse_headers(raw)
        .and_then(|m| m.message_id().map(str::to_string));
    let at = now();
    let id = sqlx::query_scalar(
        "INSERT INTO pending_append (account_id, raw_hash, message_id, next_try_at, created_at) \
         VALUES (?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(account_id)
    .bind(&hash)
    .bind(message_id)
    .bind(at)
    .bind(at)
    .fetch_one(pool)
    .await?;
    Ok(Queued { id, raw_hash: hash })
}

/// Montre tout de suite le message dans « Envoyés » (ligne provisoire d'UID `-id`). Sans effet tant que le dossier
/// n'est pas connu (première synchro pas encore faite) : la copie y apparaîtra quand le serveur l'aura.
pub async fn show_now(
    pool: &SqlitePool,
    attachments_dir: &Path,
    account_id: &str,
    queued: &Queued,
    raw: &[u8],
) -> Result<()> {
    let Some(folder_id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM folders WHERE account_id = ? AND role = 'sent' LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(());
    };
    let Some(parsed) = ingest::parse_with(raw, Some(now()), false) else {
        return Ok(()); // illisible : la copie du serveur, elle, ne dépend pas de notre lecture
    };
    let thread = ingest::store_with(
        pool,
        attachments_dir,
        ingest::MessageLoc {
            account_id,
            folder_id,
            uid: -queued.id,
            size: Some(raw.len() as i64),
        },
        ingest::Flags {
            seen: true,
            flagged: false,
        },
        parsed,
        ingest::StoreOpts {
            raw_hash: Some(&queued.raw_hash),
            lazy_attachments: true,
        },
    )
    .await?;
    ingest::refresh_threads(pool, &std::collections::HashSet::from([thread])).await?;
    ingest::refresh_folder_unread(pool, folder_id).await
}

/// Retire de `folder_id` les lignes provisoires dont la ligne de file a disparu (copie déposée, rangée en local ou
/// abandonnée) ; rend leurs conversations, à rafraîchir. Appelée à la fin de la synchro du dossier, après le
/// rapatriement des nouveautés : la vraie copie est déjà là, le message ne clignote pas.
pub async fn sweep(pool: &SqlitePool, folder_id: i64) -> Result<Vec<i64>> {
    let threads: Vec<Option<i64>> = sqlx::query_scalar(concat!(
        "DELETE FROM messages WHERE folder_id = ? AND ",
        provisional_row!(),
        " AND -uid NOT IN (SELECT id FROM pending_append) RETURNING thread_id"
    ))
    .bind(folder_id)
    .fetch_all(pool)
    .await?;
    Ok(threads.into_iter().flatten().collect())
}

async fn due(pool: &SqlitePool, account_id: &str, at: i64) -> Result<Vec<Pending>> {
    Ok(sqlx::query_as(
        "SELECT id, raw_hash, message_id, attempts, refusals, created_at FROM pending_append \
         WHERE account_id = ? AND next_try_at <= ? ORDER BY id",
    )
    .bind(account_id)
    .bind(at)
    .fetch_all(pool)
    .await?)
}

async fn done(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM pending_append WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn reschedule(
    pool: &SqlitePool,
    p: &Pending,
    delay: i64,
    refused: bool,
    error: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE pending_append SET attempts = attempts + 1, refusals = refusals + ?, next_try_at = ?, last_error = ? WHERE id = ?",
    )
    .bind(i64::from(refused))
    .bind(now() + delay)
    .bind(error)
    .bind(p.id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Dépose une copie sur le serveur, sauf si elle y est déjà (Message-ID trouvé dans « Envoyés »). `sent` : chemin du
/// dossier, résolu une seule fois pour toute la passe.
async fn append(
    pool: &SqlitePool,
    raw_dir: &Path,
    account_id: &str,
    session: &mut imap::ImapSession,
    sent: &mut Option<String>,
    p: &Pending,
) -> Result<()> {
    let path = match sent {
        Some(path) => path,
        None => {
            let known: Option<String> = sqlx::query_scalar(
                "SELECT path FROM folders WHERE account_id = ? AND role = 'sent'",
            )
            .bind(account_id)
            .fetch_optional(pool)
            .await?;
            let path = match known {
                Some(path) => path,
                None => imap::find_or_create(session, "sent", "Sent").await?,
            };
            sent.insert(path)
        }
    };
    if let Some(criteria) = p.message_id.as_deref().and_then(search_criteria) {
        // Un dossier qu'on ne peut pas ouvrir ne bloque pas : l'APPEND dira ce qu'il en est.
        if session.select(&*path).await.is_ok()
            && session
                .uid_search(criteria)
                .await
                .is_ok_and(|f| !f.is_empty())
        {
            return Ok(());
        }
    }
    let raw = rawstore::get(raw_dir, &p.raw_hash).await?;
    match session.append(&*path, Some("(\\Seen)"), None, &raw).await {
        // Vu sur un vrai serveur (Dovecot, quota dépassé) : un refus avant l'invite `+` — réponse `NO` étiquetée — sort
        // de la bibliothèque en `Append`, sans le `NO`, comme une connexion coupée. Si la connexion répond encore,
        // c'était un refus (durable) ; sinon une coupure (passagère).
        Err(async_imap::error::Error::Append) if session.noop().await.is_ok() => Err(
            async_imap::error::Error::No("the server refused the message (APPEND)".into()).into(),
        ),
        other => Ok(other?),
    }
}

/// Range la copie dans le dossier local « Sent-<adresse> » (lisible hors ligne, jamais perdue).
async fn save_locally(
    pool: &SqlitePool,
    attachments_dir: &Path,
    raw_dir: &Path,
    acct: &Account,
    p: &Pending,
) -> Result<()> {
    let folder =
        local::get_or_create_folder(pool, &local::sanitize_name(&format!("Sent-{}", acct.email)))
            .await?;
    if ingest::has_raw(pool, folder, &p.raw_hash).await? {
        return Ok(());
    }
    let raw = rawstore::get(raw_dir, &p.raw_hash).await?;
    let parsed = ingest::parse_with(&raw, Some(p.created_at), false)
        .ok_or_else(|| AppError::Invalid(msg!("sentcopy.unreadable", "unreadable sent message")))?;
    ingest::store_with(
        pool,
        attachments_dir,
        ingest::MessageLoc {
            account_id: LOCAL_ID,
            folder_id: folder,
            uid: ingest::next_uid(pool, folder).await?,
            size: Some(raw.len() as i64),
        },
        ingest::Flags {
            seen: true,
            flagged: false,
        },
        parsed,
        ingest::StoreOpts {
            raw_hash: Some(&p.raw_hash),
            lazy_attachments: true,
        },
    )
    .await?;
    Ok(())
}

/// Après un échec : réessayer plus tard, ou ranger en local (et prévenir) ; sans issue, la ligne reste en file.
async fn on_failure(
    app: &AppHandle,
    pool: &SqlitePool,
    dirs: (&Path, &Path),
    acct: &Account,
    p: &Pending,
    e: &AppError,
) {
    let (attachments_dir, raw_dir) = dirs;
    let wire = e.to_wire();
    let next = after_failure(is_refusal(e), p.attempts, p.refusals, now() - p.created_at);
    let (delay, refused) = match next {
        Next::Retry { delay, refused } => (delay, refused),
        Next::Local => match save_locally(pool, attachments_dir, raw_dir, acct, p).await {
            Ok(()) => {
                let _ = done(pool, p.id).await;
                let _ = app.emit("sent-copy:local", &wire);
                return;
            }
            // Même le dossier local échoue : on garde la ligne et on réessaie plus tard.
            Err(local_err) => {
                tracing::error!("local copy failed: {local_err}");
                (MAX_DELAY_SECS, false)
            }
        },
    };
    let _ = reschedule(pool, p, delay, refused, &wire).await;
}

/// Traite les copies en attente d'un compte (appelé en début de synchro, avec sa session). Ne fait jamais échouer la
/// synchro : une copie en échec reste en file, ou est rangée en local.
pub async fn drain(
    app: &AppHandle,
    pool: &SqlitePool,
    dirs: (&Path, &Path), // (pièces jointes, messages bruts)
    acct: &Account,
    session: &mut imap::ImapSession,
) {
    if !acct.keeps_sent_copy() {
        return;
    }
    let pending = match due(pool, &acct.id, now()).await {
        Ok(p) => p,
        Err(e) => return tracing::warn!(account = %acct.id, "pending Sent copies: {e}"),
    };
    let mut sent = None;
    for p in pending {
        match append(pool, dirs.1, &acct.id, session, &mut sent, &p).await {
            Ok(()) => {
                let _ = done(pool, p.id).await;
            }
            Err(e) => {
                tracing::warn!(account = %acct.id, "could not copy to Sent: {e}");
                on_failure(app, pool, dirs, acct, &p, &e).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reseau_coupe_on_reessaie_avec_un_delai_croissant_et_plafonne() {
        let d = |a| match after_failure(false, a, 0, 0) {
            Next::Retry { delay, refused } => {
                assert!(!refused);
                delay
            }
            Next::Local => panic!("une panne réseau ne range pas en local"),
        };
        assert_eq!(d(0), 30);
        assert_eq!(d(1), 60);
        assert_eq!(d(5), 960);
        assert_eq!(d(50), MAX_DELAY_SECS);
    }

    #[test]
    fn trois_refus_du_serveur_rangent_en_local() {
        assert!(matches!(
            after_failure(true, 0, 0, 0),
            Next::Retry { refused: true, .. }
        ));
        assert!(matches!(after_failure(true, 1, 1, 0), Next::Retry { .. }));
        assert_eq!(after_failure(true, 2, 2, 0), Next::Local);
        // Les pannes réseau ne comptent pas comme des refus, même nombreuses.
        assert!(matches!(after_failure(false, 20, 0, 0), Next::Retry { .. }));
    }

    #[test]
    fn serveur_injoignable_trop_longtemps_range_en_local() {
        assert!(matches!(
            after_failure(false, 0, 0, MAX_AGE_SECS - 1),
            Next::Retry { .. }
        ));
        assert_eq!(after_failure(false, 0, 0, MAX_AGE_SECS), Next::Local);
    }

    #[test]
    fn message_id_jamais_injecte_dans_la_recherche() {
        assert_eq!(
            search_criteria("abc.123@example.com").as_deref(),
            Some("HEADER Message-ID \"<abc.123@example.com>\"")
        );
        for bad in ["", "a\"b", "a b", "a\r\nb", "a>b", "a\\b", "é@x"] {
            assert_eq!(search_criteria(bad), None, "{bad:?}");
        }
        assert_eq!(search_criteria(&"a".repeat(201)), None);
    }

    #[tokio::test]
    async fn la_file_garde_le_brut_et_l_identifiant() {
        let db = crate::db::init_memory().await.unwrap();
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&db)
            .await
            .unwrap();
        let dir = std::env::temp_dir().join(format!("envelope-sentcopy-{}", uuid::Uuid::new_v4()));
        let raw = b"Message-ID: <x1@example.com>\r\nSubject: s\r\n\r\ncorps";
        let id = enqueue(&db, &dir, "acc", raw).await.unwrap().id;
        let rows = due(&db, "acc", now()).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].message_id.as_deref(), Some("x1@example.com"));
        assert_eq!(rawstore::get(&dir, &rows[0].raw_hash).await.unwrap(), raw);

        // Reportée : plus due ; autre compte : jamais.
        reschedule(&db, &rows[0], 60, true, "e").await.unwrap();
        assert!(due(&db, "acc", now()).await.unwrap().is_empty());
        let r = &due(&db, "acc", now() + 61).await.unwrap()[0];
        assert_eq!((r.attempts, r.refusals), (1, 1));
        assert!(due(&db, "autre", now() + 61).await.unwrap().is_empty());

        done(&db, id).await.unwrap();
        assert!(due(&db, "acc", now() + 999).await.unwrap().is_empty());
        assert!(
            rawstore::exists(&dir, &rows[0].raw_hash).await,
            "le brut n'est jamais supprimé"
        );
    }

    #[tokio::test]
    async fn affichage_immediat_puis_remplace_par_la_copie_du_serveur() {
        let db = crate::db::init_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','me@x.fr','a','i','s')")
            .execute(&db).await.unwrap();
        let dir = std::env::temp_dir().join(format!("envelope-sentcopy-{}", uuid::Uuid::new_v4()));
        let raw = b"Message-ID: <m1@x.fr>\r\nFrom: me@x.fr\r\nTo: bob@x.fr\r\nSubject: Devis\r\n\r\ncorps";
        let queued = enqueue(&db, &dir, "a", raw).await.unwrap();
        // Dossier « Envoyés » pas encore connu (première synchro à faire) : rien à montrer, sans erreur.
        show_now(&db, &dir, "a", &queued, raw).await.unwrap();
        let shown = || async {
            sqlx::query_as::<_, (i64, String, bool, Option<String>)>(
                "SELECT uid, subject, is_read, raw_hash FROM messages",
            )
            .fetch_all(&db)
            .await
            .unwrap()
        };
        assert!(shown().await.is_empty());

        sqlx::query("INSERT INTO folders (id, account_id, path, name, role) VALUES (7,'a','Sent','Sent','sent')")
            .execute(&db).await.unwrap();
        show_now(&db, &dir, "a", &queued, raw).await.unwrap();
        let rows = shown().await;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].0, rows[0].1.as_str(), rows[0].2),
            (-queued.id, "Devis", true)
        );
        assert_eq!(
            rows[0].3.as_deref(),
            Some(queued.raw_hash.as_str()),
            "brut rangé : pièces jointes lisibles"
        );
        let (count, folder_count): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT message_count FROM threads LIMIT 1), (SELECT COUNT(*) FROM threads)",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!((count, folder_count), (1, 1));

        // Tant que la copie est en file, la ligne reste ; la copie déposée (ligne de file retirée), elle part.
        assert!(sweep(&db, 7).await.unwrap().is_empty());
        done(&db, queued.id).await.unwrap();
        assert_eq!(sweep(&db, 7).await.unwrap().len(), 1);
        assert!(shown().await.is_empty());
        // Un message du serveur (UID positif) n'est jamais balayé.
        sqlx::query("INSERT INTO messages (account_id, folder_id, uid, subject, from_addr, date) VALUES ('a',7,12,'réel','x@y.fr','2026-01-01T00:00:00Z')")
            .execute(&db).await.unwrap();
        assert!(sweep(&db, 7).await.unwrap().is_empty());
        assert_eq!(shown().await.len(), 1);
    }

    /// Dialogue IMAP réel (Dovecot, quota de 40 Ko par boîte) : dépôt avec `\\Seen`, copie déjà déposée non doublée
    /// (réponse perdue), refus `NO` (quota) reconnu comme durable, création d'un dossier absent.
    /// Lancer : `scripts/imap-test-server.sh start` puis
    /// `ENVELOPE_TEST_IMAP=127.0.0.1:3143 cargo test -p envelope dialogue_imap_reel -- --ignored`.
    #[tokio::test]
    #[ignore = "demande un serveur IMAP : ENVELOPE_TEST_IMAP=hôte:port (scripts/imap-test-server.sh)"]
    async fn dialogue_imap_reel() {
        use futures::TryStreamExt;
        let addr = std::env::var("ENVELOPE_TEST_IMAP").expect("ENVELOPE_TEST_IMAP");
        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        let io: Box<dyn imap::Io> = Box::new(tcp);
        let user = format!("u{}", uuid::Uuid::new_v4().simple()); // boîte neuve à chaque essai
        let mut session = async_imap::Client::new(io)
            .login(&user, "pass")
            .await
            .map_err(|(e, _)| e)
            .unwrap();

        let db = crate::db::init_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','me@x.fr','a','i','s')")
            .execute(&db).await.unwrap();
        let dir = std::env::temp_dir().join(format!("envelope-sentcopy-{}", uuid::Uuid::new_v4()));
        let mut sent = None;
        let next = |body: String| {
            let db = db.clone();
            let dir = dir.clone();
            async move {
                let raw = format!("Message-ID: <{}@x.fr>\r\nFrom: me@x.fr\r\nTo: bob@x.fr\r\nSubject: s\r\n\r\n{body}", uuid::Uuid::new_v4().simple());
                let q = enqueue(&db, &dir, "a", raw.as_bytes()).await.unwrap();
                let p = due(&db, "a", now() + 1)
                    .await
                    .unwrap()
                    .into_iter()
                    .find(|p| p.id == q.id)
                    .unwrap();
                p
            }
        };

        // 1. Dépôt : le dossier « Sent » du serveur est trouvé, le message y est, lu.
        let first = next("bonjour".into()).await;
        append(&db, &dir, "a", &mut session, &mut sent, &first)
            .await
            .unwrap();
        assert_eq!(sent.as_deref(), Some("Sent"));
        session.select("Sent").await.unwrap();
        let fetched: Vec<_> = session
            .fetch("1:*", "(FLAGS)")
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(fetched.len(), 1);
        assert!(fetched[0]
            .flags()
            .any(|f| matches!(f, async_imap::types::Flag::Seen)));

        // 2. Réponse perdue : la même copie reprise ne crée pas de doublon.
        append(&db, &dir, "a", &mut session, &mut sent, &first)
            .await
            .unwrap();
        session.select("Sent").await.unwrap();
        assert_eq!(
            session.uid_search("ALL").await.unwrap().len(),
            1,
            "pas de doublon"
        );

        // 3. Quota dépassé : refus `NO`, durable (→ dossier local après quelques essais), rien déposé.
        let big = next("x".repeat(60_000)).await;
        let err = append(&db, &dir, "a", &mut session, &mut sent, &big)
            .await
            .unwrap_err();
        assert!(is_refusal(&err), "{err:?}");
        session.select("Sent").await.unwrap();
        assert_eq!(session.uid_search("ALL").await.unwrap().len(), 1);

        // 4. Dossier absent (boîte neuve) : créé et abonné sous le nom par défaut.
        let path = imap::find_or_create(&mut session, "trash", "Trash")
            .await
            .unwrap();
        assert_eq!(path, "Trash");
    }

    #[tokio::test]
    async fn desactiver_la_copie_abandonne_la_file_du_compte() {
        let db = crate::db::init_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','a@x.fr','a','i','s'),('b','b@x.fr','b','i','s')")
            .execute(&db)
            .await
            .unwrap();
        let dir = std::env::temp_dir().join(format!("envelope-sentcopy-{}", uuid::Uuid::new_v4()));
        enqueue(&db, &dir, "a", b"Subject: 1\r\n\r\nx")
            .await
            .unwrap();
        enqueue(&db, &dir, "b", b"Subject: 2\r\n\r\ny")
            .await
            .unwrap();
        crate::mail::account::set_copy_sent(&db, "a", false)
            .await
            .unwrap();
        assert!(due(&db, "a", now() + 1).await.unwrap().is_empty());
        assert_eq!(due(&db, "b", now() + 1).await.unwrap().len(), 1);
        let on: bool = sqlx::query_scalar("SELECT copy_sent FROM accounts WHERE id = 'a'")
            .fetch_one(&db)
            .await
            .unwrap();
        assert!(!on);
        assert!(crate::mail::account::set_copy_sent(&db, "inconnu", true)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn la_liste_des_comptes_compte_les_copies_en_attente() {
        let db = crate::db::init_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','a@x.fr','a','i','s'),('b','b@x.fr','b','i','s')")
            .execute(&db)
            .await
            .unwrap();
        let dir = std::env::temp_dir().join(format!("envelope-sentcopy-{}", uuid::Uuid::new_v4()));
        let first = enqueue(&db, &dir, "a", b"Subject: 1\r\n\r\nx")
            .await
            .unwrap()
            .id;
        enqueue(&db, &dir, "a", b"Subject: 2\r\n\r\ny")
            .await
            .unwrap();
        let p = &due(&db, "a", now()).await.unwrap()[0];
        assert_eq!(p.id, first);
        reschedule(&db, p, 60, false, "boom").await.unwrap();
        let accounts = crate::mail::account::list(&db).await.unwrap();
        let get = |id: &str| accounts.iter().find(|a| a.id == id).unwrap();
        assert_eq!(get("a").pending_copies, 2);
        assert_eq!(get("a").pending_copy_error.as_deref(), Some("boom"));
        assert_eq!(get("b").pending_copies, 0);
        assert_eq!(get("b").pending_copy_error, None);
    }
}

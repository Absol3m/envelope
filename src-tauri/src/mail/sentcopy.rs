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

/// Range le message envoyé et le met en file ; l'envoi est déjà fait, on ne refait rien d'autre.
pub async fn enqueue(
    pool: &SqlitePool,
    raw_dir: &Path,
    account_id: &str,
    raw: &[u8],
) -> Result<i64> {
    let hash = rawstore::put(raw_dir, raw).await?;
    let message_id = mail_parser::MessageParser::default()
        .parse(raw)
        .and_then(|m| m.message_id().map(str::to_string));
    let at = now();
    Ok(sqlx::query_scalar(
        "INSERT INTO pending_append (account_id, raw_hash, message_id, next_try_at, created_at) \
         VALUES (?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(account_id)
    .bind(hash)
    .bind(message_id)
    .bind(at)
    .bind(at)
    .fetch_one(pool)
    .await?)
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

/// Dépose une copie sur le serveur, sauf si elle y est déjà (Message-ID trouvé dans « Envoyés »).
async fn append(
    pool: &SqlitePool,
    raw_dir: &Path,
    account_id: &str,
    session: &mut imap::ImapSession,
    p: &Pending,
) -> Result<()> {
    let known: Option<String> =
        sqlx::query_scalar("SELECT path FROM folders WHERE account_id = ? AND role = 'sent'")
            .bind(account_id)
            .fetch_optional(pool)
            .await?;
    let path = match known {
        Some(p) => p,
        None => imap::find_or_create(session, "sent", "Sent").await?,
    };
    if let Some(criteria) = p.message_id.as_deref().and_then(search_criteria) {
        // Un dossier qu'on ne peut pas ouvrir ne bloque pas : l'APPEND dira ce qu'il en est.
        if session.select(&path).await.is_ok()
            && session
                .uid_search(criteria)
                .await
                .is_ok_and(|f| !f.is_empty())
        {
            return Ok(());
        }
    }
    let raw = rawstore::get(raw_dir, &p.raw_hash).await?;
    session.append(&path, Some("(\\Seen)"), None, &raw).await?;
    Ok(())
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

/// Traite les copies en attente d'un compte (appelé en début de synchro, avec sa session). Ne fait jamais échouer la
/// synchro : une copie en échec reste en file, ou est rangée en local.
pub async fn drain(
    app: &AppHandle,
    pool: &SqlitePool,
    attachments_dir: &Path,
    raw_dir: &Path,
    acct: &Account,
    session: &mut imap::ImapSession,
) {
    if acct.server_files_sent() {
        return;
    }
    let pending = match due(pool, &acct.id, now()).await {
        Ok(p) => p,
        Err(e) => return tracing::warn!(account = %acct.id, "copies à déposer : {e}"),
    };
    for p in pending {
        match append(pool, raw_dir, &acct.id, session, &p).await {
            Ok(()) => {
                let _ = done(pool, p.id).await;
            }
            Err(e) => {
                tracing::warn!(account = %acct.id, "copie dans Envoyés impossible : {e}");
                let age = now() - p.created_at;
                match after_failure(is_refusal(&e), p.attempts, p.refusals, age) {
                    Next::Retry { delay, refused } => {
                        let _ = reschedule(pool, &p, delay, refused, &e.to_wire()).await;
                    }
                    Next::Local => {
                        match save_locally(pool, attachments_dir, raw_dir, acct, &p).await {
                            Ok(()) => {
                                let _ = done(pool, p.id).await;
                                let _ = app.emit("sent-copy:local", e.to_wire());
                            }
                            // Même le dossier local échoue : on garde la ligne et on réessaie plus tard.
                            Err(local_err) => {
                                tracing::error!("copie locale impossible : {local_err}");
                                let _ =
                                    reschedule(pool, &p, MAX_DELAY_SECS, false, &e.to_wire()).await;
                            }
                        }
                    }
                }
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
        let id = enqueue(&db, &dir, "acc", raw).await.unwrap();
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
}

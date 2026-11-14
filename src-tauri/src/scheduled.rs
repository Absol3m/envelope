//! Envois programmés : la file (SQLite) et la boucle qui envoie les messages à leur heure.
//!
//! IMAP/SMTP n'offrent pas d'envoi différé côté serveur : c'est l'application qui envoie, donc elle doit être
//! ouverte à l'heure prévue. Un message en retard de plus d'une heure (application fermée) n'est **pas** envoyé
//! de lui-même : un rappel « lundi 8 h » parti le mardi serait pire qu'un message en attente de décision.
//!
//! La même file sert de **boîte d'envoi** : un message dont l'envoi échoue faute de connexion (`AppError::Unreachable`)
//! y reste (`next_try_at` renseigné) et repart seul, avec un délai croissant, dès que le serveur répond. Il n'est jamais « en
//! retard » : il n'a pas d'heure à respecter, seulement un réseau à retrouver.

use std::time::Duration;

use futures::StreamExt;
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};

use crate::clock::now;
use crate::error::{AppError, Result};
use crate::mail::smtp::OutgoingMessage;
use crate::AppState;

/// Attente maximale entre deux vérifications (voir `wait_until`).
const TICK_MAX: Duration = Duration::from_secs(300);
/// Retard toléré au-delà de l'heure prévue (veille de l'ordinateur, application relancée juste après).
const GRACE_SECS: i64 = 3600;
const MAX_ROWS: i64 = 200;
/// Taille maximale d'un message en file (JSON) : 25 Mo de pièces jointes, en base64, plus le texte.
const MAX_BYTES: usize = 40 * 1024 * 1024;
/// Boîte d'envoi : délai entre deux essais, de 30 s doublé à chaque échec jusqu'à 10 min.
const RETRY_BASE_SECS: i64 = 30;
const RETRY_MAX_SECS: i64 = 600;

/// État d'une ligne de la file (`scheduled.status`) ; mêmes valeurs en base et vers l'interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
pub enum Status {
    Pending,
    Sending,
    Failed,
}

/// Réveille la boucle d'envoi : un message vient d'entrer, son heure est peut-être plus proche que la prochaine échéance.
static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Ligne affichée dans la liste (sans le corps ni les pièces jointes).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Scheduled {
    pub id: i64,
    pub account_id: String,
    pub send_at: i64,
    pub subject: String,
    pub to: Vec<String>,
    pub status: Status,
    pub error: Option<String>,
    /// En boîte d'envoi : parti faute de connexion, repart seul (`send_at` n'est alors que la date de mise en file).
    pub outbox: bool,
}

/// Délai avant le prochain essai d'un message de la boîte d'envoi, après `attempts` échecs.
fn retry_delay(attempts: i64) -> i64 {
    crate::backoff::delay_secs(RETRY_BASE_SECS, RETRY_MAX_SECS, attempts)
}

pub async fn add(
    pool: &SqlitePool,
    account_id: &str,
    send_at: i64,
    message: &OutgoingMessage,
) -> Result<i64> {
    if send_at <= now() {
        return Err(AppError::Invalid(msg!(
            "scheduled.time_future",
            "the send time must be in the future"
        )));
    }
    insert(pool, account_id, send_at, None, message).await
}

/// Met en boîte d'envoi un message que le serveur n'a pas pu recevoir (`error` : erreur codée, `AppError::to_wire`).
pub async fn add_outbox(
    pool: &SqlitePool,
    account_id: &str,
    message: &OutgoingMessage,
    error: &str,
) -> Result<i64> {
    let at = now();
    let retry = Some((at + retry_delay(0), error));
    insert(pool, account_id, at, retry, message).await
}

async fn insert(
    pool: &SqlitePool,
    account_id: &str,
    send_at: i64,
    // Boîte d'envoi : (prochain essai, dernière erreur) ; `None` pour un envoi programmé.
    retry: Option<(i64, &str)>,
    message: &OutgoingMessage,
) -> Result<i64> {
    let json =
        serde_json::to_string(message).map_err(|e| AppError::Invalid((e.to_string()).into()))?;
    if json.len() > MAX_BYTES {
        return Err(AppError::Invalid(msg!(
            "scheduled.too_big",
            "message too large to schedule"
        )));
    }
    let (next_try_at, last_error) = retry.unzip();
    // Plafond contrôlé dans l'insertion même : deux ajouts simultanés ne le dépassent pas.
    let done = sqlx::query(
        "INSERT INTO scheduled (account_id, send_at, message, next_try_at, error) \
         SELECT ?, ?, ?, ?, ? WHERE (SELECT COUNT(*) FROM scheduled) < ?",
    )
    .bind(account_id)
    .bind(send_at)
    .bind(json)
    .bind(next_try_at)
    .bind(last_error)
    .bind(MAX_ROWS)
    .execute(pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(AppError::Invalid(msg!(
            "scheduled.too_many",
            "too many scheduled sends ({max} at most)",
            max = MAX_ROWS
        )));
    }
    WAKE.notify_one();
    Ok(done.last_insert_rowid())
}

fn parse(json: &str) -> Result<OutgoingMessage> {
    serde_json::from_str(json).map_err(|e| {
        AppError::Invalid(msg!(
            "scheduled.unreadable",
            "unreadable scheduled message: {detail}",
            detail = e
        ))
    })
}

/// Ligne de la file sans le corps du message (voir `list`).
#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    account_id: String,
    send_at: i64,
    subject: Option<String>,
    to_json: Option<String>,
    status: Status,
    error: Option<String>,
    outbox: bool,
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Scheduled>> {
    // Objet et destinataires extraits en SQL : on ne relit ni le corps ni les pièces jointes pour afficher la liste.
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, account_id, send_at, json_extract(message, '$.subject') AS subject, \
         json_extract(message, '$.to') AS to_json, status, error, next_try_at IS NOT NULL AS outbox FROM scheduled ORDER BY send_at, id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Scheduled {
            id: r.id,
            account_id: r.account_id,
            send_at: r.send_at,
            subject: r.subject.unwrap_or_default(),
            to: r
                .to_json
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default(),
            status: r.status,
            error: r.error,
            outbox: r.outbox,
        })
        .collect())
}

/// Retire un message de la file et le rend (pour le rouvrir dans le composer). Refusé pendant son envoi.
pub async fn cancel(pool: &SqlitePool, id: i64) -> Result<OutgoingMessage> {
    let json: String =
        sqlx::query_scalar("DELETE FROM scheduled WHERE id = ? AND status <> ? RETURNING message")
            .bind(id)
            .bind(Status::Sending)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| {
                AppError::Invalid(msg!(
                    "scheduled.not_found",
                    "send not found, or already in progress"
                ))
            })?;
    parse(&json)
}

/// Message pris dans la file pour être envoyé.
pub struct Claimed {
    pub account_id: String,
    pub message: OutgoingMessage,
    pub outbox: bool,
    /// Échecs réseau déjà essuyés (boîte d'envoi).
    pub attempts: i64,
}

/// Prend un message pour l'envoyer ; faux s'il a déjà été pris ou retiré (jamais deux envois du même message).
pub async fn claim(pool: &SqlitePool, id: i64) -> Result<Option<Claimed>> {
    let row: Option<(String, String, bool, i64)> = sqlx::query_as(
        "UPDATE scheduled SET status = ?, error = NULL WHERE id = ? AND status <> ? RETURNING account_id, message, next_try_at IS NOT NULL, attempts",
    )
    .bind(Status::Sending)
    .bind(id)
    .bind(Status::Sending)
    .fetch_optional(pool)
    .await?;
    row.map(|(account_id, json, outbox, attempts)| {
        Ok(Claimed {
            account_id,
            message: parse(&json)?,
            outbox,
            attempts,
        })
    })
    .transpose()
}

/// Le serveur n'a pas répondu : le message passe (ou reste) en boîte d'envoi, prochain essai après un délai croissant.
pub async fn requeue(pool: &SqlitePool, id: i64, attempts: i64, error: &str) -> Result<()> {
    sqlx::query(
        "UPDATE scheduled SET status = ?, attempts = attempts + 1, next_try_at = ?, error = ? WHERE id = ?",
    )
    .bind(Status::Pending)
    .bind(now() + retry_delay(attempts + 1)) // `attempts` : avant cet échec ; `add_outbox` a déjà compté le délai 0
    .bind(error)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM scheduled WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn fail(pool: &SqlitePool, id: i64, error: &str) -> Result<()> {
    sqlx::query("UPDATE scheduled SET status = ?, error = ? WHERE id = ?")
        .bind(Status::Failed)
        .bind(error)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Au démarrage : un envoi « en cours » à l'arrêt a peut-être abouti ; on ne le renvoie pas seul (doublon irréparable
/// chez le destinataire) : l'utilisateur décide, « Envoyés » en main.
pub async fn recover(pool: &SqlitePool) -> Result<()> {
    let interrupted = AppError::Invalid(msg!(
        "scheduled.interrupted",
        "send interrupted: check the Sent folder before sending it again"
    ));
    sqlx::query("UPDATE scheduled SET status = ?, error = ? WHERE status = ?")
        .bind(Status::Failed)
        .bind(interrupted.to_wire())
        .bind(Status::Sending)
        .execute(pool)
        .await?;
    Ok(())
}

/// Messages à envoyer maintenant : (id, en retard au-delà de la tolérance ?). Un message de la boîte d'envoi n'est
/// jamais en retard : il repart dès que son délai est écoulé.
pub async fn due(pool: &SqlitePool, at: i64) -> Result<Vec<(i64, bool)>> {
    let rows: Vec<(i64, i64, bool)> = sqlx::query_as(
        "SELECT id, send_at, next_try_at IS NOT NULL FROM scheduled WHERE status = ? \
         AND COALESCE(next_try_at, send_at) <= ? ORDER BY send_at, id",
    )
    .bind(Status::Pending)
    .bind(at)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, send_at, outbox)| (id, !outbox && at - send_at > GRACE_SECS))
        .collect())
}

/// Résultat d'un envoi programmé, transmis à l'interface (toast + liste).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Outcome {
    subject: String,
    error: Option<String>,
    /// Parti de la boîte d'envoi (et non à une heure choisie) : le message affiché n'est pas le même.
    outbox: bool,
}

/// Envoie maintenant le message `id` (heure venue, ou demande de l'utilisateur) et rend compte à l'interface.
pub async fn deliver_now(app: &AppHandle, state: &AppState, id: i64) -> Result<()> {
    let Some(Claimed {
        account_id,
        message,
        outbox,
        attempts,
    }) = claim(&state.db, id).await?
    else {
        return Ok(()); // déjà pris ou retiré
    };
    let result = crate::commands::deliver(app, state, &account_id, &message).await;
    // Ce que l'interface doit savoir : un envoi réussi ou en échec (toast), ou rien (le message reste en boîte d'envoi).
    let outcome = |error| Outcome {
        subject: message.subject.clone(),
        error,
        outbox,
    };
    let outcome = match &result {
        Ok(_) => {
            // Le message est parti : un échec de nettoyage ne doit pas le faire passer pour un échec d'envoi.
            if let Err(e) = remove(&state.db, id).await {
                tracing::warn!("scheduled send {id} went out but is still queued: {e}");
            }
            Some(outcome(None))
        }
        // Pas de réseau : le message reste en boîte d'envoi, sans bruit (il repartira tout seul).
        Err(e @ AppError::Unreachable(_)) => {
            requeue(&state.db, id, attempts, &e.to_wire()).await?;
            None
        }
        Err(e) => {
            fail(&state.db, id, &e.to_wire()).await?; // codé : le front le traduit à l'affichage
            Some(outcome(Some(e.to_wire())))
        }
    };
    let _ = app.emit("scheduled:changed", outcome);
    result.map(|_| ())
}

/// Prochaine échéance d'un message en attente (heure choisie, ou prochain essai de la boîte d'envoi).
pub async fn next_due(pool: &SqlitePool) -> Result<Option<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT MIN(COALESCE(next_try_at, send_at)) FROM scheduled WHERE status = ?",
    )
    .bind(Status::Pending)
    .fetch_one(pool)
    .await?)
}

/// Attente avant la prochaine vérification : jusqu'à la prochaine échéance, sans dépasser `TICK_MAX` (l'horloge
/// monotone de tokio s'arrête pendant la veille de l'ordinateur : au réveil, l'attente restante serait trop longue).
fn wait_until(next: Option<i64>, at: i64) -> Duration {
    let secs = next.map_or(TICK_MAX.as_secs() as i64, |n| n - at);
    Duration::from_secs(secs.clamp(1, TICK_MAX.as_secs() as i64) as u64)
}

async fn tick(app: &AppHandle, state: &AppState) {
    let Ok(due) = due(&state.db, now()).await else {
        return;
    };
    // En parallèle : un serveur SMTP lent ne retarde pas les autres messages (`claim` interdit tout double envoi).
    futures::stream::iter(due)
        .for_each_concurrent(4, |(id, late)| async move {
            if late {
                let missed = AppError::Invalid(msg!(
                    "scheduled.missed",
                    "the app wasn't open at the scheduled time: send it now or edit it"
                ));
                let _ = fail(&state.db, id, &missed.to_wire()).await;
                let _ = app.emit("scheduled:changed", Option::<Outcome>::None);
            } else if let Err(e) = deliver_now(app, state, id).await {
                tracing::warn!("scheduled send {id}: {e}");
            }
        })
        .await;
}

/// Boucle d'envoi : une vérification au démarrage (messages échus pendant l'arrêt), puis à chaque échéance ou ajout.
pub async fn run(app: AppHandle) {
    let state = app.state::<AppState>();
    if let Err(e) = recover(&state.db).await {
        tracing::warn!("resuming scheduled sends: {e}");
    }
    loop {
        tick(&app, &state).await;
        let next = next_due(&state.db).await.unwrap_or(None);
        tokio::select! {
            () = tokio::time::sleep(wait_until(next, now())) => {}
            () = WAKE.notified() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attente_jusqu_a_la_prochaine_echeance_bornee() {
        assert_eq!(wait_until(None, 1000), TICK_MAX);
        assert_eq!(wait_until(Some(1042), 1000), Duration::from_secs(42));
        assert_eq!(wait_until(Some(900), 1000), Duration::from_secs(1)); // échu : tout de suite
        assert_eq!(wait_until(Some(1000 + 86_400), 1000), TICK_MAX);
    }

    fn message(subject: &str) -> OutgoingMessage {
        serde_json::from_value(serde_json::json!({
            "to": ["bob@x.fr"], "cc": [], "bcc": [], "subject": subject, "bodyText": "Bonjour", "references": []
        }))
        .unwrap()
    }

    async fn pool() -> SqlitePool {
        let pool = crate::db::init_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','me@x.fr','a','i','s')")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn file_liste_annule_et_refuse_le_passe() {
        let pool = pool().await;
        assert!(add(&pool, "a", now() - 5, &message("passé")).await.is_err());
        let later = add(&pool, "a", now() + 7200, &message("plus tard"))
            .await
            .unwrap();
        let soon = add(&pool, "a", now() + 60, &message("bientôt"))
            .await
            .unwrap();
        let l = list(&pool).await.unwrap();
        assert_eq!(
            l.iter().map(|s| s.subject.as_str()).collect::<Vec<_>>(),
            ["bientôt", "plus tard"]
        );
        assert_eq!(l[0].to, ["bob@x.fr"]);
        let back = cancel(&pool, soon).await.unwrap();
        assert_eq!(back.subject, "bientôt");
        assert!(cancel(&pool, soon).await.is_err(), "déjà retiré");
        assert_eq!(list(&pool).await.unwrap()[0].id, later);
    }

    #[tokio::test]
    async fn echus_en_retard_et_un_seul_envoi_par_message() {
        let pool = pool().await;
        let json = serde_json::to_string(&message("m")).unwrap();
        for at in [1000, 5000] {
            sqlx::query("INSERT INTO scheduled (account_id, send_at, message) VALUES ('a', ?, ?)")
                .bind(at)
                .bind(&json)
                .execute(&pool)
                .await
                .unwrap();
        }
        // À t = 5100 : le premier a 4100 s de retard (au-delà de la tolérance), le second 100 s.
        assert_eq!(due(&pool, 5100).await.unwrap(), [(1, true), (2, false)]);
        assert!(due(&pool, 500).await.unwrap().is_empty());
        assert!(claim(&pool, 2).await.unwrap().is_some());
        assert!(claim(&pool, 2).await.unwrap().is_none(), "déjà pris");
        assert!(
            cancel(&pool, 2).await.is_err(),
            "pas d'annulation pendant l'envoi"
        );
        assert!(due(&pool, 5100)
            .await
            .unwrap()
            .iter()
            .all(|(id, _)| *id != 2));
        // Reprise après arrêt brutal : l'envoi en cours est signalé, pas renvoyé.
        recover(&pool).await.unwrap();
        let l = list(&pool).await.unwrap();
        assert_eq!((l[1].status, l[1].error.is_some()), (Status::Failed, true));
        // Un échec reste dans la file, et peut être repris.
        fail(&pool, 1, "boum").await.unwrap();
        assert!(claim(&pool, 1).await.unwrap().is_some());
    }

    #[test]
    fn delai_croissant_plafonne() {
        assert_eq!(
            (0..8).map(retry_delay).collect::<Vec<_>>(),
            [30, 60, 120, 240, 480, 600, 600, 600]
        );
    }

    #[tokio::test]
    async fn boite_d_envoi_repart_seule_et_n_est_jamais_en_retard() {
        let pool = pool().await;
        let id = add_outbox(&pool, "a", &message("hors ligne"), "boum")
            .await
            .unwrap();
        let l = list(&pool).await.unwrap();
        assert!(
            l[0].outbox && l[0].status == Status::Pending && l[0].error.as_deref() == Some("boum")
        );
        // Pas avant le premier délai ; ensuite, même très longtemps après, jamais « manqué ».
        assert!(due(&pool, now()).await.unwrap().is_empty());
        let later = now() + 10 * 86_400;
        assert_eq!(due(&pool, later).await.unwrap(), [(id, false)]);
        // Échec réseau : reste en file, délai doublé, dernière erreur conservée.
        let c = claim(&pool, id).await.unwrap().unwrap();
        assert!(c.outbox && c.message.subject == "hors ligne");
        requeue(&pool, id, c.attempts, "toujours pas")
            .await
            .unwrap();
        let (status, attempts, next): (String, i64, i64) =
            sqlx::query_as("SELECT status, attempts, next_try_at FROM scheduled WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!((status.as_str(), attempts), ("pending", 1));
        assert!((next - now() - 60).abs() <= 2, "deuxième délai : 60 s");
        assert!(due(&pool, now() + 30).await.unwrap().is_empty());
        // Un envoi programmé qui perd le réseau à son heure passe aussi en boîte d'envoi.
        let at = add(&pool, "a", now() + 60, &message("programmé"))
            .await
            .unwrap();
        let c = claim(&pool, at).await.unwrap().unwrap();
        requeue(&pool, at, c.attempts, "x").await.unwrap();
        assert!(list(&pool).await.unwrap().iter().all(|s| s.outbox));
        // On peut toujours le reprendre pour le modifier.
        assert_eq!(cancel(&pool, id).await.unwrap().subject, "hors ligne");
    }

    #[tokio::test]
    async fn supprimer_le_compte_vide_sa_file() {
        let pool = pool().await;
        add(&pool, "a", now() + 60, &message("m")).await.unwrap();
        sqlx::query("DELETE FROM accounts WHERE id = 'a'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(list(&pool).await.unwrap().is_empty());
    }
}

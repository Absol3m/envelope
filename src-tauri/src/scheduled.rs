//! Envois programmés : la file (SQLite) et la boucle qui envoie les messages à leur heure.
//!
//! IMAP/SMTP n'offrent pas d'envoi différé côté serveur : c'est l'application qui envoie, donc elle doit être
//! ouverte à l'heure prévue. Un message en retard de plus d'une heure (application fermée) n'est **pas** envoyé
//! de lui-même : un rappel « lundi 8 h » parti le mardi serait pire qu'un message en attente de décision.
//!
//! La même file sert de **boîte d'envoi** : un message dont l'envoi échoue faute de connexion (`AppError::Unreachable`)
//! y reste (`outbox = 1`) et repart seul, avec un délai croissant, dès que le serveur répond. Il n'est jamais « en
//! retard » : il n'a pas d'heure à respecter, seulement un réseau à retrouver.

use std::time::Duration;

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};

use crate::error::{AppError, Result};
use crate::mail::smtp::OutgoingMessage;
use crate::AppState;

const TICK: Duration = Duration::from_secs(30);
/// Retard toléré au-delà de l'heure prévue (veille de l'ordinateur, application relancée juste après).
const GRACE_SECS: i64 = 3600;
const MAX_ROWS: i64 = 200;
/// Taille maximale d'un message en file (JSON) : 25 Mo de pièces jointes, en base64, plus le texte.
const MAX_BYTES: usize = 40 * 1024 * 1024;
/// Boîte d'envoi : délai entre deux essais, de 30 s doublé à chaque échec jusqu'à 10 min.
const RETRY_BASE_SECS: i64 = 30;
const RETRY_MAX_SECS: i64 = 600;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Ligne affichée dans la liste (sans le corps ni les pièces jointes).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Scheduled {
    pub id: i64,
    pub account_id: String,
    pub send_at: i64,
    pub subject: String,
    pub to: Vec<String>,
    /// `pending` | `sending` | `failed`
    pub status: String,
    pub error: Option<String>,
    /// En boîte d'envoi : parti faute de connexion, repart seul (`send_at` n'est alors que la date de mise en file).
    pub outbox: bool,
}

/// Délai avant le prochain essai d'un message de la boîte d'envoi, après `attempts` échecs.
fn retry_delay(attempts: i64) -> i64 {
    (RETRY_BASE_SECS << attempts.clamp(0, 10)).min(RETRY_MAX_SECS)
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
    let outbox = Outbox {
        next_try_at: at + retry_delay(0),
        error,
    };
    insert(pool, account_id, at, Some(outbox), message).await
}

/// Boîte d'envoi : prochain essai et dernière erreur d'une ligne (`None` : envoi programmé).
struct Outbox<'a> {
    next_try_at: i64,
    error: &'a str,
}

async fn insert(
    pool: &SqlitePool,
    account_id: &str,
    send_at: i64,
    outbox: Option<Outbox<'_>>,
    message: &OutgoingMessage,
) -> Result<i64> {
    let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scheduled")
        .fetch_one(pool)
        .await?;
    if queued >= MAX_ROWS {
        return Err(AppError::Invalid(msg!(
            "scheduled.too_many",
            "too many scheduled sends ({max} at most)",
            max = MAX_ROWS
        )));
    }
    let json =
        serde_json::to_string(message).map_err(|e| AppError::Invalid((e.to_string()).into()))?;
    if json.len() > MAX_BYTES {
        return Err(AppError::Invalid(msg!(
            "scheduled.too_big",
            "message too large to schedule"
        )));
    }
    Ok(sqlx::query(
        "INSERT INTO scheduled (account_id, send_at, message, outbox, next_try_at, error) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(account_id)
    .bind(send_at)
    .bind(json)
    .bind(outbox.is_some())
    .bind(outbox.as_ref().map(|o| o.next_try_at))
    .bind(outbox.as_ref().map(|o| o.error))
    .execute(pool)
    .await?
    .last_insert_rowid())
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
    status: String,
    error: Option<String>,
    outbox: bool,
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Scheduled>> {
    // Objet et destinataires extraits en SQL : on ne relit ni le corps ni les pièces jointes pour afficher la liste.
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, account_id, send_at, json_extract(message, '$.subject') AS subject, \
         json_extract(message, '$.to') AS to_json, status, error, outbox FROM scheduled ORDER BY send_at, id",
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
    let json: String = sqlx::query_scalar(
        "DELETE FROM scheduled WHERE id = ? AND status <> 'sending' RETURNING message",
    )
    .bind(id)
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
}

/// Prend un message pour l'envoyer ; faux s'il a déjà été pris ou retiré (jamais deux envois du même message).
pub async fn claim(pool: &SqlitePool, id: i64) -> Result<Option<Claimed>> {
    let row: Option<(String, String, bool)> = sqlx::query_as(
        "UPDATE scheduled SET status = 'sending', error = NULL WHERE id = ? AND status <> 'sending' RETURNING account_id, message, outbox",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.map(|(account_id, json, outbox)| {
        Ok(Claimed {
            account_id,
            message: parse(&json)?,
            outbox,
        })
    })
    .transpose()
}

/// Le serveur n'a pas répondu : le message passe (ou reste) en boîte d'envoi, prochain essai après un délai croissant.
pub async fn requeue(pool: &SqlitePool, id: i64, error: &str) -> Result<()> {
    let attempts: i64 = sqlx::query_scalar("SELECT attempts FROM scheduled WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .unwrap_or(0);
    sqlx::query(
        "UPDATE scheduled SET status = 'pending', outbox = 1, attempts = attempts + 1, next_try_at = ?, error = ? WHERE id = ?",
    )
    .bind(now() + retry_delay(attempts + 1))
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
    sqlx::query("UPDATE scheduled SET status = 'failed', error = ? WHERE id = ?")
        .bind(error)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Au démarrage : un envoi « en cours » à l'arrêt de l'application a peut-être abouti ; on ne le renvoie pas seul.
pub async fn recover(pool: &SqlitePool) -> Result<()> {
    let interrupted = AppError::Invalid(msg!(
        "scheduled.interrupted",
        "send interrupted: check the Sent folder before sending it again"
    ));
    sqlx::query("UPDATE scheduled SET status = 'failed', error = ? WHERE status = 'sending'")
        .bind(interrupted.to_wire())
        .execute(pool)
        .await?;
    Ok(())
}

/// Messages à envoyer maintenant : (id, en retard au-delà de la tolérance ?). Un message de la boîte d'envoi n'est
/// jamais en retard : il repart dès que son délai est écoulé.
pub async fn due(pool: &SqlitePool, at: i64) -> Result<Vec<(i64, bool)>> {
    let rows: Vec<(i64, i64, bool)> = sqlx::query_as(
        "SELECT id, send_at, outbox FROM scheduled WHERE status = 'pending' \
         AND (CASE WHEN outbox = 1 THEN next_try_at ELSE send_at END) <= ? ORDER BY send_at, id",
    )
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
    }) = claim(&state.db, id).await?
    else {
        return Ok(()); // déjà pris ou retiré
    };
    let result = crate::commands::deliver(app, state, &account_id, &message).await;
    let error = match &result {
        Ok(_) => {
            // Le message est parti : un échec de nettoyage ne doit pas le faire passer pour un échec d'envoi.
            if let Err(e) = remove(&state.db, id).await {
                tracing::warn!("scheduled send {id} went out but is still queued: {e}");
            }
            None
        }
        // Pas de réseau : le message reste en boîte d'envoi, sans bruit (il repartira tout seul).
        Err(e @ AppError::Unreachable(_)) => {
            requeue(&state.db, id, &e.to_wire()).await?;
            let _ = app.emit("scheduled:changed", Option::<Outcome>::None);
            return result.map(|_| ());
        }
        Err(e) => {
            fail(&state.db, id, &e.to_wire()).await?; // codé : le front le traduit à l'affichage
            Some(e.to_wire())
        }
    };
    let _ = app.emit(
        "scheduled:changed",
        Outcome {
            subject: message.subject,
            error,
            outbox,
        },
    );
    result.map(|_| ())
}

async fn tick(app: &AppHandle, state: &AppState) {
    let Ok(due) = due(&state.db, now()).await else {
        return;
    };
    for (id, late) in due {
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
    }
}

/// Boucle d'envoi : une vérification toutes les 30 s, la première tout de suite (messages échus pendant l'arrêt).
pub async fn run(app: AppHandle) {
    let state = app.state::<AppState>();
    if let Err(e) = recover(&state.db).await {
        tracing::warn!("resuming scheduled sends: {e}");
    }
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        tick(&app, &state).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            (l[1].status.as_str(), l[1].error.is_some()),
            ("failed", true)
        );
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
        assert_eq!(retry_delay(i64::MAX), RETRY_MAX_SECS);
    }

    #[tokio::test]
    async fn boite_d_envoi_repart_seule_et_n_est_jamais_en_retard() {
        let pool = pool().await;
        let id = add_outbox(&pool, "a", &message("hors ligne"), "boum")
            .await
            .unwrap();
        let l = list(&pool).await.unwrap();
        assert!(l[0].outbox && l[0].status == "pending" && l[0].error.as_deref() == Some("boum"));
        // Pas avant le premier délai ; ensuite, même très longtemps après, jamais « manqué ».
        assert!(due(&pool, now()).await.unwrap().is_empty());
        let later = now() + 10 * 86_400;
        assert_eq!(due(&pool, later).await.unwrap(), [(id, false)]);
        // Échec réseau : reste en file, délai doublé, dernière erreur conservée.
        let c = claim(&pool, id).await.unwrap().unwrap();
        assert!(c.outbox && c.message.subject == "hors ligne");
        requeue(&pool, id, "toujours pas").await.unwrap();
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
        claim(&pool, at).await.unwrap();
        requeue(&pool, at, "x").await.unwrap();
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

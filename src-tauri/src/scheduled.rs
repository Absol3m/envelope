//! Envois programmés : la file (SQLite) et la boucle qui envoie les messages à leur heure.
//!
//! IMAP/SMTP n'offrent pas d'envoi différé côté serveur : c'est l'application qui envoie, donc elle doit être
//! ouverte à l'heure prévue. Un message en retard de plus d'une heure (application fermée) n'est **pas** envoyé
//! de lui-même : un rappel « lundi 8 h » parti le mardi serait pire qu'un message en attente de décision.

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
    Ok(
        sqlx::query("INSERT INTO scheduled (account_id, send_at, message) VALUES (?, ?, ?)")
            .bind(account_id)
            .bind(send_at)
            .bind(json)
            .execute(pool)
            .await?
            .last_insert_rowid(),
    )
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
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Scheduled>> {
    // Objet et destinataires extraits en SQL : on ne relit ni le corps ni les pièces jointes pour afficher la liste.
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, account_id, send_at, json_extract(message, '$.subject') AS subject, \
         json_extract(message, '$.to') AS to_json, status, error FROM scheduled ORDER BY send_at, id",
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

/// Prend un message pour l'envoyer ; faux s'il a déjà été pris ou retiré (jamais deux envois du même message).
pub async fn claim(pool: &SqlitePool, id: i64) -> Result<Option<(String, OutgoingMessage)>> {
    let row: Option<(String, String)> = sqlx::query_as(
        "UPDATE scheduled SET status = 'sending', error = NULL WHERE id = ? AND status <> 'sending' RETURNING account_id, message",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.map(|(account, json)| Ok((account, parse(&json)?)))
        .transpose()
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

/// Messages dont l'heure est venue : (id, en retard au-delà de la tolérance ?).
pub async fn due(pool: &SqlitePool, at: i64) -> Result<Vec<(i64, bool)>> {
    let rows: Vec<(i64, i64)> =
        sqlx::query_as("SELECT id, send_at FROM scheduled WHERE status = 'pending' AND send_at <= ? ORDER BY send_at, id")
            .bind(at)
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, send_at)| (id, at - send_at > GRACE_SECS))
        .collect())
}

/// Résultat d'un envoi programmé, transmis à l'interface (toast + liste).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Outcome {
    subject: String,
    error: Option<String>,
}

/// Envoie maintenant le message `id` (heure venue, ou demande de l'utilisateur) et rend compte à l'interface.
pub async fn deliver_now(app: &AppHandle, state: &AppState, id: i64) -> Result<()> {
    let Some((account, message)) = claim(&state.db, id).await? else {
        return Ok(()); // déjà pris ou retiré
    };
    let result = crate::commands::deliver(app, state, &account, &message).await;
    let error = match &result {
        Ok(_) => {
            // Le message est parti : un échec de nettoyage ne doit pas le faire passer pour un échec d'envoi.
            if let Err(e) = remove(&state.db, id).await {
                tracing::warn!("envoi programmé {id} parti, mais encore dans la file : {e}");
            }
            None
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
            tracing::warn!("envoi programmé {id} : {e}");
        }
    }
}

/// Boucle d'envoi : une vérification toutes les 30 s, la première tout de suite (messages échus pendant l'arrêt).
pub async fn run(app: AppHandle) {
    let state = app.state::<AppState>();
    if let Err(e) = recover(&state.db).await {
        tracing::warn!("reprise des envois programmés : {e}");
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

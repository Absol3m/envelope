//! Rappels locaux par conversation : report (« snooze ») et relance si pas de réponse.
//!
//! Rien n'est écrit sur le serveur (IMAP n'a pas de report) : la conversation reste où elle est, l'application la
//! masque des listes jusqu'à l'échéance. Comme l'envoi programmé, l'échéance n'est vérifiée que application ouverte ;
//! une échéance passée pendant l'arrêt est traitée au démarrage (la conversation revient simplement plus tard).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};

use crate::clock::now;
use crate::error::{AppError, Result};
use crate::AppState;

const TICK: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Snooze,
    Followup,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Snooze => "snooze",
            Kind::Followup => "followup",
        }
    }
}

/// Pose (ou remplace) le rappel d'une conversation. `due_at` : secondes Unix, dans le futur.
pub async fn set(pool: &SqlitePool, thread_id: i64, kind: Kind, due_at: i64) -> Result<()> {
    if due_at <= now() {
        return Err(AppError::Invalid(msg!(
            "reminders.future",
            "the due date must be in the future"
        )));
    }
    let done = sqlx::query(
        "INSERT INTO reminders (thread_id, kind, due_at, baseline_count, fired_at) \
         SELECT id, ?, ?, message_count, NULL FROM threads WHERE id = ? \
         ON CONFLICT(thread_id) DO UPDATE SET kind = excluded.kind, due_at = excluded.due_at, \
         baseline_count = excluded.baseline_count, fired_at = NULL",
    )
    .bind(kind.as_str())
    .bind(due_at)
    .bind(thread_id)
    .execute(pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(AppError::Invalid(msg!(
            "not_found.conversation",
            "conversation not found"
        )));
    }
    Ok(())
}

/// Retire le rappel (annulation, ou acquittement d'un rappel échu).
pub async fn clear(pool: &SqlitePool, thread_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM reminders WHERE thread_id = ?")
        .bind(thread_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Un message est arrivé dans la conversation (`message_count` > `baseline_count`) : la relance n'a plus lieu d'être
/// (supprimée), et un report prend fin tout de suite (la conversation revient avec son nouveau message).
pub async fn on_new_messages(tx: &mut sqlx::SqliteConnection, thread_id: i64) -> Result<()> {
    const GROWN: &str = "(SELECT message_count FROM threads WHERE id = thread_id) > baseline_count";
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DELETE FROM reminders WHERE thread_id = ? AND kind = 'followup' AND fired_at IS NULL AND {GROWN}"
    )))
    .bind(thread_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE reminders SET fired_at = ? WHERE thread_id = ? AND kind = 'snooze' AND fired_at IS NULL AND {GROWN}"
    )))
    .bind(now())
    .bind(thread_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Traite les échéances : un report revient dans les listes ; une relance revient seulement si personne n'a répondu
/// (sinon elle est supprimée sans bruit). Rend le nombre de conversations revenues.
pub async fn fire_due(pool: &SqlitePool, at: i64) -> Result<u64> {
    // Lecture d'abord : pas de verrou d'écriture toutes les 30 s quand rien n'est échu.
    let due: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM reminders WHERE fired_at IS NULL AND due_at <= ? LIMIT 1",
    )
    .bind(at)
    .fetch_optional(pool)
    .await?;
    if due.is_none() {
        return Ok(0);
    }
    let mut tx = pool.begin().await?;
    sqlx::query(
        "DELETE FROM reminders WHERE kind = 'followup' AND fired_at IS NULL AND due_at <= ?1 \
         AND (SELECT message_count FROM threads WHERE id = thread_id) > baseline_count",
    )
    .bind(at)
    .execute(&mut *tx)
    .await?;
    let fired =
        sqlx::query("UPDATE reminders SET fired_at = ?1 WHERE fired_at IS NULL AND due_at <= ?1")
            .bind(at)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    tx.commit().await?;
    Ok(fired)
}

/// Boucle d'échéance : une vérification toutes les 30 s, la première tout de suite.
pub async fn run(app: AppHandle) {
    let state = app.state::<AppState>();
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        match fire_due(&state.db, now()).await {
            Ok(0) => {}
            Ok(n) => {
                let _ = app.emit("reminders:fired", n);
            }
            Err(e) => tracing::warn!("reminders: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use envelope_core::thread::{list_threads, ThreadQuery};

    async fn pool() -> SqlitePool {
        let pool = crate::db::init_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','me@x.fr','a','i','s')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO threads (id, account_id, last_message_at, message_count) VALUES (1,'a','2026-01-01',2),(2,'a','2026-01-02',1)")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn state(pool: &SqlitePool, id: i64) -> Option<(String, Option<i64>)> {
        sqlx::query_as("SELECT kind, fired_at FROM reminders WHERE thread_id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .unwrap()
    }

    /// L'échéance est dans le futur à la pose : on avance l'horloge en passant `at` à `fire_due`.
    #[tokio::test]
    async fn report_et_relance_a_l_echeance() {
        let pool = pool().await;
        assert!(set(&pool, 1, Kind::Snooze, now() - 1).await.is_err());
        assert!(set(&pool, 99, Kind::Snooze, now() + 60).await.is_err());
        set(&pool, 1, Kind::Snooze, now() + 60).await.unwrap();
        set(&pool, 2, Kind::Followup, now() + 60).await.unwrap();
        assert_eq!(fire_due(&pool, now()).await.unwrap(), 0);
        // Une réponse arrive dans la conversation 2 avant l'échéance : la relance disparaît sans bruit.
        sqlx::query("UPDATE threads SET message_count = 2 WHERE id = 2")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(fire_due(&pool, now() + 120).await.unwrap(), 1);
        assert!(matches!(state(&pool, 1).await, Some((k, Some(_))) if k == "snooze"));
        assert_eq!(state(&pool, 2).await, None);
        // Remplacer un rappel échu le réarme ; l'effacer le supprime.
        set(&pool, 1, Kind::Followup, now() + 60).await.unwrap();
        assert_eq!(state(&pool, 1).await, Some(("followup".into(), None)));
        clear(&pool, 1).await.unwrap();
        assert_eq!(state(&pool, 1).await, None);
    }

    #[tokio::test]
    async fn relance_sans_reponse_revient() {
        let pool = pool().await;
        set(&pool, 2, Kind::Followup, now() + 60).await.unwrap();
        assert_eq!(fire_due(&pool, now() + 120).await.unwrap(), 1);
        assert!(matches!(state(&pool, 2).await, Some((_, Some(_)))));
    }

    async fn ids(pool: &SqlitePool, q: ThreadQuery) -> Vec<i64> {
        let mut v: Vec<i64> = list_threads(pool, &q)
            .await
            .unwrap()
            .iter()
            .map(|t| t.id)
            .collect();
        v.sort();
        v
    }
    fn only() -> ThreadQuery {
        ThreadQuery {
            reminders_only: true,
            ..Default::default()
        }
    }

    /// Un message arrivé pendant le rappel : la relance disparaît, le report prend fin aussitôt.
    #[tokio::test]
    async fn arrivee_d_un_message_leve_le_rappel() {
        let pool = pool().await;
        set(&pool, 1, Kind::Snooze, now() + 60).await.unwrap();
        set(&pool, 2, Kind::Followup, now() + 60).await.unwrap();
        sqlx::query("UPDATE threads SET message_count = message_count + 1")
            .execute(&pool)
            .await
            .unwrap();
        for id in [1, 2] {
            let mut c = pool.acquire().await.unwrap();
            on_new_messages(&mut c, id).await.unwrap();
        }
        assert!(matches!(state(&pool, 1).await, Some((_, Some(_)))));
        assert_eq!(state(&pool, 2).await, None);
    }

    #[tokio::test]
    async fn report_masque_des_listes_mais_pas_de_la_recherche() {
        let pool = pool().await;
        set(&pool, 1, Kind::Snooze, now() + 60).await.unwrap();
        set(&pool, 2, Kind::Followup, now() + 60).await.unwrap();
        assert_eq!(
            ids(&pool, ThreadQuery::default()).await,
            vec![2],
            "report masqué, relance visible"
        );
        assert_eq!(ids(&pool, only()).await, vec![1, 2]);
        fire_due(&pool, now() + 120).await.unwrap();
        assert_eq!(
            ids(&pool, ThreadQuery::default()).await,
            vec![1, 2],
            "échu : revenu"
        );
        assert!(ids(&pool, only()).await.is_empty());
        // Un fil revenu passe devant, même s'il est plus ancien.
        sqlx::query("DELETE FROM reminders WHERE thread_id = 2")
            .execute(&pool)
            .await
            .unwrap();
        let first = list_threads(&pool, &ThreadQuery::default()).await.unwrap();
        assert_eq!(
            first[0].id, 1,
            "revenu en tête (thread 2 est pourtant le plus récent)"
        );
    }
}

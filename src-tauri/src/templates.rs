//! Modèles de message : liste, enregistrement (même nom = remplacement) et suppression.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use ts_rs::TS;

use crate::error::{AppError, Result};

const MAX_NAME: usize = 100;
const MAX_SUBJECT: usize = 500;
const MAX_BODY: usize = 200_000;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    pub subject: String,
    pub body_html: String,
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Template>> {
    Ok(
        sqlx::query_as("SELECT id, name, subject, body_html FROM templates ORDER BY name")
            .fetch_all(pool)
            .await?,
    )
}

/// Enregistre un modèle ; un modèle du même nom (sans tenir compte de la casse) est remplacé.
pub async fn save(pool: &SqlitePool, t: &Template) -> Result<Template> {
    let name = t.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(AppError::Invalid(msg!(
            "templates.name_length",
            "template name: 1 to {max} characters",
            max = MAX_NAME
        )));
    }
    if t.subject.chars().count() > MAX_SUBJECT || t.body_html.len() > MAX_BODY {
        return Err(AppError::Invalid(msg!(
            "templates.too_long",
            "template too long"
        )));
    }
    Ok(sqlx::query_as(
        "INSERT INTO templates (name, subject, body_html) VALUES (?, ?, ?)
         ON CONFLICT(name) DO UPDATE SET name = excluded.name, subject = excluded.subject, body_html = excluded.body_html
         RETURNING id, name, subject, body_html",
    )
    .bind(name)
    .bind(&t.subject)
    .bind(&t.body_html)
    .fetch_one(pool)
    .await?)
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM templates WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(name: &str, body: &str) -> Template {
        Template {
            id: 0,
            name: name.into(),
            subject: "Objet".into(),
            body_html: body.into(),
        }
    }

    #[tokio::test]
    async fn enregistre_remplace_liste_et_supprime() {
        let pool = crate::db::init_memory().await.unwrap();
        let a = save(&pool, &t("Relance", "<p>v1</p>")).await.unwrap();
        save(&pool, &t("Accusé", "<p>ok</p>")).await.unwrap();
        // Même nom, casse différente : remplacé, pas dupliqué.
        let b = save(&pool, &t("  relance ", "<p>v2</p>")).await.unwrap();
        assert_eq!(a.id, b.id);
        let all = list(&pool).await.unwrap();
        assert_eq!(
            all.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
            ["Accusé", "relance"]
        );
        assert_eq!(all[1].body_html, "<p>v2</p>");
        delete(&pool, a.id).await.unwrap();
        assert_eq!(list(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn refuse_nom_vide_ou_trop_long() {
        let pool = crate::db::init_memory().await.unwrap();
        assert!(save(&pool, &t("  ", "x")).await.is_err());
        assert!(save(&pool, &t(&"a".repeat(101), "x")).await.is_err());
        assert!(save(&pool, &t("ok", &"a".repeat(200_001))).await.is_err());
    }
}

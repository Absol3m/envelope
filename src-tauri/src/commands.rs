//! Commandes Tauri exposées au frontend (`invoke`).

use serde::Serialize;
use tauri::State;

use crate::{error::Result, AppState};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    name: &'static str,
    version: &'static str,
    db_path: String,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    id: String,
    email: String,
    display_name: Option<String>,
    provider: String,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        db_path: state.db_path.display().to_string(),
    }
}

#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>> {
    let rows = sqlx::query_as::<_, Account>(
        "SELECT id, email, display_name, provider FROM accounts ORDER BY created_at",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows)
}

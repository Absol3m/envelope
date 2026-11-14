//! Contrat de la liste des conversations : le scénario `tests/contract/list_threads.json` est rejoué ici contre le
//! vrai moteur (SQLite) et, côté interface, contre la démo (`src/lib/__tests__/listContract.test.ts`). Les deux
//! doivent rendre les mêmes conversations dans le même ordre : la démo ne dérive plus du moteur sans que ça se voie.
//! Toute évolution des filtres, du tri ou du résumé de la liste ajoute un cas au scénario.

use std::collections::{HashMap, HashSet};

use envelope_core::thread::{list_threads, ThreadQuery};
use serde::Deserialize;
use serde_json::{Map, Value};
use sqlx::SqlitePool;

use crate::mail::ingest;

#[derive(Deserialize)]
struct Scenario {
    accounts: Vec<String>,
    folders: Vec<Folder>,
    threads: Vec<Thread>,
    queries: Vec<Query>,
    summaries: HashMap<i64, Map<String, Value>>,
}

#[derive(Deserialize)]
struct Folder {
    id: i64,
    account: String,
    role: String,
}

#[derive(Deserialize)]
struct Thread {
    id: i64,
    account: String,
    folder: i64,
    starred: bool,
    attachments: bool,
    reminder: Option<Reminder>,
    messages: Vec<Message>,
}

#[derive(Deserialize)]
struct Reminder {
    kind: String,
    due: i64,
    fired: Option<i64>,
}

#[derive(Deserialize)]
struct Message {
    from: (String, String),
    subject: String,
    date: String,
    read: bool,
    text: String,
}

#[derive(Deserialize)]
struct Query {
    name: String,
    query: ThreadQuery,
    ids: Vec<i64>,
}

async fn seed(pool: &SqlitePool, s: &Scenario) {
    // Le compte local est créé par la migration : le scénario pose ses propres comptes.
    sqlx::query("DELETE FROM accounts WHERE id = 'local'")
        .execute(pool)
        .await
        .unwrap();
    for id in &s.accounts {
        sqlx::query(
            "INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES (?1,?1,?1,'i','s')",
        )
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }
    for f in &s.folders {
        sqlx::query("INSERT INTO folders (id,account_id,path,name,role) VALUES (?1,?2,?3,?3,?3)")
            .bind(f.id)
            .bind(&f.account)
            .bind(&f.role)
            .execute(pool)
            .await
            .unwrap();
    }
    let mut uid = 0;
    for t in &s.threads {
        sqlx::query("INSERT INTO threads (id,account_id,subject,last_message_at) VALUES (?,?,?,?)")
            .bind(t.id)
            .bind(&t.account)
            .bind(&t.messages[0].subject)
            .bind(&t.messages[0].date)
            .execute(pool)
            .await
            .unwrap();
        for (i, m) in t.messages.iter().enumerate() {
            uid += 1;
            // Le drapeau « suivi » et la pièce jointe sont portés par le dernier message de la conversation.
            let last = i == t.messages.len() - 1;
            sqlx::query(
                "INSERT INTO messages (account_id,folder_id,thread_id,uid,message_id,subject,from_name,from_addr,date,\
                 body_text,is_read,is_starred,has_attachments) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&t.account)
            .bind(t.folder)
            .bind(t.id)
            .bind(uid)
            .bind(format!("m{uid}@contract"))
            .bind(&m.subject)
            .bind(&m.from.0)
            .bind(&m.from.1)
            .bind(&m.date)
            .bind(&m.text)
            .bind(m.read)
            .bind(last && t.starred)
            .bind(last && t.attachments)
            .execute(pool)
            .await
            .unwrap();
        }
    }
    // Les agrégats d'abord, les rappels ensuite : `refresh_threads` retire un rappel de relance dès que la conversation grossit.
    let ids: HashSet<i64> = s.threads.iter().map(|t| t.id).collect();
    ingest::refresh_threads(pool, &ids).await.unwrap();
    for (t, r) in s
        .threads
        .iter()
        .filter_map(|t| t.reminder.as_ref().map(|r| (t, r)))
    {
        sqlx::query("INSERT INTO reminders (thread_id,kind,due_at,fired_at) VALUES (?,?,?,?)")
            .bind(t.id)
            .bind(&r.kind)
            .bind(r.due)
            .bind(r.fired)
            .execute(pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn le_moteur_respecte_le_contrat_de_la_liste() {
    let scenario: Scenario =
        serde_json::from_str(include_str!("../../tests/contract/list_threads.json")).unwrap();
    let pool = crate::db::init_memory().await.unwrap();
    seed(&pool, &scenario).await;

    for q in &scenario.queries {
        let got: Vec<i64> = list_threads(&pool, &q.query)
            .await
            .unwrap()
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(got, q.ids, "{}", q.name);
    }

    let all = list_threads(&pool, &ThreadQuery::default()).await.unwrap();
    for (id, want) in &scenario.summaries {
        let t = all
            .iter()
            .find(|t| t.id == *id)
            .unwrap_or_else(|| panic!("conversation {id}"));
        let got = serde_json::to_value(t).unwrap();
        for (field, value) in want {
            assert_eq!(&got[field], value, "conversation {id}, champ {field}");
        }
    }
}

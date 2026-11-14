//! Base de test volumineuse + mesure des requêtes (mémoire et temps), sans toucher à la vraie base.
//!
//!   cargo run --release --example seed_local -- <fichier.db> <nombre de messages>
//!
//! Lancer ensuite l'application avec `HOME` pointant vers un dossier dont
//! `Library/Application Support/dev.envelope.app/envelope.db` est ce fichier.

use std::path::PathBuf;
use std::time::Instant;

use envelope_core::role::FolderRole;
use envelope_core::thread::{list_threads, ThreadQuery};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(db), Some(n)) = (
        args.next().map(PathBuf::from),
        args.next().and_then(|n| n.parse::<usize>().ok()),
    ) else {
        eprintln!("usage : seed_local <fichier.db> <nombre>");
        std::process::exit(2);
    };
    let t = Instant::now();
    if let Err(e) = envelope_lib::dev::seed_local(&db, n).await {
        eprintln!("échec : {e}");
        std::process::exit(1);
    }
    println!(
        "{n} messages générés en {:.1} s ({} Mo)",
        t.elapsed().as_secs_f32(),
        std::fs::metadata(&db).map_or(0, |m| m.len()) / 1_000_000
    );

    let pool = envelope_lib::dev::open_pool(&db).await.expect("base");
    async fn time<F: std::future::Future<Output = usize>>(label: &str, f: impl Fn() -> F) {
        let mut best = f64::MAX;
        let mut rows = 0;
        for _ in 0..5 {
            let t = Instant::now();
            rows = f().await;
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        println!("{label:<46} {best:>8.1} ms  ({rows} lignes)");
    }
    let q = |query: Option<&str>, role: Option<FolderRole>, offset: i64| ThreadQuery {
        role,
        query: query.map(Into::into),
        offset: Some(offset),
        limit: Some(100),
        ..Default::default()
    };
    time("boîte de réception, page 1", || async {
        list_threads(&pool, &q(None, Some(FolderRole::Inbox), 0))
            .await
            .unwrap()
            .len()
    })
    .await;
    time("boîte de réception, page 20", || async {
        list_threads(&pool, &q(None, Some(FolderRole::Inbox), 2000))
            .await
            .unwrap()
            .len()
    })
    .await;
    time("recherche « budget »", || async {
        list_threads(&pool, &q(Some("budget"), None, 0))
            .await
            .unwrap()
            .len()
    })
    .await;
    time("recherche « budget projet client »", || async {
        list_threads(&pool, &q(Some("budget projet client"), None, 0))
            .await
            .unwrap()
            .len()
    })
    .await;
    time("recherche de:alice", || async {
        list_threads(&pool, &q(Some("de:alice"), None, 0))
            .await
            .unwrap()
            .len()
    })
    .await;
    time("recherche de:alice avant:2026-01 non-lu", || async {
        list_threads(&pool, &q(Some("de:alice avant:2026-01 non-lu"), None, 0))
            .await
            .unwrap()
            .len()
    })
    .await;
    time("recherche objet:facture apres:2025", || async {
        list_threads(&pool, &q(Some("objet:facture apres:2025"), None, 0))
            .await
            .unwrap()
            .len()
    })
    .await;
    time("compteurs de dossiers", || async {
        envelope_core::search::list_folders(&pool, None)
            .await
            .unwrap()
            .len()
    })
    .await;
    let id: i64 = sqlx::query_scalar("SELECT id FROM threads ORDER BY message_count DESC LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    time("ouverture de la plus grosse conversation", || async {
        envelope_core::thread::get_thread_detail(&pool, id)
            .await
            .unwrap()
            .map_or(0, |d| d.messages.len())
    })
    .await;
}

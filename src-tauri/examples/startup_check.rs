//! Temps de démarrage du moteur sur une base existante (ouverture, migrations, première liste, compteurs).
//!
//!   CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true cargo run --release --example startup_check -- <fichier.db>
//!
//! (Le module `dev` n'existe pas dans un build release ordinaire.) À lancer sur une COPIE de la base.

use std::path::PathBuf;
use std::time::Instant;

use envelope_core::thread::{list_threads, ThreadQuery};

#[tokio::main]
async fn main() {
    let Some(db) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage : startup_check <fichier.db>");
        std::process::exit(2);
    };
    let t = Instant::now();
    let pool = envelope_lib::dev::open_pool(&db).await.expect("base");
    println!(
        "ouverture + migrations           {:>8.1} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );

    let t = Instant::now();
    let rows = list_threads(
        &pool,
        &ThreadQuery {
            role: Some("inbox".into()),
            limit: Some(100),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    println!(
        "première liste (100 conversations) {:>6.1} ms ({} lignes)",
        t.elapsed().as_secs_f64() * 1000.0,
        rows.len()
    );

    let t = Instant::now();
    let folders = envelope_core::search::list_folders(&pool, None)
        .await
        .unwrap();
    println!(
        "dossiers + compteurs             {:>8.1} ms ({} dossiers)",
        t.elapsed().as_secs_f64() * 1000.0,
        folders.len()
    );
}

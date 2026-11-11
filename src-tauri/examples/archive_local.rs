//! Essai de l'archivage en local sur un vrai compte : archive la conversation de la boîte de
//! réception dont l'objet contient le texte donné (elle est supprimée du serveur !).
//!
//!   cargo run --example archive_local -- "<texte de l'objet>" [dossier de données]

use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(subject) = args.next() else {
        eprintln!("usage : archive_local <texte de l'objet> [dossier de données]");
        std::process::exit(2);
    };
    let dir = args.next().map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join("Library/Application Support/dev.envelope.app")
    });
    if std::env::var("COUNT_ONLY").is_ok() {
        match envelope_lib::dev::server_inbox_count(&dir, &subject).await {
            Ok(n) => println!("{n} message(s) « {subject} » dans INBOX sur le serveur"),
            Err(e) => eprintln!("échec : {e}"),
        }
        return;
    }
    match envelope_lib::dev::archive_local_by_subject(&dir, &subject).await {
        Ok(title) => println!("archivé en local : « {title} »"),
        Err(e) => {
            eprintln!("échec : {e}");
            std::process::exit(1);
        }
    }
}

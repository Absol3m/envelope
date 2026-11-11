//! Remplit la boîte de réception d'un compte configuré avec des messages d'essai.
//!
//!   cargo run --example seed_mailbox -- vous@example.com [chemin/envelope.db]
//!
//! Outil de développement : lit le mot de passe dans le trousseau (macOS demandera l'autorisation).

use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(email) = args.next() else {
        eprintln!("usage : seed_mailbox <adresse du compte> [envelope.db]");
        std::process::exit(2);
    };
    let db = args.next().map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join("Library/Application Support/dev.envelope.app/envelope.db")
    });
    match envelope_lib::dev::seed_inbox(&db, &email).await {
        Ok(n) => println!("{n} messages déposés dans la boîte de réception de {email}."),
        Err(e) => {
            eprintln!("échec : {e}");
            std::process::exit(1);
        }
    }
}

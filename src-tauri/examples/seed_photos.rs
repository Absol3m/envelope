//! Dépose une conversation avec des images jointes dans la boîte de réception d'un compte fictif, pour juger l'affichage des pièces jointes.
//!
//!   cargo run --example seed_photos -- <dossier de données (dev.envelope.app)> <dossier des images>

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1).map(std::path::PathBuf::from);
    let (Some(data), Some(images)) = (args.next(), args.next()) else {
        eprintln!("usage : seed_photos <dossier de données> <dossier des images>");
        std::process::exit(2);
    };
    match envelope_lib::dev::seed_photo_thread(&data, &images).await {
        Ok(n) => {
            println!("{n} messages déposés dans la boîte de réception de alex.morel@example.com")
        }
        Err(e) => {
            eprintln!("échec : {e}");
            std::process::exit(1);
        }
    }
}

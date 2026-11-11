//! Contrôle d'import : lit un profil Thunderbird (ou un dossier de boîtes) et importe dans une base
//! temporaire. Ne touche pas à la vraie base de l'application.
//!
//!   cargo run --example import_check -- <dossier du profil>

#[tokio::main]
async fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage : import_check <dossier>");
        std::process::exit(2);
    };
    match envelope_lib::dev::import_check(std::path::Path::new(&path)).await {
        Ok(text) => print!("{text}"),
        Err(e) => {
            eprintln!("échec : {e}");
            std::process::exit(1);
        }
    }
}

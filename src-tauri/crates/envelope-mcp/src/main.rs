//! `envelope-mcp [--db CHEMIN]` : serveur MCP sur stdio. stdout ne porte que le protocole.

use envelope_mcp::db;
use envelope_mcp::tools::EnvelopeTools;
use rmcp::{transport::stdio, ServiceExt};

const USAGE: &str = "usage: envelope-mcp [--db CHEMIN] [--allow-locked]\n  CHEMIN par défaut : $ENVELOPE_DB, sinon le dossier de données de l'application.\n  --allow-locked : démarrer même si le verrouillage d'Envelope est actif (ce serveur lit la base sans passer par l'application : il ne respecte pas le verrou).";

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let mut allow_locked = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--db" => path = args.next().map(Into::into),
            "--allow-locked" => allow_locked = true,
            "-h" | "--help" => return println!("{USAGE}"),
            other => return fail(&format!("option inconnue `{other}`\n{USAGE}")),
        }
    }
    let Some(path) = path.or_else(db::default_path) else {
        return fail("dossier de données introuvable ; utilisez --db");
    };
    let pool = match db::open_readonly(&path).await {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    if !allow_locked && db::lock_enabled(&pool).await {
        return fail("le verrouillage d'Envelope est actif et ce serveur le contournerait (il lit la base directement) : désactivez le verrouillage, ou relancez avec --allow-locked en connaissance de cause");
    }
    eprintln!("envelope-mcp : base {} (lecture seule)", path.display());

    // Le SDK porte le protocole (JSON-RPC, cycle de vie, versions) ; on ne fournit que les outils.
    match EnvelopeTools::new(pool).serve(stdio()).await {
        Ok(service) => {
            if let Err(e) = service.waiting().await {
                fail(&format!("protocole : {e}"));
            }
        }
        Err(e) => fail(&format!("démarrage : {e}")),
    }
}

fn fail(msg: &str) {
    eprintln!("envelope-mcp : {msg}");
    std::process::exit(1);
}

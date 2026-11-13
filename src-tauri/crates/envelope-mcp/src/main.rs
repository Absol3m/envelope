//! `envelope-mcp [--db PATH]` : serveur MCP sur stdio. stdout ne porte que le protocole.

use envelope_mcp::db;
use envelope_mcp::tools::EnvelopeTools;
use rmcp::{transport::stdio, ServiceExt};

const USAGE: &str = "usage: envelope-mcp [--db PATH] [--allow-locked]\n  PATH defaults to $ENVELOPE_DB, otherwise the app's data folder.\n  --allow-locked: start even if Envelope's app lock is on (this server reads the database without going through the app, so it does not honor the lock).";

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
            other => return fail(&format!("unknown option `{other}`\n{USAGE}")),
        }
    }
    let Some(path) = path.or_else(db::default_path) else {
        return fail("data folder not found; use --db");
    };
    let pool = match db::open_readonly(&path).await {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    if !allow_locked && db::lock_enabled(&pool).await {
        return fail("Envelope's app lock is on and this server would bypass it (it reads the database directly): turn the lock off, or restart with --allow-locked if you accept that");
    }
    eprintln!("envelope-mcp: database {} (read-only)", path.display());

    // Le SDK porte le protocole (JSON-RPC, cycle de vie, versions) ; on ne fournit que les outils.
    match EnvelopeTools::new(pool).serve(stdio()).await {
        Ok(service) => {
            if let Err(e) = service.waiting().await {
                fail(&format!("protocol: {e}"));
            }
        }
        Err(e) => fail(&format!("startup: {e}")),
    }
}

fn fail(msg: &str) {
    eprintln!("envelope-mcp: {msg}");
    std::process::exit(1);
}

//! `envelope-mcp [--db PATH]` : serveur MCP sur stdio. stdout ne porte que le protocole.

use envelope_mcp::db;
use envelope_mcp::tools::EnvelopeTools;
use rmcp::{transport::stdio, ServiceExt};

const USAGE: &str = "usage: envelope-mcp [--db PATH] [--account ID_OR_EMAIL] [--allow-locked]\n  PATH defaults to $ENVELOPE_DB, otherwise the app's data folder.\n  --account: expose only this account (its id or address); the other accounts stay invisible to the agent.\n  --allow-locked: start even if Envelope's app lock is on (this server reads the database without going through the app, so it does not honor the lock).";

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let mut allow_locked = false;
    let mut account = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--db" => path = args.next().map(Into::into),
            "--account" => account = args.next(),
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
    let mut tools = EnvelopeTools::new(pool.clone());
    if let Some(key) = account {
        let Some(id) = db::resolve_account(&pool, &key).await else {
            return fail(&format!(
                "no account `{key}` in this database (id or address)"
            ));
        };
        eprintln!("envelope-mcp: restricted to account {id}");
        tools = tools.scoped_to(id);
    }
    eprintln!("envelope-mcp: database {} (read-only)", path.display());

    // Le SDK porte le protocole (JSON-RPC, cycle de vie, versions) ; on ne fournit que les outils.
    match tools.serve(stdio()).await {
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

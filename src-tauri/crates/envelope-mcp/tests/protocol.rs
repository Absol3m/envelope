//! Tests de bout en bout : une vraie base SQLite (migrations de l'application), le serveur MCP
//! ouvert en lecture seule, et un **vrai client MCP** (celui du SDK officiel) relié en mémoire.
//! Le protocole (JSON-RPC, cycle de vie, validation des paramètres) est celui du SDK ; ici on
//! vérifie nos outils et nos garanties : lecture seule, contenu non fiable signalé, sorties compactes.

use envelope_mcp::db::open_readonly;
use envelope_mcp::tools::EnvelopeTools;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::{RoleClient, RunningService};
use rmcp::{ClientHandler, ServiceExt};
use serde_json::{json, Value};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn temp_db() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "envelope-mcp-test-{}-{}.db",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Base de test : 2 conversations, dont une avec pièce jointe et citations.
async fn database() -> PathBuf {
    let path = temp_db();
    let rw = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::migrate!("../../migrations").run(&rw).await.unwrap();
    for sql in [
        // Le fixture fixe ses identifiants : on écarte le compte local créé par la migration.
        "DELETE FROM accounts WHERE id = 'local'",
        "INSERT INTO accounts (id,email,username,imap_host,smtp_host,provider) VALUES ('a','me@x.fr','me','i','s','imap')",
        "INSERT INTO folders (id,account_id,path,name,role,unread_count,total_count) VALUES (1,'a','INBOX','INBOX','inbox',2,3)",
        "INSERT INTO threads (id,account_id,subject,last_message_at) VALUES (1,'a','Budget trimestriel','2026-03-02T11:00:00Z'), (2,'a','Facture','2026-03-03T09:00:00Z')",
        "INSERT INTO messages (id,account_id,folder_id,thread_id,uid,message_id,subject,from_name,from_addr,to_addrs,date,snippet,body_text,is_read)
           VALUES (1,'a',1,1,1,'m1','Budget trimestriel','Alice','alice@x.fr','[{\"name\":null,\"address\":\"me@x.fr\"}]','2026-03-02T10:00:00Z','Peut-on parler du budget ?','Peut-on parler du budget ?',1)",
        "INSERT INTO messages (id,account_id,folder_id,thread_id,uid,message_id,in_reply_to,subject,from_addr,to_addrs,date,snippet,body_text,is_read)
           VALUES (2,'a',1,1,2,'m2','m1','Re: Budget trimestriel','me@x.fr','[{\"name\":\"Alice\",\"address\":\"alice@x.fr\"}]','2026-03-02T11:00:00Z','Oui, demain.','Oui, demain.\n\n> Peut-on parler du budget ?\n> Merci\n\nÀ 10h ?',0)",
        "INSERT INTO messages (id,account_id,folder_id,thread_id,uid,message_id,subject,from_addr,to_addrs,date,snippet,body_text,is_read,has_attachments)
           VALUES (3,'a',1,2,3,'m3','Facture','carol@x.fr','[]','2026-03-03T09:00:00Z','Voir pièce jointe.','Voir pièce jointe.',0,1)",
        "INSERT INTO messages (id,account_id,folder_id,thread_id,uid,message_id,subject,from_addr,to_addrs,date,snippet,body_text,is_read)
           VALUES (4,'a',1,2,4,'m4','Facture','carol@x.fr','[]','2026-03-03T10:00:00Z','long',replace(hex(zeroblob(1500)),'0','é'),1)",
        "INSERT INTO attachments (message_id,filename,mime_type,size,storage_path) VALUES (3,'facture.pdf','application/pdf',5,'/secret/chemin/facture.pdf')",
    ] {
        sqlx::query(sql).execute(&rw).await.unwrap();
    }
    rw.close().await;
    path
}

#[derive(Clone, Default)]
struct TestClient;
impl ClientHandler for TestClient {}

type Client = RunningService<RoleClient, TestClient>;

/// Serveur et client reliés en mémoire ; le serveur ouvre la base en lecture seule, comme en production.
async fn connect() -> (Client, PathBuf) {
    let path = database().await;
    let ro = open_readonly(&path).await.unwrap();
    let (server_io, client_io) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        let running = EnvelopeTools::new(ro).serve(server_io).await.unwrap();
        let _ = running.waiting().await;
    });
    (TestClient.serve(client_io).await.unwrap(), path)
}

fn args(v: Value) -> serde_json::Map<String, Value> {
    v.as_object().cloned().unwrap()
}

/// Appelle un outil : le résultat et le texte de tous ses blocs.
async fn call(c: &Client, name: &str, arguments: Value) -> (CallToolResult, String) {
    let r = c
        .call_tool(CallToolRequestParams::new(name.to_string()).with_arguments(args(arguments)))
        .await
        .unwrap_or_else(|e| panic!("{name} : {e}"));
    let text = r
        .content
        .iter()
        .filter_map(|b| b.as_text())
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (r, text)
}

fn block_text(r: &CallToolResult, i: usize) -> String {
    r.content[i].as_text().unwrap().text.clone()
}

#[tokio::test]
async fn initialize_and_tool_list() {
    let (c, _) = connect().await;
    let info = c.peer_info().expect("informations du serveur");
    assert_eq!(
        info.server_info.as_ref().map(|i| i.name.as_str()),
        Some("envelope")
    );
    assert!(info.instructions.as_deref().unwrap().contains("untrusted"));
    let tools = c.list_all_tools().await.unwrap();
    let mut names: Vec<&str> = tools.iter().map(|t| &*t.name).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "get_thread",
            "list_accounts",
            "list_folders",
            "search_emails",
            "summarize_thread"
        ]
    );
    // Temps 1 : rien que de la lecture, et les clients doivent le savoir.
    for t in &tools {
        let a = t
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("annotations de {}", t.name));
        assert_eq!(a.read_only_hint, Some(true), "{}", t.name);
        assert_eq!(a.destructive_hint, Some(false), "{}", t.name);
    }
}

#[tokio::test]
async fn search_is_compact_and_flags_untrusted_content() {
    let (c, _) = connect().await;
    let (r, text) = call(&c, "search_emails", json!({"query": "budg"})).await;
    assert_ne!(r.is_error, Some(true));
    assert!(
        block_text(&r, 0).contains("untrusted"),
        "l'avertissement précède les données"
    );
    let data: Value = serde_json::from_str(&block_text(&r, 1)).unwrap();
    assert_eq!(data["count"], 2);
    let first = &data["results"][0];
    assert!(first["thread_id"].is_number() && first["snippet"].is_string());
    // Valeurs par défaut omises : un message lu sans pièce jointe n'a ni `unread` ni `attachments`.
    let read_msg = data["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == 1)
        .unwrap();
    assert!(read_msg.get("unread").is_none() && read_msg.get("attachments").is_none());
    assert!(
        !text.contains("body"),
        "la recherche ne renvoie pas de corps"
    );

    let (_, none) = call(
        &c,
        "search_emails",
        json!({"query": "\"; DROP TABLE messages; --"}),
    )
    .await;
    assert!(none.contains("\"count\":0"));
    let (_, scoped) = call(
        &c,
        "search_emails",
        json!({"query": "budget", "account_id": "autre"}),
    )
    .await;
    assert!(scoped.contains("\"count\":0"));
}

#[tokio::test]
async fn get_thread_truncates_and_hides_attachment_paths() {
    let (c, _) = connect().await;
    let (_, text) = call(&c, "get_thread", json!({"thread_id": 2})).await;
    assert!(text.contains("facture.pdf") && text.contains("application/pdf"));
    assert!(!text.contains("/secret"), "jamais de chemin disque");

    let (_, short) = call(&c, "get_thread", json!({"thread_id": 1, "max_chars": 100})).await;
    assert!(short.contains("Peut-on parler"));
    // Corps de 3000 caractères (accentués : octets ≠ caractères) : borné à 2000 par défaut, signalé tronqué.
    let (r, _) = call(&c, "get_thread", json!({"thread_id": 2})).await;
    let data: Value = serde_json::from_str(&block_text(&r, 1)).unwrap();
    let find = |id: i64| {
        data["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == id)
            .unwrap()
            .clone()
    };
    let long = find(4);
    assert_eq!(long["truncated"], true);
    assert_eq!(
        long["body"].as_str().unwrap().chars().count(),
        2001,
        "2000 caractères + …"
    );
    let short = find(3);
    assert!(
        short.get("truncated").is_none() && short.get("to").is_none(),
        "valeurs vides omises"
    );

    let (r, _) = call(&c, "get_thread", json!({"thread_id": 999})).await;
    assert_eq!(r.is_error, Some(true));
}

#[tokio::test]
async fn summarize_strips_quotes_and_orders_chronologically() {
    let (c, _) = connect().await;
    let (_, text) = call(&c, "summarize_thread", json!({"thread_id": 1})).await;
    assert!(text.contains("2 message(s)"));
    assert!(text.contains("À 10h ?") && text.contains("Oui, demain."));
    assert!(
        !text.contains("> Peut-on") && !text.contains("Merci"),
        "citations retirées"
    );
    assert!(
        text.find("alice@x.fr").unwrap() < text.find("me@x.fr").unwrap(),
        "du plus ancien au plus récent"
    );
}

#[tokio::test]
async fn accounts_and_folders() {
    let (c, _) = connect().await;
    let (_, acc) = call(&c, "list_accounts", json!({})).await;
    assert!(acc.contains("me@x.fr") && !acc.contains("imap_host") && !acc.contains("imapHost"));
    let (_, f) = call(&c, "list_folders", json!({})).await;
    assert!(f.contains("INBOX") && f.contains("\"unreadCount\":2"));
}

/// Les paramètres invalides sont refusés par le SDK (schéma de l'outil) avant d'atteindre notre code.
#[tokio::test]
async fn invalid_calls_never_reach_a_tool() {
    let (c, _) = connect().await;
    for (name, arguments) in [
        ("nope", json!({})),
        ("search_emails", json!({})), // `query` manquant
        ("search_emails", json!({"query": "x", "bogus": 1})), // argument inconnu
        ("search_emails", json!({"query": 5})), // mauvais type
        ("get_thread", json!({"thread_id": "1"})), // mauvais type
    ] {
        let result = c
            .call_tool(
                CallToolRequestParams::new(name.to_string())
                    .with_arguments(args(arguments.clone())),
            )
            .await;
        let refused = match result {
            Err(_) => true,                    // erreur de protocole
            Ok(r) => r.is_error == Some(true), // ou résultat d'erreur de l'outil
        };
        assert!(refused, "{name} {arguments} aurait dû être refusé");
    }
    // Le serveur reste utilisable après des appels invalides.
    assert!(call(&c, "list_accounts", json!({}))
        .await
        .1
        .contains("me@x.fr"));
}

/// Aucun outil n'écrit : même avec des arguments hostiles, la base ne change pas (ouverte en lecture seule).
#[tokio::test]
async fn hostile_arguments_never_write() {
    let (c, path) = connect().await;
    for q in [
        "'; DELETE FROM messages; --",
        "\"\" OR 1=1",
        "NEAR(a b)",
        "a:b",
        "*",
        &"x".repeat(5000),
    ] {
        let _ = call(&c, "search_emails", json!({"query": q})).await;
    }
    let ro = open_readonly(&path).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&ro)
        .await
        .unwrap();
    assert_eq!(n, 4);
}

#[tokio::test]
async fn database_is_read_only() {
    let path = database().await;
    let ro = open_readonly(&path).await.unwrap();
    let err = sqlx::query("DELETE FROM messages")
        .execute(&ro)
        .await
        .unwrap_err();
    assert!(err.to_string().to_lowercase().contains("readonly"), "{err}");
    assert!(open_readonly(&path.with_extension("absente"))
        .await
        .unwrap_err()
        .contains("not found"));
}

//! Tests de bout en bout : une vraie base SQLite (migrations de l'application), le serveur MCP
//! ouvert en lecture seule, et des messages JSON-RPC comme un client les enverrait.

use envelope_mcp::db::open_readonly;
use envelope_mcp::protocol::{Info, Server};
use envelope_mcp::tools::EnvelopeTools;
use serde_json::{json, Value};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

const INFO: Info = Info {
    name: "envelope",
    title: "Envelope",
    version: "test",
    max_line: 4096,
};

fn temp_db() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "envelope-mcp-test-{}-{}.db",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Base de test : 2 conversations, dont une avec pièce jointe et citations.
async fn fixture() -> (Server<EnvelopeTools>, PathBuf) {
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
    let ro = open_readonly(&path).await.unwrap();
    (Server::new(INFO, EnvelopeTools::new(ro)), path)
}

async fn rpc(s: &mut Server<EnvelopeTools>, msg: Value) -> Value {
    serde_json::from_str(
        &s.handle_line(&msg.to_string())
            .await
            .expect("réponse attendue"),
    )
    .unwrap()
}

/// Appelle un outil et renvoie (résultat, texte des blocs concaténés).
async fn call(s: &mut Server<EnvelopeTools>, name: &str, args: Value) -> (Value, String) {
    let r = rpc(s, json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})).await;
    let text = r["result"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (r, text)
}

#[tokio::test]
async fn initialize_and_tool_list() {
    let (mut s, _) = fixture().await;
    let r = rpc(&mut s, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}})).await;
    assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
    assert!(r["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("pas fiable"));
    assert!(s
        .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .await
        .is_none());

    let tools = rpc(
        &mut s,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await["result"]["tools"]
        .clone();
    let tools = tools.as_array().unwrap();
    assert_eq!(tools.len(), 5);
    // Temps 1 : rien que de la lecture, et les clients doivent le savoir.
    for t in tools {
        assert_eq!(t["annotations"]["readOnlyHint"], true, "{}", t["name"]);
        assert_eq!(t["annotations"]["destructiveHint"], false);
    }
}

#[tokio::test]
async fn search_is_compact_and_flags_untrusted_content() {
    let (mut s, _) = fixture().await;
    let (r, text) = call(&mut s, "search_emails", json!({"query": "budg"})).await;
    assert!(r["result"]["isError"].is_null());
    let blocks = r["result"]["content"].as_array().unwrap();
    assert!(
        blocks[0]["text"].as_str().unwrap().contains("non fiable"),
        "l'avertissement précède les données"
    );
    let data: Value = serde_json::from_str(blocks[1]["text"].as_str().unwrap()).unwrap();
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
        &mut s,
        "search_emails",
        json!({"query": "\"; DROP TABLE messages; --"}),
    )
    .await;
    assert!(none.contains("\"count\":0"));
    let (_, scoped) = call(
        &mut s,
        "search_emails",
        json!({"query": "budget", "account_id": "autre"}),
    )
    .await;
    assert!(scoped.contains("\"count\":0"));
}

#[tokio::test]
async fn get_thread_truncates_and_hides_attachment_paths() {
    let (mut s, _) = fixture().await;
    let (_, text) = call(&mut s, "get_thread", json!({"thread_id": 2})).await;
    assert!(text.contains("facture.pdf") && text.contains("application/pdf"));
    assert!(!text.contains("/secret"), "jamais de chemin disque");

    let (_, short) = call(
        &mut s,
        "get_thread",
        json!({"thread_id": 1, "max_chars": 100}),
    )
    .await;
    assert!(short.contains("Peut-on parler"));
    // Corps de 3000 caractères (accentués : octets ≠ caractères) : borné à 2000 par défaut, signalé tronqué.
    let (r, _) = call(&mut s, "get_thread", json!({"thread_id": 2})).await;
    let data: Value =
        serde_json::from_str(r["result"]["content"][1]["text"].as_str().unwrap()).unwrap();
    let long = data["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == 4)
        .unwrap();
    assert_eq!(long["truncated"], true);
    assert_eq!(
        long["body"].as_str().unwrap().chars().count(),
        2001,
        "2000 caractères + …"
    );
    let short = data["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == 3)
        .unwrap();
    assert!(
        short.get("truncated").is_none() && short.get("to").is_none(),
        "valeurs vides omises"
    );

    let (r, _) = call(&mut s, "get_thread", json!({"thread_id": 999})).await;
    assert_eq!(r["result"]["isError"], true);
}

#[tokio::test]
async fn summarize_strips_quotes_and_orders_chronologically() {
    let (mut s, _) = fixture().await;
    let (_, text) = call(&mut s, "summarize_thread", json!({"thread_id": 1})).await;
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
    let (mut s, _) = fixture().await;
    let (_, acc) = call(&mut s, "list_accounts", json!({})).await;
    assert!(acc.contains("me@x.fr") && !acc.contains("imap_host") && !acc.contains("imapHost"));
    let (_, f) = call(&mut s, "list_folders", json!({})).await;
    assert!(f.contains("INBOX") && f.contains("\"unreadCount\":2"));
}

#[tokio::test]
async fn invalid_calls_never_reach_a_tool() {
    let (mut s, _) = fixture().await;
    for (name, args, needle) in [
        ("nope", json!({}), "unknown tool"),
        (
            "search_emails",
            json!({}),
            "missing required argument `query`",
        ),
        (
            "search_emails",
            json!({"query": "x", "bogus": 1}),
            "unknown argument `bogus`",
        ),
        ("search_emails", json!({"query": 5}), "must be of type"),
        (
            "search_emails",
            json!({"query": "x", "limit": 1000}),
            "must be between",
        ),
        ("get_thread", json!({"thread_id": "1"}), "must be of type"),
    ] {
        let (r, _) = call(&mut s, name, args).await;
        assert_eq!(r["error"]["code"], -32602, "{name}");
        assert!(
            r["error"]["message"].as_str().unwrap().contains(needle),
            "{name}: {}",
            r["error"]["message"]
        );
    }
    let r = rpc(
        &mut s,
        json!({"jsonrpc":"2.0","id":3,"method":"resources/list"}),
    )
    .await;
    assert_eq!(r["error"]["code"], -32601);
}

#[tokio::test]
async fn hostile_input_is_answered_not_fatal() {
    let (mut s, _) = fixture().await;
    // Ligne géante, JSON invalide, UTF-8 cassé, lot interdit, puis une requête valide : le serveur survit.
    let mut input = vec![b'x'; 10_000];
    input.push(b'\n');
    input.extend_from_slice(b"{pas du json\n");
    input.extend_from_slice(&[0xff, 0xfe, b'\n']);
    input.extend_from_slice(b"[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}]\n");
    input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}\n");
    let mut out = Vec::new();
    s.serve(Cursor::new(input), &mut out).await.unwrap();
    let lines: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let codes: Vec<i64> = lines
        .iter()
        .filter_map(|l| l["error"]["code"].as_i64())
        .collect();
    assert_eq!(codes, vec![-32600, -32700, -32700, -32600]);
    assert_eq!(lines.last().unwrap()["id"], 9);
    assert!(lines.last().unwrap()["result"].is_object());
}

#[tokio::test]
async fn database_is_read_only() {
    let (_, path) = fixture().await;
    let ro = open_readonly(&path).await.unwrap();
    let err = sqlx::query("DELETE FROM messages")
        .execute(&ro)
        .await
        .unwrap_err();
    assert!(err.to_string().to_lowercase().contains("readonly"), "{err}");
    assert!(open_readonly(&path.with_extension("absente"))
        .await
        .unwrap_err()
        .contains("introuvable"));
}

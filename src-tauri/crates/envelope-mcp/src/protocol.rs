//! JSON-RPC 2.0 sur stdio (un message par ligne) et cycle de vie MCP.
//! Générique : ce module ne connaît ni la base ni le mail.

use std::future::Future;
use std::io;

use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

use crate::schema::check_arguments;

pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const SUPPORTED_VERSIONS: &[&str] = &[PROTOCOL_VERSION, "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Les outils d'une application, servis par [`Server`].
pub trait Tools {
    /// Ce que tout client reçoit à `initialize`.
    fn instructions(&self) -> String;
    /// Définitions des outils (voir [`crate::schema::tool`]).
    fn list(&self) -> Vec<Value>;
    /// Exécute l'outil `name`. Appelé après validation des arguments contre [`Tools::list`].
    /// Les échecs sont des résultats `isError: true`, jamais des panics.
    fn call(&self, name: &str, args: &Value) -> impl Future<Output = Value>;
}

#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub name: &'static str,
    pub title: &'static str,
    pub version: &'static str,
    /// Plus longue ligne de requête acceptée, en octets.
    pub max_line: usize,
}

pub struct Server<T: Tools> {
    info: Info,
    tools: T,
    initialized: bool,
}

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}
fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

enum Line {
    Eof,
    Text(String),
    Oversized,
    NotUtf8,
}

/// Lit une ligne en espace borné : une requête géante sans retour à la ligne ne peut pas
/// épuiser la mémoire ; le reste de la ligne est consommé puis ignoré.
async fn read_line<R: AsyncBufRead + Unpin>(input: &mut R, max: usize) -> io::Result<Line> {
    let mut line = Vec::new();
    let (mut oversized, mut received) = (false, false);
    loop {
        let chunk = input.fill_buf().await?;
        if chunk.is_empty() {
            break;
        }
        let count = chunk
            .iter()
            .position(|&b| b == b'\n')
            .map_or(chunk.len(), |i| i + 1);
        let newline = chunk[count - 1] == b'\n';
        let content = count - usize::from(newline);
        if !oversized {
            if line.len().saturating_add(content) > max {
                oversized = true;
                line.clear();
            } else {
                line.extend_from_slice(&chunk[..content]);
            }
        }
        input.consume(count);
        received = true;
        if newline {
            break;
        }
    }
    Ok(match (received, oversized) {
        (false, _) => Line::Eof,
        (_, true) => Line::Oversized,
        _ => String::from_utf8(line).map_or(Line::NotUtf8, Line::Text),
    })
}

impl<T: Tools> Server<T> {
    pub fn new(info: Info, tools: T) -> Self {
        Self {
            info,
            tools,
            initialized: false,
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Sert jusqu'à la fermeture de l'entrée. Seuls des messages de protocole vont sur `output` ;
    /// les diagnostics vont sur stderr.
    pub async fn serve<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin>(
        &mut self,
        mut input: R,
        mut output: W,
    ) -> io::Result<()> {
        loop {
            let reply = match read_line(&mut input, self.info.max_line).await? {
                Line::Eof => return Ok(()),
                Line::Oversized => {
                    Some(error(Value::Null, INVALID_REQUEST, "request too large").to_string())
                }
                Line::NotUtf8 => Some(
                    error(Value::Null, PARSE_ERROR, "parse error: input is not UTF-8").to_string(),
                ),
                Line::Text(line) => self.handle_line(&line).await,
            };
            if let Some(reply) = reply {
                output.write_all(reply.as_bytes()).await?;
                output.write_all(b"\n").await?;
                output.flush().await?;
            }
        }
    }

    /// Une ligne en entrée, au plus une ligne en sortie (les notifications n'ont pas de réponse).
    pub async fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(_)) => Some(error(
                Value::Null,
                INVALID_REQUEST,
                "JSON-RPC batches are not supported",
            )),
            Ok(msg) => self.handle(msg).await,
            Err(e) => Some(error(Value::Null, PARSE_ERROR, format!("parse error: {e}"))),
        };
        reply.map(|r| r.to_string())
    }

    async fn handle(&mut self, msg: Value) -> Option<Value> {
        let Value::Object(o) = &msg else {
            return Some(error(
                Value::Null,
                INVALID_REQUEST,
                "message must be an object",
            ));
        };
        let id = o.get("id").cloned();
        let Some(method) = o.get("method").and_then(Value::as_str) else {
            // Réponse à une requête de notre part (nous n'en envoyons pas) ou bruit.
            return (!o.contains_key("result") && !o.contains_key("error")).then(|| {
                error(
                    id.unwrap_or(Value::Null),
                    INVALID_REQUEST,
                    "missing `method`",
                )
            });
        };
        let params = o.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            if method == "notifications/initialized" {
                self.initialized = true;
            }
            return None;
        };
        Some(match self.request(method, &params).await {
            Ok(r) => response(id, r),
            Err((code, m)) => error(id, code, m),
        })
    }

    async fn request(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL_VERSION);
                let version = if SUPPORTED_VERSIONS.contains(&asked) {
                    asked
                } else {
                    PROTOCOL_VERSION
                };
                let Info {
                    name,
                    title,
                    version: server_version,
                    ..
                } = self.info;
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": name, "title": title, "version": server_version},
                    "instructions": self.tools.instructions(),
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": self.tools.list()})),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or((INVALID_PARAMS, "missing tool `name`".to_string()))?;
                let args = match params.get("arguments") {
                    None | Some(Value::Null) => json!({}),
                    Some(a) => a.clone(),
                };
                check_arguments(&self.tools.list(), name, &args)
                    .map_err(|e| (INVALID_PARAMS, e))?;
                Ok(self.tools.call(name, &args).await)
            }
            _ => Err((METHOD_NOT_FOUND, format!("method `{method}` not found"))),
        }
    }
}

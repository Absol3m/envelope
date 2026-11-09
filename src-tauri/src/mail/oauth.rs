//! OAuth2 (Authorization Code + PKCE S256, redirection loopback) pour Gmail et Outlook.
//!
//! Les identifiants d'application ne sont pas fournis avec le code : il faut enregistrer une
//! application « bureau » chez le fournisseur et les passer par variables d'environnement :
//!   `ENVELOPE_GOOGLE_CLIENT_ID` (+ `ENVELOPE_GOOGLE_CLIENT_SECRET` si requis par Google)
//!   `ENVELOPE_MICROSOFT_CLIENT_ID`

use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::error::{AppError, Result};
use crate::mail::account;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OAuthProvider {
    Google,
    Microsoft,
}

struct ProviderConfig {
    auth_url: &'static str,
    token_url: &'static str,
    scope: &'static str,
    extra_auth_params: &'static [(&'static str, &'static str)],
    client_id_env: &'static str,
    client_secret_env: Option<&'static str>,
}

impl OAuthProvider {
    /// Valeur stockée dans `accounts.provider`.
    pub fn as_str(self) -> &'static str {
        match self {
            OAuthProvider::Google => "gmail",
            OAuthProvider::Microsoft => "outlook",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "gmail" | "google" => Some(Self::Google),
            "outlook" | "microsoft" => Some(Self::Microsoft),
            _ => None,
        }
    }
    fn config(self) -> ProviderConfig {
        match self {
            Self::Google => ProviderConfig {
                auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
                token_url: "https://oauth2.googleapis.com/token",
                scope: "https://mail.google.com/",
                extra_auth_params: &[("access_type", "offline"), ("prompt", "consent")],
                client_id_env: "ENVELOPE_GOOGLE_CLIENT_ID",
                client_secret_env: Some("ENVELOPE_GOOGLE_CLIENT_SECRET"),
            },
            Self::Microsoft => ProviderConfig {
                auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
                token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
                scope: "https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send offline_access",
                extra_auth_params: &[("prompt", "select_account")],
                client_id_env: "ENVELOPE_MICROSOFT_CLIENT_ID",
                client_secret_env: None,
            },
        }
    }
    fn client_id(self) -> Result<String> {
        let env = self.config().client_id_env;
        std::env::var(env)
            .map_err(|_| AppError::Auth(format!("variable d'environnement {env} manquante")))
    }
    /// Champs d'identification d'application communs aux échanges de jetons.
    fn client_auth(self) -> Result<Vec<(&'static str, String)>> {
        let mut v = vec![("client_id", self.client_id()?)];
        if let Some(secret) = self
            .config()
            .client_secret_env
            .and_then(|e| std::env::var(e).ok())
        {
            v.push(("client_secret", secret));
        }
        Ok(v)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Timestamp UNIX (secondes) d'expiration du jeton d'accès.
    pub expires_at: i64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn random_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Lance le flux complet. `open_browser` reçoit l'URL d'autorisation à ouvrir.
pub async fn authorize(
    provider: OAuthProvider,
    open_browser: impl FnOnce(&str) -> Result<()>,
) -> Result<OAuthTokens> {
    let cfg = provider.config();
    let client_id = provider.client_id()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect_uri = format!("http://127.0.0.1:{}", listener.local_addr()?.port());

    let verifier = random_token(); // 64 caractères hex ∈ [43,128]
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = uuid::Uuid::new_v4().to_string();

    let mut params = vec![
        ("client_id", client_id.as_str()),
        ("response_type", "code"),
        ("redirect_uri", redirect_uri.as_str()),
        ("scope", cfg.scope),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ];
    params.extend_from_slice(cfg.extra_auth_params);
    let url = reqwest::Url::parse_with_params(cfg.auth_url, &params)
        .map_err(|e| AppError::Auth(e.to_string()))?;
    open_browser(url.as_str())?;

    let code = tokio::time::timeout(Duration::from_secs(300), wait_for_code(&listener, &state))
        .await
        .map_err(|_| {
            AppError::Auth("délai dépassé : autorisation non terminée dans le navigateur".into())
        })??;

    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", verifier),
    ];
    form.extend(provider.client_auth()?);
    let resp = token_request(cfg.token_url, &form).await?;
    Ok(OAuthTokens {
        access_token: resp.access_token,
        refresh_token: resp.refresh_token,
        expires_at: now() + resp.expires_in.unwrap_or(3600),
    })
}

/// Attend la redirection du navigateur et renvoie le `code` (après vérification du `state`).
async fn wait_for_code(listener: &TcpListener, expected_state: &str) -> Result<String> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await?;
        let req = String::from_utf8_lossy(&buf[..n]);
        let path = req
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("/");
        let url = reqwest::Url::parse(&format!("http://127.0.0.1{path}"))
            .map_err(|e| AppError::Auth(e.to_string()))?;
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();

        let (status, body, outcome) = if let Some(err) = q.get("error") {
            (
                "400 Bad Request",
                "Autorisation refusée. Vous pouvez fermer cet onglet.",
                Some(Err(AppError::Auth(err.clone()))),
            )
        } else if let Some(code) = q.get("code") {
            if q.get("state").map(String::as_str) != Some(expected_state) {
                (
                    "400 Bad Request",
                    "État invalide.",
                    Some(Err(AppError::Auth("state OAuth2 invalide (CSRF ?)".into()))),
                )
            } else {
                (
                    "200 OK",
                    "Envelope est autorisé. Vous pouvez fermer cet onglet.",
                    Some(Ok(code.clone())),
                )
            }
        } else {
            ("404 Not Found", "", None) // favicon, etc.
        };
        let html = format!("<!doctype html><meta charset=utf-8><body style=\"font-family:sans-serif;padding:3rem\">{body}");
        let _ = sock
            .write_all(format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len()).as_bytes())
            .await;
        if let Some(o) = outcome {
            return o;
        }
    }
}

/// Client HTTP partagé (pool de connexions + racines TLS chargés une seule fois).
fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

async fn token_request(url: &str, form: &[(&str, String)]) -> Result<TokenResponse> {
    let resp = http_client().post(url).form(form).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Auth(format!(
            "échange de jeton refusé ({status}) : {body}"
        )));
    }
    Ok(resp.json().await?)
}

fn refresh_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Jeton d'accès valide pour un compte : rafraîchit et persiste s'il expire dans < 60 s.
pub async fn fresh_access_token(provider: OAuthProvider, account_id: &str) -> Result<String> {
    let _guard = refresh_lock().lock().await; // évite deux rafraîchissements concurrents
    let tokens = account::load_oauth(account_id)?;
    if tokens.expires_at - 60 > now() {
        return Ok(tokens.access_token);
    }
    let refresh = tokens
        .refresh_token
        .ok_or_else(|| AppError::Auth("pas de refresh_token : reconnectez le compte".into()))?;
    let cfg = provider.config();
    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh.clone()),
        ("scope", cfg.scope.to_string()),
    ];
    form.extend(provider.client_auth()?);
    let resp = token_request(cfg.token_url, &form).await?;
    let new = OAuthTokens {
        access_token: resp.access_token,
        refresh_token: resp.refresh_token.or(Some(refresh)), // Google ne renvoie pas toujours le refresh_token
        expires_at: now() + resp.expires_in.unwrap_or(3600),
    };
    account::store_oauth(account_id, &new)?;
    Ok(new.access_token)
}

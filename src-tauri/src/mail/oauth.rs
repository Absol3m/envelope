//! OAuth2 (Authorization Code + PKCE S256, redirection loopback) pour Gmail et Outlook.
//!
//! Les identifiants d'application ne sont pas fournis avec le code : chaque utilisateur enregistre
//! une application « bureau » chez le fournisseur et la saisit dans l'interface (stockée dans le
//! trousseau de l'OS). Repli pour le développement : variables d'environnement
//!   `ENVELOPE_GOOGLE_CLIENT_ID` (+ `ENVELOPE_GOOGLE_CLIENT_SECRET`) et `ENVELOPE_MICROSOFT_CLIENT_ID`.

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
    pub fn label(self) -> &'static str {
        match self {
            Self::Google => "Google",
            Self::Microsoft => "Microsoft",
        }
    }
    /// Identifiants d'application : trousseau d'abord, puis variables d'environnement.
    pub fn client(self) -> Result<ClientCreds> {
        if let Some(c) = account::load_client(self)? {
            return Ok(c);
        }
        let cfg = self.config();
        let client_id = std::env::var(cfg.client_id_env).map_err(|_| {
            AppError::Auth(format!(
                "identifiant client {} non configuré (onglet Comptes)",
                self.label()
            ))
        })?;
        Ok(ClientCreds {
            client_id,
            client_secret: cfg.client_secret_env.and_then(|e| std::env::var(e).ok()),
        })
    }
    pub fn is_configured(self) -> bool {
        self.client().is_ok()
    }
}

/// Application OAuth2 « bureau » enregistrée par l'utilisateur chez le fournisseur.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCreds {
    pub client_id: String,
    pub client_secret: Option<String>,
}

impl ClientCreds {
    /// Champs d'identification d'application communs aux échanges de jetons.
    fn form_fields(&self) -> Vec<(&'static str, String)> {
        let mut v = vec![("client_id", self.client_id.clone())];
        if let Some(secret) = &self.client_secret {
            v.push(("client_secret", secret.clone()));
        }
        v
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
    let client = account::blocking_keychain(move || provider.client()).await?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect_uri = format!("http://127.0.0.1:{}", listener.local_addr()?.port());

    let verifier = random_token(); // 64 caractères hex ∈ [43,128]
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = uuid::Uuid::new_v4().to_string();

    let mut params = vec![
        ("client_id", client.client_id.as_str()),
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

    let code = tokio::time::timeout(
        Duration::from_secs(300),
        wait_for_code(&listener, &state, CALLBACK_CONN_TIMEOUT),
    )
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
    form.extend(client.form_fields());
    let resp = token_request(cfg.token_url, &form).await?;
    Ok(OAuthTokens {
        access_token: resp.access_token,
        refresh_token: resp.refresh_token,
        expires_at: now() + resp.expires_in.unwrap_or(3600),
    })
}

/// Temps accordé à chaque connexion sur le port de rappel (lecture comme écriture).
const CALLBACK_CONN_TIMEOUT: Duration = Duration::from_secs(5);

/// Attend la redirection du navigateur et renvoie le `code` (après vérification du `state`).
///
/// Le port de rappel est local mais accessible à tout processus de la machine : une connexion qui
/// n'envoie rien, une requête illisible ou un `state` faux ne doivent jamais bloquer ni faire
/// échouer la connexion. Chacune est écartée et on continue d'attendre la vraie redirection.
async fn wait_for_code(
    listener: &TcpListener,
    expected_state: &str,
    conn_timeout: Duration,
) -> Result<String> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let Ok(Ok(n)) = tokio::time::timeout(conn_timeout, sock.read(&mut buf)).await else {
            continue; // silencieuse ou en erreur
        };
        let req = String::from_utf8_lossy(&buf[..n]);
        let path = req
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("/");
        let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{path}")) else {
            continue; // requête illisible
        };
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();

        // Le `state` conditionne tout (code comme erreur) : sans lui, la requête n'émane pas du navigateur
        // que nous avons ouvert, elle est écartée et n'interrompt rien.
        let state_ok = q.get("state").map(String::as_str) == Some(expected_state);
        let (status, body, outcome) =
            if !state_ok && (q.contains_key("code") || q.contains_key("error")) {
                ("400 Bad Request", "État invalide.", None)
            } else if let Some(err) = q.get("error") {
                (
                    "400 Bad Request",
                    "Autorisation refusée. Vous pouvez fermer cet onglet.",
                    Some(Err(AppError::Auth(err.clone()))),
                )
            } else if let Some(code) = q.get("code") {
                (
                    "200 OK",
                    "Envelope est autorisé. Vous pouvez fermer cet onglet.",
                    Some(Ok(code.clone())),
                )
            } else {
                ("404 Not Found", "", None) // favicon, etc.
            };
        let html = format!("<!doctype html><meta charset=utf-8><body style=\"font-family:sans-serif;padding:3rem\">{body}");
        let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len());
        let _ = tokio::time::timeout(conn_timeout, sock.write_all(response.as_bytes())).await;
        if let Some(o) = outcome {
            return o;
        }
    }
}

/// Client HTTP partagé (pool de connexions + racines TLS chargés une seule fois).
fn http_client() -> Result<&'static reqwest::Client> {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let client = crate::tls::http_builder()?.build()?;
    Ok(CLIENT.get_or_init(|| client))
}

async fn token_request(url: &str, form: &[(&str, String)]) -> Result<TokenResponse> {
    let resp = http_client()?.post(url).form(form).send().await?;
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

/// Jeton révoqué ou expiré (Google : 7 jours tant que l'application est en mode « Test »).
fn explain_refresh_error(e: AppError) -> AppError {
    match e {
        AppError::Auth(m) if m.contains("invalid_grant") => AppError::Auth(
            "autorisation expirée ou révoquée : reconnectez le compte (Google expire les jetons \
             après 7 jours tant que l'application est en mode « Test »)"
                .into(),
        ),
        e => e,
    }
}

/// Jeton d'accès valide pour un compte : rafraîchit et persiste s'il expire dans < 60 s.
pub async fn fresh_access_token(provider: OAuthProvider, account_id: &str) -> Result<String> {
    let _guard = refresh_lock().lock().await; // évite deux rafraîchissements concurrents
    let id = account_id.to_string();
    let tokens = account::blocking_keychain(move || account::load_oauth(&id)).await?;
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
    form.extend(
        account::blocking_keychain(move || provider.client())
            .await?
            .form_fields(),
    );
    let resp = token_request(cfg.token_url, &form)
        .await
        .map_err(explain_refresh_error)?;
    let new = OAuthTokens {
        access_token: resp.access_token,
        refresh_token: resp.refresh_token.or(Some(refresh)), // Google ne renvoie pas toujours le refresh_token
        expires_at: now() + resp.expires_in.unwrap_or(3600),
    };
    let (id, saved) = (account_id.to_string(), new.clone());
    account::blocking_keychain(move || account::store_oauth(&id, &saved)).await?;
    Ok(new.access_token)
}

#[cfg(test)]
mod callback_tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpStream;

    async fn send(port: u16, request: &str) {
        let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        c.write_all(request.as_bytes()).await.unwrap();
        let mut sink = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(2), c.read_to_end(&mut sink)).await;
    }

    /// Un processus local malveillant ou défaillant ne doit ni bloquer ni faire échouer l'autorisation.
    #[tokio::test]
    async fn la_vraie_redirection_aboutit_malgre_les_connexions_parasites() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiter = tokio::spawn(async move {
            wait_for_code(&listener, "bon-etat", Duration::from_millis(300)).await
        });
        let _silent = TcpStream::connect(("127.0.0.1", port)).await.unwrap(); // n'envoie rien
        send(port, "\x00\x01 pas du http").await;
        send(port, "GET /?code=pirate&state=mauvais HTTP/1.1\r\n\r\n").await; // faux state
        send(port, "GET /?error=access_denied HTTP/1.1\r\n\r\n").await; // erreur sans state : ne casse rien
        send(port, "GET /favicon.ico HTTP/1.1\r\n\r\n").await;
        send(
            port,
            "GET /?code=le-vrai-code&state=bon-etat HTTP/1.1\r\n\r\n",
        )
        .await;
        let code = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(code, "le-vrai-code");
    }

    /// Un refus de l'utilisateur chez le fournisseur (avec le bon state) termine bien l'autorisation en erreur.
    #[tokio::test]
    async fn un_refus_legitime_termine_en_erreur() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiter = tokio::spawn(async move {
            wait_for_code(&listener, "bon-etat", Duration::from_millis(300)).await
        });
        send(
            port,
            "GET /?error=access_denied&state=bon-etat HTTP/1.1\r\n\r\n",
        )
        .await;
        let result = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_err());
    }
}

//! OAuth2 (Authorization Code + PKCE S256, redirection loopback) pour Gmail et Outlook.
//!
//! Les identifiants d'application ne sont pas fournis avec le code : chaque utilisateur enregistre
//! une application « bureau » chez le fournisseur et la saisit dans l'interface (stockée dans le
//! trousseau de l'OS). Repli pour le développement : variables d'environnement
//!   `ENVELOPE_GOOGLE_CLIENT_ID` (+ `ENVELOPE_GOOGLE_CLIENT_SECRET`) et `ENVELOPE_MICROSOFT_CLIENT_ID`.

use std::borrow::Cow;
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
    auth_url: Cow<'static, str>,
    token_url: Cow<'static, str>,
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
                auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
                token_url: "https://oauth2.googleapis.com/token".into(),
                scope: "https://mail.google.com/ openid email",
                extra_auth_params: &[("access_type", "offline"), ("prompt", "consent")],
                client_id_env: "ENVELOPE_GOOGLE_CLIENT_ID",
                client_secret_env: Some("ENVELOPE_GOOGLE_CLIENT_SECRET"),
            },
            Self::Microsoft => ProviderConfig {
                auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".into(),
                token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token".into(),
                scope: "https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send offline_access openid email profile",
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
            AppError::Auth(msg!(
                "oauth.client_not_configured",
                "{provider} client ID not configured (Accounts tab)",
                provider = self.label()
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
    id_token: Option<String>,
}

/// Adresse du compte autorisé d'après l'`id_token` (OIDC) : reçu directement du point de terminaison de jetons
/// par TLS, sa signature n'a pas à être vérifiée (OIDC Core §3.1.3.7) et on n'en tire qu'une comparaison, jamais
/// une autorisation. `None` si le fournisseur n'en renvoie pas.
fn id_token_email(id_token: &str) -> Option<String> {
    let payload = URL_SAFE_NO_PAD.decode(id_token.split('.').nth(1)?).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    ["email", "preferred_username"]
        .iter()
        .find_map(|k| claims.get(k)?.as_str())
        .filter(|e| e.contains('@'))
        .map(str::to_owned)
}

/// Refuse un compte autorisé dans le navigateur qui n'est pas celui saisi (sans effet si le fournisseur ne dit rien).
pub fn check_authorized_email(authorized: Option<&str>, expected: &str) -> Result<()> {
    match authorized {
        Some(a) if !a.eq_ignore_ascii_case(expected) => Err(AppError::Auth(msg!("oauth.wrong_account", "the account authorized in the browser is {authorized}, not {expected}: try again and choose the right account", authorized = a, expected = expected))),
        _ => Ok(()),
    }
}

/// Résultat d'une autorisation : jetons et, si le fournisseur l'indique, l'adresse du compte autorisé.
pub struct Authorized {
    pub tokens: OAuthTokens,
    pub email: Option<String>,
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
) -> Result<Authorized> {
    let client = account::blocking_keychain(move || provider.client()).await?;
    authorize_with(&provider.config(), &client, open_browser).await
}

/// Le flux lui-même, indépendant du fournisseur (les points de terminaison viennent de `cfg`).
async fn authorize_with(
    cfg: &ProviderConfig,
    client: &ClientCreds,
    open_browser: impl FnOnce(&str) -> Result<()>,
) -> Result<Authorized> {
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
    let url = reqwest::Url::parse_with_params(&cfg.auth_url, &params)
        .map_err(|e| AppError::Auth((e.to_string()).into()))?;
    open_browser(url.as_str())?;

    let code = tokio::time::timeout(
        Duration::from_secs(300),
        wait_for_code(&listener, &state, CALLBACK_CONN_TIMEOUT),
    )
    .await
    .map_err(|_| {
        AppError::Auth(msg!(
            "oauth.timeout",
            "timed out: authorization not completed in the browser"
        ))
    })??;

    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", verifier),
    ];
    form.extend(client.form_fields());
    let resp = token_request(&cfg.token_url, &form).await?;
    Ok(Authorized {
        email: resp.id_token.as_deref().and_then(id_token_email),
        tokens: OAuthTokens {
            access_token: resp.access_token,
            refresh_token: resp.refresh_token,
            expires_at: now() + resp.expires_in.unwrap_or(3600),
        },
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
                    Some(Err(AppError::Auth((err.clone()).into()))),
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
        return Err(AppError::Auth(msg!(
            "oauth.token_refused",
            "token exchange refused ({status}): {body}",
            status = status,
            body = body
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
        AppError::Auth(m) if m.text.contains("invalid_grant") => AppError::Auth(msg!("oauth.expired", "authorization expired or revoked: reconnect the account (Google expires tokens after 7 days while the app is in “Testing” mode)")),
        e => e,
    }
}

/// Corps de la demande de rafraîchissement. Pas de `scope` : il vaut par défaut celui accordé à l'origine
/// (RFC 6749 §6) ; en renvoyer un plus large que celui d'un compte connecté avant l'ajout d'`openid` ferait
/// refuser le rafraîchissement (`invalid_scope`).
fn refresh_form(refresh_token: &str, client: &ClientCreds) -> Vec<(&'static str, String)> {
    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.to_string()),
    ];
    form.extend(client.form_fields());
    form
}

/// Jeton d'accès valide pour un compte : rafraîchit et persiste s'il expire dans < 60 s.
pub async fn fresh_access_token(provider: OAuthProvider, account_id: &str) -> Result<String> {
    let _guard = refresh_lock().lock().await; // évite deux rafraîchissements concurrents
    let id = account_id.to_string();
    let tokens = account::blocking_keychain(move || account::load_oauth(&id)).await?;
    if tokens.expires_at - 60 > now() {
        return Ok(tokens.access_token);
    }
    let refresh = tokens.refresh_token.ok_or_else(|| {
        AppError::Auth(msg!(
            "oauth.no_refresh_token",
            "no refresh_token: reconnect the account"
        ))
    })?;
    let client = account::blocking_keychain(move || provider.client()).await?;
    let resp = token_request(
        &provider.config().token_url,
        &refresh_form(&refresh, &client),
    )
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
mod identity_tests {
    use super::*;

    fn token(claims: &str) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims))
    }

    #[test]
    fn adresse_lue_dans_l_id_token() {
        assert_eq!(
            id_token_email(&token(r#"{"email":"a@b.fr"}"#)).as_deref(),
            Some("a@b.fr")
        );
        assert_eq!(
            id_token_email(&token(r#"{"preferred_username":"c@d.fr"}"#)).as_deref(),
            Some("c@d.fr")
        );
        assert_eq!(
            id_token_email(&token(r#"{"preferred_username":"pas-une-adresse"}"#)),
            None
        );
        assert_eq!(id_token_email("illisible"), None);
    }

    #[test]
    fn mauvais_compte_refuse_absence_tolérée() {
        assert!(check_authorized_email(Some("A@B.fr"), "a@b.fr").is_ok());
        assert!(check_authorized_email(None, "a@b.fr").is_ok());
        assert!(check_authorized_email(Some("x@y.fr"), "a@b.fr").is_err());
    }
}

/// Flux complet contre un faux fournisseur local : navigateur simulé (ouvre l'URL d'autorisation puis suit la redirection)
/// et point de terminaison de jetons qui renvoie un `id_token`.
#[cfg(test)]
mod flow_tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpStream;

    /// Sert une seule réponse de jetons et renvoie le corps de la requête reçue.
    async fn fake_token_endpoint(id_token: String) -> (u16, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut data = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = sock.read(&mut buf).await.unwrap();
                data.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&data).into_owned();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if body.len() >= len {
                        let json = format!(
                            r#"{{"access_token":"at","refresh_token":"rt","expires_in":3600,"id_token":"{id_token}"}}"#
                        );
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{json}",
                            json.len()
                        );
                        sock.write_all(resp.as_bytes()).await.unwrap();
                        return body.to_string();
                    }
                }
            }
        });
        (port, handle)
    }

    async fn run_flow(claims: &str) -> (Authorized, String, String) {
        let id_token = format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims));
        let (port, token_request_body) = fake_token_endpoint(id_token).await;
        let cfg = ProviderConfig {
            auth_url: "http://127.0.0.1:1/auth".into(),
            token_url: format!("http://127.0.0.1:{port}/token").into(),
            scope: "mail openid email",
            extra_auth_params: &[],
            client_id_env: "",
            client_secret_env: None,
        };
        let client = ClientCreds {
            client_id: "mon-client".into(),
            client_secret: None,
        };
        let seen_url = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let seen = seen_url.clone();
        let auth = authorize_with(&cfg, &client, move |url| {
            *seen.lock().unwrap() = url.to_string();
            let url = reqwest::Url::parse(url).unwrap();
            let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
            let redirect = reqwest::Url::parse(&q["redirect_uri"]).unwrap();
            let (redirect_port, state) = (redirect.port().unwrap(), q["state"].clone());
            tokio::spawn(async move {
                let mut c = TcpStream::connect(("127.0.0.1", redirect_port))
                    .await
                    .unwrap();
                let req = format!("GET /?code=le-code&state={state} HTTP/1.1\r\n\r\n");
                c.write_all(req.as_bytes()).await.unwrap();
                let mut sink = Vec::new();
                let _ = c.read_to_end(&mut sink).await;
            });
            Ok(())
        })
        .await
        .unwrap();
        let url = seen_url.lock().unwrap().clone();
        (auth, url, token_request_body.await.unwrap())
    }

    #[tokio::test]
    async fn le_flux_complet_renvoie_jetons_et_adresse_autorisee() {
        let (auth, url, token_body) = run_flow(r#"{"email":"moi@exemple.fr"}"#).await;
        assert_eq!(auth.tokens.access_token, "at");
        assert_eq!(auth.tokens.refresh_token.as_deref(), Some("rt"));
        assert_eq!(auth.email.as_deref(), Some("moi@exemple.fr"));
        assert!(check_authorized_email(auth.email.as_deref(), "MOI@exemple.fr").is_ok());
        assert!(check_authorized_email(auth.email.as_deref(), "autre@exemple.fr").is_err());
        assert!(
            url.contains("code_challenge_method=S256") && url.contains("scope=mail+openid+email")
        );
        assert!(
            token_body.contains("code=le-code")
                && token_body.contains("code_verifier=")
                && token_body.contains("client_id=mon-client")
        );
    }

    #[test]
    fn le_rafraichissement_n_envoie_pas_de_scope() {
        let client = ClientCreds {
            client_id: "c".into(),
            client_secret: Some("s".into()),
        };
        let form = refresh_form("rt", &client);
        let keys: Vec<_> = form.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            keys,
            ["grant_type", "refresh_token", "client_id", "client_secret"]
        );
    }
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

//! Connexion IMAP : TCP → TLS (rustls) → login (mot de passe) ou AUTHENTICATE XOAUTH2.
//! Seul le TLS implicite (port 993) est supporté pour IMAP ; STARTTLS IMAP est refusé explicitement.

use std::fmt::Debug;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use crate::error::{AppError, Result};
use crate::mail::account::{Account, Credential, Security};

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send + Debug {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Debug> Io for T {}

pub type ImapSession = async_imap::Session<Box<dyn Io>>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

fn tls_config() -> Arc<ClientConfig> {
    static CFG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CFG.get_or_init(|| {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        Arc::new(
            ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    })
    .clone()
}

/// SASL XOAUTH2. Si le serveur répond par un défi (= erreur JSON), on renvoie une ligne vide.
struct XOAuth2 {
    payload: String,
    sent: bool,
}
impl async_imap::Authenticator for XOAuth2 {
    type Response = String;
    fn process(&mut self, _challenge: &[u8]) -> String {
        if self.sent {
            String::new()
        } else {
            self.sent = true;
            self.payload.clone()
        }
    }
}

pub async fn connect(account: &Account, cred: &Credential) -> Result<ImapSession> {
    if Security::parse(&account.imap_security) != Security::Tls {
        return Err(AppError::Invalid(
            "IMAP : seul le TLS implicite (port 993) est supporté".into(),
        ));
    }
    let host = account.imap_host.as_str();
    let tcp = tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((host, account.imap_port as u16)),
    )
    .await
    .map_err(|_| AppError::Invalid(format!("délai de connexion dépassé : {host}")))??;
    tcp.set_nodelay(true)?;
    let name = ServerName::try_from(host.to_string())
        .map_err(|e| AppError::Invalid(format!("hôte invalide : {e}")))?;
    let tls = TlsConnector::from(tls_config()).connect(name, tcp).await?;

    let mut client = async_imap::Client::new(Box::new(tls) as Box<dyn Io>);
    let _greeting = client.read_response().await?; // bannière « * OK ... »

    let session = match cred {
        Credential::Password { user, password } => client.login(user, password).await,
        Credential::OAuth { user, access_token } => {
            let payload = format!("user={user}\x01auth=Bearer {access_token}\x01\x01");
            client
                .authenticate(
                    "XOAUTH2",
                    XOAuth2 {
                        payload,
                        sent: false,
                    },
                )
                .await
        }
    };
    session.map_err(|(e, _client)| AppError::Auth(format!("connexion IMAP refusée : {e}")))
}

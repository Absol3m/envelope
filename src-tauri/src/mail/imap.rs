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

use async_imap::types::NameAttribute;
use futures::TryStreamExt;

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

/// Dernier segment d'un chemin de dossier (« INBOX.Sent » → « Sent »).
pub fn leaf<'a>(path: &'a str, delimiter: Option<&str>) -> &'a str {
    match delimiter {
        Some(d) if !d.is_empty() => path.rsplit(d).next().unwrap_or(path),
        _ => path,
    }
}

/// Rôle d'un dossier : attribut SPECIAL-USE du serveur, à défaut son nom (« Sent », « Envoyés »…).
pub fn folder_role(
    path: &str,
    delimiter: Option<&str>,
    attrs: &[NameAttribute],
) -> Option<&'static str> {
    if path.eq_ignore_ascii_case("INBOX") {
        return Some("inbox");
    }
    let by_attr = attrs.iter().find_map(|a| match a {
        NameAttribute::Sent => Some("sent"),
        NameAttribute::Drafts => Some("drafts"),
        NameAttribute::Trash => Some("trash"),
        NameAttribute::Junk => Some("junk"),
        NameAttribute::Archive => Some("archive"),
        NameAttribute::All => Some("all"),
        NameAttribute::Flagged => Some("flagged"),
        _ => None,
    });
    by_attr.or_else(|| match leaf(path, delimiter).to_lowercase().as_str() {
        "sent" | "sent items" | "sent messages" | "envoyés" | "envoyes" | "éléments envoyés" => {
            Some("sent")
        }
        "drafts" | "draft" | "brouillons" => Some("drafts"),
        "trash" | "deleted items" | "deleted messages" | "corbeille" | "éléments supprimés" => {
            Some("trash")
        }
        "junk" | "spam" | "junk e-mail" | "courrier indésirable" | "indésirables" => Some("junk"),
        "archive" | "archives" => Some("archive"),
        _ => None,
    })
}

/// Chemin du dossier « Envoyés » sur le serveur ; il est créé (et abonné) s'il n'existe pas encore,
/// ce qui arrive sur une boîte neuve.
pub async fn find_or_create_sent(session: &mut ImapSession) -> Result<String> {
    let names: Vec<_> = session
        .list(Some(""), Some("*"))
        .await?
        .try_collect()
        .await?;
    let found = names.iter().find(|n| {
        !n.attributes()
            .iter()
            .any(|a| matches!(a, NameAttribute::NoSelect))
            && folder_role(n.name(), n.delimiter(), n.attributes()) == Some("sent")
    });
    if let Some(n) = found {
        return Ok(n.name().to_string());
    }
    session.create("Sent").await?;
    let _ = session.subscribe("Sent").await; // non bloquant : certains serveurs l'abonnent d'office
    Ok("Sent".to_string())
}

/// Dépose `raw` (message déjà envoyé) dans le dossier « Envoyés », marqué lu.
/// `known` : chemin déjà connu de la synchro (évite un LIST) ; à défaut on le cherche ou on le crée.
pub async fn append_sent(session: &mut ImapSession, known: Option<&str>, raw: &[u8]) -> Result<()> {
    let path = match known {
        Some(p) => p.to_string(),
        None => find_or_create_sent(session).await?,
    };
    session.append(&path, Some("(\\Seen)"), None, raw).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_from_attribute_or_name() {
        assert_eq!(folder_role("inbox", None, &[]), Some("inbox"));
        assert_eq!(
            folder_role("Whatever", Some("/"), &[NameAttribute::Sent]),
            Some("sent")
        );
        assert_eq!(
            folder_role("INBOX.Éléments envoyés", Some("."), &[]),
            Some("sent")
        );
        assert_eq!(folder_role("Sent", Some("/"), &[]), Some("sent"));
        assert_eq!(folder_role("Brouillons", None, &[]), Some("drafts"));
        assert_eq!(folder_role("Projets/Sent-2024", Some("/"), &[]), None);
    }
}

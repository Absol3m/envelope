//! Connexion IMAP : TCP → TLS (rustls) → login (mot de passe) ou AUTHENTICATE XOAUTH2.
//! TLS implicite (port 993) ou STARTTLS (port 143, RFC 3501 §6.2.1). STARTTLS est **obligatoire**
//! une fois choisi : s'il est refusé, la connexion échoue (jamais de repli en clair, qui exposerait
//! le mot de passe). Une connexion sans chiffrement est refusée.

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
    connect_with(account, cred, tls_config()).await
}

/// `connect` avec une configuration TLS donnée (les tests y mettent leur propre autorité).
pub(crate) async fn connect_with(
    account: &Account,
    cred: &Credential,
    tls: Arc<ClientConfig>,
) -> Result<ImapSession> {
    let security = Security::parse(&account.imap_security);
    if security == Security::None {
        return Err(AppError::Invalid(
            "IMAP : connexion non chiffrée refusée (choisissez TLS ou STARTTLS)".into(),
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
    let connector = TlsConnector::from(tls);
    let stream: Box<dyn Io> = match security {
        Security::Starttls => {
            // Échange en clair limité à la bannière et à STARTTLS ; le mot de passe ne part qu'après la mise en TLS.
            let mut plain = async_imap::Client::new(tcp);
            let _greeting = plain.read_response().await?; // bannière « * OK ... »
            plain
                .run_command_and_check_ok("STARTTLS", None)
                .await
                .map_err(|e| AppError::Auth(format!("le serveur refuse STARTTLS : {e}")))?;
            Box::new(connector.connect(name, plain.into_inner()).await?)
        }
        _ => Box::new(connector.connect(name, tcp).await?),
    };

    let mut client = async_imap::Client::new(stream);
    if security == Security::Tls {
        let _greeting = client.read_response().await?; // bannière « * OK ... »
    } // après STARTTLS, le serveur n'envoie pas de nouvelle bannière

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

/// Chemin du dossier de ce rôle sur le serveur ; il est créé sous `default_name` (et abonné) s'il
/// n'existe pas encore, ce qui arrive sur une boîte neuve (ni « Envoyés » ni « Corbeille »).
pub async fn find_or_create(
    session: &mut ImapSession,
    role: &str,
    default_name: &str,
) -> Result<String> {
    let names: Vec<_> = session
        .list(Some(""), Some("*"))
        .await?
        .try_collect()
        .await?;
    let found = names.iter().find(|n| {
        !n.attributes()
            .iter()
            .any(|a| matches!(a, NameAttribute::NoSelect))
            && folder_role(n.name(), n.delimiter(), n.attributes()) == Some(role)
    });
    if let Some(n) = found {
        return Ok(n.name().to_string());
    }
    session.create(default_name).await?;
    let _ = session.subscribe(default_name).await; // non bloquant : certains serveurs l'abonnent d'office
    Ok(default_name.to_string())
}

/// Dépose `raw` (message déjà envoyé) dans le dossier « Envoyés », marqué lu.
/// `known` : chemin déjà connu de la synchro (évite un LIST) ; à défaut on le cherche ou on le crée.
pub async fn append_sent(session: &mut ImapSession, known: Option<&str>, raw: &[u8]) -> Result<()> {
    let path = match known {
        Some(p) => p.to_string(),
        None => find_or_create(session, "sent", "Sent").await?,
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

#[cfg(test)]
mod starttls_tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use tokio_rustls::rustls::ServerConfig;
    use tokio_rustls::TlsAcceptor;

    fn account(port: u16, security: &str) -> Account {
        Account {
            id: "a".into(),
            email: "me@x.fr".into(),
            display_name: None,
            provider: "imap".into(),
            auth_kind: "password".into(),
            username: "me".into(),
            imap_host: "localhost".into(),
            imap_port: i64::from(port),
            imap_security: security.into(),
            smtp_host: "s".into(),
            smtp_port: 465,
            smtp_security: "tls".into(),
            last_sync_at: None,
            sync_status: "idle".into(),
            sync_error: None,
            signature: String::new(),
        }
    }

    fn cred() -> Credential {
        Credential::Password {
            user: "me".into(),
            password: "secret".into(),
        }
    }

    /// Autorité de test : certificat auto-signé pour `localhost`, côté serveur et côté client.
    fn tls_pair() -> (TlsAcceptor, Arc<ClientConfig>) {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let ck = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert: CertificateDer<'static> = ck.cert.der().clone();
        let key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(ck.signing_key.serialize_der()));
        let server = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert.clone()], key)
            .unwrap();
        let mut roots = RootCertStore::empty();
        roots.add(cert).unwrap();
        let client = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        (TlsAcceptor::from(Arc::new(server)), Arc::new(client))
    }

    /// Serveur IMAP minimal : bannière, STARTTLS (accepté ou refusé), puis LOGIN. Retourne ce qu'il a lu.
    async fn mock_server(
        accept_starttls: bool,
        acceptor: TlsAcceptor,
    ) -> (u16, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut seen = Vec::new();
            let mut reader = BufReader::new(tcp);
            reader
                .get_mut()
                .write_all(b"* OK IMAP4rev1 ready\r\n")
                .await
                .unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            seen.push(format!("clair: {}", line.trim()));
            let tag = line.split_whitespace().next().unwrap_or("A").to_string();
            if !accept_starttls {
                reader
                    .get_mut()
                    .write_all(format!("{tag} NO STARTTLS non disponible\r\n").as_bytes())
                    .await
                    .unwrap();
                // Si le client insistait en clair, on le verrait ici.
                let mut rest = String::new();
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(300),
                    reader.read_line(&mut rest),
                )
                .await;
                if !rest.is_empty() {
                    seen.push(format!("clair: {}", rest.trim()));
                }
                return seen;
            }
            reader
                .get_mut()
                .write_all(format!("{tag} OK Begin TLS negotiation now\r\n").as_bytes())
                .await
                .unwrap();
            let tls = acceptor.accept(reader.into_inner()).await.unwrap();
            let mut reader = BufReader::new(tls);
            let mut login = String::new();
            reader.read_line(&mut login).await.unwrap();
            seen.push(format!("tls: {}", login.trim()));
            let tag = login.split_whitespace().next().unwrap_or("A").to_string();
            reader
                .get_mut()
                .write_all(format!("{tag} OK LOGIN completed\r\n").as_bytes())
                .await
                .unwrap();
            seen
        });
        (port, task)
    }

    #[tokio::test]
    async fn starttls_negocie_puis_authentifie_sous_tls() {
        let (acceptor, client_cfg) = tls_pair();
        let (port, server) = mock_server(true, acceptor).await;
        connect_with(&account(port, "starttls"), &cred(), client_cfg)
            .await
            .expect("connexion STARTTLS");
        let seen = server.await.unwrap();
        // Seul STARTTLS circule en clair ; le mot de passe n'est envoyé qu'une fois la session chiffrée.
        assert!(
            seen[0].starts_with("clair:") && seen[0].contains("STARTTLS"),
            "{seen:?}"
        );
        assert!(
            seen[1].starts_with("tls:") && seen[1].contains("LOGIN"),
            "{seen:?}"
        );
        assert!(!seen[0].contains("secret"));
    }

    #[tokio::test]
    async fn starttls_refuse_pas_de_repli_en_clair() {
        let (acceptor, client_cfg) = tls_pair();
        let (port, server) = mock_server(false, acceptor).await;
        let err = connect_with(&account(port, "starttls"), &cred(), client_cfg)
            .await
            .expect_err("doit échouer")
            .to_string();
        assert!(err.contains("STARTTLS"), "{err}");
        let seen = server.await.unwrap();
        assert_eq!(
            seen.len(),
            1,
            "le client n'a pas envoyé son mot de passe en clair : {seen:?}"
        );
    }

    #[tokio::test]
    async fn sans_chiffrement_refuse_sans_se_connecter() {
        let (_, client_cfg) = tls_pair();
        // Aucun serveur n'écoute : l'erreur vient de la politique, pas du réseau.
        let err = connect_with(&account(1, "none"), &cred(), client_cfg)
            .await
            .expect_err("doit échouer")
            .to_string();
        assert!(err.contains("non chiffrée"), "{err}");
    }
}
